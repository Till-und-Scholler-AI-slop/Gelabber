# AGENTS.md

Mandatory for every coding agent. Read this before changing the repo.

## Hosting

`origin` is GitHub (`Till-und-Scholler-AI-slop/Gelabber`). Push there and use
`gh` for PRs. Releases are published by `release.yml` (see below).

## Branches and pull requests

The **development line** is the newest version branch on `origin`:
`git branch -r --list 'origin/v[0-9]*' --sort=-version:refname | head -1`.
The **shipped line** is the latest release tag on `main`.

- **Branch names** say what kind of work the branch holds, not who or what
  wrote it: `<type>/<issue>-<short-description>`, lowercase with hyphens,
  where `<issue>` is the Linear ID, e.g. `fix/gel-21-edit-under-block` or
  `feature/gel-30-message-search`. The ID in the name links the branch and
  its pull request to the Linear issue. The type follows the issue's label:
  `feature/` (Feature), `fix/` (Bug), and otherwise the closest of `docs/`,
  `ci/`, `refactor/` or `chore/` (dependencies, cleanup). Without an issue,
  open one first (see Planning). Release work uses `release/vX.Y.Z`,
  `hotfix/vX.Y.Z-<short-description>` and `sync/main-into-vX.Y`. Only
  version branches start with `v`. Dependabot keeps its own names.
- **Features:** branch off the development line; the PR targets it. Older
  version branches are complete.
