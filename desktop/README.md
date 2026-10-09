# Gelabber Desktop

Native desktop client (issue #165): Tauri 2 shell around the existing web UI,
with a native media core instead of the browser's WebRTC. Linux
(Omarchy/Hyprland/Wayland) first, then Windows.

Status: **released** for Linux x64 since v0.5.0, as a tarball on the release
page and as a pacman package (`packaging/arch/`). From v0.6 on a Windows x64
installer is built, installed and smoke-tested by CI and attached to the
release; that app has voice, camera and watching, but no screen share, Go
Live or application sound yet. What users get to read ships from
`app/package/`.

## Layout

| Path | What |
|---|---|
| `native/` | C++ media core: libmediasoupclient + libwebrtc behind a C ABI (`include/gelabber_media.h`), built as one shared library |
| `native/libwebrtc.env` | Pins: libwebrtc M140 commit, libmediasoupclient commit, libsdptransform tag |
| `native/scripts/` | Reproducible libwebrtc builds (Linux, Windows); a test desktop session with a ScreenCast portal |
| `native/patches/` | Patches applied to libwebrtc |
| `native/tests/app_audio_mix_test.cc` | The source-audio mix against simulated sound cards whose clocks are off; plain C++, built and run by `core/tests/voice.rs` with the host's compiler |
| `core/` | Rust crate `gelabber-media-core`: safe API over the C ABI |
| `core/tests/mediasoup_loopback.rs` | Loopback call against mediasoup 0.29 with the server's router codecs |
| `core/tests/source_preview.rs` | Video sinks for views in the app: a local source without a producer, scaling and rate limits, sinks going away mid-frame |
| `core/tests/screen_capture.rs` | Screen capture through the portal, H264, mediasoup, native decode, and the screen as the web client shares it: VP8 in two simulcast layers (`GELABBER_TEST_SCREEN=1`) |
| `app/` | Tauri 2 app `gelabber-desktop`: window on the server origin, bundled setup page, media commands (`src/media.rs`) |
| `app/scripts/smoke.sh` | Starts the app on a stand-in origin under Xvfb and checks which commands the page reaches and that the frames of a test-pattern view arrive; then kills the page's web process and checks that the app loads the page again |
| `app/scripts/smoke.ps1` | The same on Windows, against a built or installed `gelabber-desktop.exe` |
| `app/tauri.bundle.windows.json` | What the Windows installer (NSIS) holds: the app, `gelabber_media.dll` from `app/bundle-input/`, README and third-party notices |
| `app/package/` | What ships next to the binaries: `README.txt` (Linux), `README-windows.txt` (installed as `README.txt`), `THIRD-PARTY-NOTICES.txt` (generated) |
| `packaging/third-party-notices.py` | Writes `THIRD-PARTY-NOTICES.txt`; its tests are in `packaging/tests/` |
| `packaging/arch/` | The pacman package and repository; see its README |
| `core/tests/voice.rs` | Microphone modes, RNNoise, device selection, Opus through mediasoup, playout (`GELABBER_TEST_AUDIO=1`) |

## Why a shared library with a C ABI

libwebrtc is built with Chromium's clang, sysroot and libc++ (`std::__Cr`).
The Tauri process also loads WebKitGTK, which uses the system libstdc++.
The core links Chromium's libc++ statically, hides every symbol except `gm_*`
(version script + `--exclude-libs,ALL`) and exchanges only C types and JSON.
So neither C++ runtime leaks into the other, and the Rust side needs no C++
toolchain. On Windows the same holds with the MSVC STL and a DLL.

## Building (Linux)

The libwebrtc build needs network access to `*.googlesource.com`, ~30 GB
disk and takes hours on 4 cores; CI caches the result (`.github/workflows/desktop-native.yml`).

```sh
desktop/native/scripts/build-libwebrtc-linux.sh /mnt/webrtc /mnt/libwebrtc-package
export GELABBER_LIBWEBRTC_DIR=/mnt/libwebrtc-package
cargo test --manifest-path desktop/Cargo.toml -- --nocapture
```

The package carries its own clang, lld, sysroot (Debian bullseye, glibc 2.31)
and libc++ headers, so the core builds the same on Ubuntu and Arch and runs on
any glibc ≥ 2.31 system. PipeWire is loaded at runtime (`dlopen`), X11 is off.

`GELABBER_MEDIA_LIB_DIR` points the Rust crate at an already built
`libgelabber_media.so` instead of building it.

## Building (Windows)

CI builds, installs and tests the Windows app in the job `core-windows` of
`.github/workflows/desktop-native.yml`; what follows is that job by hand.
Needed: Visual Studio 2022 with the C++ workload and a Windows SDK, and on
`PATH` `clang-cl` (LLVM), CMake, Ninja, Python, Node and the Rust toolchain
of `rust-toolchain.toml`.

```powershell
git config --global core.longpaths true
pwsh desktop\native\scripts\build-libwebrtc-windows.ps1 -Work C:\webrtc -Package C:\libwebrtc-package
$env:GELABBER_LIBWEBRTC_DIR = 'C:\libwebrtc-package'
# MSVC's tools stop at 260 characters; the CMake and Meson trees are deep.
$env:CARGO_TARGET_DIR = 'C:\t'
# The C runtime linked statically, as in the release: without it the app
# needs the Visual C++ redistributable.
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --manifest-path desktop\Cargo.toml -p gelabber-media-core
cargo build --release --locked --manifest-path desktop\Cargo.toml -p gelabber-desktop
```

The libwebrtc build takes hours, like the Linux one. The core is compiled
with `clang-cl`, which built the package, and linked with Visual Studio's
libraries; `GELABBER_MEDIA_COMPILER` names another compiler (`cl`, or a
path). The result is `gelabber_media.dll` with its import library, and the
app's build script copies the DLL next to `gelabber-desktop.exe` in
`C:\t\release`: Windows has no rpath, the DLL must stay next to the
executable. `GELABBER_MEDIA_LIB_DIR` points at a prebuilt core here too.

The installer, from what was just built (the pinned Tauri CLI only bundles):

```powershell
New-Item -ItemType Directory -Force desktop\app\bundle-input
Copy-Item C:\t\release\gelabber_media.dll desktop\app\bundle-input\
cd desktop\app
npx.cmd --yes '@tauri-apps/cli@2.12.1' bundle --bundles nsis --config tauri.bundle.windows.json
# -> C:\t\release\bundle\nsis\Gelabber_<version>_x64-setup.exe
```

`<version>` is the one of `desktop/Cargo.toml`. CI builds a release only
when its tag is that version (`v<version>`).

The smoke test takes a built or an installed app and the ABI version it
must report (`GM_ABI_VERSION` in `native/include/gelabber_media.h`). It
needs Python and the WebView2 Runtime and asks for dummy audio itself
(`GELABBER_AUDIO=dummy`, for machines without audio devices):

```powershell
pwsh desktop\app\scripts\smoke.ps1 -App C:\t\release\gelabber-desktop.exe -Abi <GM_ABI_VERSION>
```

The core's tests call a mediasoup worker that is built with Meson, against
the same static C runtime:

```powershell
# mediasoup-sys does not find its own copy of invoke when the cargo registry
# and the target directory are on different drives.
python -m pip install invoke==3.0.3
$env:PYTHON = 'python'
$env:MESON_ARGS = '--vsenv -Db_vscrt=mt'
cargo test --release --locked --manifest-path desktop\Cargo.toml -p gelabber-media-core -- --nocapture --test-threads=1
```

libwebrtc sends from the machine's own address. Where that address does not
reach `127.0.0.1`, as reported of Windows, `GELABBER_TEST_LISTEN_IP=<address>`
puts the tests' mediasoup on it; the CI job tries which one works.

## Tests

| What | How | Where |
|---|---|---|
| Core: loopback call, video sinks; with their switches screen capture and voice | `cargo test --manifest-path desktop/Cargo.toml`; `GELABBER_TEST_SCREEN=1`, `GELABBER_TEST_AUDIO=1` inside `native/scripts/fake-desktop-session.sh` | CI, Linux and Windows (screen and voice: Linux) |
| App: the commands a server origin reaches, frames of a test pattern | `app/scripts/smoke.sh`, `app/scripts/smoke.ps1` | CI, Linux and Windows |
| Viewer window: colours on a screenshot | `cargo test --manifest-path desktop/Cargo.toml -p gelabber-desktop -- --ignored viewer` under Xvfb | CI, Linux |
| Video inside the page: the web client's renderer, feed and tiles on real pixels | in `web/`: `npm run test:native-video-smoke` | **only locally** |
| Third-party notices | `python3 -m unittest discover -s desktop/packaging/tests`, `packaging/third-party-notices.py --check` | CI |

The video inside the page is drawn by the web client
(`web/src/voice/native/frames.ts`, `videoFeed.ts`, the tiles), and `npm test`
drives that renderer against a stand-in for WebGL: it never compiles the
shaders. `npm run test:native-video-smoke` does, in headless Chromium with a
stand-in for the app, and reads the tiles' pixels back; no server, about ten
seconds. It needs a Chromium (`npx playwright install chromium` once, or
`GELABBER_NATIVE_VIDEO_BROWSER_EXECUTABLE=/usr/bin/chromium`). CI runs no
browser smoke tests (`.github/workflows/ci.yml`), so run it after a change to
those files. WebKitGTK itself is not covered by it.

## Third-party notices

Every package ships `app/package/THIRD-PARTY-NOTICES.txt`. It is generated;
`packaging/third-party-notices.py` writes it (Linux x86-64 only, with the
toolchain of `rust-toolchain.toml`). Run without arguments it lists the Rust
crates afresh from `desktop/Cargo.lock` and keeps the two parts on the
native core. Run it after every change to `desktop/Cargo.lock` and commit
the file: the CI job "Third-party notices" fails when the committed file is
not what the script would write (`--check`), and the uploads to a release
wait for that job. When a pin of the native core or the build configuration
the file describes has changed, the script stops and says what it needs
(`--webrtc-src`, `--core-build`; its header has the details).

## Running the app

```sh
cargo run --manifest-path desktop/Cargo.toml -p gelabber-desktop -- --server https://gelabber.example.org
```

Server choice, first match wins: `--server <url>`, `GELABBER_SERVER`, then
`server` in `~/.config/io.github.till-und-scholler-ai-slop.gelabber/desktop.json`
(Windows: `%APPDATA%\io.github.till-und-scholler-ai-slop.gelabber\desktop.json`).
Without one the window shows the bundled setup page, which writes that file
once something accepts connections at the address. The window has no menu
bar. If the stored server does not answer at start, the setup page opens with
the reason and a retry button. **Strg+Umschalt+S** and "Server wechseln …" in
the web client's user menu lead back to that page at any time; **F5** or
**Strg+R** reload the current page.
Only the configured origin gets the `media` permission set
(`app/permissions/media.toml`) plus `allow-open-setup`; every page load closes the previous page's
transports, producers and sources. WebKitGTK needs `libwebkit2gtk-4.1`.
On the NVIDIA driver the app sets `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1`
before GTK starts: WebKitGTK's DMA-BUF renderer otherwise fails under Wayland
with "Error 71 (Protocol error) dispatching to Wayland display". Handing its
buffers over through shared memory avoids that and keeps accelerated
compositing; with the renderer disabled altogether (what the app set up to
0.5.2) the web process paints video on the CPU, half a core and more for one
stream across the window. Setting that variable or
`WEBKIT_DISABLE_DMABUF_RENDERER` yourself (to any value) leaves both as you
set them. On the same driver the app ends WebKit's web process the moment
its window is closed (`webkit_web_view_terminate_web_process`): once a page
has drawn with WebGL, as the video tiles do, a web process left to shut
down crashes inside the driver when it frees its GL contexts
(`eglDestroyContext` in libnvidia-eglcore; driver 610.57.04, WebKitGTK
2.52.6, with either renderer setting), which left a core dump and a crash
notification behind every close. An app that is killed instead of closed
(SIGTERM, the end of the session) still leaves the web process to that.
When the web process ends by itself while the app runs (a crash, the
kernel's out-of-memory killer), the app closes what the page had running,
as a page load does, and loads the page again: the reload keys are a script
of the page and gone with it. If it ends again within a minute, the setup
page opens with the reason; if that does not stay up either, the view is
left as it is.

## Design rules

- Signaling stays in TypeScript (`web/src/voice/mediasoupConnection.ts`):
  tickets, ACL, source epochs, generations and `consumerReady` are not
  duplicated. The core only turns `transport`/`connect`/`produce`/`consumer`
  data into native media (`TransportEvent::Connect`/`Produce` are answered
  by the caller after the server replied).
- The Tauri window loads the server origin. Its native commands are narrow
  (media core only), because server-side XSS reaches them.
- Features: one web client is served to every installed app, old and new,
  on both systems, so it asks. `media_info` answers with `features`, what
  this build can do besides voice and watching (`FEATURES` in
  `app/src/media.rs`; the Windows app has `camera` and `video-frames`, no
  `screen` and no `app-audio`). The web client (`web/src/voice/capabilities.ts`)
  keeps the button of a capture the app lacks in its place, greyed out and
  with the reason, and hides the controls for application sound. An app up
  to 0.5.x answers without a list and is taken for what it was: the Linux
  app with screen capture, camera and application sound. The other way
  round nothing can be done: a web client before v0.6 does not ask, and
  offers the Windows app screen share and Go Live. Screen share then simply
  does not start. Go Live leaves a claim on the server that blocks Go Live
  for the whole channel until that user leaves the call: the old client
  sends the claim before the capture starts and does not take it back when
  the capture is refused. Hence "a server from 0.6 on" in
  `app/package/README-windows.txt`.
- Video encode: the app sends VP8, encoded in software by libwebrtc's
  libvpx. The web client names the codec of every producer and takes VP8
  wherever the device has it (`codec` in
  `web/src/voice/mediasoupConnection.ts`), and the core produces what it is
  asked for. The Linux core has H264 all the same, and its tests run it:
  through the system's GStreamer when that has a hardware element
  (`nvh264enc`, `vah264enc`, `vah264lpenc`, `vaapih264enc`; gst-plugins-bad),
  otherwise OpenH264; `GELABBER_H264_ENCODER=<element>|none` overrides the
  choice, and a caller that names no codec gets H264 before VP8. None of
  that reaches the app: there gst-plugins-bad and the variable change
  nothing. Hardware encoding in the app would start in the web client (ask
  for `video/h264` when the core reports a hardware encoder), and shipping
  H264 needs a licensing decision first. The Windows core is built without
  H264. Decode is libwebrtc's: libvpx for VP8/VP9, on Linux FFmpeg (Chrome
  branding) for H264.
- Simulcast: libvpx and OpenH264 encode a producer's layers in one encoder,
  and only when every layer has exactly the top layer's aspect. A layer is
  the picture divided by its `scaleResolutionDownBy`, so a 1366x768 screen
  over the web client's 4 and 1 would be 342x192 below 1366x768: refused,
  and nothing is sent. Local video sources therefore crop to multiples of 4
  (1364x768), and the software encoders tell libwebrtc that their alignment
  holds for every layer, which makes it ask the source for what other
  factors need. libwebrtc's own answer, `SimulcastEncoderAdapter` with an
  encoder per layer, is not in the Windows package.
- Screen capture (Linux): xdg-desktop-portal ScreenCast picks the source,
  PipeWire delivers frames (libwebrtc's `BaseCapturerPipeWire`). The portal's
  GLib callbacks run on the capture thread, so no host main loop is needed.
  `native/scripts/fake-desktop-session.sh` runs a command in a throwaway
  session with the real portal frontend and a test backend whose "monitor"
  is a shared-memory PipeWire stream (`fake_screen.c`); CI uses it. The
  compositor side (xdg-desktop-portal-hyprland, DMA-BUF frames) needs a real
  desktop.
- Camera: libwebrtc's video capture module (V4L2 on Linux, DirectShow on
  Windows) at the closest format to the requested profile. CI runners have
  no camera, so only the "no camera" path is tested there.
- Video display: WebKitGTK has no WebRTC and cannot take a native texture,
  so the page pulls the core's frames and draws them itself, on a `<canvas>`
  where the browser build has a `<video>` (`app/src/frames.rs`).
  `media_view_open` opens a view of a remote video consumer or of a local
  source (camera, screen: the self view, which needs no producer),
  `media_view_frame` answers with the next frame the page has not seen (a
  long poll with a raw response: a 32-byte header, then the I420 planes),
  `media_view_configure` says how large the page draws, `media_view_close`
  ends it. A slow page skips frames: a view keeps only the newest. One native
  sink per consumer or source (`gm_consumer_set_video_sink`,
  `gm_source_set_video_sink`) feeds all its views. Before frames cross into
  Rust the core scales them down to the largest view (libyuv; never up, so a
  4K screen costs a small tile little) and drops those above the views' rate
  limit. Views end with the page and with their consumer or source. The
  header says which matrix the colours need: BT.601 for remote video and for
  a self view at any size, because that is what the core converts a captured
  screen with and what Chromium takes received video as when the stream does
  not say (one 1280x720 VP8 stream measured in a Chromium 153 tile and in the
  app; by size, as up to 0.5 in the viewer window, a stream also changed its
  colours with its simulcast layer). A stream that does carry its colour
  description (H.264 with VUI, VP9, AV1) is not looked at yet. Frames
  go through an ordinary command because Tauri checks commands against the
  server origin's capability; a scheme of the app's own or a pushed channel
  measured no faster. A Content-Security-Policy on the server that keeps the
  page from fetching `ipc://localhost` (Windows: `http://ipc.localhost`)
  makes Tauri fall back to postMessage with frames as JSON number arrays,
  which is far too slow for video: allow it in `connect-src`.
  `GELABBER_VIDEO_TEST_PATTERN=1` lets `media_view_open` take
  `testPattern: {width, height, fps}` (colour bars) instead of a consumer
  or source; the smoke test uses it.
- Self view: a view of a local source shows the picture as it goes to the
  encoders, not the capture. While an encoder has the source step down (a
  weak uplink, the first seconds of a producer) the view shows that smaller
  picture or lower rate, unlike a browser's local `<video>`. A view from
  before that adaptation would need a second output in every source
  (camera, screen, test pattern): they adapt before they hand a frame on.
- Viewer window: `media_viewer_open` shows a remote video in a native
  window (`app/src/viewer.rs`), for web clients from before the views in the
  page and as a pop-out for newer ones: winit on its own thread next to the
  webview's GTK loop, wgpu drawing the decoded I420 planes (BT.601, as the
  views in the page), letterboxed. It shares the consumer's sink with the page views and
  shows the stream unscaled, so while a window is open the views of that
  consumer get full-size frames too, whatever size they asked for; only a
  view's own `maxFps` still holds. The web client therefore keeps no view
  next to a window: it closes its views of the consumer before it opens the
  window and draws the stream in the page again once the window is closed.
  The window's Wayland app id and X11
  class are `gelabber-viewer` for compositor rules. CI draws a frame through
  lavapipe under Xvfb and checks the colors on a screenshot.
