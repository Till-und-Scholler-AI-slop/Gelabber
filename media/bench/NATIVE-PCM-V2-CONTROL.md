# Opt-in finite native PCM V2 direct control

`native-pcm-v2-control.mjs` is a separate local adapter around the reviewed
whole replay/tail guard. The 13 causal clock files and the three replay-contract
files remain unchanged. New bridge, browser and receiver modules prepare a
direct native publisher/browser receiver instrument. This is not an SFU, device,
product or resource comparison. Every report keeps `comparison_available:false`
and `pcm_latency_calibrated:false`, even when captured replay evidence qualifies.

The default invocation produces a **read-only plan**. It hashes actual files and
reads the frozen archive headers/book; it starts no native process, browser,
peer, socket or RTP and creates no output directory. Only explicit `--execute`
creates a fresh private output directory, one owned native process, one local
HTTP bridge and one fresh browser instance/server. Source review and browser ownership
must be settled before actual controls. No hour-long test is included.

The native starter reuses only `NativeRpc` constructor/serialized `call`/bounded
`close`, and supplies the actual V2 parser order:

```text
native --allow-test-audio-hold --peer0 VIDEO MIC_V2 SOURCE_V2 127.0.0.1
```

The browser uses public Playwright `launchServer`/`BrowserServer.process` and
`connect`, retaining the actual new child handle. Immediately after launch the
runner measures `/proc` PID, parent PID, detached process group/session,
start-time ticks and actual executable hash. This identity is unrelated to old
handoff PIDs. Node wraps browser preparation, completion and disposal, including
all nested `getStats`, SDP and AudioContext awaits, with outer deadlines.

The reviewed frozen native SHA is `328d63a837a1db474fd3078ade33f7082a97daf029be1e4e641a5ba448966baf`;
libopus SHA is `ce07b3578b14e1d25ed603670f2336cd2b32b7f24c2c5b9aab8bbeb0f410b8d6`.
Node 26.8.2 and Chromium 153 version/revision/hashes come from the immutable
clock policy. A newer installed Chromium is not a substitute. The starter checks
the actual child executable and unique mapped libopus path under `/proc`, then
binds the captured greeting to both complete imported archives. CDP identifies
and hashes the Chromium executable that actually runs. Helpers and both original
and copied source files are hashed before/after execution.

## Fresh source and read-only plan

`--pair` is the independently frozen `readReplayPair` manifest with absolute
Mic/Source paths, both archive hashes and complete generator provenance.
`--generation` retains the separately frozen actual source generation record
with the same run ID/provenance and artifact hashes indexed by basename. Its
Python/runtime environment describes that recorded generation, not a new
generator execution by this runner. A newly generated host fixture must retain
its new hashes/environment; it is not the historical Docker archive/runtime.

The October 6 fresh fixture uses one canonical shared run UUID, full 20-second
measurement plus 1-second tail, actual host Python 3.14.7, pinned Node and mapped
libopus. The root's independent native import is a separate offline integrity
gate. The following plan only reads that fixture; it is not an executed control:

```sh
/tmp/gelabber-v04-toolchain/node-v26.8.2-linux-x64/bin/node \
  media/bench/native-pcm-v2-control.mjs \
  --pair /tmp/gelabber-native-v2-inputs-20261006-y7znj_h8/pair.json \
  --pair-sha256 5c512896db015dc246e94978f9c8538c842ae85a69506ee0413595e43c892da9 \
  --generation /tmp/gelabber-native-v2-inputs-20261006-y7znj_h8/generation.json \
  --generation-sha256 5849591a4e6671fc5292984a313bd1e0935455ef5d04fd100c923bfb2e99698c \
  --video /tmp/gelabber-native-v2-inputs-20261006-y7znj_h8/video.rtpbin \
  --video-sha256 00e33765442fdbd3d9d104530269745c5de64c77e1a120ea7f017f9671062c2f \
  --binary /tmp/gelabber-v04-native-resume-target/release/gelabber-fixed-native-peer \
  --library /usr/lib/libopus.so.0.11.1 \
  --chromium /home/rafi/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell \
  --output /tmp/gelabber-native-v2-direct-control-0ms-20261006 \
  --audio-hold-ms 0
```

Add `--execute` only for an authorized actual run after source review. Use one
fresh output directory per run. Repeat the same frozen whole pair/video and
unshifted timeline policy for `--audio-hold-ms 0`, `50`, `200`, `500`; no custom
duration, rewind, loop, marker subset or tail truncation is accepted. Temporary
inputs may disappear; recreate/freeze a new complete pair instead of asserting
the old hashes. The runtime copies source archives and relevant helpers into the
owned output directory before launching them.

## Preparation and actual received evidence

The browser HTTP whitelist permits only one `create`/`offer`/`remote` sequence,
bounded ICE and status/clock calls. Browser `start`, `close`, foreign peers,
duplicate JSON keys and extra legacy fields are refused. The Node controller
alone sends the trusted entire `replayRequest` after independently checking
the returned preflight against its actual captured native offer/status.

