# Gelabber Desktop

Native desktop client (issue #165): Tauri 2 shell around the existing web UI,
with a native media core instead of the browser's WebRTC. Linux
(Omarchy/Hyprland/Wayland) first, then Windows.

Status: **spikes** (steps 1 and 2 of the ticket: build, Linux screen capture).
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
  `native/scripts/fake-screencast-session.sh` runs a command in a throwaway
  session with the real portal frontend and a test backend whose "monitor"
  is a shared-memory PipeWire stream (`fake_screen.c`); CI uses it. The compositor side
  (xdg-desktop-portal-hyprland, DMA-BUF frames) needs a real desktop.
