# Fixed native video diagnostic instrument

The browser canvas WAN pilots did not supply equal 1080p60 inputs. Requested
Chrome minimum/start/maximum bitrate hints did not guarantee actual 4 Mbit/s or
60 fps. Those observations cannot rank media-server resources. The separate
fixed-source instrument below removes the live video encoder from the source;
it does not change the existing browser matrix or production backend.

## Prepared components and actual offline result

`vp8-fixture.py` builds one ten-second FFmpeg `testsrc2` clip with the installed
libvpx encoder, checks every IVF timestamp and keyframe resolution, and fully
decodes it with FFprobe. It records the actual FFmpeg/FFprobe/libvpx binary
hashes and command. No codec or system package is installed or upgraded.

The first local offline fixture contained 600 decoded 1920×1080 VP8 frames at
60/1 fps, five keyframes at two-second intervals, and 4,993,841 coded bytes:
3,995,072.8 bit/s without padding. Its RTP payload, including the VP8 descriptor,
is 4,009,520.8 bit/s. This is fixed VBR content with keyframe-size peaks, **not
per-packet CBR**. A second encode with identical arguments/versions produced a
different coded SHA and 3,959,688 bit/s; repeated raw input checks matched.
The encoder recreation is therefore not claimed to be bit-identical.

Every engine must consume **the same frozen archive**, rather than rerun the
encoder. Retain the IVF, RTP archive and JSON together as run inputs. A changed
archive/source hash is a different workload even if both clips are individually
valid. The checked-in evidence is `evidence/fixed-vp8-source-2026-10-05.json`;
the larger generated video/archive remain external local artifacts.

