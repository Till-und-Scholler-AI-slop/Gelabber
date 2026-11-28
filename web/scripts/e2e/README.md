# Actual-app acceptance scenarios

## Where it runs

`.github/workflows/e2e-extended.yml` runs these scenarios against its own API,
SFU, Postgres, Redis and MinIO (`ci-fault-prepare.mjs`, then `fault-run.mjs`
per browser and profile from `profiles.mjs`): the fault profiles `core-faults`
and `media-faults` in Chromium and Firefox, the profiles `core`, `media` and
`access` in Chromium. It runs after pushes to a
version branch that touch `backend/` or `web/`, on release and hotfix pull
requests to `main`, and by hand. After a push run the issue "Extended E2E
fails on <branch>" is opened or updated when the run is red and closed when it
is green again (GEL-378). The same workflow runs the browser checks against a
running app (`server-checks.mjs`: `chat-workflows.mjs`, `search-context.mjs`,
`webapp-accessibility.mjs` and four smokes) and `../smoke-pwa.mjs`;
`../README.md` lists every script. The rest of this file records how the
scenarios came about.

A profile can list scenarios known to fail (`expectedFailures`: id ->
`{ issue, reason, site? }`). Such a scenario that fails with that reason (and
at that file:line) leaves the gate green and is logged as `EXPECTED FAIL`; one
that fails otherwise, passes, or is BLOCKED turns the gate red, a pass until
its entry goes. `leftOut` names scenarios of the suite a profile leaves out,
with the reason the run prints (`LEFT OUT`).

Gelabber uses mediasoup 0.33 through the official Rust binding
and public mediasoup-client 3.26.0, with Gelabber's own v4 WebSocket/authority
contract. A publishing voice connection uses separate SDK send and receive PCs
(at most two active PCs); a Live is watched inside the call. Native
transport compaction preserves captures and creates fresh Producer/Consumer IDs.
Use the current executed test/runtime freeze when reporting limits or results.
The previous engine's `media/tests/browser-lifecycle.mjs` (removed in v0.7.1),
dated media test docs and historical reports are not v4 acceptance gates. See
[SOURCE_AUDIO.md](../../../backend/media/SOURCE_AUDIO.md) and the
[migration document](../../../docs/v0.4-mediasoup-migration.md).

