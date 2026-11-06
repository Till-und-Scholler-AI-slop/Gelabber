#!/usr/bin/env bash
# Writes webkitgtk/: the localStorage databases the Tauri app of 0.8.9
# (WebKitGTK on Arch) leaves after the fixture page ran on 127.0.0.1:8124,
# then 127.0.0.1:8123 (desktop.json switched between the runs).
#
#   capture-webkitgtk.sh <gelabber-desktop-linux-x64.tar.gz of v0.8.9>
#
# Needs Docker. The image with WebKitGTK is built with the network; the
# capture itself runs offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tarball="$(realpath "${1:?usage: capture-webkitgtk.sh <tauri-tarball>}")"
image=gelabber-webview-fixture-webkitgtk:local
out="$here/webkitgtk"
docker build --tag "$image" - <<'DOCKERFILE'
FROM archlinux:base-devel
RUN pacman -Syu --noconfirm --needed webkit2gtk-4.1 gtk3 libsoup3 gst-plugins-base \
      gst-plugins-good libpulse xorg-server-xvfb xorg-xauth python && pacman -Scc --noconfirm
DOCKERFILE
rm -rf "$out" && mkdir "$out"
docker run --rm --network none -v "$here/page:/page:ro" -v "$tarball:/in/tauri.tar.gz:ro" \
  -v "$out:/out" -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" "$image" bash -euo pipefail -c '
    mkdir /opt/tauri && tar -xzf /in/tauri.tar.gz -C /opt/tauri --strip-components=1
    id=io.github.till-und-scholler-ai-slop.gelabber
    mkdir -p "/root/.config/$id"
    for port in 8124 8123; do
      printf "{\"server\":\"http://127.0.0.1:%s\"}" "$port" >"/root/.config/$id/desktop.json"
      python3 -I /page/serve.py "$port" "/tmp/done-$port" fixed 2>/dev/null & server=$!
      WEBKIT_DISABLE_COMPOSITING_MODE=1 setsid xvfb-run -a /opt/tauri/gelabber-desktop \
        >"/tmp/app-$port.log" 2>&1 & app=$!
      for _ in $(seq 60); do [ -e "/tmp/done-$port" ] && break; sleep 1; done
      test -e "/tmp/done-$port" || { cat "/tmp/app-$port.log"; exit 1; }
      sleep 2
      kill -TERM -- "-$app"; wait "$app" || true
      kill "$server"; wait "$server" || true
    done
    store="/root/.local/share/$id/localstorage"
    ls -l "$store"
    cp "$store"/http_127.0.0.1_812[34].localstorage* /out/
    rm -f /out/*-shm
    chown -R "$HOST_UID:$HOST_GID" /out
  '
ls -l "$out"
