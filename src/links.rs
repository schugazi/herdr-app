//! Links in terminal text: Ghostty's URL and file path pattern, what a match
//! points to, and handing it to the system to open.

use fancy_regex::Regex;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Ghostty's default link pattern (`src/config/url.zig`), except that a scheme
/// never starts inside a word or path (`Makefile:12`, `src/build-file:12` and
/// `src/file:12` aren't the URL `file:12`).
static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    let schemes = r"(?<![\w\-/])(?:https?://|mailto:|ftp://|file:|ssh:|git://|ssh://|tel:|magnet:|ipfs://|ipns://|gemini://|gopher://|news:)";
    let ipv6 = r"(?:\[[:0-9a-fA-F]+(?:[:0-9a-fA-F]*)+\](?::[0-9]+)?)";
    let scheme_chars = r"[\w\-.~:/?#@!$&*+,;=%]";
    let path_chars = r"[\w\-.~:\/?#@!$&*+;=%]";
    let bracketed = r"(?:[\(\[]\w*[\)\]])?";
    let no_trailing_punctuation = r"(?<![,.])";
    let no_trailing_colon = r"(?<!:)";
    let dotted = r"(?=[\w\-.~:\/?#@!$&*+;=%]*\.)";
    let undotted = r"(?![\w\-.~:\/?#@!$&*+;=%]*\.)";
    let dotted_spaces = r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]*[\/.])*";
    let any_spaces = r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]+)*";
    let rooted = r"(?:\.\.\/|\.\/|(?<!\w)~\/|(?:[\w][\w\-.]*\/)*(?<!\w)\$[A-Za-z_]\w*\/|\.[\w][\w\-.]*\/|(?<![\w~\/])\/(?!\/))";
    let scheme_url = format!("(?:{schemes})(?:{ipv6}|{scheme_chars}+{bracketed})+{no_trailing_punctuation}");
    let rooted_path = format!(
        "{rooted}(?:{dotted}{path_chars}+{dotted_spaces}{no_trailing_colon}|{undotted}{path_chars}+{any_spaces}{no_trailing_colon})"
    );
    let bare_path = format!(r"{dotted}(?<!\$\d*)(?<!\w)[\w][\w\-.]*\/{path_chars}+{no_trailing_colon}");
    Regex::new(&format!("{scheme_url}|{rooted_path}|{bare_path}")).expect("link pattern")
});

/// Names Ghostty's pattern misses, which only a terminal on another machine
/// links, since only a lookup there can tell whether they're files: a file name
/// with an extension (`README.md`, `main.rs:12`) and a path with a line but no
/// extension (`src/Makefile:12`). Not a bare name with a line alone, which
/// reads like `localhost:8080` or `12:30`.
static BARE: LazyLock<Regex> = LazyLock::new(|| {
    let line = r"(?::\d+(?::\d+)?)";
    let named = format!(r"[\w][\w\-]*(?:\.[\w\-]+)*\.[A-Za-z][A-Za-z0-9]{{0,9}}{line}?");
    let slashed = format!(r"[\w][\w\-.]*(?:/[\w\-.]+)+{line}");
    Regex::new(&format!(r"(?<![\w.~/$@:\-])(?:{named}|{slashed})(?![\w/@\-])")).expect("bare file pattern")
});

/// The link in `text` that covers byte `at`.
pub fn find(text: &str, at: usize) -> Option<Range<usize>> {
    find_in(&PATTERN, text, at)
}

fn find_in(pattern: &Regex, text: &str, at: usize) -> Option<Range<usize>> {
    for m in pattern.find_iter(text) {
        let m = m.ok()?;
        if m.start() > at {
            return None;
        }
        if at < m.end() {
            return Some(m.range());
        }
    }
    None
}

/// The address-shaped word under the pointer, excluding surrounding punctuation.
pub fn word(text: &str, at: usize) -> Option<Range<usize>> {
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.');
    if !text.get(at..)?.chars().next().is_some_and(allowed) {
        return None;
    }
    let start = text[..at].char_indices().rev().find(|(_, c)| !allowed(*c)).map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[at..].char_indices().find(|(_, c)| !allowed(*c)).map_or(text.len(), |(i, _)| at + i);
    let trimmed = text[start..end].trim_matches('.');
    let start = start + text[start..end].len() - text[start..end].trim_start_matches('.').len();
    let end = start + trimmed.len();
    (start <= at && at < end).then_some(start..end)
}