- Voice: libwebrtc's audio device module (Linux: PulseAudio API, served by
  pipewire-pulse) with its APM for echo cancellation, noise suppression and
  AGC. The web client's modes carry over: `enhanced` runs RNNoise (same
  source commit and model as `web/public/audio`) after the APM, `browser`
  uses the APM's noise suppression/AGC, `original` captures stereo without
  either. Input gain and meters (0..100, the web formula) run in the same
  post-processor; per-consumer playback volume replaces the web's audio
  elements. The APM runs with `WebRTC-MutedStateKillSwitch`: otherwise
  closing one audio producer marks the shared APM's output unused and stops
  RNNoise, gain and meters for the remaining microphone stream. The
  microphone test (`gm_audio_monitor`) keeps the device module recording
  without a call through a pass-through module that only holds back the
  voice pipeline's stop while the test runs. The test
  session also provides null-sink "speakers" and a
  noise-playing "microphone" for `core/tests/voice.rs`. The noise lasts
  ten minutes from the session's start; the test says so when it is started
  too late in it, so build it before the session
  (`cargo test --no-run`).
- Microphone routing: libwebrtc's device module hands its capture to every
  audio send stream, which would mix the microphone into source audio. The
  device module's callback is therefore our `CaptureTransport`: it runs the
  APM itself and feeds only the microphone sources; playout (and with it the
  echo canceller's reference) passes straight through to the engine. The APM
  always runs mono: switching its capture channel count mid-stream aborts in
  its post filter (libc++ bounds check). Original mode without echo
  cancellation gets the device's stereo past the APM, with only the gain
  applied. Other audio sources must report empty `AudioOptions`, because the
  voice engine applies a source's options to the shared APM.
- Source audio (Linux): PipeWire, dlopened like libwebrtc does. One passive
  capture stream per playback node of the chosen application ("" = every
  application), mixed to 48 kHz stereo and produced as the `sa`/`la` Opus
  stereo track. Streams the application opens later are picked up. Whose a
  playback node is comes from its client object: the registry lists the node
  with `application.name` and `client.id` only, the client's info has
  `application.process.id` and `application.process.binary`. The binary is
  the application's id; its name, the id up to 0.5.2, still selects it.
  Applications that run the same binary (a system Electron, Wine, an
  interpreter) are that binary's one entry, which selects them all. While
  they play under different names of their own, each name is listed as a
  further entry with the name as its id; a name a sound library gives
  (`WEBRTC VoiceEngine`, `PipeWire ALSA [...]`) does not count. The other
  way round, programs on different binaries may call themselves the same
  (each with an Electron of its own, all "Chromium"): their entries are
  shown as `name (binary)`, and the name is an entry as well, which selects
  them all as it did when it was the id. No two entries share a name.
  Gelabber's own process and the ones it started (the webview's helpers) are
  never captured, whatever is selected: the stream would carry the call. A
  helper in a pid namespace of its own is only recognized as a native
  PipeWire client (`pipewire.sec.pid`); pipewire-pulse passes on the pid the
  client reports.
  The playback stream of a virtual device is no application's either: an
  echo canceller, an equaliser or virtual surround (filter chain), a combined
  or remapped sink and a loopback play on what was played into them, which
  is Gelabber's playout where such a device is its output, and every other
  application a second time. PipeWire builds them all from its loopback,
  filter-chain, echo-cancel and combine-stream modules (pipewire-pulse's
  `module-loopback`, `-echo-cancel`, `-combine-sink`, `-remap-sink` and
  `-virtual-sink` included), which mark both of their streams `node.virtual`
  and give them a `node.link-group`. Neither is in the registry's listing,
  so the watcher binds each playback node and a stream waits for the node's
  own info as well; a node with either property is left out of capture and
  list. That also drops a loopback of a line input or a microphone monitor
  from "every application". What it cannot tell is a program that records
  other programs and plays the result as an ordinary stream of its own:
  that stream is its application's, with whatever it carries.
  A capture stream takes its playback stream or nothing (`node.dont-fallback`,
  and `node.linger` to wait for it): WirePlumber 0.5 otherwise links the
  default source, the microphone, to a capture whose target it has not
  prepared yet. When the application changes to an output with another
  channel layout, its ports and the links from them are replaced and the
  session manager leaves the capture unlinked, so the core replaces a capture
  stream that lost its links.
  The mix goes at the pace of the sound card. Samples arrive a graph
  quantum at a time at the card's rate, and a mix on the system clock ran a
  stream's buffer dry every few seconds when the card was a little slower
  (every 13 s at 50 ppm and a quantum of 1024), each time with a block that
  was part silence. Each stream keeps 20 ms below its bursts, and the time
  to the next block shifts by up to 0.3 % with what the stream that has
  least to spare is off that cushion (`native/src/app_audio_mix.h`); the
  samples are left alone. With applications on two cards the mix keeps to
  the slower one and cuts the other back when it is too far ahead.
  `gm_source_state` counts both (`underruns`, `overruns`). A test session's
  null sinks run on the system clock, so
  `native/tests/app_audio_mix_test.cc` runs the mix against simulated cards.
  The voice test covers the list and "" while the test process plays a
  consumer out and a player it started plays too, the same with the consumer
  played out through a null sink that a loopback plays on to the speakers, a
  player of its own that is chosen by name and changes outputs, two players
  on one binary under different names, and two on different binaries under
  the same name.
