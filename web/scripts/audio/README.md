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

### Enhanced voice: Strong (DeepFilterNet3, GEL-28)

"Enhanced voice: Strong" runs DeepFilterNet3 (DFN3) instead of RNNoise's
output, in the configuration measured in
`docs/v0.13-deepfilternet-measurement.md`: the full model, attenuation limit
100 dB, local-SNR thresholds −15/35/35 dB set explicitly, no post filter.
Standard (RNNoise) stays the default. Only clients that choose Strong load
`public/audio/deepfilternet.wasm` (libDF with tract, wasm32 with SIMD128,
11.9 MB, 3.3 MB as nginx gzips it) and `public/audio/deepfilternet3-model.bin`
(the DFN3 model archive, 8 MB, named `.bin` because Vite's dev server
sends a `.gz` file as `Content-Encoding: gzip`, which breaks the hash
check). `DEEPFILTERNET-PROVENANCE.json` records the pins,
`DEEPFILTERNET-LICENSE.txt` the licences of DeepFilterNet, the model and
every crate in the module.

`bash scripts/audio/build-deepfilternet.sh` (from `web`) downloads the
DeepFilterNet source at the pinned commit, checks its SHA-256 and the
model's, and builds the crate in `scripts/audio/deepfilternet/` with Rust
1.100.0 (rustup, with the `wasm32-unknown-unknown` target), its
`Cargo.lock` and a fresh `CARGO_HOME`. No Docker. The result must match the
hash in the script. What that check covers: two builds on the same machine
gave the same module only once the build directory was fixed
(`/tmp/gelabber-deepfilternet-build`); another machine with the same
toolchain is expected, not verified, to give the same bytes. If it does
not, the script stops before it installs anything. Because the directory
is fixed, run one build at a time: a second build started meanwhile
deletes the first one's files.

The loader (`src/voice/deepFilterNet.ts`) fetches both files with their
hash in the URL, checks their SHA-256, compiles the module and builds the
model in a worker (`public/audio/deepfilternet-worker.js`), off the main
and the audio thread. The worker answers with the module's memory right
after the build; the worklet's `gelabber-strong` processor restores that
memory into a fresh instance of the same module. The module imports
nothing and keeps all its state in linear memory, so the copy is the
model. The page keeps the copy (about 31 MB) for the next capture.

RNNoise keeps running beside DFN3 for the input gate's speech probability,
shifted by one 10 ms block because DFN3's output is 10 ms later (as in
`docs/v0.13-deepfilternet/gate.mjs`). Strong falls back to Standard:

- when the module or model does not download, check or build (same retry
  delays as RNNoise; RNNoise failing as well still ends in browser filters);
- when the Strong processor does not start: no memory for the model copy
  (it says so right after its hello and runs RNNoise) or no hello within
  2 s (the page starts the Standard processor instead);
- in the worklet, in place (from the next block on it sends RNNoise's
  output, which skips 10 ms of sound once, as RNNoise is 10 ms ahead of
  DFN3), when DFN3 and RNNoise take more than 7 ms of each 10 ms block on
  average over 3 s, when more than one block in twenty takes longer than
  10 ms, when the audio clock falls more than 10 % behind the wall clock
  in two 5 s windows in a row (underruns), or when DFN3 fails. The first
  second is not judged. A single pause of more than 150 ms between two
  render quanta (a suspended context, an output device switch, a clock
  step) is an interruption, not lag: the underrun measurement starts over.

The worker's warm-up time is no verdict: the worker is not a real-time
thread. The fallback is shown in the voice diagnostics and stays until the
user clicks Strong again (in the settings or the microphone test), chooses
another mode, or reloads the page. A load failure is not kept: the next
capture tries again after the retry delay, or at once when Strong is
clicked again.

While Strong runs, the diagnostics show
DFN3's mean time per 10 ms block over the last second and the slowest
block: wall-clock time on the audio thread from `performance.now()` where
the worklet has it, otherwise from `Date.now()` in whole milliseconds,
averaged over 100 blocks. Headless Chromium (Playwright build 1255) gave its
worklet only `Date.now()` in the audio smoke; the diagnostics then add
"browser clock in whole milliseconds". A browser has no CPU clock, so this
is not the thread CPU time of the measurement in `docs/`. DFN3 adds 10 ms of delay
over RNNoise; the worklet's own buffer (reblocking and gate look-ahead) is
the same as Standard's.

`src/voice/strongFilter.test.mjs` runs the worker and the processor in
Node with the checked-in files: output sample for sample against DFN3
alone, the gate's shifted speech probability, and each fallback.

Run `npm run test:audio-dsp` to process deterministic synthetic noise, measure
compute time per 10 ms frame, and compare the actual worklet sample-for-sample
with direct RNNoise across varying browser render quanta, with the input
sensitivity gate off (GEL-27; on, it adds a 20 ms look-ahead, covered by
`src/voice/inputGate.test.mjs`). Reblocking adds exactly 480 samples
(10 ms); this excludes RNNoise's own algorithmic delay,
browser capture/output buffers, codec, transport and receiver playback delay.
Synthetic attenuation is a functional check, not a speech-quality score.

The same command separately estimates the full input-PCM to worklet-output-PCM
delay, including RNNoise's model delay, using deterministic voiced harmonics
and an aperiodic chirp. Normalized cross-correlation searches 0–50 ms in three
fixed half-second windows after warmup. A peak must correlate at least 0.95 and
exceed periodic aliases outside a 1 ms neighborhood by at least 0.02; an
unidentifiable/suppressed signal fails acceptance. Direct RNNoise and the actual
worklet must differ by exactly 480 samples, and each worklet estimate must stay
within 30 ms. Signal filtering can shift the correlation peak slightly, so
these are measured estimates for this PCM, not a universal latency guarantee.
The report keeps frame compute time separate. The PCM measurement includes no
browser capture/output buffering, scheduling overruns, codec, network or
physical-device latency; those remain release acceptance gates.

Start `npm run dev -- --host 127.0.0.1 --port 5179`, then run
`npm run test:audio-smoke`. Chromium and Firefox use browser fake devices, genuine
AudioWorklet/WASM, real MediaRecorder and HTML audio playback. The local test
checks Enhanced speech, Strong (DFN3 reports its time per block; a missing
DFN3 module falls back to Standard), Original capture, missing-model
fallback, A/B recording, playback/deletion and a 390 px layout. It does not override autoplay permission.
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
