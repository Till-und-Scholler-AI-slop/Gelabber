# Final11b media / access / authority faults

This delta owns test scripts, scenarios, reports and the separately authorized CI follow-up. Product code, dependencies, pins, original smoke and shared runtimes are unchanged. Coordinator owns independent review/integration. No remote push, release, deployment or production access.

## Runtime and evidence attribution

Approved immutable Web/SFU source `6e221705c34e944bd3804aa3c9482488e770f249`; Web snapshot `/tmp/gelabber-web-snapshot-6e22170/web`. Copied SFU SHA256 `ea095c68223b89c686cadd960df0f7597f3add312618fa4bbfa253ab13736873`. Copied API source `243c86ccc8f8a4bdfca9bc73da32e34520577bf9`, SHA256 `d383dea15e559d0602c11eb2e48e1a9de4ca96b1540ef995e5a95ef15977fdc8`. Runner hashes copies before launching. Browser versions Chromium153.0.8010.12 / Firefox155.0; Playwright1.63.0 / Node26.8.2 / Linux. Capture is changing synthetic canvas and fake microphone; autoplay uses browser defaults.

Own ports: Vite15186, API18086, SFU18087, Redisproxy16386 and storageproxy19086. API has the exclusively owned fault DB/bucket; no other API worker can consume its jobs. SFU Redis authority is redirected through that same owned proxy, with private config preserved. Binds refuse occupied ports and never kill another process. API suspend uses only the captured child PID; Redis faults affect only proxy clients. Read-only Live-key observations validate the fixture's channel/server owner in the dedicated DB before reading exactly `gb:live:<owned channel>`. No lease mutation or global Redis reset.

Author app controls are explicitly dirty24211fb/5f841f9 working-tree runs with per-module SHA256, not independent review or clean committed-source claims. Clean42bc812 and24211fb Firefox follow-ups have their own exact controls and independent approvals; historical failed whole runs remain failed. [media-access-validation.json](evidence/media-access-validation.json) indexes raw artifact hashes, selected-case results, cleanup and observed native paths; generated full reports remain outside source. See [AUTOMATION-HANDOFF.md](AUTOMATION-HANDOFF.md) for the previously approved core/fault scope.

## Executable matrix

`media` requires15 cases, `access`6, and `media-faults` the one two-plane authority scenario. Both-browser direct coverage and Chromium local forced-TURN coverage are assembled from full and targeted author controls. Firefox default forced-TURN remains red/unclassified. A separately authorized local Firefox-loopback-pref plus own-SFU-loopback adapter has native and actual-app positive relay/RTP/render controls. The subsequent whole adapter matrix was aborted after a harness interface hang; its FAIL and failed cleanup remain. The explicitly authorized remaining18 cases are checked in selected sequential groups, with separate case-to-report/source-hash mapping. LateWatch in the aborted run is a PASS observation with failed cleanup, not a successful whole-run gate. No assembled coverage is presented as a new clean whole-profile execution. A selected pass remains `completeAcceptance:false`; unknown/unreached/required BLOCKED, restoration and cleanup failures stay nonzero. Unselected manual cases remain NOT_RUN rather than PASS.

