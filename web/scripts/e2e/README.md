# Actual-app acceptance scenarios

Current 11b/c execution, the row-by-row acceptance matrix, fault-runtime instructions and CI limits are in [AUTOMATION-HANDOFF.md](AUTOMATION-HANDOFF.md). [HANDOFF.md](HANDOFF.md) preserves historical 11a findings; it is not the current result matrix.

This runner provides scenario construction and automated local/CI subset gates. Complete acceptance also requires the documented manual, HTTPS/WAN and production gates. The runner creates isolated `example.test` accounts and its own server through the UI, uses the actual API, Gateway, SFU and object store, and deletes only its own server. Accounts remain as test fixtures; no shared Redis keys, services or existing data are reset. A fresh browser cookie jar is used for each participant. Core scenarios verify and restore each fixture account before and after every check; failed restoration blocks following scenarios.

The original 11a delivery added `web/scripts/e2e/**` and two npm script entries. The separate smoke fix4028136 subsequently isolated netem to selected SFU UDP ports while preserving camera/screen/forced-TURN/audio/recovery/upload assertions; see [SMOKE-NETEM.md](../SMOKE-NETEM.md). Dependencies, pins, lockfile and product source remain unchanged. CI integration is authorized for the next phase.

## Invoke

Use the pinned toolchain and the existing installed dependencies. From `web/`:

