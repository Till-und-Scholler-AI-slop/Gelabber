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
are 128 kbit/s per audio track and 6 Mbit/s video; these are fixture settings,
not new product limits. Mediasoup's starting bandwidth estimate is set to
6 Mbit/s so a short local run is not dominated by its 600 kbit/s default.

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
audio latency. Neither runner nor evaluator can issue migration acceptance.

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
configuration and measures no true end-to-end audio latency; those gates are
open, not implicitly passing. Use a synchronized audio marker/correlation
measurement for latency and report distribution rather than only averages.

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
