#!/bin/sh
# Run from deploy/compose. Failure leaves writers stopped and preserves evidence.
# With GELABBER_BACKUP_AGE_RECIPIENT set (an age public key), the dump and the
# MinIO archive are stored only encrypted (*.age); SHA256SUMS still names the
# plain files, so check it after decrypting.
set -eu
backup_age_recipient=${GELABBER_BACKUP_AGE_RECIPIENT:-}
if [ -n "$backup_age_recipient" ] && ! command -v age >/dev/null 2>&1; then
  printf '%s\n' "GELABBER_BACKUP_AGE_RECIPIENT ist gesetzt, aber age fehlt. Nichts gestoppt." >&2
  exit 1
fi
trap 'code=$?; if [ "$code" -ne 0 ]; then printf "%s\n" "Backup fehlgeschlagen. Teilbackup prüfen; ggf. docker compose start minio api web media ausführen." >&2; fi' 0
umask 077
restore_backup_dir="$PWD/backups/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$PWD/backups"
mkdir "$restore_backup_dir"
restore_minio_container=$(docker compose ps -q minio)
restore_minio_volume=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Name}}{{end}}{{end}}' "$restore_minio_container")
restore_minio_image=$(docker inspect --format '{{.Image}}' "$restore_minio_container")
test -n "$restore_minio_volume"
printf '%s\n' "$restore_minio_image" > "$restore_backup_dir/minio-image-id.txt"
docker compose stop api web media
docker compose stop minio
docker compose exec -T postgres sh -c 'exec pg_dump -U "$POSTGRES_USER" --format=custom "$POSTGRES_DB"' \
  > "$restore_backup_dir/database.pgdump"
docker run --rm --pull never --network none --read-only --user 0 \
  --mount "type=volume,src=$restore_minio_volume,dst=/data,readonly" \
  --entrypoint tar "$restore_minio_image" -czf - -C /data . \
  > "$restore_backup_dir/minio-data.tgz"
(cd "$restore_backup_dir" && sha256sum database.pgdump minio-data.tgz minio-image-id.txt > SHA256SUMS)
docker compose start minio api web media
if [ -n "$backup_age_recipient" ]; then
  for backup_file in database.pgdump minio-data.tgz; do
    age -r "$backup_age_recipient" -o "$restore_backup_dir/$backup_file.age" "$restore_backup_dir/$backup_file"
    rm "$restore_backup_dir/$backup_file"
  done
fi
