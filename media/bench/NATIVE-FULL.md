# Native peer0 full participant fixture

This local instrument replaces browser participant0 with an ordinary native
WebRTC participant. It has **N** voice participants, not N+1: native0 publishes
its microphone, the fixed VP8 video and separate source audio, and actually
decodes every other microphone with libopus. Browser participants1..N−1 each
receive every other microphone, source audio and 1080p60 video. Thus the graph
contains N²−1 audio edges and N−1 video decoder edges. Every browser watches the
video. The manifest and measured graph retain these roles for N2/N8/N16/N32.

The same archived VP8 RTP and independently imported Opus packets run through
ordinary ICE/DTLS/SRTP on every engine. There are no Plain/Cascade transports.
The existing V1 audio archives repeat ten-second packets; no native marker or
PCM-latency qualification follows. The independent V2 PN reader/clock controls
are a subsequent gate. Fixed source rate and browser microphone128k settings
are benchmark inputs; they add no product bitrate ceiling.

Build the local browser bundle using the locked package dependencies:

```sh
cd media/bench
npm ci
npm run build
```

Use a successful `prepare-native-runtime.py --execute --keep-image` output
whose binary/library/Node hashes and actual Debian13 importer controls match.
The pilot requires executed Node26.8.2 and Chrome153, and hashes their actual
binaries. Inspect a plan before a fresh local run:

```sh
python3 media/bench/run-native-full.py \
  --runtime /tmp/frozen-native-runtime --output /tmp/fresh-native-full \
  --peers 2 --seconds 20 --warmup 10 --source-seconds 120
# --execute starts the three cases sequentially; --engines current limits a case.
```

The runner creates only UUID-labelled containers and a private Docker bridge
with no published ports. Its default unused subnet is172.30.232.0/24; overlapping
host routes/Docker networks are rejected. It needs a normal default route inside
the bridge because the unchanged current WebRTC mDNS socket joins INADDR_ANY;
a Docker `--internal` network without a route returns ENODEV. Docker adds
temporary bridge/veth interfaces and the corresponding private host route,
then removes the owned bridge during cleanup. It uses no host-network mode or
manual host network configuration. Production services, firewall rules and
system configuration are untouched.
Outputs freeze helper sources, bundle, package lock, collector and all image
IDs/labels. Each case uses a one-CPU/512MiB/256-PID server and a separate
four-CPU/4GiB/512-PID generator container on the same physical host.

Actual signaling binds each source: current user/tag/MSID, mediasoup producer/
consumer IDs and worker SSRC, or Janus feed/MID and offered SSRC. Native receipt
then binds the actual OnTrack SSRC; graph annotations alone cannot qualify an
edge. Every measured sample requires all expected connected transports and the
same actual sender/decoder identity. Real codec MIME, RTP clock and payload
mapping are checked. Every audio edge must advance actual decoded PCM, every
video edge must advance actual 1080p60 frames, with no new loss/concealment/order
errors. Empty RTP **with the actual padding bit** is counted separately from
real media. Padding never advances the expected960-sample audio timestamp;
missing media and unknown empty or invalid Opus packets still fail.

Each edge/sender must span the requested measurement duration minus at most
100ms for sequential getStats collection jitter. Reports retain per-stream
one-second bitrate distributions, all actual audio rates and decoder FPS,
join-to-DTLS/first-RTP/full-graph timing, native pacing lateness and common
CLOCK_MONOTONIC provenance. This pilot's mean-rate qualification does not claim
that every one-second video packet interval is constant. The fixed archive has
real frame/packet size variation; distributions remain available for review.

Idle, active measurement, cleanup and post-leave phases are separate. Actual
backend process-tree RSS/CPU includes current's control adapter and every
mediasoup worker. Missing PIDs or counters during measurement invalidate
resources, never become zero load. Post-leave resources require at least three
actual seconds and all owned peer/room/transport counters must clear before
containers are removed. Expected generator exit during cleanup is reported
separately. Generator process-tree load includes native source, browser and
frontend proxy. Private Redis is separately measured; TURN is absent locally.
Process RSS sums may count shared pages per process.