```bash
source /tmp/gelabber-toolchain/env.sh
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

`FAIL` or required `BLOCKED` makes the process exit nonzero, as do failed cleanup or fixture restoration. Explicitly unselected `NOT_RUN` rows do not fail a selected subset gate. The report lists exclusions and marks `completeAcceptance:false` even for a passing subset; manual cases selected explicitly remain required `BLOCKED`. Reports distinguish `confirmed-product-failure` (a recorded positive control followed by a failed boundary), `unconfirmed-failure` (needs a control or harness investigation), `observed-pass`, and `blocked`. The handoff also records corrected test errors; historical failed probes are not product regressions just because they were red. Generic Playwright exceptions retain only a fixed reason and a test-file line, never their call logs or URLs.

Native interface deadlines distinguish native-getStats from outer-page-evaluate. Observation budgets are absolute and shared by all probes; late success and already-expired work cannot pass. Timeout produces test-error/FAIL, quarantines the page, closes it once and blocks later cases in that group. Finally never evaluates quarantined pages; native restore, context/browser close and owned runtime/cache teardown are bounded or independently attempted and visible red on failure. No missing stats or rendered frames are converted to a pass.

The local Firefox TURN exception requires explicit `GELABBER_E2E_FIREFOX_LOOPBACK_ICE=true` plus `GELABBER_E2E_SFU_LOOPBACK_ICE=true`, Firefox forced relay and the private own `media-fault-loopback-control.env`. Both flags default off. Scope guards reject unsupported combinations before acquiring runtime resources; no automatic retry override. Actual process/binary/config categories and baseline hashes are reported. This exception changes only the isolated local test profile/SFU topology and provides no Firefox-default/WAN acceptance. See MEDIA-HANDOFF for the preserved red baseline, native/App controls, aborted whole adapter run and subsequent selected evidence.

## Scenario contracts

- Media: actual UI Go Live and watch-only without microphone/camera/display calls or senders; default autoplay; late watch aged at least 30 seconds from publication; native selected ICE path; decoded and rendered frame growth with changing synthetic pixels and distinct live/screen/camera color bands. Bytes alone cannot pass. First-frame budgets: 5 seconds direct, 10 seconds relay. Native publisher encoding must begin first.
- Recovery: targeted browser Media-WS/Gateway disconnects, automatic signaling return within 20 seconds, same live capture track sent without another display request. Gateway-only recovery must keep the healthy peer. No publisher click after the fault. Browser offline for 8 seconds is separately labeled: established UDP may remain flowing; it does not claim an OS network outage or server restart.
- Lifecycle: all 20 live start/stop cycles run before client resource bounds are assessed, followed by an independent late viewer. At most one active publisher/watch peer; stop leaves audio only, at most four transceivers (audio/video plus two spare slots) and 32 KiB local SDP. These deliberately finite client bounds catch linear accumulation. They do not prove unexposed SFU publication/task bounds. No reload of the publisher during the cycles.
- UI: SPA channel navigation must not label old-channel video as the new channel; stop/leave/mute/live controls remain reachable. Native picker cancellation and losing an existing live claim must release capture. Simulated `NotAllowedError` requires a visible playback retry; actual default autoplay is a separate native-browser check, without policy overrides.
- Watch audio: every active publication in the same authorized voice channel exactly once; publisher selection filters Live video only. Positive other-room audio/Live must not leak. Native source/RTP and duplicate-playback observations complement session-wide Volume/Deafen/click-retry checks. Human audible quality stays manual.
- Forced track arrival: before holding/reversing callbacks, require exactly three live fixture capture objects (one camera, two display sources), each bound to a distinct current native sender. Read each sender's own stats and require fresh encoded-frame growth for every source across two observations. One legacy encoding and multiple simulcast RIDs are both valid; several RTP records for one source cannot substitute for a missing source. Native track identifiers stay in browser memory; evidence uses fixture ordinals and RID labels. Run the focused ownership/deadline checks with `node --test scripts/e2e/publisher-encoders.checks.mjs`.
- Core: real registration/invite/channel setup, reload/login/logout, cross-client chat create/edit/delete, new DM discovery while receiver is already connected, JPEG/PNG/WebP presign/PUT/bind/reload/byte-exact download in both channel and DM, 55-message history/paging, explicit error/retry and failed-send navigation, delayed A-account 401/CSRF403 after login as B with another tab observing B's session.
- Access: Leave/Kick/Ban/Logout while an independent native Gateway subscriber remains open, plus an actual decoded watch peer whose local close is deliberately suppressed, plus an unconsumed pre-revocation SFU ticket. Positive pre-revocation Gateway and frame controls are recorded. Post-revocation REST, new tickets, existing events, continuing decoded frames and the held ticket are all assessed. Closing the normal UI socket alone cannot pass. Another independent session must survive logout. Raw sockets and tracks are released afterward.

The logout check registers a POST `/api/auth/logout` response wait before clicking, requires HTTP200, then requires session HTTP200 with `user:null`. Client-side navigation occurs before the POST finishes and is not the success criterion. No arbitrary sleep substitutes for that response.

## Open acceptance

Core/Storage/Epoch, concurrent attempts and row rollback, real slow raw TCP reader and transaction-forced Ban/Join now have actual Chromium and Firefox controls on the approved immutable core snapshot. Their exact results and earlier test errors are recorded in the current handoff. Final approved08b runtime now supports the executable media15/access6 and own API-/Redis-lease cases; exact author/clean-source coverage is recorded in [MEDIA-HANDOFF.md](MEDIA-HANDOFF.md). Native screen-picker, audible two-device quality, WAN/production and full remote netem remain explicit separate gates. The original relay/audio/recovery assertions and strict selected-case/cleanup gates remain in force.

See [HANDOFF.md](HANDOFF.md) and [evidence/](evidence/) for executed checks, controls and remaining blockers. The coordinator authorized 11b/c automation after the separate 11a fix; runtime prerequisites and final 11d gates remain explicit. No product edits or self-deployment.

## Continuous call and resource return

The long local check joins two voice publishers and a third watch-only client,
starts Go Live, and samples native duplex audio and decoded video every 30 seconds.
It requires advancing media, stable peers/transceivers/captures/playback tracks,
no duplicate audio playback and no ICE failures. Stop/Leave must release all
client media tracks and bring the dedicated SFU room/peer gauges back to zero.
The runner uses the existing real app fixture and deletes only its own server.
It requires an otherwise idle, dedicated loopback SFU and a static built web
candidate without HMR. Other calls must use a separate SFU instance.

```bash
GELABBER_SMOKE_URL=http://127.0.0.1:5177 \
GELABBER_SOAK_MEDIA_URL=http://127.0.0.1:8081 \
GELABBER_E2E_CASES=continuous-call-and-resource-return \
GELABBER_E2E_REPORT=/tmp/gelabber-call-soak.json \
npm run test:call-soak
```

The default duration is 3600 seconds; `GELABBER_SOAK_SECONDS=30` runs a fixture
control and explicitly reports `oneHour:false`. Durations over two hours are
rejected. `GELABBER_SOAK_CHECKPOINT` sets the intermediate report path. Optional
Linux `GELABBER_SOAK_API_PID` and `GELABBER_SOAK_MEDIA_PID` record RSS and thread
counts from the supplied Gelabber processes; executable names and process start
identity are checked on every sample. No process is killed or restarted.
The report records the mapped executable SHA256 and cumulative process CPU time.
CPU percentage uses monotonic sample intervals and the system's `CLK_TCK`;
100% means one fully occupied CPU core, and the first sample has no percentage.
RSS is retained as measured: closed tracks do not prove an allocator returned
memory. The app/runtime SHA declarations retain the standard harness attribution
limits. Fake capture, a local network and a simulated display do not establish
physical microphone quality, real iOS suspend/resume or WAN/TURN acceptance.
This long check stays local and is not added to CI.

The separate [iOS and Omarchy acceptance sheet](../../../docs/v0.4-device-acceptance.md)
records the physical hearing, device, background/resume and TURN checks. It starts
as not performed and requires the exact candidate and actual browser/device evidence.

The [2026-10-05 local hour evidence](evidence/v04-one-hour-call-2026-10-05.json)
records 3600.106 seconds and 120 samples on an earlier frozen candidate. It
used the temporary predecessor of this portable runner. The current portable
runner has two separate 30-second controls. The recorded hour proves advancing
media and released tracks/rooms, while shared API activity confounds its RAM
comparison; viewer layers, physical devices and WAN/TURN remain separate gates.
