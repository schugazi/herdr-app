// HERDR: built-in pure-Rust syntax highlighting for script editors, adapted from Base's showcase.
use std::{collections::HashMap, ops::Range, sync::LazyLock};

use gpui::{Context, HighlightStyle, SharedString, Window};
use gpui_base::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope,
};
use syntect::{
    parsing::{ParseState, Scope, ScopeStack, SyntaxDefinition, SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

/// Loading the default syntax definitions deserializes a few megabytes, so share
/// one set across every highlighter instance.
/// HERDR fork: plus a T-SQL grammar, which wins .sql over the default SQL one
/// (lookups search the last added first).
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(|| {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    let tsql = SyntaxDefinition::load_from_str(include_str!("tsql.sublime-syntax"), true, None)
        .expect("tsql.sublime-syntax parses");
    builder.add(tsql);
    builder.build()
});

/// A small, WASM-compatible example adapter for Base's parser-independent
/// highlighting API. Syntect identifies scopes; the application-owned resolver
/// supplies all colors and font styles.
pub(crate) struct SyntectHighlighter {
    language: SharedString,
    /// Non-overlapping highlights, ordered by start offset.
    highlights: Vec<(Range<usize>, &'static str)>,
    fold_ranges: Vec<FoldRange>,
    /// Scope ids are cheap to compare but expensive to stringify, so remember
    /// the semantic name each one maps to.
    semantic_names: HashMap<Scope, Option<&'static str>>,
}

impl SyntectHighlighter {
    pub(crate) fn new(language: &str) -> Option<Self> {
        find_syntax(language)?;

        Some(Self {
            language: language.to_owned().into(),
            highlights: Vec::new(),
            fold_ranges: Vec::new(),
            semantic_names: HashMap::new(),
        })
    }

    fn push_highlight(&mut self, range: Range<usize>, scopes: &ScopeStack) {
        if range.is_empty() {
            return;
        }

        if let Some(name) = stack_name(&mut self.semantic_names, &scopes.scopes) {
            self.highlights.push((range, name));
        }
    }
}

/// The innermost scope with a semantic name names the range, except that a
/// string inside a mapping key (JSON's and JS's quoted keys) is a property, as
/// YAML's plain keys are, and a bare name takes the name it declares.
fn stack_name(
    names: &mut HashMap<Scope, Option<&'static str>>,
    scopes: &[Scope],
) -> Option<&'static str> {
    let mut named = scopes
        .iter()
        .rev()
        .filter_map(|scope| *names.entry(*scope).or_insert_with(|| semantic_name(*scope)));
    match named.next()? {
        // Python wraps every identifier in meta.generic-name, a declared
        // function's or class's name and an imported one too; any other is a
        // variable.
        "variable.generic" => match named.next() {
            Some(outer @ ("function" | "type" | "type.namespace")) => Some(outer),
            _ => Some("variable"),
        },
        "string" if named.any(|outer| outer == "property") => Some("property"),
        // Markdown's ***both*** nests one emphasis in the other; keep both.
        "bold" if named.any(|outer| outer == "italic") => Some("bold.italic"),
        "italic" if named.any(|outer| outer == "bold") => Some("bold.italic"),
        name => Some(name),
    }
}

/// Whether `scope` is `prefix` or one of its dotted children.
fn is(scope: &str, prefix: &str) -> bool {
    scope
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    SYNTAX_SET
        .find_syntax_by_token(language)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension(language))
}

impl InputHighlighter for SyntectHighlighter {
    fn language(&self) -> SharedString {
        self.language.clone()
    }

    fn update(
        &mut self,
        _edit: Option<InputEdit>,
        text: &Rope,
        folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        // `syntect` has no incremental mode, so the whole document is reparsed.
        // Read the rope once and reuse that string for folding too.
        let text = text.to_string();
        let syntax = find_syntax(self.language.as_ref())
            .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
        let mut parser = ParseState::new(syntax);
        let mut scopes = ScopeStack::new();
        let mut offset = 0;
        self.highlights.clear();

        for line in LinesWithEndings::from(&text) {
            if let Ok(operations) = parser.parse_line(line, &SYNTAX_SET) {
                let mut cursor = 0;
                for (index, operation) in operations {
                    self.push_highlight(offset + cursor..offset + index, &scopes);
                    let _ = scopes.apply(&operation);
                    cursor = index;
                }
                self.push_highlight(offset + cursor..offset + line.len(), &scopes);
            }
            offset += line.len();
        }

        self.fold_ranges = if folding {
            brace_fold_ranges(&text)
        } else {
            Vec::new()
        };
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        resolve_styles(&self.highlights, range, resolver)
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        self.fold_ranges.clone()
    }

    fn fold_ranges_for_edit(&self, _: Range<usize>, _: &Rope) -> Vec<FoldRange> {
        self.fold_ranges.clone()
    }
}

/// One line's pieces, each with the semantic name the editor colors it by
/// (None where it names nothing).
pub type HighlightedLine = Vec<(String, Option<&'static str>)>;

/// HERDR fork: `text` highlighted as an editor in `language` would, for code
/// shown outside one (a mod's diff): one entry per `\n`-separated line, its
/// pieces in order spelling the line (a `\r` before the `\n` stays in its last
/// piece), adjacent pieces of one name joined. None when no grammar knows the
/// language.
pub fn highlight_lines(language: &str, text: &str) -> Option<Vec<HighlightedLine>> {
    let mut parser = ParseState::new(find_syntax(language)?);
    let mut scopes = ScopeStack::new();
    let mut names = HashMap::new();
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(text) {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let mut pieces = HighlightedLine::new();
        let mut cursor = 0;
        if let Ok(operations) = parser.parse_line(line, &SYNTAX_SET) {
            for (index, operation) in operations {
                let end = index.min(body.len());
                if end > cursor {
                    push_piece(&mut pieces, &body[cursor..end], stack_name(&mut names, &scopes.scopes));
                    cursor = end;
                }
                let _ = scopes.apply(&operation);
            }
        }
        if cursor < body.len() {
            push_piece(&mut pieces, &body[cursor..], stack_name(&mut names, &scopes.scopes));
        }
        lines.push(pieces);
    }
    // LinesWithEndings yields nothing for "" and no empty last line after a final "\n".
    if text.is_empty() || text.ends_with('\n') {
        lines.push(Vec::new());
    }
    Some(lines)
}

fn push_piece(pieces: &mut HighlightedLine, text: &str, name: Option<&'static str>) {
    match pieces.last_mut() {
        Some((last, last_name)) if *last_name == name => last.push_str(text),
        _ => pieces.push((text.to_owned(), name)),
    }
}

/// Turn the highlights overlapping `range` into gap-free style runs.
///
/// `highlights` is ordered and non-overlapping, so the first candidate is found
/// by binary search instead of scanning the whole document on every frame.
fn resolve_styles(
    highlights: &[(Range<usize>, &'static str)],
    range: &Range<usize>,
    resolver: &dyn HighlightStyleResolver,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let first = highlights.partition_point(|(highlight, _)| highlight.end <= range.start);
    let mut runs = Vec::new();
    let mut cursor = range.start;

    for (highlight_range, name) in &highlights[first..] {
        if highlight_range.start >= range.end {
            break;
        }

        let start = highlight_range.start.max(range.start);
        let end = highlight_range.end.min(range.end);
        if start >= end || end <= cursor {
            continue;
        }
        if cursor < start {
            runs.push((cursor..start, HighlightStyle::default()));
        }
        runs.push((start..end, resolver.style(name).unwrap_or_default()));
        cursor = end;
    }

    if cursor < range.end {
        runs.push((cursor..range.end, HighlightStyle::default()));
    }
    runs
}

fn semantic_name(scope: Scope) -> Option<&'static str> {
    let scope = scope.build_string();
    // Finer names (keyword.control, variable.language, …) fall back to their
    // root's color when the editor's syntax map leaves them out.
    let name = if scope.starts_with("comment") || is(&scope, "punctuation.definition.comment") {
        "comment"
    } else if is(&scope, "punctuation.definition.string") {
        // Quotes take their string's color, as in VS Code.
        "string"
    } else if scope.starts_with("constant.character.escape") {
        // Before regexp, which tags its own escapes (constant.character.escape.backslash.regexp).
        "string.escape"
    } else if is(&scope, "string.regexp") || is(&scope, "source.regexp") || scope.ends_with(".regexp") {
        "string.regexp"
    } else if is(&scope, "punctuation.section.interpolation") {
        "punctuation.embedded"
    } else if is(&scope, "keyword.operator.logical") {
        "operator.logical"
    } else if is(&scope, "keyword.control") || is(&scope, "keyword.other.await") {
        "keyword.control"
    } else if is(&scope, "variable.function") {
        "function"
    } else if is(&scope, "variable.annotation") || is(&scope, "punctuation.definition.annotation") {
        "function.decorator"
    } else if is(&scope, "variable.language") {
        "variable.language"
    } else if is(&scope, "variable.other.constant") {
        "variable.constant"
    } else if is(&scope, "variable.parameter") {
        "variable.parameter"
    } else if is(&scope, "meta.generic-name") {
        "variable.generic"
    } else if is(&scope, "support.variable")
        || is(&scope, "constant.other.placeholder")
        || is(&scope, "text.variable")
    {
        // __name__, __file__, %-format placeholders and SQL's @variables, as
        // VS Code colors them.
        "variable"
    } else if is(&scope, "entity.other.inherited-class") {
        "type"
    } else if is(&scope, "meta.statement.import") || is(&scope, "meta.import-name") {
        // Python's import lines name modules and what they export.
        "type.namespace"
    } else if is(&scope, "constant.language") && !is(&scope, "constant.language.boolean") {
        "constant.language"
    } else if is(&scope, "markup.inserted") {
        "inserted"
    } else if is(&scope, "markup.deleted") {
        "deleted"
    } else if is(&scope, "markup.underline.link") || is(&scope, "string.other.link") {
        "link"
    } else if is(&scope, "entity.name.tag") && scope.ends_with(".yaml") {
        "property"
    } else if is(&scope, "entity.name.tag") {
        "tag"
    } else if is(&scope, "entity.other.attribute-name") {
        "attribute"
    } else if is(&scope, "support.type.property-name")
        || is(&scope, "meta.structure.dictionary.key")
        || is(&scope, "meta.object-literal.key")
        || is(&scope, "meta.property.object")
    {
        "property"
    } else if is(&scope, "markup.heading") || is(&scope, "entity.name.section") {
        "heading"
    } else if is(&scope, "markup.bold") {
        "bold"
    } else if is(&scope, "markup.italic") {
        "italic"
    } else if is(&scope, "markup.raw") {
        "code"
    } else if scope.starts_with("string") {
        "string"
    } else if scope.starts_with("constant.numeric") {
        "number"
    } else if scope.starts_with("constant.language.boolean") {
        "boolean"
    } else if scope.starts_with("keyword.operator") {
        "operator"
    } else if scope.starts_with("keyword") || scope.starts_with("storage") {
        "keyword"
    } else if scope.starts_with("entity.name.function") || scope.starts_with("support.function") {
        "function"
    } else if scope.starts_with("entity.name.type")
        || scope.starts_with("entity.name.class")
        || scope.starts_with("support.type")
    {
        "type"
    } else if scope.starts_with("variable") {
        "variable"
    } else if scope.starts_with("constant") {
        "constant"
    } else if scope.starts_with("punctuation") {
        "punctuation"
    } else {
        return None;
    };
    Some(name)
}

fn brace_fold_ranges(text: &str) -> Vec<FoldRange> {
    let mut starts = Vec::new();
    let mut ranges = Vec::new();

    for (line_number, line) in text.lines().enumerate() {
        let mut chars = line.chars().peekable();
        let mut quoted = false;
        let mut escaped = false;
        while let Some(character) = chars.next() {
            if !quoted && character == '/' && chars.peek() == Some(&'/') {
                break;
            }
            if character == '"' && !escaped {
                quoted = !quoted;
            } else if !quoted && character == '{' {
                starts.push(line_number);
            } else if !quoted && character == '}' {
                if let Some(start_line) = starts.pop() {
                    if start_line < line_number {
                        ranges.push(FoldRange::new(start_line, line_number));
                    }
                }
            }
            escaped = quoted && character == '\\' && !escaped;
            if character != '\\' {
                escaped = false;
            }
        }
    }
    ranges
}

/// HERDR: semantic overrides fall back to readable, dark-editor defaults.
#[derive(Default)]
pub(crate) struct SyntaxStyles(pub std::collections::HashMap<String, gpui::Hsla>);

impl HighlightStyleResolver for SyntaxStyles {
    fn style(&self, name: &str) -> Option<HighlightStyle> {
        let root = name.split('.').next()?;
        let color = self
            .0
            .get(name)
            .or_else(|| self.0.get(root))
            .copied()
            .or_else(|| {
                let rgb = match root {
                    "comment" => 0x7f848e,
                    "string" => 0x98c379,
                    "number" => 0xd19a66,
                    "keyword" => 0xc678dd,
                    "boolean" | "constant" => 0xe5c07b,
                    "function" => 0x61afef,
                    "type" => 0xe5c07b,
                    "operator" => 0x56b6c2,
                    "property" | "tag" | "heading" | "deleted" => 0xe06c75,
                    "attribute" => 0xd19a66,
                    "code" | "inserted" => 0x98c379,
                    "link" => 0x61afef,
                    // Unclassified identifiers and punctuation follow the theme foreground.
                    _ => return None,
                };
                Some(gpui::rgb(rgb).into())
            });
        // Markdown's emphasis keeps the text color unless one is given, but shows its weight or slant.
        let font_weight = (root == "bold").then_some(gpui::FontWeight::BOLD);
        let font_style = (root == "italic" || name == "bold.italic").then_some(gpui::FontStyle::Italic);
        (color.is_some() || font_weight.is_some() || font_style.is_some()).then(|| HighlightStyle {
            color,
            font_weight,
            font_style,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_styles_cover_the_requested_range_without_gaps() {
        let runs = resolve_styles(
            &[(2..4, "keyword"), (6..8, "string")],
            &(0..10),
            &SyntaxStyles::default(),
        );
        let ranges: Vec<_> = runs.into_iter().map(|(range, _)| range).collect();
        assert_eq!(ranges, vec![0..2, 2..4, 4..6, 6..8, 8..10]);
    }

    #[test]
    fn semantic_overrides_and_nested_names_resolve() {
        let color = gpui::rgb(0xc678dd).into();
        let styles = SyntaxStyles(HashMap::from([("string".into(), color)]));
        assert_eq!(styles.style("string.escape").unwrap().color, Some(color));
        assert!(styles.style("keyword").is_some());
        assert!(find_syntax("rust").is_some());
        assert!(find_syntax("rs").is_some());
        assert!(find_syntax("unknown-language").is_none());
    }

    /// Each token of one line with its semantic name.
    fn names(language: &str, line: &'static str) -> Vec<(&'static str, Option<&'static str>)> {
        let mut parser = ParseState::new(find_syntax(language).unwrap());
        let mut scopes = ScopeStack::new();
        let mut cache = HashMap::new();
        let mut out = Vec::new();
        let mut cursor = 0;
        for (index, operation) in parser.parse_line(line, &SYNTAX_SET).unwrap() {
            if index > cursor {
                out.push((&line[cursor..index], stack_name(&mut cache, &scopes.scopes)));
            }
            scopes.apply(&operation).unwrap();
            cursor = index;
        }
        out
    }

    #[test]
    fn finer_names_resolve() {
        let md = names("md", "***both***\n");
        assert!(md.contains(&("both", Some("bold.italic"))));
        let style = SyntaxStyles::default().style("bold.italic").unwrap();
        assert_eq!(style.font_weight, Some(gpui::FontWeight::BOLD));
        assert_eq!(style.font_style, Some(gpui::FontStyle::Italic));
        let json = names("json", "{\"id\": \"root\"}\n");
        assert!(json.contains(&("id", Some("property"))));
        assert!(json.contains(&("root", Some("string"))));
        let yaml = names("yaml", "name: x\n");
        assert!(yaml.contains(&("name", Some("property"))));
        let html = names("html", "<a href=\"x\">\n");
        assert!(html.contains(&("a", Some("tag"))));
        assert!(html.contains(&("href", Some("attribute"))));
    }

    #[test]
    fn python_names_follow_its_grammar() {
        let def = names("py", "def run(self, n): return self.x(n) or None\n");
        for token in [
            ("def", "keyword"),
            ("run", "function"),
            ("self", "variable.parameter"),
            ("return", "keyword.control"),
            ("self", "variable.language"),
            ("x", "function"),
            ("n", "variable"),
            ("or", "operator.logical"),
            ("None", "constant.language"),
        ] {
            assert!(def.contains(&(token.0, Some(token.1))), "{token:?} in {def:?}");
        }
        let strings = names("py", "x = __name__ % re.compile(r\"a\\n\") % \"v=%s\"\n");
        assert!(strings.contains(&("__name__", Some("variable"))), "{strings:?}");
        assert!(strings.contains(&("\\n", Some("string.escape"))), "{strings:?}");
        assert!(strings.contains(&("a", Some("string.regexp"))), "{strings:?}");
        assert!(strings.contains(&("%s", Some("variable"))), "{strings:?}");
        let import = names("py", "from typing import Optional\n");
        assert!(import.contains(&("typing", Some("type.namespace"))));
        assert!(import.contains(&("Optional", Some("type.namespace"))));
        assert!(import.contains(&("import", Some("keyword.control"))));
        let class = names("py", "class Foo(Base): pass\n");
        assert!(class.contains(&("Foo", Some("type"))));
        assert!(class.contains(&("Base", Some("type"))));
    }

    #[test]
    fn sql_is_t_sql() {
        let sql = names(
            "sql",
            "IF @n IS NULL SELECT c.id, GETDATE(), N'a''b' FROM #tmp -- x\n",
        );
        for token in [
            ("IF", "keyword.control"),
            ("@n", "variable"),
            ("IS", "operator.logical"),
            ("NULL", "operator.logical"),
            ("SELECT", "keyword"),
            ("c", "constant"),
            ("id", "constant"),
            ("GETDATE", "function"),
            ("''", "string.escape"),
            ("FROM", "keyword"),
            ("-- x\n", "comment"),
        ] {
            assert!(sql.contains(&(token.0, Some(token.1))), "{token:?} in {sql:?}");
        }
        // #tmp is a temp table's name, not a comment.
        assert!(sql.iter().any(|(token, name)| token.contains("#tmp") && name.is_none()), "{sql:?}");
        let joins = names("sql", "FROM a LEFT JOIN b ON LEFT(x, 1) = y\n");
        assert_eq!(joins[2], ("LEFT", Some("operator.logical")), "{joins:?}");
        assert!(joins.contains(&("JOIN", Some("operator.logical"))), "{joins:?}");
        assert!(joins.contains(&("LEFT", Some("function"))), "{joins:?}");
        let more = names("sql", "SELECT [a']]'b], account@id, COUNT (*) FROM t\n");
        assert!(more.contains(&("[a']]'b]", None)), "{more:?}");
        assert!(more.contains(&("COUNT", Some("function"))), "{more:?}");
        assert!(more.contains(&("FROM", Some("keyword"))), "{more:?}");
        assert!(!more.iter().any(|(_, name)| *name == Some("variable")), "{more:?}");
    }

    #[test]
    fn highlighted_lines_spell_each_line() {
        let lines = highlight_lines("js", "const x = 'a';\r\n\nlet y").unwrap();
        let spelled: Vec<String> = lines
            .iter()
            .map(|line| line.iter().map(|(piece, _)| piece.as_str()).collect())
            .collect();
        assert_eq!(spelled, ["const x = 'a';\r", "", "let y"]);
        assert!(lines[0].iter().any(|(piece, name)| piece == "const" && *name == Some("keyword")), "{lines:?}");
        assert!(lines[0].windows(2).all(|pair| pair[0].1 != pair[1].1), "{lines:?}");
        assert_eq!(highlight_lines("js", "x\n").unwrap().len(), 2);
        assert_eq!(highlight_lines("js", "").unwrap().len(), 1);
        assert!(highlight_lines("unknown-language", "x").is_none());
    }
}
