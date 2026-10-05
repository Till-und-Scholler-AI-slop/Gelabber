#!/bin/sh
set -eu
folder=$(mktemp -d)
cp /opt/janus/etc/janus/*.jcfg "$folder/"
if [ -n "${BENCH_HTTP_PORT:-}" ]; then
    case "$BENCH_HTTP_PORT" in *[!0-9]*) exit 2 ;; esac
    printf 'general: { json="compact"; base_path="/janus"; http=true; port=%s; https=false; }\nadmin: { admin_http=false; admin_https=false; }\n' "$BENCH_HTTP_PORT" > "$folder/janus.transport.http.jcfg"
fi
exec /opt/janus/bin/janus -F "$folder" "$@"
