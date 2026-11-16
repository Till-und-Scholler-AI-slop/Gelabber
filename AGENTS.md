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
  e.g. `fix/123-reconnect-after-sleep` or `feature/124-message-search`. The
  type follows the issue: `feature/` (Feature), `fix/` (Bug), and for a Task
  the closest of `docs/`, `ci/`, `refactor/` or `chore/` (dependencies,
  cleanup). Without an issue, open one first (see Planning). Release work
  uses `release/vX.Y.Z`, `hotfix/vX.Y.Z-<short-description>` and
  `sync/main-into-vX.Y`. Only version branches start with `v`. Dependabot
  keeps its own names.
- **Features:** branch off the development line; the PR targets it. Older
  version branches are complete.
- **Releases:** a release branch from the development line with `origin/main`
  merged in, the version bump and `.github/release-notes/vX.Y.Z.md`, opened as
  a PR to `main`. Merging it releases: after CI and image publishing on
  `main`, `release.yml` tags and publishes the new version by itself. Merge
  nothing else into `main` until that release is out: a later merge cancels
  the release commit's CI, and `image_set.py` then never promotes the version
  (recover by re-running CI on `main`'s head). Then sync `main` back into the
  development line by PR.
- **Hotfixes** for the shipped line: PR to `main`, then a sync PR from `main`
  to the development line that keeps the hotfix commits, so ancestry proves the
  sync.
- **Each fix** gets its own reviewed PR before a release PR bundles it.
- **Protection:** the `Version branches` ruleset covers `refs/heads/v[0-9]*`
  (PR required, same reviews as `main`); every change reaches `main` and `v*`
  through a PR. A new version branch name must match that pattern, or gets the
  same protection by hand before its first commit.
- **Review:** a product PR is reviewed by a maintainer other than the person
  who asked for the work.

## Planning

Work is tracked in the organization project
[Gelabber](https://github.com/orgs/Till-und-Scholler-AI-slop/projects/2).

- **Issue first:** all work has an issue. If there is none, open one (type
  Bug, Feature or Task) before the branch.
- **Assignee** means "working on it now". When you start, assign the issue to
  the person who asked for the work, not to the author of the PR. When you
  stop without finishing, remove the assignee and say on the issue why.
- **Milestone** is the release the work ships in. Use an open milestone; if it
  is unclear which one, ask. Create, rename or close milestones only when
  asked or as part of a release (below).
- **Pull requests** say `Closes #<n>` for their issue, so merging the PR
  closes it, into a version branch too.
- **Status** on the project follows the work, and keeping it current is part
  of the work: **In progress** while someone works on the issue, **In review**
  once its PR is ready for review, **Done** when the PR is merged.
  `.github/workflows/project-status.yml` moves the card from the assignee and
  from the PR's `Closes #<n>`. After assigning and after marking the PR ready,
  check the issue's Status; if it did not move, run that workflow by hand
  (`workflow_dispatch` with the issue number and the Status), and if that
  fails too, say so on the issue.
- **Each milestone** states the release goal in its description, in one to
  three sentences. After a release: close its milestone, move open issues to
  the next one, write that one's goal.

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

Web (`web/`, Node from `engines` in `package.json`): `npm run lint`,
`npm test`, `npm run test:checks`, `npm run build`.

Desktop (`desktop/`) is its own Cargo workspace and needs a prebuilt libwebrtc
package; see `desktop/README.md` ("Native app" for the native egui app's build,
smoke test, parity and packages).

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
  assets, never a cloud audio service.
- **Pins:** keep the locked versions; a change updates only the dependencies it
  needs.
- **Performance claims** state only what was measured. Historical benchmark
  FAIL/BLOCKED results stay as recorded.
