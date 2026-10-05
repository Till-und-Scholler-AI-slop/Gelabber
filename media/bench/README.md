# Media backend evaluation

Local, opt-in tooling for the authorized alternative-SFU evaluation. It changes
no production backend, deployment file, workspace dependency, or lockfile. The
official Rust mediasoup API is evaluated first; Janus VideoRoom is the second
candidate. Product migration is blocked until the requirements below pass.

## Fixed implementations

| Engine | Probe and media process topology | Pin |
| --- | --- | --- |
| Current | Unchanged `gelabber_media::app` with a small fixture lease-minting route; Rust process | Repository revision and production dependencies preserved in `current-probe/Cargo.lock`, WebRTC 0.20.5 |
| mediasoup | Rust HTTP adapter plus native C++ worker threads in the same process; no Node server | `mediasoup = 0.29.0`, `mediasoup-sys = 0.19.0` |
| Janus | Native C server, HTTP transport and multistream VideoRoom plugin | v1.4.2, commit `0a24110ae55a172c4293749b763dbb66a138f9ec` |

The Janus Docker base is pinned by digest; apt resolves native dependencies at
build time. Save the image ID and `dpkg-query -W` output with evidence for an
exact native-runtime snapshot. Both candidate Cargo packages are independent
workspaces with checked-in locks. Production stack pins are unchanged.

## Build and run a local probe

Requirements: Linux `/proc`, Rust 1.98.1, Node with npm, Docker, Python 3 with
pip (the native mediasoup build creates a Python environment), C/C++ build tools,
and Playwright's Chromium browser. npm dependencies are locked separately.

```sh
# From the repository root; PYTHON may select an existing Python with pip.
bash media/bench/build.sh
# If the pinned Playwright browser is not installed:
cd media/bench && npx playwright install chromium && cd ../..

# Fresh output directory required. Full matrix: 36 independent backend runs.
python3 media/bench/run-local.py --video --duration 60 --warmup 10 \
  --output /tmp/gelabber-media-benchmark
python3 media/bench/evaluate.py /tmp/gelabber-media-benchmark

# Short infrastructure smoke, never acceptance:
python3 media/bench/run-local.py --matrix 2 --runs 1 --video \
  --duration 8 --warmup 8 --output /tmp/gelabber-media-smoke
```

If the current shell lacks the Docker supplementary group, execute the same
commands inside `newgrp docker`. The runner creates fresh UUID-named Redis or
Janus containers and native child processes, kills owned load-generator process
groups on interruption/failure, and removes only its own containers. It never
connects to a production Redis. Builds retain only benchmark binaries and the
benchmark Janus image; the image defaults to `--help`, not an unauthenticated
running server.

Every run creates fresh peers and a fresh backend. The order reverses on even
rounds. The default participant matrix is 2, 8, 16 and 32 (stress), each repeated
three times. The current wrapper mints real production-format lease claims in
its private Redis; that fixture is **not** proof that application authentication
or permission semantics can be migrated.

The runner freezes binaries, browser bundle, load-generator script and locks
in the output's `inputs/` directory before starting. Janus runs by its captured
image ID. Rebuilds in the checkout therefore cannot alter later rounds. Its
Node dependency tree is linked to the pinned installed `node_modules`; do not
reinstall those dependencies while a run is active. Both Rust fixture adapters
use two Tokio threads; this is an explicit benchmark configuration, not a
claim about production's default runtime thread count.

## Common stream graph and evidence

Every peer publishes one synthetic mono microphone. Every other peer receives
it. With `--video`, peer zero additionally publishes a 1920×1080 moving canvas
at 60 fps and a separate screen-source audio track, received by every other
peer. Current signaling sets screen Watch before publishing. Both native SDP
and mediasoup clients choose VP8, one encoding, resolution scaling 1 and
`maintain-resolution`. Opus inputs use 48 kHz. The requested encoder ceilings
are 128 kbit/s per audio track and `--video-bitrate` per video (default 6 Mbit/s);
the same setting applies to every engine. These are fixture settings,
not new product limits. Mediasoup's starting bandwidth estimate is set to
6 Mbit/s so a short local run is not dominated by its 600 kbit/s default.
Candidate-specific encoder minimum/start bitrate hints are absent. Use e.g.
`--video-bitrate 4000000 --warmup 30 --duration 60` on every engine if its native
VBR encoder cannot reach the same measured rate at 6 Mbit/s. The measured
per-stream bitrate gate must still pass; a requested rate is not evidence.

