#!/bin/sh
# Hands the origin of MINIO_PUBLIC_ENDPOINT to nginx.conf, which then adds
# fetch directives to the shell's Content-Security-Policy. Without a usable
# value nginx.conf keeps them out, so attachments and uploads never depend on
# a value the container may not have. With ATTACHMENT_DELIVERY=proxy browsers
# never talk to MinIO (#348): it hands over 'self' and the policy names no
# storage host. Never fails the container: the worst case is the policy of a
# container without the variable.

ME=$(basename "$0")
# GELABBER_STORAGE_MAP only for the unit test (src/pwa/nginx.test.mjs).
MAP=${GELABBER_STORAGE_MAP:-/etc/nginx/gelabber/storage-origin.map}

log() {
    [ -n "${NGINX_ENTRYPOINT_QUIET_LOGS:-}" ] || echo "$ME: $*"
}

keep_out() {
    rm -f "$MAP" 2>/dev/null
    log "$1; the Content-Security-Policy has no fetch directives (deploy/README.md)"
    exit 0
}

# One line for the map in nginx.conf: the empty key and this value.
hand_over() {
    if mkdir -p "$(dirname "$MAP")" 2>/dev/null &&
        printf '"" "%s";\n' "$1" >"$MAP" 2>/dev/null; then
        log "$2"
    else
        keep_out "warning: cannot write $MAP"
    fi
    exit 0
}

delivery=$(printf '%s' "${ATTACHMENT_DELIVERY:-}" | tr 'A-Z' 'a-z' | tr -d ' ')
[ "$delivery" != proxy ] ||
    hand_over "'self'" "uploads and attachments go through the API (ATTACHMENT_DELIVERY=proxy); the Content-Security-Policy names no storage host"

endpoint=$(printf '%s' "${MINIO_PUBLIC_ENDPOINT:-}" | tr 'A-Z' 'a-z')
[ -n "$endpoint" ] || keep_out "MINIO_PUBLIC_ENDPOINT is not set"

# Scheme, host and port; the path is not part of an origin. A CSP source
# takes letters, digits, dots and hyphens as host and no IPv6 address.
origin=$(printf '%s\n' "$endpoint" |
    sed -n -E 's#^(https?://[a-z0-9-]+(\.[a-z0-9-]+)*(:[0-9]{1,5})?)(/.*)?$#\1#p')
[ -n "$origin" ] ||
    keep_out "warning: MINIO_PUBLIC_ENDPOINT=$MINIO_PUBLIC_ENDPOINT has no origin a Content-Security-Policy can name"

hand_over "$origin" "the Content-Security-Policy allows uploads and attachments on $origin"
