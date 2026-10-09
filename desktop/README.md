# Gelabber Desktop

Native desktop client (issue #165): Tauri 2 shell around the existing web UI,
with a native media core instead of the browser's WebRTC. Linux
(Omarchy/Hyprland/Wayland) first, then Windows.

Status: **spikes** (steps 1 to 4 of the ticket: build, Linux screen capture,
voice in the core, Tauri shell in progress).
Not a product yet.

## Layout

| Path | What |
|---|---|
| `native/` | C++ media core: libmediasoupclient + libwebrtc behind a C ABI (`include/gelabber_media.h`), built as one shared library |
| `native/libwebrtc.env` | Pins: libwebrtc M140 commit, libmediasoupclient commit, libsdptransform tag |
| `native/scripts/` | Reproducible libwebrtc builds (Linux, Windows); a test desktop session with a ScreenCast portal |
| `native/patches/` | Patches applied to libwebrtc |
| `core/` | Rust crate `gelabber-media-core`: safe API over the C ABI |
| `core/tests/mediasoup_loopback.rs` | Loopback call against mediasoup 0.29 with the server's router codecs |
| `core/tests/source_preview.rs` | Video sinks for views in the app: a local source without a producer, scaling and rate limits, sinks going away mid-frame |
| `core/tests/screen_capture.rs` | Screen capture through the portal, H264, mediasoup, native decode (`GELABBER_TEST_SCREEN=1`) |
| `app/` | Tauri 2 app `gelabber-desktop`: window on the server origin, bundled setup page, media commands (`src/media.rs`) |
| `app/scripts/smoke.sh` | Starts the app on a stand-in origin under Xvfb and checks which commands the page reaches and that the frames of a test-pattern view arrive |
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

## Running the app

```sh
cargo run --manifest-path desktop/Cargo.toml -p gelabber-desktop -- --server https://gelabber.example.org
```

Server choice, first match wins: `--server <url>`, `GELABBER_SERVER`, then
`server` in `~/.config/io.github.till-und-scholler-ai-slop.gelabber/desktop.json`.
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
set them.

## Design rules

- Signaling stays in TypeScript (`web/src/voice/mediasoupConnection.ts`):
  tickets, ACL, source epochs, generations and `consumerReady` are not
  duplicated. The core only turns `transport`/`connect`/`produce`/`consumer`
  data into native media (`TransportEvent::Connect`/`Produce` are answered
  by the caller after the server replied).
- The Tauri window loads the server origin. Its native commands are narrow
  (media core only), because server-side XSS reaches them.
- Video encode: H264 preferred, VP8 fallback. H264 uses the system's
  GStreamer when it has a hardware element (`nvh264enc`, `vah264enc`,
  `vah264lpenc`, `vaapih264enc`; needs gst-plugins-bad), otherwise OpenH264.
  `GELABBER_H264_ENCODER=<element>|none` overrides the choice. Decode is
  libwebrtc's: FFmpeg (Chrome branding) for H264, libvpx for VP8/VP9.
  Shipping software H264 needs a licensing decision first.
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
  header says which matrix the colours need: BT.709 for remote video from
  720 lines up (the convention of browsers and of the viewer window), BT.601
  otherwise and always for a self view, because that is what the core
  converts a captured screen with and what cameras deliver as a rule. Frames
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
  page and as a pop-out next to them: winit on its own thread next to the
  webview's GTK loop, wgpu drawing the decoded I420 planes with a BT.601/709
  shader, letterboxed. It shares the consumer's sink with the page views and
  shows the stream unscaled, so while a window is open the views of that
  consumer get full-size frames too. The window's Wayland app id and X11
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
  noise-playing "microphone" for `core/tests/voice.rs`.
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
  (`WEBRTC VoiceEngine`, `PipeWire ALSA [...]`) does not count.
  Gelabber's own process and the ones it started (the webview's helpers) are
  never captured, whatever is selected: the stream would carry the call. A
  helper in a pid namespace of its own is only recognized as a native
  PipeWire client (`pipewire.sec.pid`); pipewire-pulse passes on the pid the
  client reports.
  Only the process a playback stream belongs to decides. Sound that another
  process plays on is that process's stream: behind a virtual sink of
  PipeWire's loopback or filter-chain module, "" captures an application
  twice, at its own stream and at the chain's output, and Gelabber's playout
  routed through such a sink is captured at the chain's output. A chosen
  application is captured at its own stream alone.
  A capture stream takes its playback stream or nothing (`node.dont-fallback`,
  and `node.linger` to wait for it): WirePlumber 0.5 otherwise links the
  default source, the microphone, to a capture whose target it has not
  prepared yet. When the application changes to an output with another
  channel layout, its ports and the links from them are replaced and the
  session manager leaves the capture unlinked, so the core replaces a capture
  stream that lost its links.
  The voice test covers the list and "" while the test process plays a
  consumer out and a player it started plays too, a player of its own that is
  chosen by name and changes outputs, and two players on one binary under
  different names.
