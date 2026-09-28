# 11a actual-app scenarios

This is scenario construction and local evidence, not integrated 11b/c/d acceptance. The runner creates isolated `example.test` accounts and its own server through the UI, uses the actual API, Gateway, SFU and object store, and deletes only its own server. Accounts remain as test fixtures; no shared Redis keys, services or existing data are reset. A fresh browser cookie jar is used for each participant. Core scenarios verify and restore each fixture account before and after every check; failed restoration blocks following scenarios.

Only `web/scripts/e2e/**` and two npm script entries are added. The existing `test:browser-smoke` and its camera/screen/forced-TURN/netem/upload assertions remain byte-for-byte unchanged. Dependencies, pins, lockfile, product source, `media/tests/browser-lifecycle.mjs`, CI and deployment are untouched.

## Invoke

Use the pinned toolchain and the existing installed dependencies. From `web/`:

```bash
source /tmp/gelabber-toolchain/env.sh
npm run test:e2e-harness
GELABBER_SMOKE_URL=http://127.0.0.1:5174 \
GELABBER_E2E_WEB_SHA=3a30d44 \
GELABBER_E2E_API_SHA=828ee23 \
GELABBER_E2E_MEDIA_SHA=28d4505 \
GELABBER_E2E_SUITE=media \
GELABBER_E2E_REPORT=/tmp/gelabber-e2e/media.json \
npm run test:e2e-app
```

`GELABBER_E2E_SUITE`: `all` (default), `media`, `core`, `access`. Browser: `GELABBER_E2E_BROWSER=chromium` (default) or `firefox`. Network: `GELABBER_E2E_NETWORK=relay` forces native peers through TURN; default direct checks the selected candidate pair actually excludes relay. Reports contain only candidate types, not addresses. `GELABBER_E2E_CASES` accepts comma-separated exact scenario IDs for isolated control probes; excluded checks are BLOCKED, never PASS. Runtime SHAs are operator declarations, explicitly not verified service digests. Unknown attribution remains `unknown`. Test HEAD, dirty state and SHA256 of each test module identify the code under test independently from the app runtime.

The new runner deliberately accepts only loopback HTTP/HTTPS origins and rejects credentials/query strings. It is not a production account-creating smoke. The coordinator owns integration, review, remote invocation and final HTTPS/WAN acceptance. Local browser launches need the already authorized socket/process access. Do not stop or restart the shared stack to run these scenarios.

`FAIL` or `BLOCKED` (including unselected/manual scenarios) makes the process exit nonzero. Reports distinguish `confirmed-product-failure` (a recorded positive control followed by a failed boundary), `unconfirmed-failure` (needs a control or harness investigation), `observed-pass`, and `blocked`. The handoff also records corrected test errors; historical failed probes are not product regressions just because they were red. Generic Playwright exceptions retain only a fixed reason and a test-file line, never their call logs or URLs.

## Scenario contracts

- Media: actual UI Go Live and watch-only without microphone/camera/display calls or senders; default autoplay; late watch aged at least 30 seconds from publication; native selected ICE path; decoded and rendered frame growth with changing synthetic pixels and distinct live/screen/camera color bands. Bytes alone cannot pass. First-frame budgets: 5 seconds direct, 10 seconds relay. Native publisher encoding must begin first.
- Recovery: targeted browser Media-WS/Gateway disconnects, automatic signaling return within 20 seconds, same live capture track sent without another display request. Gateway-only recovery must keep the healthy peer. No publisher click after the fault. Browser offline for 8 seconds is separately labeled: established UDP may remain flowing; it does not claim an OS network outage or server restart.
- Lifecycle: all 20 live start/stop cycles run before client resource bounds are assessed, followed by an independent late viewer. At most one active publisher/watch peer; stop leaves audio only, at most four transceivers (audio/video plus two spare slots) and 32 KiB local SDP. These deliberately finite client bounds catch linear accumulation. They do not prove unexposed SFU publication/task bounds. No reload of the publisher during the cycles.
- UI: SPA channel navigation must not label old-channel video as the new channel; stop/leave/mute/live controls remain reachable. Native picker cancellation and losing an existing live claim must release capture. Simulated `NotAllowedError` requires a visible playback retry; actual default autoplay is a separate native-browser check, without policy overrides.
- Core: real registration/invite/channel setup, reload/login/logout, cross-client chat create/edit/delete, new DM discovery while receiver is already connected, JPEG/PNG/WebP presign/PUT/bind/reload/byte-exact download in both channel and DM, 55-message history/paging, explicit error/retry and failed-send navigation, delayed A-account 401/CSRF403 after login as B with another tab observing B's session.
- Access: Leave/Kick/Ban/Logout while an independent native Gateway subscriber remains open, plus an actual decoded watch peer whose local close is deliberately suppressed, plus an unconsumed pre-revocation SFU ticket. Positive pre-revocation Gateway and frame controls are recorded. Post-revocation REST, new tickets, existing events, continuing decoded frames and the held ticket are all assessed. Closing the normal UI socket alone cannot pass. Another independent session must survive logout. Raw sockets and tracks are released afterward.

The logout check registers a POST `/api/auth/logout` response wait before clicking, requires HTTP200, then requires session HTTP200 with `user:null`. Client-side navigation occurs before the POST finishes and is not the success criterion. No arbitrary sleep substitutes for that response.

## Open acceptance

Every operational/manual prerequisite has an explicit BLOCKED row: audible speech/mute/deafen quality on two devices, native display picker, per-room SFU task/publication bounds, forced track arrival ordering/rejected SDP, isolated API/Redis outage and epoch reset, slow raw reader backpressure, object-store/database failure cleanup, deleted-room negative cases, remaining concurrent send/rollback/paging scenarios pending 07, and HTTPS/WAN. The existing privileged netem check is retained; it was not runnable locally. Native screen and audible two-device acceptance remain with the coordinator.

See [HANDOFF.md](HANDOFF.md) and [evidence/](evidence/) for executed checks, controls and remaining blockers. Stop after this 11a handoff; do not self-start 11b/c/d or product work.
