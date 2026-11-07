#!/usr/bin/env bash
# Starts the native app with no server and empty settings for a few seconds
# (GELABBER_SMOKE) and checks that it paints its setup screen and ends
# cleanly: window, wgpu (lavapipe in CI), fonts and the platform shell.
#
#   smoke.sh <gelabber> [seconds]
#
# Without a display it runs under xvfb-run. Nothing outside a temporary
# directory is written.
set -euo pipefail
binary="$(realpath "${1:?usage: smoke.sh <gelabber> [seconds]}")"
seconds="${2:-5}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -m 700 "$work/run"

run=(env -u GELABBER_SERVER -u WAYLAND_DISPLAY
  GELABBER_CONFIG_DIR="$work/config" XDG_RUNTIME_DIR="$work/run"
  GELABBER_SMOKE="$seconds" RUST_LOG="${RUST_LOG:-info}" "$binary")
if [ -z "${DISPLAY:-}" ]; then
  run=(xvfb-run -a -s "-screen 0 1280x800x24" "${run[@]}")
fi
status=0
timeout --kill-after=10 "$((seconds + 60))" "${run[@]}" >"$work/log" 2>&1 || status=$?
cat "$work/log"
if [ "$status" -ne 0 ] || ! grep -q '^gelabber smoke: ok on setup ' "$work/log"; then
  echo "smoke: the app did not reach its setup screen (exit status $status)" >&2
  exit 1
fi
# The run takes over nothing and writes no settings of its own.
if [ -e "$work/config/settings.json" ]; then
  echo "smoke: settings.json was written without a change" >&2
  exit 1
fi
echo "smoke: OK"
