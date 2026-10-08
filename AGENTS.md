# AGENTS.md

Mandatory for every coding agent. Read this before changing the repo.

## Version branches (standing)

Name each major-cut branch `v<major>`: `v0.2`, `v0.3`, `v1.0`, ….

Work for the **current** major happens on that `v*` branch, **not** on `main`.

When starting a new `v*` branch, copy this setup **immediately**. Do not leave an unprotected `v*` branch.

Preferred protection: a repo ruleset whose pattern covers every version branch (`refs/heads/v[0-9]*`), same effect as on `v0.3`:

- Require a **pull request**. No direct pushes (`git push origin v0.3` of feature commits is forbidden; same for `main` and every future `v*`).
- Do not require extra approvers beyond whatever the repo already needs for own-PRs.
- Require CI status checks only if `main` already requires them.

If a new `v*` name would miss that pattern, add the same protection by hand before any product commits.

## Pull requests

- Feature work uses feature branches off the current `v*` (e.g. `rft/<short>-01d0`, or Silas’s usual names). **PRs target that `v*` branch, not `main`.**
- Merge a feature into the current `v*` when that feature is done and fixed (CI green, review as usual).
- Merge `v*` → `main` **via a PR** only when the **whole** major is done and fixed. Then cut a **release** (tag `v0.2.0` etc.). Do not tag or release from a half-finished major or from a random feature branch.
- Hotfixes for the currently shipped minor go to `main` (or a hotfix branch), followed by a sync PR to the current `v*` branch. The latest published tag on `main` identifies that shipped line. Older release lines require an explicitly scoped maintenance branch; do not mix their fixes into the current line unless Rafael says so.
- After every merged hotfix on `main`, promptly open a sync PR from `main` (or a dedicated sync branch containing it) to the current `v*` branch. Preserve the hotfix commits so ancestry proves the sync; require the usual review and CI before merging. Feature branches then merge the updated version branch.
- Keep fixes independently reviewable. Review and merge each focused fix before a release PR bundles it; a release bundle does not replace the individual reviews.

## Product

No LiveKit, Daily, Agora, Twilio, Stream, or Socket.IO. Own WebSocket and own product signaling. Keep the locked stack pins.

For v0.4, Rafael explicitly selected self-hosted mediasoup with its official Rust
binding on 2026-10-06 and authorized full replacement of the existing SFU. There
is no runtime backend selector or old-SFU fallback. Benchmark selection and new
load tests are not prerequisites of this instruction. Preserve historical
benchmark FAIL/BLOCKED results; do not claim calibrated latency or performance
acceptance from the selection. Keep product tickets, ACL/revocation, Watch,
Go Live and separate source lifetimes under Gelabber's own gateway. See
`docs/v0.4-mediasoup-migration.md` for build, functional checks and the coherent
previous-application rollback boundary. Only necessary mediasoup dependencies
change; preserve unrelated stack pins. Browser-local DSP uses pinned,
self-hosted RNNoise assets. Do not introduce a cloud audio service.

SilasSch reviews product PRs when that is the standing rule. Do not invent extra process.

Rafael explicitly waived human review for the finished v0.4 feature integration
on 2026-10-06. This exception does not waive PRs or successful technical checks.
Restore the normal review protection after that integration. He also excluded
the one-hour call test; do not run it or record it as passed.

The subsequent mediasoup implementation remains feature-PR work targeting
`v0.4`. Rafael allows skipping human review; successful technical checks and
PR protection still apply. Physical iOS/Omarchy acceptance remains deferred.
This instruction does not authorize main merges, release or deployment.

## Current development line: v0.5

`v0.5` was cut from `main` (`v0.4.0` plus the shipped-docs update) on
2026-10-07 at Silas' request and is protected by the **Version branches**
ruleset (`refs/heads/v[0-9]*`: PRs required, same review requirements as
`main`). Major product work for the next line, starting with the native
desktop client (issue #165), targets `v0.5` through feature PRs. Do not target
new feature PRs at `v0.4`. Release a completed line only from `main` after its
version PR, CI and image publishing have finished.

On 2026-10-08 Silas decided to release `v0.5.0` with the Linux desktop app
(#170); Windows and the remaining steps of #165 follow on the next line.

## Shipped line: latest published main tag

The latest published release is currently `v0.5.0`. Hotfixes for the shipped
minor go to `main`, followed by a sync PR to the current development branch once
one exists; a feature-branch version is not a release.

The existing product includes the Living Room interface and persistent call dock,
account themes, configurable stream quality, fullscreen viewing, and separate
screen/Go Live audio, alongside existing permissions, chat, direct messages,
screen share, Go Live and moderation. Configurable join/leave and mute/deafen
sounds, unread state, search, drafts, reactions, local RNNoise audio processing
and the mediasoup media backend shipped with `v0.4.0`. The native Linux desktop
app (Tauri shell, native libwebrtc/libmediasoupclient media core, pacman
repository) shipped with `v0.5.0`.

Hotfix work for the shipped line stays on `main`; future major product work uses
its own protected version branch.

The old `v0.2` development branch is frozen for historical reference. The active
`Frozen v0.2` ruleset blocks updates and deletion with no bypass actors; releases
remain available through their tags. Do not target new PRs at `v0.2`.
