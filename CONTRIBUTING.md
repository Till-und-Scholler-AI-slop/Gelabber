# Contributing to Gelabber

Thanks for looking into Gelabber. Two people maintain it, so small, focused
changes are reviewed fastest.

## Reporting a bug

Open an [issue](https://github.com/Till-und-Scholler-AI-slop/Gelabber/issues)
with:

- the Gelabber version (the release or image tag you run),
- the client (browser and version, or desktop app on Linux or Windows),
- what you did, what you expected and what happened,
- for call problems: the diagnostics export from the call settings.

A security problem does not belong in a public issue: see
[SECURITY.md](SECURITY.md).

## Proposing a change

For anything larger than a fix, open an issue first and describe what you
want to change and why. A few decisions are settled, and a pull request
against them will not be merged:

- Signaling is Gelabber's own WebSocket protocol. No LiveKit, Daily, Agora,
  Twilio, Stream or Socket.IO.
- mediasoup is the only SFU, with no fallback or backend selector.
- Audio processing runs in the client with self-hosted RNNoise assets.
- Dependency versions are pinned exactly. A change updates only the
  dependencies it needs.
- The two clients stay at parity: a feature of the web client gets its
  counterpart in the desktop app.

## Planning

Issues are planned in the project
[Gelabber](https://github.com/orgs/Till-und-Scholler-AI-slop/projects/2).
Each release is a milestone whose description states its goal, and the
assignee of an issue is the person working on it right now. The rules are in
[AGENTS.md](AGENTS.md#planning).

## Setting up

[README.md](README.md) has the quick start and the development commands;
[desktop/README.md](desktop/README.md) covers the desktop app, which needs
a prebuilt libwebrtc package.

## Branches and pull requests

- The **development line** is the newest version branch (`v0.10`, `v0.11`,
  …). Branch off it and open your pull request against it. `main` holds
  what is released and only receives release and hotfix pull requests.
- One pull request per fix or feature.
- A maintainer reviews every pull request before it is merged. Changes
  reach `main` and the version branches only through pull requests.
- Commit messages say what changed, in the style of the history:
  `<area>: <what changed>`, then a few lines on why.

[AGENTS.md](AGENTS.md) holds the full rules for branches, releases and CI.
It is written for coding agents and binds people the same way.

## Before you open a pull request

Run the checks of the area you touched:

| Area | Commands |
|---|---|
| Backend (`backend/`) | `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, the tests as described in [AGENTS.md](AGENTS.md) |
| Web (`web/`) | `npm run lint`, `npm test`, `npm run test:checks`, `npm run build` |
| Desktop (`desktop/`) | `cargo fmt`, `cargo clippy` and `cargo test` as in [desktop/README.md](desktop/README.md) |
| Deployment and workflows | `python3 -m unittest discover -s deploy/compose/tests` |

CI runs the areas a pull request touches; `ci-ok` is the one required
check.

A few habits of this repository:

- A fix comes with a test that fails without it, where the code can be
  tested.
- Both clients speak German and English: interface text goes into the
  catalogs in both languages ([docs/i18n.md](docs/i18n.md)). The German is
  informal ("du"); match the wording around it.
- Say only what was measured about performance.
- Shipped database migrations are never edited; add a new one.

## License

By contributing you agree that your contribution is licensed under the
[MIT License](LICENSE).
