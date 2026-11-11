---
name: sync-upstream
description: Update Herdr 2. Merges upstream's latest app release into the fork and installs it, and brings every herdr-core to the latest compatible stable release. Use when asked to sync, update Herdr 2, or merge upstream.
disable-model-invocation: true
---

# Sync the fork with upstream, and the cores

Two tracks, each assessed on its own every run. The app and herdr-core are released separately and their version numbers differ (app 0.1.15 needs only core 0.1.11 or newer), so the app's version never picks the core's.
- **App track (steps 1–5):** only when upstream has a newer release tag than master holds.
- **Core track (step 6):** its read-only assessment always runs. It changes cores only after a clean app track, meaning the app was already current (`UP-TO-DATE`) or its merge was built, tested, installed and published. After any app-track hard stop, step 6 only assesses and reports, and changes no core.

**When this runs is decided outside the skill.** The dot or the user starts it, behind the dot's own gates (idle or overnight, one update at a time, advance notice). This skill adds no idle, activity or time gates of its own. Within the run:
- **Advance notice:** before the first disruptive step (asking to quit Herdr in step 4, the first core update in step 6), tell the user what will restart and what it does.
- **Permissions:** a denied permission or a dialog the user declines is their answer. Stop that step and report it; never route around it with another command, `--force` or a retry.

The fork (`origin` = schugazi/herdr-app) takes upstream (`upstream` = herdrdev/herdr-app) by **merge, never rebase**. `origin/master` is published and installed, so its history is never rewritten. It takes **only upstream's newest `vX.Y.Z` tag** (releases ship from tags; GitHub shows no releases), never `upstream/master`, which carries unreleased work. One merge commit per sync, titled `Merge upstream herdrdev/herdr-app through <version>`. Work in `~/herdr-app` on mycroftxxx. Nothing is pushed to `master` until multivac has built, tested and installed it. Updates happen only through this flow: never turn on the app's own updater (a source build leaves it off), and never install the official app build over the fork.

## 1. Prepare (app track)

```bash
(
set -euo pipefail
cd ~/herdr-app && git switch master
[ -z "$(git status --porcelain)" ] || { echo "STOP: dirty tree"; exit 1; }
git pull --ff-only origin master
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/master)" ] || { echo "STOP: local master has unpublished commits (an unfinished sync?)"; exit 1; }
TAG=$(git ls-remote --tags upstream 'v*' | grep -v '\^{}' | awk -F/ '{print $3}' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1)
[ -n "$TAG" ] || { echo "STOP: no release tag"; exit 1; }
git update-ref -d refs/remotes/upstream/stable
git fetch --no-tags upstream "+refs/tags/$TAG:refs/remotes/upstream/stable"
git update-ref refs/remotes/upstream/stable "$(git rev-parse 'refs/remotes/upstream/stable^{commit}')"
if git merge-base --is-ancestor upstream/stable HEAD; then echo "UP-TO-DATE $TAG"; else echo "MERGE $TAG"; fi
)
```

A non-zero exit is a hard stop. `UP-TO-DATE` counts only once the installed app is `origin/master`'s build. multivac's checkout must be on it, and the installed app must be that checkout's last build. Anything else is a hard stop:

```bash
V=$(mktemp) && ~/voice-hub/dispatcher/herdr2-versions --json >"$V" && python3 -I - "$(git -C ~/herdr-app rev-parse origin/master)" "$V" <<'PY'
import json, sys
head, path = sys.argv[1], sys.argv[2]
r = json.load(open(path))
src = (r.get("source") or {}).get("multivac") or {}
app = (r.get("app") or {}).get("installed") or {}
ok = src.get("head") == head and app.get("same_as_build") is True
print("INSTALLED origin/master" if ok else f"STOP: the installed app isn't origin/master's build (checkout {src.get('head')}, same_as_build {app.get('same_as_build')})")
sys.exit(not ok)
PY
```

Once that prints `INSTALLED origin/master`, the app needs nothing: skip the merge, review, build and install and go to step 6. On `MERGE`, skim `git log --oneline HEAD..upstream/stable` and that version's section of `git show upstream/stable:CHANGELOG.md`.

## 2. Merge and resolve

```bash
git merge --no-ff --no-commit upstream/stable
git submodule update --init --recursive
```

Resolve every conflict by reading what both sides meant to do.
- **Cursor settings:** re-apply the fork's on top of upstream's new code. If upstream now ships its own cursor style, blink or thickness setting, take upstream's and drop the fork's duplicate, then update `CLAUDE.md` and the host README's cursor line.
- **Version files and `CHANGELOG.md`:** upstream's.
- **Fork-only files** (`CLAUDE.md`, `scripts/fork-install.ps1`, this skill): stay.
- **Clean merges:** check files that merged cleanly but touch `src/vt.rs`, `src/terminal.rs` and `src/settings.rs` too.

