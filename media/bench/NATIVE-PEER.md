# Fixed native peer0 instrument

`rtp-source` now also builds `gelabber-fixed-native-peer`. It is separate from
the earlier video-only binary. One native participant can publish its ordinary
WebRTC microphone, fixed VP8 screen video, and separate screen audio, while
receiving and actually decoding other microphones with libopus. This component
is the prerequisite for replacing browser peer0 in the original N-participant
fixture; the full three-engine adapters and graphs are still unqualified.

The committed fixture/reader uses the existing Rust/rtc/WebRTC pins. The only
new direct crate is the already resolved serde 1.0.229; serde_json 1.0.151 enables
exact float round trips. libopus 1.6.1 and Node 26.8.2 are executed and hashed in a
separate generator image, without changing production services or pins.

Build and run the offline importer tests:

```sh
CARGO_BUILD_JOBS=2 cargo build --locked --manifest-path media/bench/rtp-source/Cargo.toml
python3 -m unittest discover -s media/bench/tests -p test_native_audio_import.py
```

`--inspect-audio MIC SOURCE` opens no sockets. The Rust reader independently
decodes all 500 actual 320-byte 20 ms packets per track, checks mono TOC, decoded
float hashes, the regenerated fixed PCM hash, source settings and correlation,
and queries the configured encoder lookahead. The actual mapped decoder's hash
and version must match the frozen producer library. Python 3.13/3.14 compensated
float summation is also used when regenerating the four-tone source; ordinary
float addition changes some near-zero float32 bits. This is based on
[CPython 3.13.5's sum implementation](https://github.com/python/cpython/blob/v3.13.5/Python/bltinmodule.c).
Duplicated nested metadata keys, forged waveform labels, stereo payloads,
wrong packet schedules and changed decoder claims are rejected.

`prepare-native-runtime.py` defaults to a read-only JSON plan. Its base is the
exact independently recorded local Debian13 browser image
`sha256:a1087f71b2dbd30bd90b9f7dbd7dc42b2781c21b3c5873b14fc81ec9d158cc1b`.
It copies only the verified libopus 1.6.1, Node 26.8.2 and native binary into its
own UUID image. `--execute` runs new fixture generation, Python inspection and
native import under one CPU/512 MiB/256 PIDs with network disabled. It records image
IDs/labels, input/source hashes, git revision/dirty state, actual glibc/ldd and
mapped-library evidence. It installs no packages. Containers and temporary base
tags are removed by exact UUID name/label; `--keep-image` retains a successful
image for subsequent controls. All output folders must be fresh.

```sh
python3 media/bench/prepare-native-runtime.py \
  --library /path/to/pinned/libopus.so.0.11.1 \
  --node /path/to/pinned/node-v26.8.2-linux-x64/bin/node \
  --native-binary media/bench/rtp-source/target/debug/gelabber-fixed-native-peer \
  --video-archive /path/to/frozen.rtpbin --output /tmp/fresh-native-runtime
# Add --execute --keep-image after reviewing the plan.
```

The replay CLI is `--peer0 VIDEO MIC SOURCE BIND_IPV4`, with private stdin RPC.
It creates no PeerConnection until `create`, supports up to three PCs for a
single publisher and separate ordinary receiving transport, and exposes
`offer`, `remote`, `ice`, `start`, `status`, `bind` and `clock`. `bind` requires an
actual receive SSRC and a unique remote microphone identity. It cannot invent
an edge before receiving RTP. Native decoder counters expose missing/reordered
packets, timestamp discontinuities, actual decoded samples, RMS/peak and decode
errors. Missing packets are not silently treated as decoded/concealed PCM.
There is no acoustic device or native jitter-buffer/playout implementation.

All three sources share one 100 ms future `CLOCK_MONOTONIC` anchor. Nanosecond
clock values are decimal strings to avoid JavaScript precision loss. The
clock/Instant conversion is bracketed and refused if its bracket exceeds 100 µs.
Microphone SSRC is 0x474d4943, screen audio SSRC 0x47534130 and video SSRC 0x47565038.
Audio packets advance sequence and 48 kHz RTP timestamp by 960 across ten-second
loops. SDP remains `opus/48000/2` for actual mono content, per
[RFC7587](https://www.rfc-editor.org/rfc/rfc7587#section-6.1).
The source records planned sample ordinal and before/after enqueue timestamps.
Audio enqueue lateness over 20 ms or video lateness over one 60 fps frame fails the
instrument; the source does not silently catch up a stalled schedule.

The planned recorded PCM time is the packet's **first** sample time. The packet
is already encoded and queued at that time; capture waiting, live encoding and
DSP CPU time are excluded. Ten-second packet reuse retains actual codec
lookahead and is not a sample-exact seamless PCM oscillator. No native PCM
latency claim follows from these unmarked fixtures. A future latency mode needs
whole-run unique UID 0/64 PN marker archives, verified source/receiver clock
transfer and conservative decoded Worklet callback bounds. Browser rendering
quantum batching prevents treating an affine frame/wall-clock fit as exact.

`local-native-peer.mjs` is a 20..60 s actual Chrome 153 control with exactly one
native participant and one browser participant on the same local namespace. It
checks both native microphone/source-audio decoder edges at the browser,
fixed 1080p60 video, and the browser microphone's actual reverse Opus decoder.
Normal native exit/browser/HTTP cleanup is required. `--runtime-provenance`
binds the report to the frozen image/binary evidence. A container needs a
nonloopback interface for Chromium's ICE gatherer; `--bind-interface eth0`
selects its observed unique IPv4 address without changing host configuration.
This controlled local generator can use four CPUs/4 GiB/512 PIDs. It does not
measure backend resource acceptance or establish WAN/full-N qualification.

For upcoming full-N measurements, native/source/browser/proxy processes are
generator load. Current server+necessary signaling/ticket adapter and all
mediasoup probe/worker processes are backend load. Shared Redis is reported
separately. The Janus browser-event broker currently only replaces the browser
HTTP1 connection pool with N+1 real backend polls: it is client test logic.
Janus still lacks the required product ticket/ACL gateway. Its C-server resource
counts cannot qualify a replacement until that adapter exists and is included.
Active measurement and cleanup phases must remain separate: an expected child
exit after measurement is a cleanup fact; a missing PID during measurement is
an invalid resource sample, never zero CPU/RSS.
