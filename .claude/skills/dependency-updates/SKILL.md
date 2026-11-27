---
name: dependency-updates
description: Turn the open Dependabot pull requests into one pull request against the development line, with the generated files renewed. Use when Dependabot pull requests are open or fail CI, or when the weekly dependency update is due.
---

# Dependency updates

Dependabot (`.github/dependabot.yml`) opens one grouped pull request per
ecosystem every Monday, plus single security updates at any time. They target
`main`, which is the only branch Dependabot reads, and **none of them is ever
merged**: they are raw material. This procedure carries their commits onto the
development line in one pull request, renews the files the updates make stale,
and closes the Dependabot pull requests it replaces.

Run every command from the repository root. Shell variables do not carry over
between commands, so start each command that uses `$base` with this line:

```sh
base=$(git branch -r --list 'origin/v[0-9]*' --sort=-version:refname | head -1 | sed 's#^ *origin/##')
```

`$base` is the development line (`v0.14` at the time of writing).

## 1. Take stock

```sh
git fetch origin --tags --prune
gh pr list --author app/dependabot --state open --limit 100 --json number,title,url
gh pr list --state open --json number,headRefName,url --jq '.[] | select(.headRefName | test("^chore/gel-[0-9]+-dependency-updates$"))'
```

- No open Dependabot pull request: stop; report "no Dependabot pull requests open".
- An earlier dependency-update pull request still open (second command prints
  one): stop; report its URL. Dependabot repeats everything that pull request
  did not land, so nothing is lost by waiting for its merge.
- Two open Dependabot pull requests that update the same dependency in the
  same directory: keep the higher number; close the other with
  `gh pr close <n> --comment "Superseded by #<higher>."`.

The rest are **the batch**. For each, note its updates from its commit
message: one line `Updates \`<name>\` from <old> to <new>` per update, and
the directory from the title or the "Bumps ... in the /<dir> directory" lines.

```sh
gh pr view <n> --json commits,title,body --jq '.title, (.commits[].oid)'
git fetch origin "pull/<n>/head" && git log -1 --format=%B FETCH_HEAD
```

## 2. Issue and branch

All work has a Linear issue (AGENTS.md "Planning"). Title:
`Dependency updates $(date +%G-W%V)`, e.g. `Dependency updates 2026-W48`.

- **With the Linear tools** (`mcp__claude_ai_Linear__save_issue`): look for
  an issue with that title first (`list_issues`, project `Gelabber`, query
  the title) and reuse it. Otherwise create it: team `GEL`, project
  `Gelabber`, label `Improvement`, assignee `me`, state `In Progress`, and
  the milestone printed by

  ```sh
  base=$(git branch -r --list 'origin/v[0-9]*' --sort=-version:refname | head -1 | sed 's#^ *origin/##')
  last=$(git tag --list "$base.*" --sort=-v:refname | head -1)
  if [ -z "$last" ]; then echo "$base"; else echo "${last%.*}.$(( ${last##*.} + 1 ))"; fi
  ```

  (no such milestone in the project: leave it empty and say so in the report).
  Description: one sentence on what the issue is, then the Dependabot pull
  requests by bare number and their updates, e.g.
  `1050 (ndarray 0.15.6 → 0.17.2 in /desktop)`. Write the numbers without
  `#`: Linear turns `#1050` into a link to that pull request.
- **Without Linear:** `gh issue create --title "<title>" --label Improvement --body "<same description>"`,
  then stop and report the issue URL: the branch needs the `GEL-<n>` that
  someone with Linear access reads off the synced issue.

```sh
base=$(git branch -r --list 'origin/v[0-9]*' --sort=-version:refname | head -1 | sed 's#^ *origin/##')
git switch -c chore/gel-<n>-dependency-updates "origin/$base"
```

## 3. Apply the updates: cherry-pick

For each pull request of the batch, lowest number first:

```sh
git fetch origin "pull/<n>/head"
git cherry-pick -x $(gh pr view <n> --json commits --jq '.commits[].oid')
```

Cherry-picking is the method, not re-running update commands: it is the same
for all five ecosystems (Docker, Compose and the Actions pins have no update
command), it carries exactly the versions Dependabot resolved, and a
`cargo update -p` or `npm install` of its own may move other dependencies
too, which the pin rule in AGENTS.md forbids. The lock files of `main` and the
development line differ only where development work changed dependencies, so
the three-way merge applies cleanly in the usual case.