Before that finite common start, the actual SDP must bind all three distinct
Mic/Video/Source tracks by MID, media MSID, fixed SSRC and codec. The browser
holds the exact `RTCRtpReceiver`, transceiver, track and stream objects from
the corresponding real `ontrack` events. Its graph must be connected with three
receivers, two separate enabled/live audio tracks, one video track and no local
publication. Native negotiated sender MID/MSID/SSRC inventory must match, with
no source started. Each audio worklet is fed only its held actual receiver
track. The video track is attached to the owned muted video element.

Each held audio receiver must expose one actual inbound report with matching
trackIdentifier/MID/SSRC/Opus codec. Before source start, all packet/sample and
loss/PLC/discard/insertion/removal counters must actually exist and be zero.
No missing report/statistic is filled with a zero, inferred ID, synthetic report
or warmup subtraction. A browser that creates inbound reports only after first
RTP therefore **fails preflight**, and finite replay does not start. This is an
explicit remaining runtime constraint, not established browser acceptance.

After Node sends the actual start, its common `CLOCK_MONOTONIC` anchor is passed
unchanged to both worklets. The hold shifts actual native audio enqueue deadlines
while source planning/video remain unshifted. Browser polling keeps the original
held objects and report/codec identity, checks the complete inbound graph and
retains every available raw table. Missing/regressed counters, any observed
loss/PLC/discard/stretch, graph or source scheduling failure and totals exceeding
the finite archive fail. Native completion never substitutes for receiver data.

Completion waits for the native full sources, exact whole decoded counts, every
genuine PN peak and a real published AudioWorklet input block through the derived
tail end. The SAB read uses sequence fences and complete actual input; it never
projects a future frame from context time. Subsequent native probes are retained
so required callbacks have actual after-brackets. The final same-report stats
are captured after the needed input coverage, and the unchanged replay guard
then checks every marker, whole tail, causal callback range, context state and
receiver counters. Future PLC observed during this final collection remains a
failure; it is not removed from the totals to force success.

`--observer-pause-ms` (default `.25`), `--capacity` (default `4096`),
`--clock-delay-ms` (default `0`) and `--suspend-ms` (default `0`) are explicit
bounded failure-control seams. Paused observer/ring loss, wide clock brackets
and actual AudioContext suspension must fail captured qualification. PLC and
sample stretching are rejected from real statistics; this adapter does not
manufacture their counters or claim an unexecuted PLC control.

## Output and checks

The plan and report retain `deadline_budgets`: browser prepare at most 120s;
finish exactly `total_seconds*1000 + audio_hold_ms + 20000`ms (41..41.5s for the
21s fixture); disposal at most 10s. Graceful browser/client/server close gets 5s,
with a separate maximum 5s for forced cleanup, so awaited browser close phases
are bounded at 10s. HTTP close gets 5s and native close 10s. Launch, connect,
CDP provenance, page creation and navigation also have explicit Node budgets;
Playwright launch itself uses its 30s timeout. No hour-long control is created.

Each timeout persists stage, budget, actual start/end and elapsed time in
`stageTimeouts`, using Node monotonic time only as a deadline diagnostic. It
invalidates qualification. Actual monotonic time is checked before invoking
the operation and after successful resolution, so blocked Node timers cannot
accept a late success or begin an operation after its budget has expired.
Hung disposal never prevents subsequent browser,
HTTP or native cleanup. On a browser close hang, a fresh `/proc` identity must
match the measured new child, including start ticks and executable hash, before
public `BrowserServer.kill()` may kill its owned detached process tree. A
changed identity refuses that force action and reports failed cleanup. A hung
force API is itself bounded; HTTP/native cleanup still follows. Forced cleanup
never turns the control green. Actual child exit and disconnected browser are
required alongside the normal resource cleanup. The standalone CLI writes its
report before terminating on a timeout or incomplete browser process cleanup,
so pending owned IPC promises cannot keep the command alive indefinitely.

`report.json` retains source/runtime hashes, raw native greeting/signaling,
preflight report tables, actual source start/status, browser stats/PN/worklet
rows/clock probes, final qualification and owned cleanup. Failed preparation
retains its available stats and diagnostic callback evidence. `summary.json`
binds the full report SHA. A failure exits nonzero, clears overall qualification,
and keeps both comparison/calibration false. Cleanup attempts every owned browser,
HTTP, worker, audio and native resource; normal native exit and closed browser,
peer connection, context/worker/node cleanup are mandatory for qualification.

Focused unit checks execute only synthetic SDP/receiver/stat fixtures, local
mock HTTP, and an actual non-executable-file EACCES negative. They launch no
real native peer or browser and cannot establish actual PCM/replay acceptance.
Never-resolving preparation/disposal/close/force promises verify outer budgets
and cleanup sequencing, while synthetic changed-identity fixtures verify that
the force API is not called for an unverified process. Synchronously blocked
Node loops also test late resolution and delayed microtask invocation.

```sh
/tmp/gelabber-v04-toolchain/node-v26.8.2-linux-x64/bin/node --test \
  media/bench/tests/nativePcmV2Bridge.test.mjs \
  media/bench/tests/nativePcmV2Receiver.test.mjs
```