The separate [fixed native RTP diagnostic instrument](FIXED-RTP.md) prepares a
frozen, fully decoded VP8 source and normal WebRTC sender after the unequal WAN
video pilots. It is outside this original browser matrix, exposes its proposed
additional-publisher topology and has not completed a three-engine pilot.

An optional separate WAN control adds `--video --video-bitrate 4000000
--fixed-video-fixture --warmup 30 --duration 60` to either runner. It sets the
same Chrome VP8 `x-google-min/start/max-bitrate=4000` hints for every engine:
current/Janus Remote-Descriptions (including renegotiation) and mediasoup's
equivalent `videoGoogleMin/Start/MaxBitrate` codec options. It also keeps the
same 4 Mbit/s `RTCRtpSender` ceiling. This is a controlled benchmark source,
not a product cap or a CBR guarantee. Qualification additionally requires the
actual video sender rate within 10% of 4 Mbit/s, every receiver's actual rate
within 10% of that sender, and every receiver decoding 1920×1080 at ≥57 fps.
Missing executed-browser provenance also rejects this control. Run a 2-peer
pilot on all engines first; a failed pilot cannot justify resource rankings
or a larger video matrix.
The summary retains executed-browser provenance and requires identical
product versions, revisions and binary hashes across all cases being compared,
including voice-only and default video. Individually valid sources from
different browsers or missing binary evidence cannot qualify for a shared
performance comparison.

