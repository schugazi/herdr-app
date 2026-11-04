---
name: sync-upstream
description: Merge upstream herdrdev/herdr-app's latest release tag into the fork's master, resolve conflicts, get exactly one sol review, then build, test and install it on multivac, push, and bring every herdr-core along. Use when asked to sync, update Herdr 2, or merge upstream.
disable-model-invocation: true
---

# Sync the fork with upstream

The fork (`origin` = schugazi/herdr-app) takes upstream (`upstream` = herdrdev/herdr-app) by **merge, never rebase**: `origin/master` is published and installed, so its history is never rewritten. It takes **only upstream's newest `vX.Y.Z` tag** (releases ship from tags; GitHub shows no releases), never `upstream/master`, which carries unreleased work. One merge commit per sync, titled `Merge upstream herdrdev/herdr-app through <version>`. Work in `~/herdr-app` on mycroftxxx. Nothing is pushed to `master` until multivac has built, tested and installed it.

## 1. Prepare

```bash
(
set -euo pipefail
cd ~/herdr-app && git switch master
[ -z "$(git status --porcelain)" ] || { echo "STOP: dirty tree"; exit 1; }
git pull --ff-only origin master
TAG=$(git ls-remote --tags upstream 'v*' | grep -v '\^{}' | awk -F/ '{print $3}' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1)
[ -n "$TAG" ] || { echo "STOP: no release tag"; exit 1; }
git update-ref -d refs/remotes/upstream/stable
git fetch --no-tags upstream "+refs/tags/$TAG:refs/remotes/upstream/stable"
git update-ref refs/remotes/upstream/stable "$(git rev-parse 'refs/remotes/upstream/stable^{commit}')"
if git merge-base --is-ancestor upstream/stable HEAD; then echo "UP-TO-DATE $TAG"; else echo "MERGE $TAG"; fi
)
```

A non-zero exit is a hard stop. `UP-TO-DATE`: report and stop. On `MERGE`, skim `git log --oneline HEAD..upstream/stable` and that version's section of `git show upstream/stable:CHANGELOG.md`.

## 2. Merge and resolve

```bash
git merge --no-ff --no-commit upstream/stable
git submodule update --init --recursive
```

Resolve every conflict by reading both sides' intent. Keep the fork's cursor settings re-applied on upstream's new code; if upstream now ships its own cursor style, blink or thickness setting, take upstream's and drop the fork's duplicate (then update `CLAUDE.md` and the host README's cursor line). Version files and `CHANGELOG.md`: upstream's. Fork-only files (`CLAUDE.md`, `scripts/fork-install.ps1`, this skill) stay. Check cleanly merged files that touch `src/vt.rs`, `src/terminal.rs` and `src/settings.rs` too. Commit locally (body: what upstream brought, what of the fork was kept or replaced).

## 3. One sol review, exactly once

One Agent call, `subagent_type: "code-reviewer"`, `model: "sol"`: review how merge `<hash>` in /home/schu/herdr-app (upstream tag `<tag>` as `upstream/stable`, local and unpublished) resolved its conflicts, starting from `git show --remerge-diff <hash>`, with `git diff upstream/stable <hash>` as the fork's remaining delta; look for fork behaviour lost or half re-applied, upstream fixes dropped, duplicate implementations and semantic breaks in cleanly merged files; report only. Apply what you agree with, `git commit --amend`, one line per rejected finding in the body. Never call it twice.

## 4. Build, test and install on multivac

```bash
git push origin HEAD:sync
ssh multivac 'cd /d %USERPROFILE%\herdr-app && git fetch -q origin && git checkout -q -B master origin/sync'
ssh multivac 'powershell -NoProfile -ExecutionPolicy Bypass -File %USERPROFILE%\herdr-app\scripts\fork-install.ps1 -Test'
```

A full build takes a while: run the last command in the background. The script installs only after the tests pass and the build is done, and only while Herdr is closed: when it stops with "Built. Quit Herdr", ask the user to quit Herdr, then run it again without `-Test` (it installs in seconds). A failure here installs nothing: fix it (that is still yours) or stop, and put multivac's checkout back with `git fetch -q origin && git checkout -q -B master origin/master`.

## 5. Publish and bring the cores along

```bash
git push origin HEAD:master && git push origin --delete sync
ssh multivac 'cd /d %USERPROFILE%\herdr-app && git fetch -q --prune origin && git checkout -q -B master origin/master && git rev-parse HEAD'
```

The printed hash must be the merge you pushed (`git rev-parse HEAD` here).

Then every core to the matching version (the app no longer updates them): Windows `ssh multivac hc update`; mycroftxxx and WSL as in `~/.local/share/herdr-app/README.md` "Updates". Running terminals keep running. The user reopens Herdr on the new build.

Finish with a short report: the tag merged, conflicts and their resolutions, sol's findings (applied or rejected), test and install results, the core versions (`ssh multivac hc machine ls`).

## Hard stops (report, don't force)

A dirty tree or failed fetch/pull in step 1; tests or the build still red after an honest fix; a rejected push. Never `--force`, never rebase `master`, never push tags.
