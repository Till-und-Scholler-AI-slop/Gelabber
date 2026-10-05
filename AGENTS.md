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
- Hotfixes for the shipped minor (`0.4.x`) go to `main` (or a hotfix branch), followed by a sync PR to `v0.4`. Older release lines require an explicitly scoped maintenance branch; do not mix their fixes into the current line unless Rafael says so.
- After every merged hotfix on `main`, promptly open a sync PR from `main` (or a dedicated sync branch containing it) to the current `v*` branch. Preserve the hotfix commits so ancestry proves the sync; require the usual review and CI before merging. Feature branches then merge the updated version branch.
- Keep fixes independently reviewable. Review and merge each focused fix before a release PR bundles it; a release bundle does not replace the individual reviews.

## Product

No LiveKit, Daily, Agora, Twilio, Stream, or Socket.IO. Own WebSocket, own signaling, own SFU. Keep the locked stack pins.

SilasSch reviews product PRs when that is the standing rule. Do not invent extra process.

## Current development line: v0.4

Call sounds are developed on feature branches from `v0.4`.
Product PRs target `v0.4`. The active **Version branches** ruleset covers
`refs/heads/v[0-9]*` and requires PRs with the same review requirements as `main`.
Do not push feature commits directly to `v0.4`. Release the completed line only
from `main` after its version PR, CI and image publishing have finished.

## Shipped line: latest released v0.4.x

The completed `v0.4` line ships as `v0.4.0`. The latest published `v0.4.x` tag on
`main` identifies the shipped version. Hotfixes for this minor go to `main`,
followed by a sync PR to `v0.4`; a feature-branch version is not a release.

The shipped scope includes configurable call join/leave and mute/deafen sounds,
the Living Room interface and persistent call dock,
account themes, configurable stream quality, fullscreen viewing, and separate
screen/Go Live audio, alongside existing permissions, chat, direct messages,
screen share, Go Live and moderation.

Hotfix work for the shipped line stays on `main`; future major product work uses
its own protected version branch.

The old `v0.2` development branch is frozen for historical reference. The active
`Frozen v0.2` ruleset blocks updates and deletion with no bypass actors; releases
remain available through their tags. Do not target new PRs at `v0.2`.