The packetizer uses [RFC 7741](https://datatracker.ietf.org/doc/html/rfc7741):
90 kHz timestamps advance by 1,500 per frame, a 15-bit PictureID identifies each
frame, S marks its first fragment and M its final fragment. RTP datagrams are
at most 1,200 bytes before SRTP/IP overhead. Fragments are spaced within each
1/60-second frame interval. Looping preserves continuous sequence numbers,
timestamps and PictureIDs while replaying identical coded frame bytes.

`rtp-source` is an independent locked Rust package using the existing
`webrtc = rtc = 0.20.5`. Every shared transitive version matches
`current-probe/Cargo.lock`. Its reader checks packet order, marker/PictureID,
exact deadlines, source rate and complete frame count; it reconstructs the
original IVF and verifies its SHA. Truncation, payload changes or altered
deadlines fail instead of silently reducing the workload.

The sender uses `TrackLocalStaticRTP` and normal WebRTC ICE/DTLS/SRTP, with no
live encoder, padding or feedback-driven rate adaptation. RTCP SR/RR and NACK
remain enabled; it advertises no RTP header extensions that its fixed packets
do not contain. PLI does not insert an extra keyframe outside the fixed clip;
the next scheduled keyframe remains available. A send delayed by more than one
frame interval fails instead of bursting through a missed schedule. Source
counters explicitly describe packets **enqueued to the WebRTC driver**; actual
outbound counters and every receiver's bytes/frames must prove transport and
decode. An unchanged enqueue counter alone cannot qualify a stream.

This non-adaptive source is for bounded controlled tests only. The fixture's
4 Mbit/s is a chosen measurement input, not a product bitrate limit or a new
general-purpose congestion-control implementation.

## Offline commands and optional local decoder control

```sh
# All output paths must be fresh. One encoder thread; no networking.
nice -n 15 python3 media/bench/vp8-fixture.py /tmp/fixed-video.ivf \
  --build --seconds 10 --rtp /tmp/fixed-video.rtpbin \
  --output /tmp/fixed-video.json

cargo build --locked --release --manifest-path media/bench/rtp-source/Cargo.toml
SOURCE=media/bench/rtp-source/target/release/gelabber-fixed-rtp-source
"$SOURCE" --inspect /tmp/fixed-video.rtpbin

# A plan validates the source offline without opening ICE sockets or Chromium.
node media/bench/local-fixed-video.mjs --binary "$SOURCE" \
  --archive /tmp/fixed-video.rtpbin --warmup 10 --seconds 20 \
  --output /tmp/fixed-video-plan.json

# Explicit separate loopback source → Chromium decoder test; no SFU or WAN.
node media/bench/local-fixed-video.mjs --binary "$SOURCE" \
  --archive /tmp/fixed-video.rtpbin --warmup 10 --seconds 20 \
  --execute --output /tmp/fixed-video-loopback.json
```

`native-video.mjs` controls the owned child privately over JSON stdin/stdout.
No additional HTTP endpoint, signaling secret or backend listener is opened.
The executed native binary/archive hashes and actual Chromium binary/revision
are retained. Child EOF/close cancels replay and closes its WebRTC transport;
the controller terminates only its owned child if orderly shutdown times out.
The local decoder check requires actual continuous 1080p at 57..63 decoded fps,
received payload within 10% of the frozen source, at most 1% loss and a complete
native schedule. Its `instrument_local_valid` is **not** backend acceptance.
An independent local loopback control replayed 1,800 frames in 30.001 seconds;
Chromium decoded 4.012 Mbit/s at 59.979 fps with zero lost packets. The actual
source sent 13,545 WebRTC packets / 15,035,703 RTP payload bytes, with maximum
schedule lateness 2.062 ms. See `evidence/fixed-vp8-loopback-2026-10-05.json`.
This establishes this one loopback instrument path, not any SFU or WAN gate.

## Local three-engine video-only diagnostic

`run-fixed-video.py` freezes the helpers, actual bundle, native binaries and
source archive under a fresh output directory, then starts each backend
sequentially using unique owned containers/children. The native publisher and
browser decoder use ordinary WebRTC signaling/DTLS/SRTP; there are no voice
participants or audio tracks. It records source, actual decoder, separate
generator/backend/Redis resource samples and post-leave backend counters.
Video validity, cleanup validity and resource-measurement validity are separate.
Missing PID/RSS/CPU measurements, failed Leave, forced native termination or
failed owned-container cleanup invalidate their respective checks and exit
nonzero. Every comparison and product-acceptance flag remains false.

```sh
npm --prefix media/bench run build
# Choose an already configured nonloopback IPv4. No host configuration changes.
python3 media/bench/run-fixed-video.py --binary "$SOURCE" \
  --archive /tmp/fixed-video.rtpbin --media-ip 192.0.2.10 \
  --warmup 10 --seconds 20 --output /tmp/fixed-video-sfu-diagnostic
```

The example address must be replaced by the host's actual existing address.
Janus skips loopback interfaces when binding candidate sockets; rewriting a
nonloopback socket to 127.0.0.1 did not produce a usable native ICE path. This
runner explicitly binds/advertises the supplied address for every engine and
restricts Janus candidate gathering to that address. HTTP remains on localhost.
The default diagnostic UDP range is 12000..12199; choose a free owned range.

After correcting adapter setup and fingerprint negotiation, the local control
delivered the same frozen archive through all three engines: current decoded
4.021 Mbit/s at 59.980 fps, mediasoup 4.000 Mbit/s at 59.979 fps, and Janus
4.004 Mbit/s at 60.033 fps, each with zero lost packets. Every source completed
2,400 frames / 40 seconds and every Leave check completed. The archive, actual
browser, source/collector hashes, earlier failed controls and their specific
causes remain in `evidence/fixed-vp8-sfu-diagnostic-2026-10-05.json`. These are
single local video-only cases; **there is no resource ranking or migration
decision**. Native/current binaries still represent the recorded older
`de1ae346` product baseline, not subsequent layer/recovery fixes.

A second, shorter control used the final strict collector and again passed
the video/Leave checks on all three engines. It exited nonzero because a
Chromium PID disappeared during Janus generator shutdown while its resource
snapshot was being read. That Janus CPU/RAM measurement is explicitly invalid;
its valid source/decoder checks do not override the missing resource evidence.
These diagnostic cases used the executed Node 26.7.0 and recorded its binary
hash. The upcoming full matrix must use the configured Node 26.8.2 runtime;
an installed-version declaration cannot substitute for executed provenance.

## Adapter and topology gates still open

`mediasoup-native-sdp.mjs` uses the already pinned official mediasoup-client
3.24.1 SDP/ORTC helpers to map a single native VP8 offer to a normal
`WebRtcTransport` answer and producer parameters. It rejects extra tracks,
codecs, encodings, mismatched SSRCs and Plain/Direct transports. Its offline
tests cover PT/SSRC preservation, DTLS roles and rejection. The bounded source
must offer `setup:actpass`; the bridge selects a real SHA-256 fingerprint from
the worker's offered certificate fingerprints. The pinned rtc verifier supports
only SHA-256, whereas the pinned official RemoteSdp helper defaults to the last
fingerprint (SHA-512 here). No hash is invented and certificate checks stay on.
Actual connected source stats, rather than an SDP role declaration, establish
successful DTLS. This helper alone proves no delivery.

The implemented diagnostic is **one video-only native publisher plus one
browser decoder, zero voice peers**. It has two media WebRTC transports; Janus
also uses a room-management session, for three actual backend event polls.
No screen-source audio or native PCM latency is supplied. Do not pass its
report to the original N-peer evaluator or claim the original
microphone+video+source-audio product graph has passed.

The preferred full N-peer topology replaces peer zero's publication with one
native microphone, one native VP8 track and one native screen-source audio
track. Peers 1..N−1 keep their existing browser sources/decoders. Implementation
work remains in four concrete areas:

1. Encode identical microphone and independent screen-source audio as 48 kHz
   Opus inputs and freeze the exact packets/provenance. Preserve three distinct
   native track IDs and the same input identities on every engine.
2. Current must mint a real fixture ticket, announce screen/source-audio and
   apply Watch before publication, handle subsequent native offer/answer/ICE,
   and receive the other N−1 microphones on peer zero's existing connection.
   mediasoup must produce all three tracks through the native send transport
   and consume/decode those microphones through its receive transport. Janus
   must use the ordinary publisher and multistream subscriber WebRTC handles.
3. Peer zero must actually decode each received Opus edge, not merely count
   inbound packets. Keep total peer/transport/track counts, all receiver edges,
   join and post-leave resources visible, and include native source/decoders in
   the separate generator CPU/RAM accounting.
4. Add new source-to-decoded PCM clock/codec calibration if latency markers are
   desired. The current Browser AudioContext calibration does not bind a native
   scheduled source and must **not** be reused. Until this clock/error bound is
   demonstrated, native-source latency remains unavailable.

All engines must share the exact source/archive/pacing/decoder policy and actual
browser identity. After a qualifying two-peer pilot, reproduce the 2/8/16/32×3
matrix under the same resource quota. Receiver FPS/rate/graph gates remain
mandatory; a frozen source declaration alone does not prove an equal workload.
Product ACL/ticket/revocation/Watch/source-audio/browser/TURN gates and the open
16-voice+1080p60/latency requirements still block migration.

Plain/Direct mediasoup input and Janus `add_remote_publisher` use a different
ingress/security/topology contract and are excluded from this comparison.
[Encoded Transform](https://w3c.github.io/webrtc-encoded-transform/) works around
the browser encoder/packetizer path; it is not used as proof of an independent
fixed-rate RTP source. The encoding settings follow the
[official FFmpeg libvpx options](https://ffmpeg.org/ffmpeg-codecs.html#libvpx).