Janus's event broker is client test logic that retains N+1 real backend polls;
it is not a product gateway. Both alternatives still lack the authenticated
product contract in [GATEWAY-CONTRACT.md](GATEWAY-CONTRACT.md). Janus C-server
measurements can establish a resource lower bound, but cannot qualify missing
gateway-inclusive resource use. Cached current images may represent the older
de1ae reference; the report records that separately from collector source.
Final comparisons must rebuild the current accepted media library/lock with
the explicit two-thread benchmark profile. That profile differs from the
production default, whose resource use must not be inferred from it.

Local N2/N8 pilots must qualify every edge and generator CPU/RAM before larger
cases. If N−1 video decoders demonstrably exhaust the generator budget, freeze
one new identical larger budget for every engine and rerun qualification. This
tool does not run a WAN matrix or rank engines: independent-host measurements,
all requested three-round cases, calibrated true media latency, product access/
Watch/Live acceptance and an executed rollback remain open. Comparison and
production acceptance flags remain false even for a passing local pilot.

## Additive source, quality and throttling observations

`nativeFullGraph` retains its original strict `valid`/`failures` result. The
historical Current N8 concealment and mediasoup source-deadline failures remain
failures. New `source_graph` observations distinguish complete, owned receiver
and sender inventories, an unchanged common source clock and actual configured
sender rates from receiver quality. A measured concealment event or decoder
stall can coexist with valid source/graph inputs; it is still retained in the
strict pilot result and in every affected edge's quality record. A source
schedule, missing edge, changed receiver, missing/reset core counter or unequal
sender rate invalidates source/graph comparability.

`quality.edges` records measurement-window deltas for packets/loss/discards,
concealed and silently concealed samples, concealment events, inserted/removed
samples, decoded sample rates/FPS and actual packet/decoder stall intervals.
Jitter-buffer actual/target/minimum means use the delta of each cumulative delay
divided by the delta of emitted samples/frames; warmup totals are excluded.
Browser sample totals include concealment, so the nonconcealed rate is reported
separately. Signed packet-loss corrections are retained without clamping them
to zero. These semantics follow the [W3C receiver statistics definitions](https://www.w3.org/TR/webrtc-stats/#dom-rtcinboundrtpstreamstats).
Native receiver counters describe actual packet order and libopus decode; they
do not invent browser playout PLC, jitter-buffer or acoustic latency values.

Missing, nonfinite, reset or inconsistent quality counters make
`quality.complete` and `source_graph.measurement_comparable` false. Unsupported
browser fields remain unavailable rather than becoming zero. Quality is to be
compared with complete current-SFU baseline observations under equal inputs;
there is no universal zero-PLC product rule. Native PCM calibration continues to
require its independent strict, loss/concealment-free controls.

The owned collector now reads each actual root PID's cgroup-v2 path and raw
`cpu.stat` during idle/measurement/post-leave, for the backend, generator and
separate infrastructure. Its resource summary retains the first/last counters,
interval deltas, throttled periods and throttled microseconds separately from
process-tree CPU means. Missing counters, changed cgroups/clocks or counter
resets make this evidence unavailable. A process-tree mean below its CPU quota
does not establish absence of burst throttling. No process/CPU-cause attribution
follows from the recorded throttling alone.

An existing pilot directory can be inspected without starting fixtures:

```sh
python3 media/bench/evaluate.py /absolute/owned-native-full-directory
```

This writes an additive `summary.json`; it does not modify the raw reports.
`source_comparison` requires an actual Current run plus a candidate, identical
frozen audio/video archives, browser/Node/decoder provenance, complete topology,
source schedule policy and matched actual per-source sender bitrates. It is
only instrument comparability. The evaluator returns a failure for incomplete
inputs, missing quality or strict pilot failures, preserving historical red
results. Full quality/CPU/latency/stability comparison and acceptance remain
unavailable until complete matched separate-host runs, every edge's calibrated
native media latency, the requested matrix and gateway-inclusive product gates
exist. A passing synthetic evaluator test supplies none of that runtime evidence.