| Required case | Native boundary |
|---|---|
| voice-only-duplex-single-source | Voice alone in both directions, increasing packets, exactly one RTP-backed source per receiver, no camera/display, no duplicated audio playback; selected native direct/relay path. Human intelligibility is separate. |
| live-watch-default-autoplay-no-mic | Correct changing640px Live, native decode/render growth, default playback, no Watch capture or senders;5s direct/10s relay. |
| late-watch-after-30s | Actual publisher age>=30s, independent watcher, no capture restart; publisher encoding continues. |
| media-ws-capture-preserving-recovery | Targeted native Media socket closure; bounded signaling return, same capture track, no new display request, new decoded/rendered frames. |
| gateway-only-preserves-peer | Exact Gateway socket fault excludes Media and HMR; original healthy peer/capture preserved. |
| offline-8s-no-new-capture-gesture | Browser context offline8s, retained capture and bounded recovery; established UDP may remain alive. |
| watch-navigation-channel-binding-global-controls | Real SPA navigation; correct channel/source labeling and reachable global stop/mute/leave. |
| live-claim-exclusive-and-loser-not-published | Both actual UI attempts prepared before either native Gateway `sig/t:p/k:l` claim is flushed. Exactly one live capture wins; loser has zero encoded video/bytes and ended capture. |
| 20-live-start-stop-bounded-client-resources | Every20 cycles and separate late observer; active native peer/transceiver/SDP bounds retained. No unexposed SFU-task claim. |
| cancelled-live-picker-keeps-voice-no-ghost | Capture failure with existing Voice retained, ended attempted capture and no ghost Live; native OS picker is separate. |
| camera-screen-live-source-identity | Simultaneous sources have correct labels/color bands and native decoded/rendered changing pixels. |
| autoplay-rejection-visible-click-retry | Actual-element play rejection with visible explicit retry; defaults tested separately without an autoplay override. |
| forced-track-arrival-reorder-source-identity | Hold three native video callbacks, reverse delivery to the real app, retain actual native peers/SDP/RTP and verify all sources' pixels. |
| rejected-native-sdp-keeps-voice-other-source-no-ghost | Real native malformed-answer rejection; attempted capture exists and ends; original microphone/no recapture plus camera and screen progress survive; no ghost Live, explicit new Live retry succeeds. |
| session-audio-mute-deafen-volume-playback-retry | Watch gets both room audio sources exactly once and chosen publisher Live only; positive other-room audio+Live excluded. After joining VoiceB, all three audio playback paths obey one session's retry, Mute/Deafen/Undeafen and0.35 volume. |
| Leave/Kick/Ban/Logout | Existing malicious native Gateway and decoded SFU with local UI-close suppressed; held unused/new tickets and REST denied. Independent other session survives Logout. Five original access cases. |
| channel-server-delete-active-sockets | Separate new fixture for channel and server deletion;204, native socket/frame stop, held/new-ticket rejection. Server denies chat REST; deleting only Voice preserves unrelated text. |
| api-redis-outage-live-lease-recovery | Both actual own API SIGSTOP and own Redis-proxy outage; native fail-closed socket/frame stop, natural5s lease expiry, passive roster corrects, explicit B join/new claim and selected B Live source recover. Fresh fixture per plane. |

Audio identity uses native MSID metadata and positive inbound RTP track identifiers; unreceived placeholder tracks cannot pass or falsely count as foreign. Repeated callbacks for the same track object are deduplicated. Private publisher/track IDs remain inside browser memory; reports export bounded counts only. Audio source filtering, receiver rendering, duplicate-element checks and volume/deafen state are complementary observations, not human listening.

## Assembled case evidence

| Local mode | Native PASS observations | Report boundary |
|---|---:|---|
| Chromium direct | 22 | 10 from successful selected-run gates;12 positive cases in an older failed parent whose cleanup succeeded. The failing SDP case has its separate latest strict PASS. No old whole-run PASS. |
| Firefox direct | 22 | 13 from successful selected-run gates;9 positive cases in the earlier closed-peer failed parent with successful cleanup. Latest strict SDP includes afterScreen/exact capture1 and has its own targeted PASS. |
| Chromium forced relay | 22 | Full22-case author run passed; later isolated strict SDP strengthens preservation. No remote/WAN claim. |
| Firefox forced relay, both explicit local adapters | 22 | 19 from successful selected-run gates; Logout2 observed PASS in the failed DeletedRoom parent whose3cleanups passed; LateWatch observed PASS in the aborted whole run with **failed cleanup**. Subsequent selected rest18 are all positive, including corrected DeletedRoom and both Leaseplanes. No synthesized whole-run PASS. |

Each row in the evidence JSON points through reportRef and sourceModule to the exact artifact hash and source-hash catalog; the hashed raw report retains the full test-module map. Last relevant asserted scenario source is retained even where the later common harness added error-path bounds. These are author controls, including dirty242/5f841f9 WIP; they are not executions of the later clean scenario commit. Earlier exact242 independent controls remain separately attributed.