Commit locally. The body says what upstream brought and what of the fork was kept or replaced.

## 3. One sol review, exactly once

Make one Agent call with `subagent_type: "code-reviewer"` and `model: "sol"`. Brief it: review how merge `<hash>` in /home/schu/herdr-app resolved its conflicts. Upstream tag `<tag>` is `upstream/stable`; the merge is local and unpublished. Start from `git show --remerge-diff <hash>`, and use `git diff upstream/stable <hash>` for what the fork still changes. Look for fork behaviour lost or only half re-applied, upstream fixes dropped, duplicate implementations, and behaviour broken in files that merged cleanly. Report only.

Apply the findings you agree with and `git commit --amend`, with one line in the body per rejected finding. Never call the review twice.

## 4. Build, test and install on multivac

```bash
git push origin HEAD:sync
ssh multivac 'cd /d %USERPROFILE%\herdr-app && git fetch -q origin && git checkout -q -B master origin/sync'
ssh multivac 'powershell -NoProfile -ExecutionPolicy Bypass -File %USERPROFILE%\herdr-app\scripts\fork-install.ps1 -Test'
```

A full build takes a while, so run the last command in the background. The script installs only after the build is done and the tests pass, and only while Herdr is closed. When it stops with "Built. Quit Herdr", ask the user to quit Herdr, then run it again without `-Test` (the install takes seconds).

A failure here installs nothing. Fixing it is still your job. If you can't, it's a hard stop: put multivac's checkout back with `git fetch -q origin && git checkout -q -B master origin/master`.

## 5. Publish

```bash
git push origin HEAD:master && git push origin --delete sync
ssh multivac 'cd /d %USERPROFILE%\herdr-app && git fetch -q --prune origin && git checkout -q -B master origin/master && git rev-parse HEAD'
```

The hash printed must be the merge you pushed (`git rev-parse HEAD` here). A rejected push is a hard stop: the merge stays installed but unpublished. Report that.

## 6. Cores

The app no longer updates any core. Steps A and B are read-only and always run; C and D change cores and run only after a clean app track.

### A. Pick the target

Pick the target without regard to the app's version: the newest officially published stable herdr-core (`releases.herdr.dev/core/latest.json`, the channel the core's own installer and `hc update` use). It must be compatible with every app build that may be running, meaning `origin/master` and the local `HEAD` (an installed but unpublished merge after a rejected push). Compatible means the same `generation` as the app's `GENERATION` in `src/hc.rs`, and a version no older than the app's `distribution/core-version`. It must also be a plain `X.Y.Z` (never a prerelease), match its own per-version manifest, and have a matching tag and dated `CHANGELOG.md` section in herdrdev/herdr-core:

```bash
(
set -euo pipefail
cd ~/herdr-app
R=https://releases.herdr.dev/core
field() { python3 -I -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"; }
L=$(curl -fsS "$R/latest.json"); T=$(field version <<<"$L"); G=$(field generation <<<"$L")
[[ $T =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "STOP: latest core '$T' is not a stable X.Y.Z"; exit 1; }
M=$(curl -fsS "$R/$T/manifest.json")
[ "$(field version <<<"$M")" = "$T" ] && [ "$(field generation <<<"$M")" = "$G" ] || { echo "STOP: $T's manifest disagrees with latest.json"; exit 1; }
for REF in origin/master HEAD; do
  AG=$(git show "$REF:src/hc.rs" | sed -n 's/^pub const GENERATION: u64 = \([0-9]*\);.*/\1/p')
  MIN=$(git show "$REF:distribution/core-version" | tr -d '[:space:]')
  [ -n "$AG" ] && [ -n "$MIN" ] || { echo "STOP: $REF's core requirements can't be read"; exit 1; }
  [ "$G" = "$AG" ] || { echo "STOP: core $T is generation $G, app at $REF speaks $AG"; exit 1; }
  [ "$(printf '%s\n' "$MIN" "$T" | sort -V | head -1)" = "$MIN" ] || { echo "STOP: core $T is older than $REF's minimum $MIN"; exit 1; }
done
git ls-remote --exit-code --tags https://github.com/herdrdev/herdr-core.git "refs/tags/v$T" >/dev/null || { echo "STOP: no herdr-core tag v$T"; exit 1; }
D=$(curl -fsS "https://raw.githubusercontent.com/herdrdev/herdr-core/v$T/CHANGELOG.md" | sed -n "s/^## \[${T//./\\.}\] - \([0-9-]*\)$/\1/p")
[ -n "$D" ] || { echo "STOP: herdr-core v$T's CHANGELOG has no dated section for it"; exit 1; }
echo "CORE-TARGET $T (generation $G, released $D)"
)
```

