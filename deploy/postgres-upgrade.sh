#!/bin/sh
# Moves the bundled PostgreSQL to the major version compose.yaml names, by
# dump and restore. Run from deploy/compose, with the checkout already on the
# release that brings the new version, before `docker compose up`.
#
# The old cluster stays on the volume as it was, so going back means the old
# compose.yaml again (changes made after the upgrade are lost then). The dump
# is kept in backups/<time>-postgres-<old>-to-<new>/ as one more copy.
#
# Writers (api, web, media) are stopped first and stay stopped on an error;
# a failure leaves the old cluster untouched. On success PostgreSQL runs on
# the new version and the rest of the rollout follows as usual.
set -eu

old_image=${GELABBER_POSTGRES_OLD_IMAGE:-postgres:18.6-alpine3.24}
old_name=gelabber-postgres-upgrade-old
new_name=gelabber-postgres-upgrade-new

fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

cleanup() {
  docker rm -f "$old_name" "$new_name" >/dev/null 2>&1 || true
}

new_image=$(docker compose config --images | grep '^postgres:' || true)
case "$new_image" in
  postgres:[0-9]*) ;;
  *) fail "compose.yaml names no postgres:<version> image (found: '${new_image:-nothing}'). Run from deploy/compose." ;;
esac
new_major=${new_image#postgres:}
new_major=${new_major%%[.-]*}
old_major=${old_image#postgres:}
old_major=${old_major%%[.-]*}
if [ "$old_major" = "$new_major" ]; then
  fail "compose.yaml still names PostgreSQL $new_major. Check out the release that brings the new version first."
fi

container=$(docker compose ps -aq postgres)
test -n "$container" || fail "No postgres container of this Compose project. Nothing to upgrade."
volume=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/var/lib/postgresql"}}{{.Name}}{{end}}{{end}}' "$container")
test -n "$volume" || fail "The postgres container has no volume on /var/lib/postgresql."

# What is on the volume: run the new image only to look.
state=$(docker run --rm --network none --entrypoint sh \
  --mount "type=volume,src=$volume,dst=/var/lib/postgresql,readonly" "$new_image" -c '
    if [ -s "/var/lib/postgresql/$1/docker/PG_VERSION" ]; then echo new
    elif [ -s "/var/lib/postgresql/$2/docker/PG_VERSION" ]; then echo old
    else echo none; fi' sh "$new_major" "$old_major")
case "$state" in
  new) printf '%s\n' "PostgreSQL $new_major already has its cluster on volume $volume. Nothing to do."; exit 0 ;;
  old) ;;
  *) fail "Volume $volume holds no PostgreSQL $old_major cluster (expected /var/lib/postgresql/$old_major/docker)." ;;
esac

trap 'code=$?; cleanup; if [ "$code" -ne 0 ]; then printf "%s\n" "Upgrade failed. The PostgreSQL $old_major data is unchanged; api, web and media are stopped. Back to the old version: check out the previous release and run docker compose up -d." >&2; fi' 0

umask 077
work="$PWD/backups/$(date -u +%Y%m%dT%H%M%SZ)-postgres-$old_major-to-$new_major"
mkdir -p "$PWD/backups"
mkdir "$work"

printf '%s\n' "Upgrading PostgreSQL $old_major -> $new_major on volume $volume."
docker compose stop api web media
docker compose stop postgres
cleanup

# wait_ready NAME: the server of container NAME accepts queries, after the
# entrypoint's own first-start setup (which runs a temporary server too).
server_up() {
  logs=$(docker logs "$1" 2>&1)
  ready=$(printf '%s\n' "$logs" | grep -c 'database system is ready to accept connections' || true)
  # A first start reports ready twice: the setup server, then the real one.
  case "$logs" in
    *'init process complete'*) [ "$ready" -ge 2 ] ;;
    *) [ "$ready" -ge 1 ] ;;
  esac && docker exec "$1" pg_isready -q >/dev/null 2>&1
}
wait_ready() {
  tries=0
  until server_up "$1"; do
    tries=$((tries + 1))
    if [ "$tries" -gt 120 ]; then
      docker logs "$1" >&2 || true
      fail "PostgreSQL in $1 did not get ready."
    fi
    sleep 1
  done
}

# The old server on its own data, unreachable from any network.
docker run -d --name "$old_name" --network none \
  --mount "type=volume,src=$volume,dst=/var/lib/postgresql" "$old_image" >/dev/null
wait_ready "$old_name"

# The new server from compose.yaml (same user, password and database from
# .env), creating its empty cluster next to the old one. The image refuses a
# first start in /var/lib/postgresql/<major>/docker while an older cluster is
# on the volume; through a second mount of the same volume, the same folder
# has another path and the image lets it be created.
docker compose run -d --no-deps --name "$new_name" \
  -v "$volume:/upgrade" -e "PGDATA=/upgrade/$new_major/docker" postgres >/dev/null
wait_ready "$new_name"

user=$(docker exec "$new_name" printenv POSTGRES_USER)
db=$(docker exec "$new_name" printenv POSTGRES_DB)

docker exec "$old_name" pg_dump -U "$user" --format=custom "$db" > "$work/database.pgdump"
docker exec -i "$new_name" pg_restore -U "$user" --no-owner --exit-on-error -d "$db" < "$work/database.pgdump"

# Same tables with the same number of rows on both sides.
count_rows='SELECT format($$SELECT %L, count(*) FROM %I.%I$$, schemaname || $$.$$ || tablename, schemaname, tablename)
  FROM pg_tables WHERE schemaname NOT IN ($$pg_catalog$$, $$information_schema$$) ORDER BY 1 \gexec'
printf '%s\n' "$count_rows" | docker exec -i "$old_name" psql -X -q -A -t -v ON_ERROR_STOP=1 -U "$user" -d "$db" > "$work/rows-$old_major.txt"
printf '%s\n' "$count_rows" | docker exec -i "$new_name" psql -X -q -A -t -v ON_ERROR_STOP=1 -U "$user" -d "$db" > "$work/rows-$new_major.txt"
test -s "$work/rows-$old_major.txt" || fail "No tables found in the PostgreSQL $old_major database $db."
if ! cmp -s "$work/rows-$old_major.txt" "$work/rows-$new_major.txt"; then
  diff "$work/rows-$old_major.txt" "$work/rows-$new_major.txt" >&2 || true
  fail "Row counts differ between PostgreSQL $old_major and $new_major (see $work)."
fi
tables=$(wc -l < "$work/rows-$old_major.txt")
(cd "$work" && sha256sum database.pgdump "rows-$old_major.txt" "rows-$new_major.txt" > SHA256SUMS)

cleanup
docker compose up -d --no-deps postgres
printf '%s\n' "Done: $tables tables with the same row counts on PostgreSQL $new_major. Dump and counts in $work." \
  "api, web and media are still stopped: continue with the rollout (deploy/README.md)." \
  "The PostgreSQL $old_major cluster stays on the volume for a way back; remove it once the new version runs well."