Final owned-resource observations are dated after the last native control: five ports free, DBotherclients0. The explicitly excluded older server7de90fa2-d229-4453-8b68-b5db82d1429c remains with6channels/4messages/1attachment, unchanged and **not deleted**. No native runs are required after this handoff unless independent review finds a concrete issue and runtime exclusivity is coordinated.

## Controlled corrections and preserved reds

No original report is rewritten or retroactively passed. The evidence index records its original status and the later interpretation separately.

* Initial concurrent-claim control incorrectly expected a nonexistent `on` field. Final source contract is Gateway `t:p`/`t:u`; native claim holding regression excludes Media and unrelated sockets. The corrected actual concurrent UI scenario passes.
* The provisional publisher-only Audio assertion was a **test-contract error**, not a product finding. Final `media/tests/LIVE_WATCH.md:35–40`, `Peer::receives` and `watch_filters_existing_and_new_video_but_keeps_every_room_audio` require all same-room audio once, selected Livevideo only. Raw reports that called this a confirmed product failure remain unchanged; that classification is explicitly retracted here. Zero-RTP placeholder tracks also cannot establish a foreign source.
* Old Firefox whole-media report passed11 and failed3 because closed-peer `localDescription` access throws. Independent exact24211fb controls passed those3 (MediaWS, navigation, all20cycles) with Cleanup204; the old whole report remains FAIL.42bc812 separately fixed unavailable playback-quality counters via native RVFC without skipping native render/pixel assertions.
* Chromium SDP fixture initially failed before injection because camera native resolution adapted to480×270. Unchanged isolated control remained red. Native diagnostic records `qualityLimitationReason=bandwidth` for6.038s, cpu0, then sender/receiver640×360 at6400ms. This is observed browser adaptation, not a proved physical bandwidth cause or SDP defect.
* Coordinator-approved SDP-only camera fixture warmup: receiver640×360 within20s, with `WarmupMs`, deadline, dimensions and result separately recorded. Then the unchanged5s decoded/rendered/pixel progress check, positive screen/audio controls, actual SDP injection and strict preservation/retry. **No warmup after injection**, no sender/bitrate override, no GoLive-budget change. Timeout stays FAIL/Exit1 with `faultExercised:false`; actual-source regression confirms no progress/injection follows that failure.
* Positive other-room Live added status text to the channel's accessible name and exposed an exact-name SPA-link selector error. Redacted stage control identified navigation as the failure, while native source boundaries remained intact. Final code clicks the exact fixture channel href; the new control checks the same strict source/playback assertions. Raw failed controls remain FAIL.

## Firefox local relay diagnosis

Original default matrix remains FAIL/unclassified. A separate temporary-profile opt-in (`GELABBER_E2E_FIREFOX_LOOPBACK_ICE=true`) changes only `media.peerconnection.ice.loopback` for local Firefox forced relay; Direct, Chromium, autoplay, persistent profiles and WAN have no such override. Report metadata states pref/adapter activation, and ICE endpoints are categorical. The pref-only control allocated a loopback Relay candidate but did not establish a selected successful pair. The subsequent one-variable own-SFU loopback control established a selected succeeded Relay→Host pair and Audio-RTP0→211; separate actual-app GoLiveWatch/Gateway controls passed decoded/rendered640px changing source pixels, original Gateway peer/capture and cleanup200/204/200. No native MID/index override is used. Missing pair/STUN statistics are unavailable/null, never measured zero.

The short native generation control ends at4888ms with ICE **new**, not failed: it proves no pair within that observation window. One local/remote ICE-generation pair and one audio MID/BUNDLE transport were observed. SDP has local loopback Relay and remote non-loopback Host UDP candidates for components1/2. The native trickle input without MID and m-line index rejects with TypeError, while the RemoteDescription already contains candidates. This is a separate confirmed native input failure; it is not a demonstrated cause of relay failure. Source evidence that Mozilla rejects loopback/non-loopback candidate pairings motivates a controlled topology hypothesis; no runtime filter telemetry or packet-blockade claim is made. Native source/report hashes and the distinct one-variable candidate-input control remain author diagnostics, separate from acceptance.

