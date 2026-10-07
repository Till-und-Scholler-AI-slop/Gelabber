#!/usr/bin/env bash
# Runs a command inside a throwaway desktop session with a working ScreenCast
# portal, to test screen capture without a monitor, GPU or dialog:
# D-Bus session, PipeWire + WirePlumber, the real xdg-desktop-portal frontend,
# and a test backend (fake_screencast_portal.py) whose "monitor" is a moving
# GStreamer test video published as a PipeWire Video/Source node.
#
# The compositor side (xdg-desktop-portal-hyprland, DMA-BUF frames) is not
# covered; that needs a real desktop.
#
# Ubuntu 24.04 packages: pipewire wireplumber xdg-desktop-portal
# gstreamer1.0-pipewire gstreamer1.0-plugins-base gstreamer1.0-tools
# python3-gi jq (dbus-run-session comes with dbus).
#
# Usage: fake-screencast-session.sh <command> [args...]
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
export XDG_RUNTIME_DIR="$runtime"
export XDG_CONFIG_HOME="$runtime/config"
export XDG_DESKTOP_PORTAL_DIR="$runtime/portals"
# libwebrtc only uses its PipeWire capturer in a Wayland session; it never
# connects to the compositor itself.
export XDG_SESSION_TYPE=wayland
export WAYLAND_DISPLAY=wayland-gelabber-test
export XDG_CURRENT_DESKTOP=gelabbertest
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DESKTOP_PORTAL_DIR"

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

gst-launch-1.0 -q videotestsrc pattern=ball is-live=true \
  ! "video/x-raw,format=BGRx,width=$width,height=$height,framerate=30/1" \
  ! pipewiresink mode=provide \
    stream-properties="properties,media.class=Video/Source,node.name=gelabber-test-screen" &
pids+=($!)
node=""
for _ in $(seq 100); do
  node="$(pw-dump 2>/dev/null | jq -r '.[] | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == "gelabber-test-screen") | .id' | head -n1)"
  [[ -n "$node" ]] && break
  sleep 0.1
done
if [[ -z "$node" ]]; then
  echo "test video node did not appear" >&2
  pw-dump >&2 || true
  exit 1
fi
echo "test screen: PipeWire node $node (${width}x${height})"

dbus-update-activation-environment XDG_RUNTIME_DIR XDG_CONFIG_HOME XDG_DESKTOP_PORTAL_DIR \
  XDG_SESSION_TYPE WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
"${GELABBER_PYTHON:-/usr/bin/python3}" -I "$here/fake_screencast_portal.py" "$node" "$width" "$height" &
pids+=($!)
gdbus wait --session --timeout 15 org.freedesktop.impl.portal.desktop.gelabbertest
/usr/libexec/xdg-desktop-portal ${GELABBER_PORTAL_VERBOSE:+--verbose} &
pids+=($!)
gdbus wait --session --timeout 15 org.freedesktop.portal.Desktop

"$@"
