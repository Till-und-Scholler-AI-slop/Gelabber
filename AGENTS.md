# AGENTS.md

Mandatory for every coding agent. Read this before changing the repo.

## Version branches (standing)

Name each major-cut branch `v<major>`: `v0.2`, `v0.3`, `v1.0`, ….

Work for the **current** major happens on that `v*` branch, **not** on `main`.

When starting a new `v*` branch, copy this setup **immediately**. Do not leave an unprotected `v*` branch.

Preferred protection: a repo ruleset whose pattern covers every version branch (`refs/heads/v[0-9]*`), same effect as on `v0.2`:

- Require a **pull request**. No direct pushes (`git push origin v0.2` of feature commits is forbidden; same for `main` and every future `v*`).
- Do not require extra approvers beyond whatever the repo already needs for own-PRs.
- Require CI status checks only if `main` already requires them.

If a new `v*` name would miss that pattern, add the same protection by hand before any product commits.

## Pull requests

- Feature work uses feature branches off the current `v*` (e.g. `rft/<short>-01d0`, or Silas’s usual names). **PRs target that `v*` branch, not `main`.**
- Merge a feature into the current `v*` when that feature is done and fixed (CI green, review as usual).
- Merge `v*` → `main` **via a PR** only when the **whole** major is done and fixed. Then cut a **release** (tag `v0.2.0` etc.). Do not tag or release from a half-finished major or from a random feature branch.
- Hotfixes for already-shipped `0.1.x` may still go to `main` (or a hotfix branch). Do not mix them into unfinished current-major product work unless Rafael says so.

## Product

No LiveKit, Daily, Agora, Twilio, Stream, or Socket.IO. Own WebSocket, own signaling, own SFU. Keep the locked stack pins.

SilasSch reviews product PRs when that is the standing rule. Do not invent extra process.

## Current cut: v0.2

Branch `v0.2`. Seed tickets: GitHub issues [#58](https://github.com/Till-und-Scholler-AI-slop/Gelabber/issues/58)–[#62](https://github.com/Till-und-Scholler-AI-slop/Gelabber/issues/62), [#68](https://github.com/Till-und-Scholler-AI-slop/Gelabber/issues/68), [#69](https://github.com/Till-und-Scholler-AI-slop/Gelabber/issues/69). Cut includes roles, search, threads, screen/tab audio, friends, moderation, reactions.