The v0.4 hand-offs (`docs/e2e-handoffs/`) and run reports (`web/scripts/e2e/evidence/`) that this file used to link were removed in GEL-368; `git show v0.14.0:<path>` shows them as recorded. Those results carry their recorded runtime hashes and do not establish acceptance of the new v4 engine. How to start an owned fault run is under [Owned fault runs](#owned-fault-runs).

This runner provides scenario construction and automated local/CI subset gates. Complete acceptance also requires the documented manual, HTTPS/WAN and production gates. The runner creates isolated `example.test` accounts and its own server through the UI, uses the actual API, Gateway, SFU and object store, and deletes only its own server. Accounts remain as test fixtures; no shared Redis keys, services or existing data are reset. A fresh browser cookie jar is used for each participant. Core scenarios verify and restore each fixture account before and after every check; failed restoration blocks following scenarios.

The original 11a delivery added `web/scripts/e2e/**` and two npm script entries. The separate smoke fix4028136 subsequently isolated netem to selected SFU UDP ports while preserving camera/screen/forced-TURN/audio/recovery/upload assertions; see [SMOKE-NETEM.md](../SMOKE-NETEM.md). Dependencies, pins, lockfile and product source remain unchanged. CI integration is authorized for the next phase.

## Invoke

Use Node from `engines` in `package.json` and the installed dependencies. From `web/`:

```bash
npm run test:e2e-harness
GELABBER_SMOKE_URL=http://127.0.0.1:5177 \
GELABBER_E2E_WEB_SHA=6e221705c34e944bd3804aa3c9482488e770f249 \
GELABBER_E2E_API_SHA=243c86ccc8f8a4bdfca9bc73da32e34520577bf9 \
GELABBER_E2E_MEDIA_SHA=6e221705c34e944bd3804aa3c9482488e770f249 \
GELABBER_E2E_SUITE=media \
GELABBER_E2E_REPORT=/tmp/gelabber-e2e/media.json \
npm run test:e2e-app
```

`GELABBER_E2E_SUITE`: `all` (default), `media`, `core`, `access`. Browser: `GELABBER_E2E_BROWSER=chromium` (default) or `firefox`. Network: `GELABBER_E2E_NETWORK=relay` forces native peers through TURN; default direct checks the selected candidate pair actually excludes relay. Reports contain only candidate types, not addresses. `GELABBER_E2E_CASES` accepts comma-separated exact scenario IDs for isolated control probes; excluded checks are explicitly `NOT_RUN`, never PASS. Required setup dependencies still execute: late-watch prepares a real live publisher, DM uploads create their own DM. Unknown or unreached requested IDs produce `BLOCKED`. Runtime SHAs are operator declarations, explicitly not verified service digests. Unknown attribution remains `unknown`. Test HEAD, dirty state and SHA256 of each test module identify the code under test independently from the app runtime.

The new runner deliberately accepts only loopback HTTP/HTTPS origins and rejects credentials/query strings. It is not a production account-creating smoke. The coordinator owns integration, review, remote invocation and final HTTPS/WAN acceptance. Local browser launches need the already authorized socket/process access. Do not stop or restart the shared stack to run these scenarios.

`FAIL` or required `BLOCKED` makes the process exit nonzero, as do failed cleanup or fixture restoration. Explicitly unselected `NOT_RUN` rows do not fail a selected subset gate. The report lists exclusions and marks `completeAcceptance:false` even for a passing subset; manual cases selected explicitly remain required `BLOCKED`. Reports distinguish `confirmed-product-failure` (a recorded positive control followed by a failed boundary), `unconfirmed-failure` (needs a control or harness investigation), `observed-pass`, and `blocked`. Historical failed probes are not product regressions just because they were red. Generic Playwright exceptions retain only a fixed reason and a test-file line, never their call logs or URLs; a Playwright timeout adds the call that ran out (`action`, e.g. `page.goto`) and the harness line (`harnessSite`). A fixture's row lists its own steps with their times in `metrics.steps`, when it passes and when it fails (GEL-423).

Native interface deadlines distinguish native-getStats from outer-page-evaluate. Observation budgets are absolute and shared by all probes; late success and already-expired work cannot pass. Timeout produces test-error/FAIL, quarantines the page, closes it once and blocks later cases in that group. Finally never evaluates quarantined pages; native restore, context/browser close and owned runtime/cache teardown are bounded or independently attempted and visible red on failure. No missing stats or rendered frames are converted to a pass.

The local Firefox TURN exception requires explicit `GELABBER_E2E_FIREFOX_LOOPBACK_ICE=true` plus `GELABBER_E2E_SFU_LOOPBACK_ICE=true`, Firefox forced relay and the private own `media-fault-loopback-control.env`, with `media-fault.env` in the same folder (the runner hashes both; without it startup ends `BLOCKED`). Both flags default off. Scope guards reject unsupported combinations before acquiring runtime resources; no automatic retry override. Actual process/binary/config categories and baseline hashes are reported. This exception changes only the isolated local test profile/SFU topology and provides no Firefox-default/WAN acceptance.

## Owned fault runs

`fault-run.mjs` starts its own API (port 18086, with Redis and storage fault
proxies on 16386 and 19086), for `media`, `access` and `media-faults` its own
SFU (18087), and a Vite server on 15186 that serves the web snapshot and sets
`GELABBER_SMOKE_URL` itself. `.github/workflows/e2e-extended.yml` is the
reference run: `ci-fault-prepare.mjs` (CI only) creates the dedicated fault
database, names a fresh bucket (`gb-fault-<hex>`, which the API creates on
start), copies the built API and SFU binaries and writes the private env files
and manifests. The preparer refuses to run unless `CI=true`; locally, write
the same files by hand: `fault.env` in `KEY="value"` lines with
`DATABASE_URL` (database `gelabber_fault_<hex>`), `REDIS_URL`,
`MINIO_ENDPOINT`, `MINIO_ROOT_USER`, `MINIO_ROOT_PASSWORD` and `MINIO_BUCKET`,
all on loopback, and a manifest JSON with `sourceSha`, `binaryPath` and the
binary's `sha256`. From the repository root:

```bash
GELABBER_E2E_PROFILE=core-faults \
GELABBER_E2E_FAULT_ENV=<private fault.env, mode 600> \
GELABBER_E2E_API_MANIFEST=<api manifest> \
GELABBER_E2E_WEB_SNAPSHOT=<absolute path to a web/ checkout> \
GELABBER_E2E_WEB_SHA=<its commit> \
GELABBER_E2E_REPORT=/tmp/gelabber-e2e/core-faults.json \
node web/scripts/e2e/fault-run.mjs
```

Add `GELABBER_E2E_BROWSER=firefox` for Firefox. The profiles `media`,
`access` and `media-faults` also need `GELABBER_E2E_MEDIA_FAULT_ENV` and
`GELABBER_E2E_MEDIA_MANIFEST`; `core-faults` needs no SFU. A profile cannot be
combined with `GELABBER_E2E_CASES` or `GELABBER_E2E_SUITE`. The runtime
refuses an env file readable by others, a database not named
`gelabber_fault_<hex>` and a binary whose SHA-256 differs from its manifest.
Ports are bound exclusively, and no service is stopped but an own API or SFU
that a dead runner left behind. The own API and SFU end with the runner (an
exit hook); a runner ended by SIGKILL, by SIGTERM or SIGINT sent to it alone
or for lack of memory runs no hook, so each of them leaves a private PID file
beside its log in `/tmp` that names it and its runner. The next start ends
such a leftover only when that runner is dead and the leftover runs this
start's own API or SFU binary, says so and records its PID as
`leftoverRemoved` in the report; while the runner is alive, the start ends
`BLOCKED live-fault-run-owns-the-ports` and touches nothing of it. A leftover
it may not end stays; when it then holds the port, or for the API the fault
database, the start ends `BLOCKED port-taken-<why>` or
`fault-db-taken-<why>`. A start deletes only the PID file it wrote (GEL-457).
A promise that start-up leaves behind and that rejects before the
harness listens fails as `stray-rejection` in
`$GELABBER_E2E_REPORT.startup-stray.json`, and the runner tears down. A Redis
reset deletes only four exact topic keys, never `FLUSH`. Proxies are restored
and owned processes closed in `finally`. A startup or teardown failure makes
the exit code nonzero and is written, redacted, beside the report as
`$GELABBER_E2E_REPORT.startup.json` or `.teardown.json`. A promise that a
scenario left behind and that rejects later fails as its own row,
`stray-rejection`, and the run goes on to its report and teardown (GEL-453).
Its `site` and `harnessSite` say where the promise came from; `during` (the
case that was running) and `after` (the last case that finished) say only
when it arrived. One that arrives after the report is written goes beside it
as `$GELABBER_E2E_REPORT.stray.json`.

## Scenario contracts

- Media: actual UI "Live gehen" watched inside the voice channel (viewers join the call; no camera/display calls and at most their microphone sender); several people live at once; default autoplay; late watch aged at least 30 seconds from publication; native selected ICE path; decoded and rendered frame growth with changing synthetic pixels and distinct Live/camera color bands. Bytes alone cannot pass. First-frame budgets: 5 seconds direct, 10 seconds relay. Native publisher encoding must begin first.
- Recovery: targeted browser Media-WS/Gateway disconnects, automatic signaling return within 20 seconds, same live capture track sent without another display request. Gateway-only recovery must keep the healthy peer. No publisher click after the fault. Browser offline for 8 seconds is separately labeled: established UDP may remain flowing; it does not claim an OS network outage or server restart.
- Lifecycle: 20 live start/stop cycles. In each, the watcher stops watching before the Live ends, so the next cycle starts from "Zuschauen"; each cycle's client resources are checked in the cycle (one connection that sends and one that only receives, per participant) and once more after the loop, followed by an independent late viewer, which leaves the call with the scenario. A re-watch's first frame has 15 seconds (each cycle records it). A voice connection has at most two active SDK PCs; stop leaves voice audio. Public transport compaction retires old native resources before retaining captures on replacement transports. Report the actual native PC/transceiver counts from the executed candidate; the previous engine's sender-reuse and fixed SDP-size results do not qualify v4. These checks do not prove unexposed SFU publication/task bounds. No reload of the publisher during the cycles.
- UI: SPA channel navigation must not label old-channel video as the new channel; stop/leave/mute/live controls remain reachable. Native picker cancellation and a refused Live must release capture. Simulated `NotAllowedError` requires a visible playback retry; actual default autoplay is a separate native-browser check, without policy overrides.
- Watch audio: room microphone audio exactly once plus the selected publisher's Live audio paired to its current Live Producer/epoch. A Live nobody opted into is not forwarded. Positive other-room audio/Live must not leak. Native source/RTP and duplicate-playback observations complement session-wide Volume/Deafen/click-retry checks. Human audible quality stays manual.
- Forced track arrival: before holding/reversing callbacks, require exactly two live fixture capture objects (one camera, one Live display), each bound to a distinct current native sender. Read each sender's own stats and require fresh encoded-frame growth for every source across two observations. One legacy encoding and multiple simulcast RIDs are both valid; several RTP records for one source cannot substitute for a missing source. Native track identifiers stay in browser memory; evidence uses fixture ordinals and RID labels. Run the focused ownership/deadline checks with `node --test scripts/e2e/publisher-encoders.checks.mjs`.
- Core: real registration/invite/channel setup, reload/login/logout, cross-client chat create/edit/delete, new DM discovery while receiver is already connected, JPEG/PNG/WebP presign/PUT/bind/reload/byte-exact download in both channel and DM, 55-message history/paging, explicit error/retry and failed-send navigation, delayed A-account 401/CSRF403 after login as B with another tab observing B's session.
- Access: Leave/Kick/Ban/Logout while an independent native Gateway subscriber remains open, plus an actual decoded Live in the call whose local close is deliberately suppressed, plus an unconsumed pre-revocation SFU ticket. Positive pre-revocation Gateway and frame controls are recorded. Post-revocation REST, new tickets, existing events, continuing decoded frames and the held ticket are all assessed. Closing the normal UI socket alone cannot pass. Another independent session must survive logout. Raw sockets and tracks are released afterward.

The logout check registers a POST `/api/auth/logout` response wait before clicking, requires HTTP200, then requires session HTTP200 with `user:null`. Client-side navigation occurs before the POST finishes and is not the success criterion. No arbitrary sleep substitutes for that response.

Channel/server deletion uses an independent muted renderer attached to the held native receiver track **before** deletion. It first proves increasing native frame counters, then retains the same renderer, receiver, track and callback identity through the fault. The counter source is selected once (`requestVideoFrameCallback` or native playback quality). A missing, decreasing or replaced counter, paused/errored renderer, detached source, ended or disabled receiver track fails the measurement; no zero is substituted. A live track becoming muted after server revocation alone cannot prove that frames stopped. Other access scenarios retain their original strict native RTP counter contract. Renderer callbacks and elements are released even after partial setup or peer cleanup failure.

The local instrument control deliberately hides native video RTP reports while authorized media continues. The unchanged renderer must still advance and reject the actual deletion frame-stop predicate. Its denied/closed side conditions are synthetic solely to isolate that predicate; it performs no revocation and is not an access acceptance result. Both commands use fresh fixture accounts and delete only their own servers:

```bash
GELABBER_SMOKE_URL=http://127.0.0.1:15182 \
GELABBER_E2E_BROWSER=chromium GELABBER_E2E_SUITE=access \
GELABBER_E2E_CASES=channel-server-delete-active-sockets \
npm run test:e2e-app
GELABBER_SMOKE_URL=http://127.0.0.1:15182 \
GELABBER_E2E_BROWSER=chromium npm run test:e2e-held-renderer-control
```

Repeat with `GELABBER_E2E_BROWSER=firefox` for the second native browser. Keep actual runtime attribution and report paths in the surrounding environment as described above. These native controls remain local; CI runs the fast ownership/counter regressions.

The final local Chromium153/Firefox155 controls on clean test freeze `d2fd0f6` pass for both deletion scopes and reject a false stop with hidden RTP reports. Its compact renderer report (`web/scripts/e2e/evidence/held-renderer-validation.json`, removed in GEL-368) recorded immutable runtime/test hashes, unchanged live/enabled ownership, real counter values, fixture cleanup and explicit remaining gates. It does not validate the later source-restart fix.

## Open acceptance

Core/Storage/Epoch, concurrent attempts and row rollback, real slow raw TCP reader and transaction-forced Ban/Join now have actual Chromium and Firefox controls on the approved immutable core snapshot. Their exact results and earlier test errors were recorded in `docs/e2e-handoffs/AUTOMATION-HANDOFF.md` (removed in GEL-368). Final approved08b runtime now supports the executable media15/access6 and own API-/Redis-lease cases; exact author/clean-source coverage was recorded in `docs/e2e-handoffs/MEDIA-HANDOFF.md` (removed in GEL-368). Native screen-picker, audible two-device quality, WAN/production and full remote netem remain explicit separate gates. The original relay/audio/recovery assertions and strict selected-case/cleanup gates remain in force.

The coordinator authorized 11b/c automation after the separate 11a fix; runtime prerequisites and final 11d gates remain explicit. No product edits or self-deployment.

The separate [iOS and Omarchy acceptance sheet](../../../docs/v0.4-device-acceptance.md)
records the physical hearing, device, background/resume and TURN checks. It starts
as not performed and requires the exact candidate and actual browser/device evidence.

The one-hour call runner (`call-soak.mjs`) was removed in GEL-377.