When the cherry-pick stops:

- **"The previous cherry-pick is now empty"**: the development line has those
  versions already (Dependabot reads `main`, so it repeats updates until the
  next release brings the development line there). `git cherry-pick --skip`.
  The pull request still counts as replaced.
- **Conflicts only in lock files** (`git diff --name-only --diff-filter=U`
  lists nothing but `backend/Cargo.lock`, `desktop/Cargo.lock`,
  `web/package-lock.json`): keep the development line's file and move it to
  the pull request's versions:

  ```sh
  git checkout --ours -- <lock file>
  # Cargo, per update of the commit message, in the directory it names:
  cargo metadata --format-version 1 --manifest-path <dir>/Cargo.toml > /dev/null
  grep -A1 '^name = "<name>"$' <dir>/Cargo.lock   # already at <new>: next update
  cargo update --manifest-path <dir>/Cargo.toml -p <name>@<old> --precise <new>
  # npm, per update:
  (cd web && npm install --package-lock-only --ignore-scripts --save-exact <name>@<new>)
  git add <lock file> && git -c core.editor=true cherry-pick --continue
  ```

  For npm, only updates listed in the commit message's `updated-dependencies`
  block with `dependency-type: direct:...`; an `indirect` one stays as the
  lock file has it, listed under "left out" in step 7.
- **Conflicts in any other file:** `git cherry-pick --abort`; this pull
  request is left out whole (section "Leaving an update out").

Done when every pull request of the batch is picked, skipped or left out, and
`git status` is clean.

## 4. Check the code

The areas are the top-level folders the branch changes:

```sh
base=$(git branch -r --list 'origin/v[0-9]*' --sort=-version:refname | head -1 | sed 's#^ *origin/##')
git diff --name-only "origin/$base"...HEAD | cut -d/ -f1 | sort -u
```

