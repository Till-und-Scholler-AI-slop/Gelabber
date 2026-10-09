#!/usr/bin/env bash
# Starts the built desktop app on a stand-in server origin (smoke_server.py)
# under Xvfb and fails unless the page reached the media commands, and only
# those, and got the frames of a test-pattern view.
# Usage: smoke.sh <path to gelabber-desktop> <expected ABI version>
set -euo pipefail
app="$1"
abi="$2"
here="$(cd "$(dirname "$0")" && pwd)"
port=18099

python3 -I "$here/smoke_server.py" "$port" "$abi" &
server=$!
# Own process group, so the app goes down with xvfb-run.
GELABBER_SERVER="http://127.0.0.1:$port" WEBKIT_DISABLE_COMPOSITING_MODE=1 \
  GELABBER_VIDEO_TEST_PATTERN=1 setsid xvfb-run -a "$app" &
app_pid=$!

status=0
for _ in $(seq 1 120); do
  if ! kill -0 "$server" 2>/dev/null; then
    wait "$server" || status=$?
    break
  fi
  sleep 0.5
done
if kill -0 "$server" 2>/dev/null; then
  echo "FAIL: no report from the app within 60 s"
  kill "$server"
  status=1
fi
kill -- "-$app_pid" 2>/dev/null || true
wait "$app_pid" 2>/dev/null || true
exit "$status"
