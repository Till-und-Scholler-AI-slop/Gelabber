#!/bin/sh
# Run from deploy/compose after an update. The API hands clients the TURN
# credentials from its own environment; coturn only accepts the ones it was
# started with. Watchtower and `up … api` recreate the API with new values but
# leave coturn running with the old ones, and voice connects without sound
# (#200). Compares the running containers, never prints the values.
set -eu
api=$(docker compose ps -q api)
# Inactive profile (homelab overlay): compose may not know the service.
coturn=$(docker compose ps -q coturn 2>/dev/null || true)
if [ -z "$api" ]; then
  printf '%s\n' "api läuft nicht." >&2
  exit 1
fi
if [ -z "$coturn" ]; then
  printf '%s\n' "Kein mitgelieferter coturn aktiv; externen TURN-Server selbst gegen TURN_USERNAME/TURN_PASSWORD bzw. TURN_AUTH_SECRET prüfen."
  exit 0
fi
value() {
  docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$1" |
    sed -n "s/^$2=//p"
}
stale=""
for key in TURN_AUTH_SECRET TURN_USERNAME TURN_PASSWORD; do
  if [ "$(value "$api" "$key")" != "$(value "$coturn" "$key")" ]; then
    stale="$stale $key"
  fi
done
if [ -n "$stale" ]; then
  printf '%s\n' "TURN-Zugangsdaten von api und coturn weichen ab:$stale" \
    "Beheben: docker compose up -d --force-recreate coturn api" >&2
  exit 1
fi
printf '%s\n' "TURN-Zugangsdaten von api und coturn stimmen überein."