/// Bare addresses refer to the hovered terminal's machine, never its names.
pub fn terminal_address<'a>(word: &str, source: &str, live: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let address = if word.contains('/') {
        word.to_owned()
    } else {
        crate::mirror::qualify(word, source.split_once('/').map(|(machine, _)| machine))
    };
    live.into_iter().any(|a| a == address).then_some(address)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Url(String),
    File(PathBuf),
}

fn has_scheme(link: &str) -> bool {
    link.split_once(':').is_some_and(|(s, _)| {
        s.len() > 1 && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
}

/// What a link in a terminal on another machine points to.
#[derive(Clone, Debug, PartialEq)]
pub enum Remote {
    /// Opened here, as any URL.
    Url(String),
    /// A path on that machine as the terminal showed it (relative, `~/`, with
    /// `:line`), or a `file:` URL's path decoded: unchecked, since only that
    /// machine can tell whether it's there.
    Path(String),
}

/// The link at byte `at` in the text of a terminal on another machine, and
/// what it points to: Ghostty's pattern (URLs, and paths it knows), else a
/// name only a lookup there can find (`BARE`, always a path: `README.md:12`
/// reads like a scheme but isn't one). A `BARE` match holding Ghostty's is the
/// whole token, so every part of `src/build-file:12` names the same path.
/// `hosts`: what a `file:` URL may call that machine (`resolve_remote`).
pub fn find_remote(text: &str, at: usize, hosts: &[&str]) -> Option<(Range<usize>, Remote)> {
    let bare = find_in(&BARE, text, at);
    match find(text, at) {
        Some(g) if !bare.as_ref().is_some_and(|b| b.start <= g.start && g.end <= b.end && *b != g) => {
            let link = resolve_remote(&text[g.clone()], hosts);
            Some((g, link))
        }
        _ => bare.map(|b| (b.clone(), Remote::Path(text[b].to_owned()))),
    }
}

/// A link of a terminal on another machine (Ghostty's match, or an OSC 8
/// target): a `file:` URL is a path there when its authority is none,
/// `localhost` or one of `hosts`, the names that machine goes by (`ls
/// --hyperlink` writes its host name); any other URL, a file URL naming
/// another host too, stays a URL. Anything without a scheme is a path.
pub fn resolve_remote(link: &str, hosts: &[&str]) -> Remote {
    if !has_scheme(link) {
        return Remote::Path(link.to_owned());
    }
    match file_url(link) {
        Some((host, path)) if host.is_empty() || host.eq_ignore_ascii_case("localhost") || names_host(host, hosts) => Remote::Path(path),
        _ => Remote::Url(link.to_owned()),
    }
}

/// `authority` is one of `hosts` exactly, or the short name of one given in
/// full (`build` for `build.lan`); never the reverse, so a short `build`
/// doesn't claim `build.other.example`.
fn names_host(authority: &str, hosts: &[&str]) -> bool {
    hosts.iter().any(|h| {
        authority.eq_ignore_ascii_case(h) || h.split_once('.').is_some_and(|(short, _)| !short.is_empty() && authority.eq_ignore_ascii_case(short))
    })
}

/// What a link opens. File paths count only when they exist here; relative
/// ones need the terminal's folder.
pub fn resolve(link: &str, cwd: Option<&Path>) -> Option<Target> {
    if has_scheme(link) {
        // Windows can't open a file URL whose path is percent-encoded, the way programs write them.
        if let Some(path) = file_url_path(link).filter(|p| p.exists()) {
            return Some(Target::File(path));
        }
        return Some(Target::Url(link.to_owned()));
    }
    let home = std::env::home_dir();
    let expanded = if let Some(rest) = link.strip_prefix("~/") {
        home?.join(rest)
    } else if let Some(var) = link.strip_prefix('$') {
        let (name, rest) = var.split_once('/').unwrap_or((var, ""));
        let base = match name {
            "PWD" => cwd?.to_path_buf(),
            "HOME" => home?,
            _ => PathBuf::from(std::env::var_os(name)?),
        };
        base.join(rest)
    } else {
        PathBuf::from(link)
    };
    let full = if expanded.is_absolute() { expanded } else { cwd?.join(expanded) };
    // Windows ignores a trailing period when checking whether a path exists.
    if !(cfg!(windows) && full.extension().is_some_and(|ext| ext.is_empty())) && full.exists() {
        return Some(Target::File(full));
    }
    // The period ending a sentence, or `:42` / `:42:7` naming a place in the file.
    let mut bare = full.to_str()?.strip_suffix('.').unwrap_or(full.to_str()?);
    for _ in 0..2 {
        match bare.rsplit_once(':') {
            Some((head, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => bare = head,
            _ => break,
        }
    }
    Path::new(bare).exists().then(|| Target::File(PathBuf::from(bare)))
}

/// The path a `file:` URL names on this computer, percent-decoded.
fn file_url_path(link: &str) -> Option<PathBuf> {
    let (host, path) = file_url(link)?;
    // Another host's file.
    if !host.is_empty() && host != "localhost" {
        return None;
    }
    // `/C:/Users/...` names `C:\Users\...`.
    if cfg!(windows) && path.as_bytes().get(2) == Some(&b':') && path.as_bytes()[1].is_ascii_alphabetic() {
        return Some(PathBuf::from(path[1..].replace('/', "\\")));
    }
    Some(PathBuf::from(path))
}

/// A `file:` URL's host (empty when none) and path as written (`/home/me/a b.txt`), percent-decoded.
fn file_url(link: &str) -> Option<(&str, String)> {
    let rest = link.strip_prefix("file://")?;
    // No path.
    let (host, rest) = rest.split_at(rest.find('/')?);
    let rest = rest.split(['?', '#']).next()?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut i = 0;
    while i < rest.len() {
        if rest.as_bytes()[i] == b'%' {
            bytes.push(u8::from_str_radix(rest.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            bytes.push(rest.as_bytes()[i]);
            i += 1;
        }
    }
    Some((host, String::from_utf8(bytes).ok()?))
}

/// Hands the target to the system's default handler.
pub fn open(target: &Target) {
    let arg = match target {
        Target::Url(u) => std::ffi::OsStr::new(u),
        Target::File(p) => p.as_os_str(),
    };
    open_system(arg);
}

#[cfg(windows)]
fn open_system(arg: &std::ffi::OsStr) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };

    let mut wide: Vec<u16> = arg.encode_wide().collect();
    if wide.contains(&0) {
        tracing::warn!("cannot open a link containing a NUL");
        return;
    }
    wide.push(0);
    // ShellExecuteW can pump messages; never reenter GPUI while it handles a click.
    std::thread::spawn(move || {
        let initialized = unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) };
        // Explorer interprets '=' as its own arguments; the handler gets a literal target.
        let result = unsafe {
            ShellExecuteW(std::ptr::null_mut(), std::ptr::null(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
        } as isize;
        if result <= 32 {
            tracing::warn!("cannot open link: ShellExecuteW returned {result}");
        }
        if initialized >= 0 {
            unsafe { CoUninitialize() };
        }
    });
}

#[cfg(not(windows))]
fn open_system(arg: &std::ffi::OsStr) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let spawned = std::process::Command::new(opener)
        .arg(arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        // Reaped off the UI thread so it never lingers as a zombie.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("cannot run {opener}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(text: &str) -> Option<&str> {
        PATTERN.find(text).unwrap().map(|m| m.as_str())
    }

    /// Ghostty's own cases (`src/config/url.zig`).
    #[test]
    fn matches_like_ghostty() {
        let cases = [
            ("hello https://example.com world", "https://example.com"),
            ("https://example.com/foo(bar) more", "https://example.com/foo(bar)"),
            ("https://example.com/foo(bar)baz more", "https://example.com/foo(bar)baz"),
            ("Link inside (https://example.com) parens", "https://example.com"),
            ("Link period https://example.com. More text.", "https://example.com"),
            ("Link trailing comma https://example.com, more text.", "https://example.com"),
            ("Link in double quotes \"https://example.com\" and more", "https://example.com"),
            ("Link in single quotes 'https://example.com' and more", "https://example.com"),
            ("some file with https://google.com https://duckduckgo.com links.", "https://google.com"),
            ("and links in it. links https://yahoo.com mailto:test@example.com ssh://1.2.3.4", "https://yahoo.com"),
            ("also match http://example.com non-secure links", "http://example.com"),
            ("match tel://+12123456789 phone numbers", "tel://+12123456789"),
            ("match with query url https://example.com?query=1&other=2 and more text.", "https://example.com?query=1&other=2"),
            (
                "url with dashes [mode 2027](https://github.com/contour-terminal/terminal-unicode-core) for better unicode support",
                "https://github.com/contour-terminal/terminal-unicode-core",
            ),
            ("dot.http://example.com", "http://example.com"),
            ("weird characters https://example.com/~user/?query=1&other=2#hash and more", "https://example.com/~user/?query=1&other=2#hash"),
            ("square brackets https://example.com/[foo] and more", "https://example.com/[foo]"),
            (
                "[13]:TooManyStatements: TempFile#assign_temp_file_to_entity has approx 7 statements [https://example.com/docs/Too-Many-Statements.md]",
                "https://example.com/docs/Too-Many-Statements.md",
            ),
            ("match ftp://example.com ftp links", "ftp://example.com"),
            ("match file://example.com file links", "file://example.com"),
            ("match ssh://example.com ssh links", "ssh://example.com"),
            ("match git://example.com git links", "git://example.com"),
            ("/tmp/test.txt http://www.google.com", "/tmp/test.txt"),
            ("match tel:+18005551234 tel links", "tel:+18005551234"),
            ("match magnet:?xt=urn:btih:1234567890 magnet links", "magnet:?xt=urn:btih:1234567890"),
            ("match ipfs://QmSomeHashValue ipfs links", "ipfs://QmSomeHashValue"),
            ("match ipns://QmSomeHashValue ipns links", "ipns://QmSomeHashValue"),
            ("match gemini://example.com gemini links", "gemini://example.com"),
            ("match gopher://example.com gopher links", "gopher://example.com"),
            ("match news:comp.infosystems.www.servers.unix news links", "news:comp.infosystems.www.servers.unix"),
            ("/Users/ghostty.user/code/example.py", "/Users/ghostty.user/code/example.py"),
            ("/Users/ghostty.user/code/../example.py", "/Users/ghostty.user/code/../example.py"),
            ("/Users/ghostty.user/code/../example.py hello world", "/Users/ghostty.user/code/../example.py"),
            ("../example.py", "../example.py"),
            ("../example.py ", "../example.py"),
            ("first time ../example.py contributor ", "../example.py"),
            ("[link](/home/user/ghostty.user/example)", "/home/user/ghostty.user/example"),
            ("Serving HTTP on :: port 8000 (http://[::]:8000/)", "http://[::]:8000/"),
            ("IPv6 address https://[2001:db8::1]:8080/path", "https://[2001:db8::1]:8080/path"),
            ("IPv6 localhost http://[::1]:3000", "http://[::1]:3000"),
            ("Complex IPv6 https://[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443/", "https://[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443/"),
            ("IPv6 with path https://[2001:db8::1]/path/to/resource", "https://[2001:db8::1]/path/to/resource"),
            ("IPv6 with query https://[2001:db8::1]:8080/api?param=value&other=123", "https://[2001:db8::1]:8080/api?param=value&other=123"),
            ("IPv6 compressed http://[2001:db8::]:80/", "http://[2001:db8::]:80/"),
            ("IPv6 multiple zeros http://[2001:0:0:0:0:0:0:1]", "http://[2001:0:0:0:0:0:0:1]"),
            ("IPv6 link-local https://[fe80::1234:5678:9abc]", "https://[fe80::1234:5678:9abc]"),
            ("IPv6 multicast http://[ff02::1]/stream", "http://[ff02::1]/stream"),
            ("IPv6 in markdown [link](http://[2001:db8::1]/docs)", "http://[2001:db8::1]/docs"),
            ("./spaces-end.   ", "./spaces-end."),
            ("./space middle", "./space middle"),
            ("../test folder/file.txt", "../test folder/file.txt"),
            ("/tmp/test folder/file.txt", "/tmp/test folder/file.txt"),
            ("/tmp/test  folder/file.txt", "/tmp/test"),
            ("diff --git a/src/font/shaper/harfbuzz.zig b/src/font/shaper/harfbuzz.zig", "a/src/font/shaper/harfbuzz.zig"),
            ("/tmp/foo /tmp/bar", "/tmp/foo"),
            ("/tmp/foo.txt /tmp/bar.txt", "/tmp/foo.txt"),
            ("src/config/url.zig", "src/config/url.zig"),
            ("app/folder/file.rb:1", "app/folder/file.rb:1"),
            ("modified:   src/config/url.zig", "src/config/url.zig"),
            ("lib/ghostty/terminal.zig:42:10", "lib/ghostty/terminal.zig:42:10"),
            ("some-pkg/src/file.txt more text", "some-pkg/src/file.txt"),
            ("src/foo.c,baz.txt", "src/foo.c"),
            ("~/foo/bar.txt", "~/foo/bar.txt"),
            ("open ~/Documents/notes.md please", "~/Documents/notes.md"),
            ("~/.config/ghostty/config", "~/.config/ghostty/config"),
            ("directory: ~/src/ghostty-org/ghostty", "~/src/ghostty-org/ghostty"),
            ("$HOME/src/config/url.zig", "$HOME/src/config/url.zig"),
            ("space dir: $PWD/src/ghostty/main.zig", "$PWD/src/ghostty/main.zig"),
            ("foo/$BAR/baz", "foo/$BAR/baz"),
            (".foo/bar/$VAR", ".foo/bar/$VAR"),
            (".config/ghostty/config", ".config/ghostty/config"),
            ("loaded from .local/share/ghostty/state.db now", ".local/share/ghostty/state.db"),
            ("../some/where", "../some/where"),
            ("  - shared/src/foo/SomeItem.m:12, shared/src/", "shared/src/foo/SomeItem.m:12"),
            ("foo.local/share", "foo.local/share"),
            ("2024/report.txt", "2024/report.txt"),
            ("./foo bar,baz", "./foo bar"),
            ("/tmp/foo bar,baz", "/tmp/foo bar"),
            ("./.config/ghostty: Needs upstream (main)", "./.config/ghostty"),
            ("./Downloads: Operation not permitted", "./Downloads"),
        ];
        for (input, expect) in cases {
            assert_eq!(first(input), Some(expect), "{input:?}");
        }
        for input in ["input/output", "foo/bar", "$10/bar", "$10/$20", "$10/bar.txt", "foo/bar,baz.txt", "foo$BAR/baz.txt", "foo~/bar.txt", "// foo bar", "//foo"] {
            assert_eq!(first(input), None, "{input:?}");
        }
    }

    #[test]
    fn query_links_keep_their_full_target() {
        for url in ["https://example.com/?a=1&b=2", "https://localhost:17480/login?t=abc%3D%3D", "file:///C:/Windows/System32/winver.exe"] {
            let text = format!("Dashboard: {url}");
            let range = find(&text, text.find(url).unwrap()).unwrap();
            assert_eq!(&text[range.clone()], url);
            // winver.exe exists on Windows, where it opens as the file.
            let expected = match url.strip_prefix("file:///") {
                Some(path) if cfg!(windows) => Target::File(path.replace('/', "\\").into()),
                _ => Target::Url(url.into()),
            };
            assert_eq!(resolve(&text[range], None), Some(expected));
        }
    }

    #[test]
    fn finds_the_link_under_the_pointer() {
        let text = "see https://a.com and /tmp/x.txt.";
        assert_eq!(find(text, 0), None);
        assert_eq!(find(text, 4).map(|r| &text[r]), Some("https://a.com"));
        assert_eq!(find(text, 16).map(|r| &text[r]), Some("https://a.com"));
        assert_eq!(find(text, 17), None);
        assert_eq!(find(text, 25).map(|r| &text[r]), Some("/tmp/x.txt."));
    }

    #[test]
    fn extracts_address_words_at_the_pointer() {
        for (text, expected) in [
            ("<t1hg>", "t1hg"),
            ("[\"macbook/t2w\"]", "macbook/t2w"),
            ("terminal=t1hg,", "t1hg"),
            ("'my-machine_2/t2w'.", "my-machine_2/t2w"),
            ("before t1hg after", "t1hg"),
            ("é t1hg", "t1hg"),
            ("<build.box/t1>.", "build.box/t1"),
            ("1.2.3.4/t1", "1.2.3.4/t1"),
            ("...t1hg.", "t1hg"),
        ] {
            let at = text.find(expected).unwrap();
            for (i, _) in expected.char_indices() {
                assert_eq!(word(text, at + i).map(|r| &text[r]), Some(expected), "{text}");
            }
        }
        for (text, at) in [("<t1>", 0), ("t1 ", 2), ("", 0), ("t1", 2), ("é", 1)] {
            assert_eq!(word(text, at), None);
        }
    }

    #[test]
    fn matches_only_live_addresses_relative_to_the_source_machine() {
        let live = ["t1", "t2", "macbook/t1", "macbook/t2w", "my-machine_2/t3", "build.box/t1", "box/t1"];
        for (word, source, expected) in [
            ("t1", "t2", Some("t1")),
            ("t1", "macbook/t2w", Some("macbook/t1")),
            ("t1", "build.box/t2", Some("build.box/t1")),
            ("t2", "macbook/t1", None),
            ("macbook/t2w", "t1", Some("macbook/t2w")),
            ("my-machine_2/t3", "macbook/t1", Some("my-machine_2/t3")),
            ("t99", "t1", None),
            ("Build", "t1", None),
            ("t", "t1", None),
            ("other/t1", "t1", None),
            ("macbook/t1/extra", "t1", None),
        ] {
            assert_eq!(terminal_address(word, source, live), expected.map(str::to_owned), "{word} in {source}");
        }
    }

    #[test]
    fn dotted_machines_never_match_a_different_suffix_machine() {
        let text = "<build.box/t1>.";
        let range = word(text, text.find("t1").unwrap()).unwrap();
        assert_eq!(&text[range.clone()], "build.box/t1");
        assert_eq!(terminal_address(&text[range.clone()], "t2", ["box/t1"]), None);
        assert_eq!(terminal_address(&text[range], "t2", ["box/t1", "build.box/t1"]), Some("build.box/t1".into()));
    }

    #[test]
    fn files_resolve_only_when_they_exist() {
        let dir = std::env::temp_dir().join(format!("herdr-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        let file = Target::File(dir.join("src/main.rs"));
        assert_eq!(resolve("src/main.rs", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve("src/main.rs:42:7", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve("src/main.rs.", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve("src/.", Some(&dir)), Some(Target::File(dir.join("src/."))));
        assert_eq!(resolve("src/..", Some(&dir)), Some(Target::File(dir.join("src/.."))));
        assert_eq!(resolve("$PWD/src/main.rs", Some(&dir)), Some(file.clone()));
        assert_eq!(resolve(dir.join("src/main.rs").to_str().unwrap(), None), Some(file));
        assert_eq!(resolve("src/gone.rs", Some(&dir)), None);
        assert_eq!(resolve("src/main.rs", None), None);
        assert_eq!(resolve("https://a.com/x", None), Some(Target::Url("https://a.com/x".into())));
        #[cfg(unix)]
        {
            let dotted = dir.join("src/main.rs.");
            std::fs::write(&dotted, "").unwrap();
            assert_eq!(resolve("src/main.rs.", Some(&dir)), Some(Target::File(dotted)));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remote_links_keep_paths_unchecked_and_urls_as_urls() {
        for (link, expected) in [
            ("src/gone.rs:42", Remote::Path("src/gone.rs:42".into())),
            ("~/notes/a.md.", Remote::Path("~/notes/a.md.".into())),
            ("/home/me/x.rs", Remote::Path("/home/me/x.rs".into())),
            ("file:///home/me/a%20b.png", Remote::Path("/home/me/a b.png".into())),
            ("file://localhost/home/me/x.rs", Remote::Path("/home/me/x.rs".into())),
            // `ls --hyperlink` names the machine's host, which may differ from its name (multivac-wsl on multivac).
            ("file://multivac/home/me/x.rs", Remote::Path("/home/me/x.rs".into())),
            ("file://MULTIVAC/home/me/x.rs", Remote::Path("/home/me/x.rs".into())),
            ("file://multivac-wsl/home/me/x.rs", Remote::Path("/home/me/x.rs".into())),
            // Only the names given: a longer name isn't proof of the same host.
            ("file://multivac.lan/home/me/x.rs", Remote::Url("file://multivac.lan/home/me/x.rs".into())),
            ("file://box/home/me/x.rs", Remote::Url("file://box/home/me/x.rs".into())),
            ("file://multivacx/home/me/x.rs", Remote::Url("file://multivacx/home/me/x.rs".into())),
            // An explicit scheme stays a URL, digits at its end or not (the line suffix is text detection's).
            ("urn:isbn:0451450523", Remote::Url("urn:isbn:0451450523".into())),
            ("file:12", Remote::Url("file:12".into())),
            ("https://a.com/x", Remote::Url("https://a.com/x".into())),
            ("https://a.com:8080", Remote::Url("https://a.com:8080".into())),
            ("vscode://file/home/me/x.rs:3", Remote::Url("vscode://file/home/me/x.rs:3".into())),
        ] {
            assert_eq!(resolve_remote(link, &["multivac-wsl", "multivac"]), expected, "{link}");
        }
        // A host given in full also answers to its short name, never the reverse.
        assert_eq!(resolve_remote("file://build/x.rs", &["box", "build.lan"]), Remote::Path("/x.rs".into()));
        assert_eq!(resolve_remote("file://build.lan/x.rs", &["build.lan"]), Remote::Path("/x.rs".into()));
        let other = "file://build.other.example/x.rs";
        assert_eq!(resolve_remote(other, &["build"]), Remote::Url(other.into()));
        assert_eq!(resolve_remote(other, &["build.lan"]), Remote::Url(other.into()));
    }

    #[test]
    fn remote_terminals_also_link_names_only_a_lookup_there_can_find() {
        let at = |text: &str, word: &str| {
            let i = text.find(word).unwrap();
            find_remote(text, i + word.len() / 2, &[]).map(|(r, link)| (text[r].to_owned(), link))
        };
        let path = |s: &str| Some((s.to_owned(), Remote::Path(s.to_owned())));
        assert_eq!(at("see README.md:12 for it", "README"), path("README.md:12"));
        assert_eq!(at("see README.md.", "README"), path("README.md"));
        assert_eq!(at("app.py (src/app.py:12)", "app.py"), path("app.py"));
        assert_eq!(at("app.py (src/app.py:12)", "src/"), path("src/app.py:12"));
        assert_eq!(at("build: src/Makefile:12:3", "Makefile"), path("src/Makefile:12:3"));
        // Every part of a path names the whole of it, never a URL inside it.
        for word in ["src", "build", "file", ":12"] {
            assert_eq!(at("see src/build-file:12 now", word), path("src/build-file:12"), "{word}");
        }
        assert_eq!(at("see src/file:12", "file"), path("src/file:12"));
        assert_eq!(at("x file://host/a b", "host"), Some(("file://host/a".into(), Remote::Url("file://host/a".into()))));
        assert_eq!(at("open ~/x/notes.md now", "notes"), path("~/x/notes.md"));
        assert_eq!(at("at https://a.com/x ok", "a.com"), Some(("https://a.com/x".into(), Remote::Url("https://a.com/x".into()))));
        for (text, word) in [
            ("Makefile:12", "Makefile"),
            ("on localhost:8080 now", "localhost"),
            ("at 12:30 today", "12:30"),
            ("version v1.2.3", "v1.2.3"),
            ("mail me@example.com", "example"),
            ("src/Makefile alone", "Makefile"),
        ] {
            assert_eq!(at(text, word), None, "{text}");
        }
        // Text detection on this computer is Ghostty's, but a scheme never starts inside a word.
        assert_eq!(find("Makefile:12", 9), None);
        assert_eq!(find("README.md:12", 2), None);
        assert_eq!(find("src/build-file:12", 12), None);
        assert_eq!(find("src/file:12", 6), None);
    }

    #[test]
    fn file_urls_open_as_the_file_they_name() {
        let dir = std::env::temp_dir().join(format!("herdr-file-urls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("测试 #1.txt");
        std::fs::write(&path, "").unwrap();
        // As programs write them: every byte outside the unreserved set encoded.
        let slashed = path.to_str().unwrap().replace('\\', "/");
        let encoded: String = slashed
            .bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect();
        let root = if cfg!(windows) { "/" } else { "" };
        let file = Some(Target::File(path.clone()));
        assert_eq!(resolve(&format!("file://{root}{encoded}"), None), file);
        assert_eq!(resolve(&format!("file://localhost{root}{encoded}"), None), file);
        assert_eq!(resolve(&format!("file://{root}{}", slashed.replace('#', "%23")), None), file);
        for url in [format!("file://{root}{encoded}.gone"), format!("file://{root}{encoded}%ZZ"), format!("file://box{root}{encoded}")] {
            assert_eq!(resolve(&url, None), Some(Target::Url(url.clone())), "{url}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