A `STOP` (or any non-zero exit) means no core is changed. Report it; a generation change waits for the user. Read the target's `CHANGELOG.md` section for the report.

### B. Check every machine

Check every machine against the target. This uses `~/voice-hub/dispatcher/herdr2-versions`, which reads each core's installed and running versions without an `hc` command (any `hc` command starts a core when none is running). It fails closed: every one of the three machines must be there, with a valid installed version and exactly one running core with a valid version. Set `T` to the target, then:

```bash
V=$(mktemp) && ~/voice-hub/dispatcher/herdr2-versions --json >"$V" && python3 -I - "$T" "$V" <<'PY'
import json, re, sys
t, path = sys.argv[1], sys.argv[2]
key = lambda v: tuple(map(int, v.split("."))) if isinstance(v, str) and re.fullmatch(r"\d+\.\d+\.\d+", v) else None
try:
    cores = json.load(open(path))["cores"]
except Exception as e:
    sys.exit(f"UNREADABLE: herdr2-versions output: {e}")
bad = 0
for name in ("multivac", "multivac-wsl", "mycroftxxx"):
    c = cores.get(name) if isinstance(cores, dict) else None
    c = c if isinstance(c, dict) else {}
    run = c.get("running")
    run = [p for p in run if isinstance(p, dict)] if isinstance(run, list) else None
    inst = c.get("installed")
    if not c or "error" in c or run is None or key(inst) is None:
        state = "UNREADABLE"
    elif len(run) != 1:
        state = "AMBIGUOUS"  # none running, or several
    elif key(run[0].get("version")) is None or not isinstance(run[0].get("replaced"), bool):
        state = "UNREADABLE"
    elif max(key(inst), key(run[0]["version"])) > key(t):
        state = "NEWER"
    elif inst == t and run[0]["version"] == t and not run[0]["replaced"]:
        state = "AT-TARGET"
    else:
        state = "BELOW"
    bad |= state != "AT-TARGET"
    print(f"{name}: {state}: installed {inst}, running {[p.get('version') for p in run or []]}, read {c.get('checked')}"
          + (f", {c['error']}" if "error" in c else ""))
sys.exit(bad)
PY
```

- **All `AT-TARGET`:** the cores are current. This is still the verified result.
- **`NEWER` on any machine:** the target would be a downgrade. Change no core and report. Never `--force`.
- **`UNREADABLE` or `AMBIGUOUS`:** that machine's state isn't known, so it is never updated. Report it as unverified; the other machines go ahead.
- **`BELOW`:** goes to step C, but only after a clean app track. Otherwise report it and stop here.

### C. Update

Give the user advance notice: which cores restart, that running terminals keep running, and that the app reconnects to each core. Then update every `BELOW` machine, pinned to the target, each through its own entry point. Never use `--force`. Without it, `hc update --version` refuses another generation and never replaces an installed binary that is already that version or newer. It still restarts an older core that is running on top of it.
- Windows: `ssh multivac hc update --version $T`
- mycroftxxx, as a transient unit through `hc-clean` from `~`: `systemd-run --user --wait --collect -p WorkingDirectory=$HOME -p UnsetEnvironment=HC_SOCKET -E HC_SESSION=default ~/.local/bin/hc-clean update --version $T`
- WSL, through Windows' `ssh wsl`, from `~`: `ssh multivac "ssh wsl \"cd ~ && herdr-core update --version $T\""`

One machine failing doesn't stop the others.

### D. Verify

Run step B's check again. A machine still `BELOW` whose installed version is the target but whose single running core isn't (still the old binary) gets one restart through the same entry point, `server restart` in place of `update --version $T`. Then check again.

Declare the cores done only when all three are `AT-TARGET`. Anything else is a partial result: name each machine, its state, its installed and running versions, and the error. Do nothing further to it, and never retry with `--force`.

## 7. Report

- **App:** the tag merged, "already current", or the hard stop that ended it. Conflicts and their resolutions, sol's findings (applied or rejected), test and install results, publish state. The user reopens Herdr on a new build.
- **Cores:** the target with its generation and release date, and its changelog's new features and fixes kept apart. Each machine's state and its installed and running versions before and after, with when they were read. Whether the cores are done, partial, assessment only (after an app-track hard stop), or stopped, and why.

## Hard stops (report, don't force)

- **App track:** a dirty tree, unpublished local commits, a failed fetch or pull, or an installed app that isn't `origin/master`'s build in step 1; tests or the build still red after an honest fix; a rejected push. Each ends the run's changes. Step 6 still assesses and reports, and changes no core.
- **Core track:** a `STOP` in step A, or a `NEWER` core, changes no core. An `UNREADABLE` or `AMBIGUOUS` machine is never updated.
- **Always:** never `git push --force`, never rebase `master`, never push tags, never `hc update --force`.
