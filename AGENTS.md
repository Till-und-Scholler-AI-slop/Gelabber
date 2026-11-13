# AGENTS.md

Mandatory for every coding agent. Read this before changing the repo.

## Hosting

`origin` is GitHub (`Till-und-Scholler-AI-slop/Gelabber`). Push there and use
`gh` for PRs. Releases are published by `release.yml` (see below).

## Branches and pull requests

The **development line** is the newest version branch on `origin`:
`git branch -r --list 'origin/v[0-9]*' --sort=-version:refname | head -1`.
The **shipped line** is the latest release tag on `main`.

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
  closes it.
- **Each milestone** states the release goal in its description, in one to
  three sentences. After a release: close its milestone, move open issues to
  the next one, write that one's goal.

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

Keep CI lean. Pull requests check only what is needed to merge safely.

- **Pull requests:** fmt, clippy/lint, tests, build, and one representative integration check per area. Aim for under 15 minutes per workflow with a warm cache.
- **Areas:** each top-level folder is one area (`backend/`, `web/`, `desktop/`, `deploy/`). `ci.yml` runs only the areas a pull request touches; `.github/workflows/scripts/changes.py` maps paths to areas. `ci-ok` is the one required check; a new job joins its `needs`.
- **After merge (push to `main`/`v*`) and releases:** packaging, installers, package repositories, signing and uploads.
- **Manual diagnostics** (fault matrices, soak and stability loops) go in their own workflow, started by hand or by a PR label, outside the PR gate (see `e2e-extended.yml`: label `e2e-extended`).
- **Lists of scripts** go behind one entry point (e.g. `npm run test:checks`). CI and developers run the same command.
- **New check:** say what it catches that existing checks miss. Add it to an existing job unless a parallel job saves wall time. Remove checks that a new one replaces.
- **Every workflow** sets top-level `permissions`, by default `contents: read`, with jobs adding only what they need. Every job sets `timeout-minutes`. Actions stay pinned to commit SHAs (`deploy/compose/tests/test_workflow_pins.py`).
- **Caches** are saved only by pushes (`save-if`, `cache-to`). Pull requests only read them, because per-PR copies exceed the repository's cache quota.
- **Slow build outputs** that the cache quota would evict are kept where nothing evicts them: the libwebrtc packages are assets of the `libwebrtc-packages` release, stored by pushes and fetched on a cache miss (`desktop-native.yml`); the app images' layer cache is each image's `buildcache` tag in the registry, written by `publish-images.yml`.
- **A release build** (`desktop-native.yml` with `release_tag`) builds, packages, makes one loopback call and tests the installed package, but runs the other code checks no more: CI on `main` ran them on the same commit.
- **`libwebrtc-packages` and `build-deps` stay pre-releases.** If one became "Latest", `releases/latest/download/latest-native.json` (the apps' updater) and the web client's download links would point at it. A stored libwebrtc package is fetched only when `desktop/native/libwebrtc-packages.sha256` lists its SHA-256.
- **Runners** are named by version, never `-latest`.
- **Comments** in workflows are one or two lines saying why. Longer reasoning and history go in commit messages or `docs/`.

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
