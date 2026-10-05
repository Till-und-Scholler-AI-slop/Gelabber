# Local microphone processing acceptance

The default Enhanced speech mode uses the checked-in RNNoise v0.2 WASM model
inside an AudioWorklet. Assets are served by the same Gelabber origin; no cloud
API or external request runs during capture. Source, model, compiler digest,
license and resulting binary hash are recorded in
`public/audio/RNNOISE-PROVENANCE.json`. The loader verifies the binary hash and
uses a hash-qualified cache URL. Browsers lacking the required WebAssembly SIMD,
AudioWorklet or 48 kHz AudioContext use an explicit browser fallback.

`bash scripts/audio/build-rnnoise.sh` (from `web`) downloads the pinned source and
model, verifies both hashes, and compiles in the pinned Emscripten Docker image.
Docker access is required. The result must match the checked-in hash. RNNoise
v0.2's portable scalar branch references an absent `os_support.h`; the build
supplies only the standard `OPUS_CLEAR` memset macro. Clang may still vectorize
scalar loops with WASM SIMD. Fixed 16 MiB memory and a 1 MiB stack avoid runtime
allocation/growth in `process()` and stack overflow in the upstream model.
The full third-party license accompanies the binary.

Run `npm run test:audio-dsp` to process deterministic synthetic noise, measure
compute time per 10 ms frame, and compare the actual worklet sample-for-sample
with direct RNNoise across varying browser render quanta. Reblocking adds
exactly 480 samples (10 ms); this excludes RNNoise's own algorithmic delay,
browser capture/output buffers, codec, transport and receiver playback delay.
Synthetic attenuation is a functional check, not a speech-quality score.

Start `npm run dev -- --host 127.0.0.1 --port 5179`, then run
`npm run test:audio-smoke`. Chromium and Firefox use browser fake devices, genuine
AudioWorklet/WASM, real MediaRecorder and HTML audio playback. The local test
checks Enhanced speech, Original capture, missing-model fallback, A/B recording,
playback/deletion and a 390 px layout. It does not override autoplay permission.
It does not need accounts, a server or production writes.

The microphone comparison is opt-in, local, capped at eight seconds and clears
object URLs/capture on close. "Input" means browser capture before the additional
processor; native capture filters may already be present. Browser settings and
actual device capabilities can differ; diagnostics retain effective values or
unknown rather than substituting requested flags. Context failure stops the
broken capture and falls back to native audio, with effective gain reported.

Before release, listen with physical microphones in Chromium/Brave and Firefox:
speech plus keyboard/fan/room noise, original stereo/music, clipping and CPU,
device changes, mute/deafen, background/resume, headphones/speakers, and a slow
subscriber/real TURN path. Those checks are distinct from synthetic acceptance.
Capture resolution/FPS remain selectable independently of bandwidth. No
Gelabber bitrate maximum is applied by default; only explicit Economy or custom
limits set sender caps. Remote SDP is preserved, so genuine peer receive limits
and WebRTC congestion control still apply.

Sender priority is a best-effort hint: microphone and display-source audio use
`priority=high`, video uses `priority=low` (the WebRTC normal priority).
`networkPriority` is changed only when `getParameters()` already exposes it.
Priority and DSCP updates run separately from cap/codec updates, so rejection
does not stop capture, detach a track or prevent a successful cap update.
Diagnostics show the browser's readback, with absent values remaining unknown.
The native loopback smoke records exact Chromium/Firefox versions and verifies
priority roundtrips plus preservation of existing limits. The tested Firefox
build exposes `priority` but no `networkPriority`; Chromium exposes both.
Neither a successful API call nor readback proves QoS under congestion. Local
sender bandwidth scheduling and DSCP are distinct controls; networks may bleach
DSCP, and this does not prioritize SFU downlink queues. See the
[W3C Priority Control API](https://www.w3.org/TR/webrtc-priority/).

**Open implementation and acceptance gate:** per-viewer adaptive video layers
are not implemented. Current publications contain one RTP broadcast; viewers
can select a source on/off but receive the same encoded packets. Simulcast/SVC
needs RID/layer-aware publication and subscription state, codec-aware frame
filtering, safe keyframe switches and sequence/timestamp rewriting before it
can be advertised. Release acceptance must include different viewer capacities
and audio under concurrent video congestion; the local priority API smoke does
not satisfy that gate.