Run the checks of each area listed (they are CI's; `AGENTS.md` "Commands"):

- **backend**: from `backend/`, `cargo fmt --all -- --check`,
  `DOCS_RS=1 cargo clippy --locked --workspace --all-targets -- -D warnings`,
  then the tests as in AGENTS.md (services: the `test-services` skill).
- **desktop**: needs `GELABBER_LIBWEBRTC_DIR` set to a libwebrtc package (a
  directory with a `VERSIONS` file) and 30 GB free on the disk of the
  checkout (`df -h .`). Without either, skip these and write "desktop Rust
  checks: not run locally (no libwebrtc package / disk), CI runs them" in the
  pull request. Otherwise, with `CARGO_BUILD_JOBS=2` on a shared machine:
  `cargo fmt --manifest-path desktop/Cargo.toml --all -- --check`,
  `cargo clippy --locked --manifest-path desktop/Cargo.toml --all-targets -- -D warnings`,
  `cargo test --locked --manifest-path desktop/Cargo.toml -p gelabber-i18n -p gelabber-client -p gelabber-call -p gelabber-ui -p gelabber`.
- **web**: from `web/`, `npm ci`, `npm run lint`, `npm test`,
  `npm run test:checks`, `npm run build`.
- **deploy** and **.github**: in step 6, after the renewal.

A check that fails: section "Leaving an update out", then this step again.
Done when every check listed passes or is written down as not run, with the
reason.

## 5. Renew the generated files

Run all three every time; each writes nothing when nothing changed.

```sh
# Caddy: CI's "Install Caddy" step (ci.yml, job deploy) downloads the version
# deploy/compose/compose.yaml pins (test_ci_caddy_uses_compose_pin_...).
v=$(sed -n 's/^ *image: caddy:\([0-9.]*\)-alpine$/\1/p' deploy/compose/compose.yaml)
sed -i -E "s#releases/download/v[0-9.]+/caddy_[0-9.]+_linux_amd64#releases/download/v$v/caddy_${v}_linux_amd64#; s#(cut -d ' ' -f 1\)\" = \"v)[0-9.]+\"#\1$v\"#" .github/workflows/ci.yml

# Node: the workflows' node-version follows the web image (web/Dockerfile).
n=$(sed -n 's/^FROM node:\([0-9.]*\)-.*/\1/p' web/Dockerfile)
sed -i -E "s/^( *node-version: )[0-9.]+$/\1$n/" .github/workflows/ci.yml .github/workflows/e2e-extended.yml

# Third-party notices of the desktop app, from desktop/Cargo.lock (CI job
# "Desktop / Third-party notices"). Needs cargo and the toolchain of
# rust-toolchain.toml; fetches the crates of the lock file.
python3 desktop/packaging/third-party-notices.py
```

When `git status --short` shows changes, commit them:
`git commit -am "chore(deps): renew the generated files (GEL-<n>)"`.

## 6. Check the generated files

```sh
python3 desktop/packaging/third-party-notices.py --check
python3 -m unittest discover -s desktop/packaging/tests
python3 -m unittest discover -s deploy/compose/tests
python3 -m unittest discover -s deploy/tools/tests
```

When the branch changes `.github/`: also the steps "Check workflows with
zizmor" and "Check workflows with actionlint" of the job `deploy` in
`.github/workflows/ci.yml`, their `run:` scripts with their `env:` values,
and `GH_TOKEN=$(gh auth token)` for zizmor.

`test_makensis_compiles_it` fails without `makensis` (`apt-get install nsis`);
where it cannot be installed, write it down as not run. Everything else must
pass. Done when it does.

## 7. Pull request

```sh
git push -u origin HEAD
```

(Plain `git push`, with no pipe or `2>&1`: a hook checks the branch name.)

Title: `chore(deps): dependency updates <YYYY>-W<ww>`. Body file:

```markdown
Fixes GEL-<n>

Dependency updates from Dependabot, carried onto the development line
(`.claude/skills/dependency-updates/SKILL.md`).

| Dependabot PR | Update | Directory |
| --- | --- | --- |
| #<n> | <name> <old> → <new> | /<dir> |

**Left out:** <name> <old> → <new> (#<n>): <the first error line>. Its
Dependabot pull request stays open. (Or: "Nothing.")

**Renewed:** <the generated files that changed, or "nothing">.

**Checks:** one line per command run, with its result (`passed`, `failed: …`,
`not run: <reason>`).
```

```sh
base=$(git branch -r --list 'origin/v[0-9]*' --sort=-version:refname | head -1 | sed 's#^ *origin/##')
gh pr create --base "$base" --title "<title>" --body-file <file>
```

Set the Linear issue to `In Review` (`save_issue`, state `In Review`).

## 8. Close the Dependabot pull requests it replaces

Each pull request of the batch whose updates are all in the new pull request
(picked or skipped as already there):

```sh
gh pr close <n> --comment "Replaced by <URL of the new pull request>, which carries this update onto the development line."
```

A pull request with an update left out stays open; comment on it instead:
`gh pr comment <n> --body "<URL> carries <the updates taken>; <left-out update> is left out: <reason>."`.
Close nothing else.

## 9. Report

The new pull request's URL, the updates taken and left out, the checks with
their results, and the Dependabot pull requests closed and left open.

## Leaving an update out

An update that makes a check of step 4 fail stays out; everything else goes
ahead.

1. **Find it.** The update is the one whose dependency name appears in the
   first error (e.g. `` multiple different versions of crate `ndarray` ``,
   or a path `…/registry/src/…/<name>-<new>/`). No name or several names
   match: drop whole Dependabot pull requests instead, last picked first,
   rerunning the failing check after each, until it passes; the last one
   dropped is left out.
2. **Drop it.**
   - The update is its pull request's only one (or the whole pull request
     goes): remove its commits, `git rebase --onto <first commit>^ <last commit>`
     (the hashes on this branch, `git log --oneline`). On a conflict:
     `git rebase --abort`, then `git revert --no-edit <commits>`.
   - One update of several: a commit that puts it back to `<old>`.
     Cargo: restore its version requirement in each `Cargo.toml` the commit
     changed (`git diff "origin/$base" -- <dir>`), then
     `cargo update --manifest-path <dir>/Cargo.toml -p <name>@<new> --precise <old>`.
     npm: `(cd web && npm install --package-lock-only --ignore-scripts --save-exact <name>@<old>)`.
     Docker, Compose, Actions: set the lines that name it back to their text
     in `origin/$base` (`git diff "origin/$base" -- <file>`).
     Commit: `chore(deps): leave out <name> <new> (GEL-<n>)`.
3. **Record it**: the first error line goes into the pull request (step 7),
   and its Dependabot pull request stays open (step 8).
