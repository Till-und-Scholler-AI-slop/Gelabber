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
| `core/tests/screen_capture.rs` | Screen capture through the portal, H264, mediasoup, native decode (`GELABBER_TEST_SCREEN=1`) |
| `app/` | Tauri 2 app `gelabber-desktop`: window on the server origin, bundled setup page, media commands (`src/media.rs`) |
| `app/scripts/smoke.sh` | Starts the app on a stand-in origin under Xvfb and checks which commands the page reaches |
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
Without one the window shows the bundled setup page, which writes that file.
Only the configured origin gets the `media` permission set
(`app/permissions/media.toml`); every page load closes the previous page's
transports, producers and sources. WebKitGTK needs `libwebkit2gtk-4.1`.

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
- Video display: the webview cannot show the core's video, so remote video
  opens in a native viewer window (`app/src/viewer.rs`): winit on its own
  thread next to the webview's GTK loop, wgpu drawing the decoded I420
  planes with a BT.601/709 shader, letterboxed. The window's Wayland app id
  and X11 class are `gelabber-viewer` for compositor rules. CI draws a frame
  through lavapipe under Xvfb and checks the colors on a screenshot.
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