## Bounded native interface and abort recovery

The old `until/observe` awaited probes without a bound; snapshot awaited `page.evaluate`, whose sample awaited native `pc.getStats`. This is a confirmed harness deadline omission, **not proof which native call caused the old hang**. The replacement propagates one absolute observation deadline and remaining budget. Expired budgets start no work; late replies cannot pass. Native stats timeout and outer evaluate timeout have distinct fixed phases, no empty stats/frame replacement. A timed-out page is quarantined before finally, close requested once, and never evaluated again. Affected subsequent cases are BLOCKED. `finish` bounds healthy stop-evaluation, pending abort-close, new cleanup context/page and context/browser closes; close/restore errors remain visible red, and report/owned runtime teardown proceeds.

A subsequent DeletedRoom report has exact `outer-page-evaluate-already-expired` phase: no native evaluation started, so this is no native getStats-hang proof. Deterministic early-waking timer source control confirms a final-poll scheduling edge. Fixed200ms cadence waits until the scheduled poll; when it reaches/passes the absolute end, the remaining window is waited without a new tail probe. `observe` retains the last measured value; `until` without a hit is the original CheckFailure with last. No deadline extension or late success. That report remains FAIL; only DeletedRoom is retried after the correction.

The same rule covers Access/Lease's held-peer stats, raw socket/ticket evaluations and scoped HTTP evaluations. Held `getStats` uses its remaining native budget; missing/timeout stats cannot become frames0 or successful revocation. Restore skips quarantined pages. Every owned context is attempted after a failed native restore/close. Lease finally attempts API resume, Redis restore and all contexts despite failures. Source-executing regressions cover never-resolving stats/outer evaluate, late resolution, normal increasing and post-revocation stagnant native frames, failed restores, and pending close paths. The targeted actual-app MediaWS control subsequently PASS/Exit0 with cleanup200/204/200 and both adapters; it did not reproduce or establish the earlier native hang cause.

Original `media-access-lease-firefox-final-relay-loopback-adapter.json` stays FAIL/incomplete, including `create-cleanup-context` failure. The authorized abort signalled only the freshly verified owned Firefox process. Separate restore proof does not pass that matrix. Subsequent authenticated API recovery deleted only the exactly mapped new fixture via normal server DELETE204, with a single temporary5-minute owner session; logout removed it. The older unassigned server and its attachment were untouched. Post-recovery and post-control dated checks show five own ports unbound and DBotherclients0. Recovery is separate evidence, never a replacement cleanup PASS for the old run.

## Run on the owned final runtime

From repo root, after sourcing the pinned toolchain (private env values must never be printed):

```bash
GELABBER_E2E_FAULT_ENV=/tmp/gelabber-toolchain/acceptance-runtime/fault.env \
GELABBER_E2E_API_MANIFEST=/tmp/gelabber-toolchain/acceptance-runtime/api-manifest.json \
GELABBER_E2E_MEDIA_FAULT_ENV=/tmp/gelabber-toolchain/acceptance-runtime/media-fault.env \
GELABBER_E2E_MEDIA_MANIFEST=/tmp/gelabber-toolchain/acceptance-runtime/media-manifest.json \
GELABBER_E2E_WEB_SNAPSHOT=/tmp/gelabber-web-snapshot-6e22170/web \
GELABBER_E2E_WEB_SHA=6e221705c34e944bd3804aa3c9482488e770f249 \
GELABBER_E2E_BROWSER=firefox GELABBER_E2E_NETWORK=relay \
GELABBER_E2E_PROFILE=media \
GELABBER_E2E_REPORT=/tmp/gelabber-e2e/firefox-relay-media.json \
node web/scripts/e2e/fault-run.mjs
```