- **Releases:** a release branch from the development line with `origin/main`
  merged in, the version bump and `.github/release-notes/vX.Y.Z.md`, opened as
  a PR to `main`. Merging it releases: after CI and image publishing on
  `main`, `release.yml` tags and publishes the new version by itself. Merge
  nothing else into `main` until that release is out: `image_set.py` promotes
  the version only while `main` still points at the release commit, and a
  merge while that commit's CI run still waits to start replaces the run.
  Either way the version is not promoted (recover by re-running CI on
  `main`'s head). Then sync `main` back into the
  development line by PR.
- **Hotfixes** for the shipped line: PR to `main`, then a sync PR from `main`
  to the development line that keeps the hotfix commits, so ancestry proves the
  sync.
- **Each fix** gets its own PR before a release PR bundles it.
- **Protection:** the rulesets `Main Protection` and `Version branches`
  (`refs/heads/v[0-9]*`) require a PR and a green `ci-ok`, and block force
  pushes and deletion. Nobody bypasses them, admins included. A new
  version branch name must match that pattern, or gets the same
  protection by hand before its first commit. The ruleset
  `Merge queue (version branches)` is disabled: the queue made each merge
  wait for its own CI run (GEL-362).
- **Merge and review:** a maintainer (write access or more) merges their own
  PR once `ci-ok` is green; no approval from the other maintainer is
  required. The maintainer reviews the change first, with AI if they
  like, and an agent merges only when that maintainer says so. Others
  contribute through forks.

## Planning

Work is planned in the Linear project
[Gelabber](https://linear.app/gelabber/project/gelabber-e58336264dfd)
(team `GEL`). Linear is the one place for status, assignee and milestone.
Each Linear issue is synced both ways with a GitHub issue in this repository
(title, description, comments, open or closed), so people without Linear can
still read and report on GitHub. Plan in Linear, not on GitHub: GitHub
milestones and the old GitHub project board are not kept up to date.

- **Issue first:** all work has a Linear issue in the project Gelabber, with
  the label Bug, Feature or Improvement. If there is none, create it in
  Linear before the branch. An issue opened on GitHub appears in Linear by
  itself; add it to the project there. Without Linear access (most agent
  sessions), open the GitHub issue and ask someone with Linear access for
  the `GEL-<n>` it gets there and to set its project, milestone and
  assignee.
- **Assignee** means "working on it now". When you start, assign the issue to
  the person who asked for the work, not to the author of the PR. When you
  stop without finishing, remove the assignee and say on the issue why.
- **Milestone** is the release the work ships in, as a milestone of the
  Linear project. Use an open one; if it is unclear which one, ask. Create,
  rename or complete milestones only when asked or as part of a release
  (below).
- **Pull requests** link their issue: the Linear ID in the branch name, and
  `Fixes GEL-<n>` in the description. Linear then moves the issue with the
  pull request and closes it on merge, into a version branch too; the GitHub
  issue closes through the sync. A pull request that should not close its
  issue says `Part of GEL-<n>` instead.
- **Status** follows the work, and keeping it current is part of the work:
  **In Progress** while someone works on the issue, **In Review** once its
  PR is ready for review, **Done** when the PR is merged. Linear's team
  automation moves it from the linked pull request. After opening the PR and
  after marking it ready, check the issue's status in Linear and set it by
  hand if it did not move.
- **Each milestone** states the release goal in its description, in one to
  three sentences. After a release: complete its milestone, move open issues
  to the next one, write that one's goal.

## Procedures

Repeatable procedures are skills in `.claude/skills/<name>/SKILL.md`; the
`description` of each says when it applies. Before such a procedure, read its
skill and follow it. When a procedure changes, change its skill in the same
pull request.

## Commands

Rust workspace in `backend/` (`api`, `media`, `shared`), from `backend/`:

- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets -- -D warnings`
- Tests run in the native toolchain image, as in CI. From the repo root, build
  it once with
  `docker build --target base --file backend/media/Dockerfile --tag gelabber-native-ci-toolchain:local .`,
  then `sh backend/build/run-native-ci.sh test --locked --workspace --all-targets`
  (CI runs the same binaries with `nextest run`, side by side).
  They need Postgres, Redis and MinIO plus `DATABASE_URL`, `REDIS_URL`,
  `MINIO_ENDPOINT`, `MINIO_ROOT_USER` and `MINIO_ROOT_PASSWORD` (values in
  `backend/api/.env.example`); storage and avatar tests fail without the MinIO
  ones. The database role needs `CREATEDB` for `sqlx::test`.

Deployment tests need the PyYAML that `deploy/compose/tests/requirements.txt`
pins, installed with a Python that has `pip` (Debian's needs `python3-pip`):

```
python3 -m pip install --require-hashes --target ~/.cache/gelabber-pyyaml -r deploy/compose/tests/requirements.txt
PYTHONPATH=~/.cache/gelabber-pyyaml python3 -m unittest discover -s deploy/compose/tests
```

Web (`web/`, Node from `engines` in `package.json`): `npm run lint`,
`npm test`, `npm run test:checks`, `npm run build`. `npm test` mounts the
components of `*.browser.test.tsx` in Chromium; install it once with
`npx playwright install chromium-headless-shell`.

Desktop (`desktop/`) is its own Cargo workspace and needs a prebuilt libwebrtc
package; see `desktop/README.md` ("Native app" for the native egui app's build,
smoke test, parity and packages).

## Tests

These rules hold for every test you add and every line you change in one;
the rules on failing and flaky tests hold for all tests.

- **A test** states behaviour that a person or another program relies on,
  and fails when that behaviour breaks: break the code once and watch the
  test fail. A test that repeats a constant, reads back its own mock, or
  compares source text where a lint rule can do the job catches nothing;
  leave it out.
- **A fix** comes with a test that fails without it, where the code can be
  tested. The pull request shows that failure: the test's output with the
  fix taken out, or a red CI run of a commit that holds only the test (let
  it finish before you push the fix).
- **A failing test** is made to pass by fixing the code. Never delete, skip
  or loosen a test to get a change green. Change its expectation only when
  the behaviour changed on purpose, and say so in the pull request. If a
  test contradicts the task or cannot pass honestly, stop and report it.
- **Timing:** functional tests wait for an event or a state, or use a clock
  the test controls; they never synchronise with a `sleep` or assert a
  wall-clock bound. A check that something does not happen first waits for
  an event that proves the work finished; only where none exists may it
  wait a short fixed window. A raised timeout states its cause in the
  commit message.
- **Prerequisites:** a test whose service, tool or file is missing fails
  when `CI` is set and may skip only without it. `#[ignore]`, `.skip` and
  their kin name the issue (`GEL-<n>`) that ends them or the workflow that
  runs them; a skip by platform names the platform.
- **Where tests run:** every test that is not ignored runs in a workflow on
  pull requests or pushes; manual diagnostics follow
  `.claude/rules/ci-workflows.md`. Check that a new test ran in the pull
  request's CI.
- **Compatibility code** for an older version says in a comment which
  version removes it (`remove in vX.Y`); its tests are removed with it.
- **Flaky tests:** when a test your change does not touch fails, add a link
  to the failing run to its open issue, or open one, before you re-run it.

## Supported versions

A minor version covers all its patch releases: v0.13.1 is v0.13. N is the
minor version the development line ships, N-1 the one before it. Before
1.0 a minor version is `0.Y`; 1.0 restates this section.

- **Clients:** the server supports web and desktop clients of N and N-1.
  Code kept only for older clients is removed.
- **Web and API** ship as one image set. The web client assumes a server of
  its own version and carries no tolerance for an older one.
- **Desktop app and server** may be one minor version apart, either way: an
  app v0.14 works with servers v0.13, v0.14 and v0.15. Servers of any other
  minor version are not supported. The apps follow the newest release (the
  Windows app updates itself every 6 hours, Arch gets it with
  `pacman -Syu`), and every server's download links point there, so a
  server stays at most one minor version behind the newest release.
- **Server upgrades** are supported from N-1 in one step. An operator on an
  older version upgrades through each minor version in between
  (`deploy/README.md`, "Supported upgrades").
- **Expiry:** compatibility code names the first minor version that no
  longer supports what it serves: code for v0.13 clients or servers says
  `remove in v0.15` (see "Tests").
  `deploy/compose/tests/test_compat_markers.py` fails once
  `backend/Cargo.toml` reaches that version.
- **Data:** the database migrations stay one chain; do not squash them.
  Data in the database is changed by a migration, never by code that
  expires: rows that old versions wrote stay readable. Code that reads what
  an older version left outside the database (Redis, browser storage, the
  app's settings files) is compatibility code with a marker.

## CI workflows (`.github/workflows`)

The rules for workflows are in `.claude/rules/ci-workflows.md`. Read them
before you change a workflow.

## Product

- **Signaling** is Gelabber's own WebSocket protocol. No LiveKit, Daily, Agora,
  Twilio, Stream or Socket.IO.
- **Media** is self-hosted mediasoup through its official Rust binding, the
  only SFU, with no backend selector or fallback. Build, functional checks and
  rollback: `docs/v0.4-mediasoup-migration.md`.
- **Gelabber's gateway** owns tickets, ACL/revocation, Watch, Go Live and the
  separate source lifetimes.
- **Audio processing** runs in the client with pinned, self-hosted RNNoise
  and DeepFilterNet3 assets, never a cloud audio service.
- **Pins:** keep the locked versions; a change updates only the dependencies it
  needs.
- **Performance claims** state only what was measured. A recorded result,
  FAIL and BLOCKED included, is never edited afterwards. A record that is no
  longer needed is deleted whole; the git history keeps it.
