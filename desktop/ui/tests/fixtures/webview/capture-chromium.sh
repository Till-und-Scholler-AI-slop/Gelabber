#!/usr/bin/env bash
# Writes chromium/: the LevelDB localStorage store (the format WebView2
# keeps) that headless Chromium leaves after the fixture page ran on
# 127.0.0.1:8124 and then 127.0.0.1:8123. Needs Docker; runs offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
image=mcr.microsoft.com/playwright:v1.63.0-noble
out="$here/chromium"
rm -rf "$out" && mkdir "$out"
docker run --rm --network none -v "$here/page:/page:ro" -v "$out:/out" \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" "$image" bash -euo pipefail -c '
    chrome="$(ls /ms-playwright/chromium-*/chrome-linux64/chrome)"
    "$chrome" --version
    profile=/tmp/profile
    for port in 8124 8123; do
      python3 -I /page/serve.py "$port" "/tmp/done-$port" & server=$!
      # The testing field trials of Chrome for Testing switch new profiles to
      # SQLite DOM storage; the stable default (and WebView2 so far) is LevelDB.
      "$chrome" --headless=new --no-sandbox --no-first-run --disable-gpu --disable-field-trial-config \
        --user-data-dir="$profile" "http://127.0.0.1:$port/?at=fixed" >/tmp/chrome.log 2>&1 & browser=$!
      for _ in $(seq 60); do [ -e "/tmp/done-$port" ] && break; sleep 1; done
      test -e "/tmp/done-$port"
      # Past the 5 s commit delay, then a graceful close (SIGTERM).
      sleep 8
      kill -TERM "$browser"; wait "$browser" || true
      kill "$server"; wait "$server" || true
    done
    store="$profile/Default/Local Storage/leveldb"
    ls -l "$store"
    cd "$store"
    cp CURRENT MANIFEST-* ./*.log ./*.ldb /out/ 2>/dev/null || cp CURRENT MANIFEST-* ./*.log /out/
    chown -R "$HOST_UID:$HOST_GID" /out
  '
ls -l "$out"