New Chromium runs obtain their actual executable, SHA-256, product version and
revision from CDP; package metadata alone does not establish which browser
ran. The installed browser inspected for this control is
`HeadlessChrome/153.0.8010.12`, Chromium revision
`971a7443b0c9b0a9b2860529b33331b76077ec62`, binary SHA-256
`ded93a9c9a53a1ae040f08124badcca95c938e9d5015ff340c3b5538c41bf39e`.
Its [Chromium DEPS](https://chromium.googlesource.com/chromium/src/+/971a7443b0c9b0a9b2860529b33331b76077ec62/DEPS)
pins WebRTC `9ea5afcad008b940468c2a15aec339592cf5a935`.
[GetBitrateConfigForCodec](https://webrtc.googlesource.com/src/+/9ea5afcad008b940468c2a15aec339592cf5a935/media/engine/webrtc_media_engine.cc#101)
converts these hints from kbit/s to BWE constraints;
[the negotiated send codec](https://webrtc.googlesource.com/src/+/9ea5afcad008b940468c2a15aec339592cf5a935/media/engine/webrtc_video_engine.cc#1352)
feeds those constraints into the transport. The
[VP8 encoder](https://webrtc.googlesource.com/src/+/9ea5afcad008b940468c2a15aec339592cf5a935/modules/video_coding/codecs/vp8/libvpx_vp8_encoder.cc#625)
still permits undershoot and frame dropping. Every run records the actual
browser again, so a changed binary requires its own source verification.

The first full voice-only WAN attempt was stopped after 15 of 36 cases because
the generator host's additional USB Ethernet interface repeatedly failed DHCP
and restarted every 45 seconds. Two Janus `ERR_NETWORK_CHANGED` failures
matched those NetworkManager transitions within milliseconds. Another case
had substantial per-edge RTP loss; that loss's exact cause remains open.
The [archived contaminated attempt](evidence/contaminated-voice-matrix-2026-10-05.json)
keeps raw-evidence hashes and all original outcomes. Every case, including
those that passed media delivery, is excluded from resource comparison.
Owned remote containers and image tags were cleaned after interruption.

One Chromium instance hosts all participants and retains each remote track in
a muted playing element, so received video is actually decoded. This makes a
local probe convenient but can saturate the generator at 16/32 peers; such a
run cannot establish server capacity. Chromium disables mDNS host candidates
for this private LAN fixture because Janus/libnice cannot resolve `.local`
addresses here. Identical browser flags are recorded for all engines.

Artifacts include raw per-second browser `getStats`, media-server process-tree
RSS/PSS/CPU/thread samples, idle samples, logs, source revision, code/lock/bundle
and binary SHA-256 values, browser version, hardware, and Janus image ID. RAM
includes the entire owned media server plus its adapter and native worker.
Shared infrastructure (the private fixture Redis) and the browser generator
are excluded. Janus RSS is read from its host PID; `record.py --container`
can additionally capture cgroup memory and CPU counters. RSS can double-count
shared pages across processes, so PSS and cgroup totals are retained where
available and should be inspected for a production process topology.

Per-peer timing records setup start, both transports' DTLS readiness (one
combined transport for current), first sent RTP, and readiness of the complete
inbound graph, polled every 100 ms during setup. Full-graph readiness includes
waiting for the other fixture participants to join; it is not a measurement
of joining an already populated production room. Sampling stops before the
steady measurement. Leave records engine counters where available (mediasoup
peers/transports/producers/consumers, Janus dynamic rooms, current's production
room/peer metrics), followed by three seconds of server resource samples. Live
peer/transport resources or rooms remaining after leave fail the fixture.

`evaluate.py` aligns server samples with the browser measurement interval,
counts every forwarding edge per peer, requires counters to advance, requires every
video receiver to decode 1920×1080 at at least 57 measured fps, and rejects
missing/failed evidence. RTX and mediasoup's separate `probator` track do not
count as extra media edges; their server overhead remains measured. Summaries
show CPU in mean cores, peak RSS, every sender bitrate and its min/max distribution,
join timing, post-leave resources and RTP jitter. Equal-stream
comparison is available only with at least three valid runs per engine and
measured median sender bitrates within 10%. RTP jitter is **not** end-to-end
audio latency. The optional calibrated PCM fixture below measures source-to-decoded
audio instead. Neither runner nor evaluator can issue migration acceptance.
Every audio receiver edge must also deliver within 10% of measured sender
bitrate bounds and report at most 1% packet loss in the measurement window.
Per-edge received rates/loss are retained; merely advancing a few packets
cannot qualify a partially delivered stress workload.

## Optional source-to-decoded PCM latency

Build the browser/worklet bundles, then calibrate the exact running browser:

```sh
cd media/bench
npm run build
node pcm-calibrate.mjs /tmp/gelabber-pcm-calibration.json
# Add to either runner's normal arguments:
# --pcm-calibration /tmp/gelabber-pcm-calibration.json --duration 60
```

The optional path adds an identical deterministic 126-ms BPSK marker every two
seconds to each microphone and separate screen-audio source. Each source has a
different 63-chip sequence, also varied by planned cycle to reject markers
delayed by a full period. All generated sources and decoded receiver tracks
use one 48-kHz AudioContext; AudioWorklet `currentFrame` timestamps the actual
source samples and a matched-filter peak in decoded PCM. Worklet messages do
not establish timestamps. The result includes codec processing, forwarding,
network, jitterbuffer and Web Audio routing. It excludes physical microphone,
speaker and acoustic latency, and does not establish subjective audio quality.
The source signal is a separate controlled fixture; the default remains unchanged.

Calibration checks 0/137/300-ms known direct delays, plus a 200-ms differential
delay on two paths of the same real Opus-decoded track. Silence, deterministic
noise and a different source code are negative controls. The declared detector
error bound is ±2 ms; the direct checks currently resolve within 0.25 ms and the
Opus differential within 0 ms. This bounds the marker locator under the tested
signal/codec conditions, rather than all acoustic or browser-device latency.
The runtime requires a passing calibration with identical detector-code hashes
and main/worklet bundle hashes, full PCM policy, and executed browser
product/revision/binary SHA-256. These must also match across every compared run.
Use `--full-chromium` for
calibration if the later diagnostic run enables `--protocol-logs`.

Every expected audio edge must identify its source and recover at least three
planned markers unambiguously within one second. Missing source emissions,
clipping, sample-frame gaps, decoder concealment above 1%, and sampleclock vs.
wallclock drift above 100 ms reject the fixture. The frame-gap counter detects
discontinuous worklet processing, rather than physical audio-driver underruns.
Both clocks must cover the RTP
measurement window. The summary retains per-edge min/median/max, pooled median
and p95; missing PCM evidence cannot become zero latency. At 16/32 peers there
are 240/992 microphone detectors plus screen-audio edges on one worklet thread:
generator resources and marker coverage are mandatory, and large loads may
fail this instrument's capacity before the backend does. PCM-enabled and
default fixtures cannot share a performance comparison.

The timestamp model follows the [AudioWorklet sample-frame clock](https://webaudio.github.io/web-audio-api/#dom-audioworkletglobalscope-currentframe).
[Jitterbuffer statistics](https://www.w3.org/TR/webrtc-stats/#dom-rtcinboundrtpstreamstats-jitterbufferdelay)
are cumulative decoder metrics and remain separate from this PCM measurement.
The [local calibration record](evidence/pcm-calibration-2026-10-05.json) identifies
the corrected instrument and exact code/browser hashes. Its known-delay checks
passed; no final three-engine or WAN latency comparison has been performed with
this corrected instrument yet.

## Separate-host measurement

Run the native adapter on a private server and `loadgen.mjs` on a different
machine. Export the same random `BENCH_TOKEN` (at least 32 bytes) on both. Use
a fresh dedicated Redis for the current fixture; never use deployment Redis.
Run only one backend at a time with the same UDP range and machine allocation:

```sh
# Current server; REDIS_URL must point to the fresh test Redis.
MEDIA_ADDR=0.0.0.0:8091 MEDIA_ICE_BIND=0.0.0.0:10000 \
MEDIA_ICE_PORT_MAX=10199 MEDIA_ADVERTISED_IP=SERVER_PRIVATE_IP \
  media/bench/current-probe/target/release/gelabber-current-probe

# mediasoup server (native worker UDP range 10000..10199).
BENCH_ADDR=0.0.0.0:8091 BENCH_ADVERTISED_IP=SERVER_PRIVATE_IP \
  media/bench/mediasoup-probe/target/release/gelabber-mediasoup-probe

# Janus server; HTTP config must use port 8091. The API secret is mandatory.
docker run --name OWNED_UNIQUE_NAME --network host \
  -v /absolute/test-http.jcfg:/opt/janus/etc/janus/janus.transport.http.jcfg:ro \
  gelabber-bench/janus:v1.4.2 --interface=SERVER_PRIVATE_IP \
  --nat-1-1=SERVER_PRIVATE_IP --apisecret="$BENCH_TOKEN"

# Load-generator machine, with its own npm ci / npm run build completed.
node media/bench/loadgen.mjs --engine mediasoup --backend http://SERVER_PRIVATE_IP:8091 \
  --peers 16 --video true --warmup 10000 --duration 60000 \
  --separate-host true --output /tmp/mediasoup-16-r1-browser.json

# Concurrently on the SERVER, using the actual owned adapter/container host PID:
python3 media/bench/record.py --pid PID --label mediasoup-16-video-r1 \
  --duration 90 --output /tmp/mediasoup-16-r1-server.json
```

The `--separate-host` value is only a claim recorded in evidence, not trusted
proof. Compare recorded server/generator host identities, allocation, clocks,
browser version and actual bitrates. Repeat 2/8/16/32×3 with both voice-only and
voice+video, fixed CPU allocation and no concurrent workload. Remote files
need the same `browser.json`/`server.json` layout as local runs for aggregation;
`summarize` can also be called directly. `record.py` has no fabricated media
or standalone synthetic bandwidth test.

## Acceptance before any production replacement

The whole media server + adapter + workers must remain at most **64 MiB idle**
and **256 MiB with 16 voices + 1080p60**, with CPU and true audio latency no
worse than the current backend under equivalent encoded streams. Three valid
runs and a separate generator host are required. Test WAN loss/jitter, TURN,
Chromium and Firefox, reconnect and overload. This initial fixture has no TURN
configuration. The optional PCM instrument requires its own calibration and
full comparable matrix; its local calibration alone cannot pass the latency
gate. Device/acoustic latency and product-policy gates remain open.

Product acceptance must separately cover production ticket lifetime/reuse,
server/channel/session ACL revocation, self/deaf/mute, Watch gating and unwatch,
screen vs Go Live source-audio separation, changes of room, teardown, codec
negotiation and playback in both browsers. Alternative fixture adapters do not
implement those policies. A successful low-RAM probe does not authorize a
backend switch.

## Primary references and licensing

* [Official mediasoup Rust API](https://docs.rs/mediasoup/0.29.0/mediasoup/) and
  [architecture](https://mediasoup.org/documentation/v3/): low-level media engine;
  application signaling remains an integration task. [ISC license](https://mediasoup.org/license/).
* [Janus VideoRoom](https://janus.conf.meetecho.com/docs/videoroom.html): multistream
  router with publisher/subscriber PeerConnections. [Pinned source](https://github.com/meetecho/janus-gateway/tree/0a24110ae55a172c4293749b763dbb66a138f9ec), GPLv3.
* [Janus AudioBridge](https://janus.conf.meetecho.com/docs/audiobridge.html) decodes,
  mixes and re-encodes audio; this fixture deliberately uses VideoRoom to preserve
  independent audio sources and equivalent SFU forwarding work.

Documentation does not establish a RAM/performance winner; the reproducible
evidence above is the prerequisite for that decision.

## Bounded remote server, local browser generator

`build-probe-images.py` packages the existing frozen binaries in the pinned
Debian 13 runtime and starts each image without network access to verify the
Rust and native worker startup. Both existing binaries passed the Debian 13
loader and runtime tests; no stack upgrade or VPS package install is needed.
Rebuild the probes first if their source changed. Janus's launcher can set its
HTTP port via `BENCH_HTTP_PORT`, and all engines support the dedicated UDP range.

```sh
python3 media/bench/build-probe-images.py
docker build --tag gelabber-bench/janus:v1.4.2 media/bench/janus

# Plan only: local image inspection, zero SSH calls and zero remote mutations.
python3 media/bench/run-remote.py --ssh-host root@TEST_SERVER \
  --ssh-control /absolute/existing/control-socket --server-ip TEST_SERVER_IP \
  --matrix 2 --runs 1 --video --duration 8 --warmup 8 \
  --output /tmp/gelabber-remote-smoke
# After inspecting server capacity, ports and the concrete plan, append --execute.
# Full matrix defaults to 2/8/16/32 ×3, duration60/warmup10; remove smoke overrides.
python3 media/bench/evaluate.py /tmp/gelabber-remote-smoke
```

The runner transfers only its exact media images via a `docker save` SSH stream,
with a conservative image-size upper bound of 1.5 GiB and no server-side archive.
It creates unique image tags and UUID-labelled containers, then runs one media
engine at a time with **one CPU quota, 512 MiB memory/swap ceiling and 256 PIDs**.
The current fixture's dedicated Redis has a separate 0.25 CPU/64 MiB budget and
a loopback-only random TCP port. Media uses host networking on **TCP18091** and
**UDP11000..11199**. Check that this range is free and reachable before execution;
the runner does not modify firewall, Compose, Watchtower or production services.
Owned container names and labels must both match before sampling or cleanup.
Owned containers, anonymous volumes and unique image tags are removed on failure
or completion; imported layers already referenced by other images remain intact.

Raw server evidence includes process-tree and cgroup memory/CPU/throttling data,
fixed resource limits and a bounded SSH clock-offset estimate for aligning
samples. The local browser inputs and exact server image IDs are frozen before
rounds. A shared production host can add contention beyond the quota; this
limitation remains an explicit acceptance blocker, even if all local workload
and resource checks pass. These runs establish candidate behavior under the
same allocation and a real WAN path, not TURN, permission-policy equivalence,
browser-family acceptance or source-to-decoded audio latency unless the
separately calibrated PCM fixture is explicitly enabled and qualified.

The collector, remote helper, full runner and browser scripts are frozen and
SHA-256 recorded before execution. Image labels retain the binary hash/source
revision, and evidence records Git's dirty status. Private Redis's exact image
ID and separate idle/load/post-leave RAM/CPU samples are retained in addition
to media-only measurements. Cohosted live service contention remains a blocker;
the runner does not interrogate or change those services.

Separate-host runs additionally retain the local Node/browser process-tree
RSS/CPU and generator hardware. This can reveal generator saturation at 16/32
participants; CPU totals are reported only while its live-process counters
remain monotonic. Negotiated DTLS/SRTP ciphers are shown: engines keep their
native security defaults, so CPU comparisons describe complete default backend
paths rather than an identical-cipher microbenchmark.

## Protocol diagnosis before changing the workload

When a long pilot loses 1080p60 or produces unequal actual rates, stop the matrix.
Do not interpret lower CPU as greater efficiency or keep reducing the fixture
rate until it appears to pass. Repeat the same two-peer pilot with
`--protocol-logs` to enable Chromium's local RTC event logging. This uses the
installed full Chromium in headless mode because headless-shell lacks its event
logger; the channel is recorded. This adds diagnostic I/O and is excluded from
performance evidence. Codec, RTP extension,
feedback and SSRC negotiation is retained in `browser.json`; ICE credentials
and candidates are omitted from that negotiation excerpt.

```sh
python3 media/bench/protocol-diagnostics.py /tmp/RUN/current-2-1
```

`protocol.json` shows the source capture/encoded frame rates and
`delta totalPacketSendDelay / delta packetsSent` in milliseconds per packet.
A cumulative send delay by itself cannot prove a stall. The optional Chromium
v2 event logs retain decrypted RTCP feedback: the helper decodes all compound
RR/SR report targets, REMB target SSRCs/rates and TWCC sequence/arrival deltas,
including blob-batched packets. Batched event timestamps remain batch anchors;
the helper does not reconstruct the compressed per-event timestamp deltas.
The first delay/loss bandwidth estimate in each batch is shown, with the number
of compressed updates not decoded; this is a sparse diagnostic timeline.
Raw logs are retained and parser errors are explicit. This is protocol failure
evidence, never an acceptance result.

The checked-in [failed video pilot evidence](evidence/failed-video-pilots-2026-10-05.json)
records long two-peer WAN pilots that failed actual rate/FPS qualification,
including a shared 4 Mbit ceiling and the diagnostic repeat. A lower CPU reading
from these runs cannot rank engines. The diagnostic repeat had matching RR
source targets, no REMB messages and little or no TWCC-reported loss; that does
not establish the cause of the bandwidth reduction. The subsequent
[loopback control](evidence/loopback-control-2026-10-05.json) qualified all three
engines at approximately 4 Mbit/s and 60 decoded fps with the same fixture.
That shows reproducible path dependence; one local diagnostic round does not
prove server capacity or a particular WAN/congestion-control cause. A separate
voice-only matrix can establish partial audio findings while the video,
production-contract and end-to-end latency gates remain open.

The later [namespace WAN pilot](evidence/namespace-video-pilot-2026-10-05.json)
used the identical optional 4-Mbit/s VP8 source hints on all engines. Its browser
had only `lo`/`eth0` and no recorded page errors, while both namespace and host
held-GET controls also reported zero errors. Those controls did not reproduce
the earlier POST/long-poll error and do not prove DHCP causality. Every engine
still failed actual video rate/FPS qualification (0.442/0.611/2.606 Mbit/s and
12.0/16.8/56.6 fps for current/mediasoup/Janus); audio stayed near128.4 kbit/s.
Runtime image/libraries, browser and server inputs are recorded. This failed
source pilot cannot justify server CPU/RAM or replacement rankings.

Primary format references: [Chromium's local event-log switch](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/content/public/common/content_switches.cc),
[WebRTC v2 schema](https://webrtc.googlesource.com/src/+/refs/heads/main/logging/rtc_event_log/rtc_event_log2.proto),
[WebRTC blob encoding](https://webrtc.googlesource.com/src/+/refs/heads/main/logging/rtc_event_log/encoder/blob_encoding.cc),
[RTP report blocks](https://www.rfc-editor.org/rfc/rfc3550.html),
[TWCC feedback](https://datatracker.ietf.org/doc/html/draft-holmer-rmcat-transport-wide-cc-extensions-01),
and [REMB feedback](https://datatracker.ietf.org/doc/html/draft-alvestrand-rmcat-remb-03).
