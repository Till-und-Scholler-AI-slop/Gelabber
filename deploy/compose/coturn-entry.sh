#!/bin/sh
# Bundled coturn. Static long-term credentials unless TURN_AUTH_SECRET is a
# private value shared with the API. The retired public default is refused
# so a copied fallback cannot turn REST on by itself.
set -eu

secret="${TURN_AUTH_SECRET:-}"
user="${TURN_USERNAME:-gelabber}"
pass="${TURN_PASSWORD:-gelabberturn}"
realm="${TURN_REALM:-gelabber.local}"
external_ip="${TURN_EXTERNAL_IP:-127.0.0.1}"
relay_min="${TURN_RELAY_MIN:-49160}"
relay_max="${TURN_RELAY_MAX:-49200}"

# Trim surrounding whitespace so a blank assignment stays static mode.
secret="${secret#"${secret%%[![:space:]]*}"}"
secret="${secret%"${secret##*[![:space:]]}"}"

if [ "$secret" = "gelabberturnsecret" ]; then
  echo "TURN_AUTH_SECRET is the public default and is not accepted. Unset it for static TURN username/password, or set a private secret on both the API and coturn." >&2
  exit 1
fi

common="
  -n
  --log-file=stdout
  --listening-port=3478
  --min-port=${relay_min}
  --max-port=${relay_max}
  --fingerprint
  --realm=${realm}
  --no-tls
  --listening-ip=0.0.0.0
  --external-ip=${external_ip}
"

if [ -n "$secret" ]; then
  # REST credentials are still long-term (`expiry:user`). No static --user.
  # shellcheck disable=SC2086
  exec turnserver $common --lt-cred-mech --use-auth-secret --static-auth-secret="$secret"
fi

# shellcheck disable=SC2086
exec turnserver $common --lt-cred-mech --user="${user}:${pass}"
