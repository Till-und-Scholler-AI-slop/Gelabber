#!/bin/sh
# Bundled coturn. Static long-term credentials unless TURN_AUTH_SECRET is a
# private value shared with the API. The retired public default is refused
# so a copied fallback cannot turn REST on by itself. Static mode needs
# TURN_PASSWORD; there is no default password.
set -eu

secret="${TURN_AUTH_SECRET:-}"
user="${TURN_USERNAME:-gelabber}"
pass="${TURN_PASSWORD:-}"
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
  --no-tcp-relay
  --no-multicast-peers
"

# Clients only ever relay to the SFU. Without deny rules coturn relays to any
# address, so a TURN user could reach Redis, the API, the host and the LAN.
for range in \
  0.0.0.0-0.255.255.255 \
  10.0.0.0-10.255.255.255 \
  100.64.0.0-100.127.255.255 \
  127.0.0.0-127.255.255.255 \
  169.254.0.0-169.254.255.255 \
  172.16.0.0-172.31.255.255 \
  192.168.0.0-192.168.255.255 \
  ::1 \
  fc00::-fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff \
  fe80::-febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff
do
  common="$common --denied-peer-ip=$range"
done

# Allow rules win over deny rules: the SFU on a LAN address stays reachable,
# plus what the operator lists (deploy/README.md, "TURN relay targets").
media_ip=$(printf '%s' "${MEDIA_ADVERTISED_IP:-}" | tr -d '[:space:]')
case "$media_ip" in
  "" | 127.* | ::1) ;;
  *) common="$common --allowed-peer-ip=$media_ip" ;;
esac
for range in $(printf '%s' "${TURN_ALLOWED_PEER_IPS:-}" | tr ',' ' '); do
  common="$common --allowed-peer-ip=$range"
done

if [ -n "$secret" ]; then
  # REST credentials are still long-term (`expiry:user`). No static --user.
  # shellcheck disable=SC2086
  exec turnserver $common --lt-cred-mech --use-auth-secret --static-auth-secret="$secret"
fi

if [ -z "$pass" ]; then
  echo "TURN_PASSWORD is not set. Set it in .env (see deploy/README.md), or set a private TURN_AUTH_SECRET shared with the API." >&2
  exit 1
fi

# shellcheck disable=SC2086
exec turnserver $common --lt-cred-mech --user="${user}:${pass}"
