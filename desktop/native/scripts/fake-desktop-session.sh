#!/usr/bin/env bash
# Runs a command inside a throwaway desktop session, to test screen capture
# and audio without a monitor, GPU, sound card or dialog:
# - D-Bus session, PipeWire + WirePlumber + pipewire-pulse;
# - the real xdg-desktop-portal frontend with a test ScreenCast backend
#   (fake_screencast_portal.py) whose "monitor" is fake_screen.c, a PipeWire
#   Video/Source with a moving pattern in shared-memory buffers like a
#   compositor's screencast stream;
# - a null sink "Gelabber-Speakers" as the output, and a microphone
#   "Gelabber-Mic" that hears white noise: a source remapped from the monitor
#   of a second null sink the noise plays into (libwebrtc lists no monitor
#   sources). GELABBER_TEST_MIC / GELABBER_TEST_SPEAKERS name both.
#
# The compositor side (xdg-desktop-portal-hyprland, DMA-BUF frames) is not
# covered; that needs a real desktop.
#
# On a desktop the session keeps to itself: its own runtime, configuration and
# state directories, and a WirePlumber that leaves sound cards, Bluetooth and
# cameras to the desktop's own (WirePlumber 0.5 or newer; 0.4 does not know
# the setting).
#
# Ubuntu 24.04 packages: pipewire wireplumber pipewire-pulse pulseaudio-utils
# xdg-desktop-portal libpipewire-0.3-dev gcc python3-gi (dbus-run-session
# comes with dbus). Arch: pipewire pipewire-audio pipewire-pulse wireplumber
# libpulse xdg-desktop-portal python-gobject gcc.
#
# Usage: fake-desktop-session.sh <command> [args...]
set -euo pipefail

if [[ -z "${GELABBER_FAKE_SESSION:-}" ]]; then
  export GELABBER_FAKE_SESSION=1
  exec dbus-run-session -- "$0" "$@"
fi

here="$(cd "$(dirname "$0")" && pwd)"
width=1280
height=720
runtime="$(mktemp -d)"
chmod 700 "$runtime"
# Whatever names the desktop's own sound server.
unset PULSE_SERVER PULSE_RUNTIME_PATH PIPEWIRE_REMOTE PIPEWIRE_RUNTIME_DIR
export XDG_RUNTIME_DIR="$runtime"
export XDG_CONFIG_HOME="$runtime/config"
# WirePlumber keeps default devices and volumes here.
export XDG_STATE_HOME="$runtime/state"
export XDG_DESKTOP_PORTAL_DIR="$runtime/portals"
# libwebrtc only uses its PipeWire capturer in a Wayland session; it never
# connects to the compositor itself.
export XDG_SESSION_TYPE=wayland
export WAYLAND_DISPLAY=wayland-gelabber-test
export XDG_CURRENT_DESKTOP=gelabbertest
mkdir -p "$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d" "$XDG_STATE_HOME" "$XDG_DESKTOP_PORTAL_DIR"

# Only the null sinks below: the desktop's own session has the real devices.
cat >"$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/90-no-hardware.conf" <<'CONF'
wireplumber.profiles = {
  main = {
    hardware.audio = disabled
    hardware.bluetooth = disabled
    hardware.video-capture = disabled
  }
}
CONF

cat >"$XDG_DESKTOP_PORTAL_DIR/gelabbertest.portal" <<'PORTAL'
[portal]
DBusName=org.freedesktop.impl.portal.desktop.gelabbertest
Interfaces=org.freedesktop.impl.portal.ScreenCast;
PORTAL
# With XDG_DESKTOP_PORTAL_DIR set, the frontend reads portals.conf from there.
cat >"$XDG_DESKTOP_PORTAL_DIR/portals.conf" <<'CONF'
[preferred]
default=gelabbertest
CONF

pids=()
cleanup() {
  local status=$?
  kill "${pids[@]}" 2>/dev/null || true
  wait 2>/dev/null || true
  # xdg-desktop-portal mounts the document portal (FUSE) under the runtime dir.
  fusermount3 -u "$runtime/doc" 2>/dev/null || fusermount -u "$runtime/doc" 2>/dev/null || true
  rm -rf "$runtime" 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT

pipewire &
pids+=($!)
for _ in $(seq 50); do
  [[ -S "$runtime/pipewire-0" ]] && break
  sleep 0.1
done
wireplumber &
pids+=($!)
pipewire-pulse &
pids+=($!)
for _ in $(seq 50); do
  pactl info >/dev/null 2>&1 && break
  sleep 0.1
done

pactl load-module module-null-sink sink_name=gelabber-speakers \
  sink_properties=device.description=Gelabber-Speakers >/dev/null
pactl load-module module-null-sink sink_name=gelabber-mic-feed rate=48000 channels=1 \
  sink_properties=device.description=Gelabber-Mic-Feed >/dev/null
# 60 s of white noise at -20 dBFS RMS, 48 kHz mono.
"${GELABBER_PYTHON:-/usr/bin/python3}" -I -c '
import random, struct, sys, wave
w = wave.open(sys.argv[1], "wb")
w.setnchannels(1); w.setsampwidth(2); w.setframerate(48000)
r = random.Random(1)
w.writeframes(b"".join(struct.pack("<h", max(-32767, min(32767, int(r.gauss(0, 3277))))) for _ in range(48000 * 60)))
' "$runtime/noise.wav"
pactl load-module module-remap-source master=gelabber-mic-feed.monitor source_name=gelabber-mic \
  source_properties=device.description=Gelabber-Mic >/dev/null
pw-play --target gelabber-mic-feed "$runtime/noise.wav" &
pids+=($!)
export GELABBER_TEST_MIC=Gelabber-Mic
export GELABBER_TEST_SPEAKERS=Gelabber-Speakers

"${GELABBER_HOST_CC:-gcc}" -O2 -o "$runtime/fake_screen" "$here/fake_screen.c" \
  $(pkg-config --cflags --libs libpipewire-0.3)
mkfifo "$runtime/node"
"$runtime/fake_screen" "$width" "$height" 30 >"$runtime/node" &
pids+=($!)
node=""
read -r -t 10 node <"$runtime/node" || true
if [[ -z "$node" ]]; then
  echo "test screen node did not appear" >&2
  exit 1
fi
echo "test screen: PipeWire node $node (${width}x${height})"

dbus-update-activation-environment XDG_RUNTIME_DIR XDG_CONFIG_HOME XDG_DESKTOP_PORTAL_DIR \
  XDG_SESSION_TYPE WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
"${GELABBER_PYTHON:-/usr/bin/python3}" -I "$here/fake_screencast_portal.py" "$node" "$width" "$height" &
pids+=($!)
gdbus wait --session --timeout 15 org.freedesktop.impl.portal.desktop.gelabbertest
# Debian and Ubuntu keep the frontend in /usr/libexec, Arch in /usr/lib.
portal=/usr/libexec/xdg-desktop-portal
[[ -x "$portal" ]] || portal=/usr/lib/xdg-desktop-portal
"$portal" ${GELABBER_PORTAL_VERBOSE:+--verbose} &
pids+=($!)
gdbus wait --session --timeout 15 org.freedesktop.portal.Desktop

"$@"
