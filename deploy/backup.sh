#!/bin/sh
# Run from deploy/compose. Failure leaves writers stopped and preserves evidence.
# With GELABBER_BACKUP_AGE_RECIPIENT set (an age public key), the dump and the
# MinIO archive are encrypted while they are written and stored only as *.age,
# never in plain text. SHA256SUMS names the files as stored: check it before
# decrypting. A directory without SHA256SUMS, or with *.part files, is incomplete.
set -eu
backup_age_recipient=${GELABBER_BACKUP_AGE_RECIPIENT:-}
if [ -n "$backup_age_recipient" ] && ! command -v age >/dev/null 2>&1; then
  printf '%s\n' "GELABBER_BACKUP_AGE_RECIPIENT is set, but age is missing. Nothing was stopped." >&2
  exit 1
fi
if [ -n "$backup_age_recipient" ] && ! printf '' | age -r "$backup_age_recipient" >/dev/null 2>&1; then
  printf '%s\n' "GELABBER_BACKUP_AGE_RECIPIENT is not a valid age recipient. Nothing was stopped." >&2
  exit 1
fi
trap 'code=$?; if [ "$code" -ne 0 ]; then printf "%s\n" "Backup fehlgeschlagen. Teilbackup prüfen; ggf. docker compose start minio api web media ausführen." >&2; fi' 0
umask 077
restore_backup_dir="$PWD/backups/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$PWD/backups"
mkdir "$restore_backup_dir"
backup_suffix=
if [ -n "$backup_age_recipient" ]; then
  backup_suffix=.age
fi
# backup_to NAME COMMAND...: COMMAND's output becomes NAME, or with a recipient
# NAME.age, encrypted on its way through a FIFO. Not a pipe: sh has no
# pipefail, so a failing COMMAND would go unnoticed. age writes NAME.age.part,
# renamed only once both succeeded: age seals whatever input it got, so a cut
# off dump would otherwise decrypt cleanly.
backup_to() {
  backup_name=$1
  shift
  if [ -z "$backup_age_recipient" ]; then
    "$@" > "$restore_backup_dir/$backup_name"
    return
  fi
  backup_fifo="$restore_backup_dir/.$backup_name.fifo"
  mkfifo "$backup_fifo"
  age -r "$backup_age_recipient" -o "$restore_backup_dir/$backup_name.age.part" < "$backup_fifo" &
  backup_age_pid=$!
  "$@" > "$backup_fifo"
  wait "$backup_age_pid"
  mv "$restore_backup_dir/$backup_name.age.part" "$restore_backup_dir/$backup_name.age"
  rm "$backup_fifo"
}
restore_minio_container=$(docker compose ps -q minio)
restore_minio_volume=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Name}}{{end}}{{end}}' "$restore_minio_container")
restore_minio_image=$(docker inspect --format '{{.Image}}' "$restore_minio_container")
test -n "$restore_minio_volume"
printf '%s\n' "$restore_minio_image" > "$restore_backup_dir/minio-image-id.txt"
docker compose stop api web media
docker compose stop minio
backup_to database.pgdump \
  docker compose exec -T postgres sh -c 'exec pg_dump -U "$POSTGRES_USER" --format=custom "$POSTGRES_DB"'
backup_to minio-data.tgz \
  docker run --rm --pull never --network none --read-only --user 0 \
  --mount "type=volume,src=$restore_minio_volume,dst=/data,readonly" \
  --entrypoint tar "$restore_minio_image" -czf - -C /data .
(cd "$restore_backup_dir" && sha256sum "database.pgdump$backup_suffix" "minio-data.tgz$backup_suffix" minio-image-id.txt > SHA256SUMS)
docker compose start minio api web media
