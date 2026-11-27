# Scripts

Every check here runs in a workflow; every tool is started by hand for a use
the document named in the table describes. Run all of them from `web/`.

## Checks

| What                                                                                                                                                                                                                  | Where it runs                                                                                    | Needs                                                                |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------- |
| `npm run test:checks` (`checks.mjs`): the E2E harness checks, the RNNoise benchmark (`audio/benchmark-rnnoise.mjs`) and the smokes `smoke-chat-scroll`, `smoke-phone-layout`, `smoke-lazy-parts`, `smoke-member-list` | every pull request and push that touches `web/` (`ci.yml`, job `Frontend`)                       | Chromium (`npx playwright install chromium-headless-shell`)          |
| `npm run test:field-gallery` (`field-gallery.mjs`)                                                                                                                                                                    | the same job                                                                                     | Chromium                                                             |
| `generate-emoji-data.py --check`                                                                                                                                                                                      | the same job                                                                                     | Python                                                               |
| The E2E scenarios (`e2e/fault-run.mjs` with the profiles in `e2e/profiles.mjs`)                                                                                                                                       | `e2e-extended.yml`, after merge and on release pull requests                                     | built API and SFU, Postgres, Redis, MinIO; see `e2e/README.md`       |
| `npm run test:e2e-server` (`e2e/server-checks.mjs`): `e2e/chat-workflows`, `e2e/search-context`, `e2e/webapp-accessibility`, `smoke-reactions`, `smoke-call-sounds`, `smoke-audio-processing`, `smoke-source-audio`   | the same workflow                                                                                | API, SFU and a Vite dev server at `GELABBER_SMOKE_URL`               |
| `npm run test:pwa-smoke` (`smoke-pwa.mjs`)                                                                                                                                                                            | the same workflow                                                                                | the built `dist` behind the production nginx; see `docs/v0.6-pwa.md` |
| `mutation/`: Stryker's own `package.json`, lock, `.npmrc` and config, for mutation testing of `web/src` (GEL-388)                                                                                                     | `mutation-web.yml`: weekly on the development line, by hand, and on pull requests that change it | its own `npm ci` in that folder; nothing in it is run by hand        |

A check that is known to fail stays in its script's list of expected failures
(`e2e/expected-failures.mjs`), with the issue that ends it and the failure
message it expects. The run fails when such a check passes, so its entry goes
with the fix, when it fails with another message, and when no check passed.

## Started by hand

| What                                                                              | For                                                                                                    | Described in                            |
| --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ | --------------------------------------- |
| `npm run test:browser-smoke` (`smoke-two-browsers.mjs`, with `smoke-netem.mjs`)   | two accounts through a running Compose stack with TURN, before a deployment change                     | `deploy/README.md`                      |
| `npm run measure:screen-sharpness` (`smoke-screen-sharpness.mjs`)                 | what a Live loses under a tight network (GEL-26): it measures and asserts nothing; needs root for `tc` | the script's header                     |
| `npm run test:e2e-app` (`e2e/run.mjs`)                                            | one profile against a running stack while working on a scenario                                        | `e2e/README.md`                         |
| `npm run test:e2e-held-renderer-control` (`e2e/held-renderer-native-control.mjs`) | the negative control of the held renderer                                                              | `e2e/README.md`                         |
| `load/` (`npm run test:load-chat`, `-connections`, `-call`)                       | load measurements against a stack                                                                      | `load/README.md`                        |
| `audio/build-*.sh`, `audio/deepfilternet*`                                        | reproducible builds of the noise filters' WASM                                                         | `audio/README.md`                       |
| `parity/`                                                                         | the web reference pictures for the desktop app's parity comparison                                     | `.claude/skills/native-parity/SKILL.md` |
| `landing/`                                                                        | screenshots for the landing page and the README                                                        | `docs/design/v0.10/README.md`           |
| `public-pages/prerender.mjs`                                                      | part of `npm run build`                                                                                | `package.json`                          |