The invocation above selects the baseline non-loopback SFU and leaves both adapters off. For the explicitly authorized **local Firefox forced-relay adapter**, add both `GELABBER_E2E_FIREFOX_LOOPBACK_ICE=true` and `GELABBER_E2E_SFU_LOOPBACK_ICE=true` and select `GELABBER_E2E_MEDIA_FAULT_ENV=/tmp/gelabber-toolchain/acceptance-runtime/media-fault-loopback-control.env`. Flag/scope guards reject Direct, Chromium, unknown values, missing browser-pref opt-in, a foreign control-env name, and an own-media origin override mismatch **before API/SFU acquisition**. The own launcher validates private bind/advertised configuration before SFU spawn; Vite routes to the returned attested SFU origin. Never enable automatically on retry. Report links the actual own SFU binary/config attestation, both adapter metadata, control-env hash and baseline hashes before/after. Baseline manifests/env stay byte-identical. This is no default/WAN acceptance.

Use profile `access` or `media-faults` separately; core-faults needs no SFU env/manifest. Do not combine PROFILE with CASES/SUITE. Ports/DB remain exclusive; coordinator must hand ownership back before an independent own-runtime run. No second worker. `finally` attempts API resume, all proxy restores, Vite/SFU/API close, runtime clear and private-cache removal despite individual failures, recording only fixed stage names/nonzero. Source regressions cover failed SFU stop/log close, API-resume/media-close and all earlier teardown failures. Cleanup login200/delete204/logout200 is recorded per owned server; scenario-deleted server204 is explicit.

## Commit/source review boundaries

A5f841f932be6d78435e2d9daf42f2e08e2a45ae8 is immutable and has40 original source checks/34 modules. Independent A review found three P2 harness defects; this handoff does not claim its approval. Separate source-only fixes are8011dd12866e6ca104a5b56d3a6f6c586fa5dee8 (queued/per-peer deadline prestart),e2374bba564ae7401545e427ac32ef49af4398c3 (ordinary healthy-stop rejection stays FAIL/Exit1), andf0f3c1da95fe68422c4f654d8c29b9757bdda98c (own media proxy/attestation coupling). Each has preserved parent-red/followup-green source checks. Coordinator reports independent8011dd1 PASS (five own repros/eight deadline checks/lint/hash validation; implementation-review11-deadline-8011dd1.md); Stop/Route follow-up reviews remain pending and overall A approval is not claimed. No additional native runtime was started for those error-path fixes. B scenario checks execute extracted parent+index source with all dependencies committed; C workflow/preparer changes are excluded. See evidence/media-scenario-source-validation.json for B hashes/checks and artifact-reference validation. Existing native reports retain their actual WIP module hashes; none is called an exact clean B execution.

## Gates still open

* **Native OS display picker:** synthetic canvas/cancelled-call controls do not invoke or accept an OS screen/window picker.
* **Audible two-device quality:** native audio packets, playing/unmuted elements and concealment counters do not demonstrate understandable speech. Two-device listening under ordinary and loss conditions remains required.
* **WAN/CGNAT/HTTPS and production digests:** all author runs use local HTTP. Chromium local forcedTURN passes; Firefox default forcedTURN remains red/unclassified, while explicitly adapted local relay has separate selected controls. The aborted adapter matrix and failed cleanup remain red. An explicit Firefox loopback-pref adapter is default-off and does not establish Firefox default/WAN acceptance. No deployment or production credential access. Safari requires a separate relevant Apple-platform control.
* **Full remote netem/CI/Docker/GHCR:** no remote push/execution authorized.4028136 kernel selected-UDP Drop/TCP-isolation controls are independently approved; complete SFU/coturn/audio/recovery under loss in actual remote CI remains open. Existing full smoke assertions are preserved.
* **SFU task/publication metrics:** no per-test-room counters exposed. Native client20-cycle bounds and API RSS observations cannot prove these server bounds; no invented RSS threshold or intrusive product telemetry.

No automatable row uses the obsolete absence of an owned API/DB/proxy or final08b runtime as a blocker. These local resources now support the actual slow-reader, deletion and lease faults. Full acceptance/release/production remains open, and independent review of this delta is still required.
