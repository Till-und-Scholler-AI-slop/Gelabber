#!/usr/bin/env bash
# Starts the built desktop app on a stand-in server origin (smoke_server.py)
# under Xvfb and fails unless the page reached the media commands, and only
# those, and got the frames of a test-pattern view. Then ends the page's web
# process as a crash would (SIGKILL) and fails unless the app loads the page
# again by itself, without the view the first page left open. While the new
# page is still held back by the server, the left view's test pattern must
# already be gone: only the app's own answer to the ended web process
# (`Media::reset` in `watch_web_process`) can have stopped it then.
# Usage: smoke.sh <path to gelabber-desktop> <expected ABI version>
set -euo pipefail
app="$1"
abi="$2"
here="$(cd "$(dirname "$0")" && pwd)"
port=18099
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The server creates the file when the page's first report was good.
python3 -I "$here/smoke_server.py" "$port" "$abi" "$work/reported" &
server=$!
# Own process group, so the app goes down with xvfb-run.
GELABBER_SERVER="http://127.0.0.1:$port" WEBKIT_DISABLE_COMPOSITING_MODE=1 \
  GELABBER_VIDEO_TEST_PATTERN=1 setsid xvfb-run -a "$app" &
app_pid=$!

status=0
ended=
# Threads of the test pattern in the app (names are cut at 15 bytes).
patterns() {
  local pid count=0
  for pid in $(pgrep -s "$app_pid" -x gelabber-deskto); do
    count=$((count + $(grep -lx 'gelabber-test-p' /proc/"$pid"/task/*/comm 2>/dev/null | wc -l)))
  done
  echo "$count"
}
# 90 s for both reports; the server gives the second one 30 s.
for _ in $(seq 1 180); do
  if ! kill -0 "$server" 2>/dev/null; then
    wait "$server" || status=$?
    break
  fi
  if [ -z "$ended" ] && [ -e "$work/reported" ]; then
    if [ "$(patterns)" -eq 0 ]; then
      echo "FAIL: no test-pattern thread for the view the page left open"
      kill "$server"
      wait "$server" 2>/dev/null || true
      status=1
      break
    fi
    # The app's session (setsid): its own web process, no other program's.
    if pkill -KILL -s "$app_pid" -x WebKitWebProces; then
      ended=1
      # The server holds the page that comes back for 2 s.
      sleep 1
      left="$(patterns)"
      if [ "$left" -ne 0 ]; then
        echo "FAIL: $left test-pattern thread(s) of the ended page still run: nothing reset the media"
        kill "$server"
        wait "$server" 2>/dev/null || true
        status=1
        break
      fi
    else
      echo "FAIL: the app has no web process to end"
      kill "$server"
      wait "$server" 2>/dev/null || true
      status=1
      break
    fi
  fi
  sleep 0.5
done
if kill -0 "$server" 2>/dev/null; then
  echo "FAIL: the app's reports did not arrive within 90 s"
  kill "$server"
  status=1
fi
kill -- "-$app_pid" 2>/dev/null || true
wait "$app_pid" 2>/dev/null || true
exit "$status"
