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
