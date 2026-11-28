#!/usr/bin/env bash
# Starts the test MinIO that deploy/compose/compose.yaml pins on
# 127.0.0.1:9000 and waits until it is ready (e2e-extended.yml's jobs):
#
#   start-minio.sh <container name> [docker run options]
#
# MINIO_RELEASE names the release; the image is ghcr.io/<repository>/minio
# (GITHUB_REPOSITORY), and fails when compose.yaml pins another.
# MINIO_ROOT_USER and MINIO_ROOT_PASSWORD go to the container,
# MINIO_ENDPOINT is where it answers.
set -euo pipefail
name="${1:?usage: start-minio.sh <container name> [docker run options]}"
shift
: "${MINIO_RELEASE:?the MinIO release compose.yaml pins}"
: "${MINIO_ENDPOINT:?where MinIO answers, http://127.0.0.1:9000}"
root="$(cd "$(dirname "$0")/../../.." && pwd)"
repo="${GITHUB_REPOSITORY,,}"
pin="ghcr.io/${repo}/minio:${MINIO_RELEASE}"
grep -F "MINIO_RELEASE: ${MINIO_RELEASE}" "$root/deploy/compose/compose.yaml"
grep -F "$pin" "$root/deploy/compose/compose.yaml"
docker pull "$pin"
docker run -d --name "$name" -p 127.0.0.1:9000:9000 \
  -e MINIO_ROOT_USER -e MINIO_ROOT_PASSWORD "$@" "$pin" server /data
deadline=$((SECONDS + 120))
until curl -fsS "$MINIO_ENDPOINT/minio/health/ready" >/dev/null; do
  if [ "$SECONDS" -ge "$deadline" ]; then
    echo "MinIO did not become ready within 120 s" >&2
    docker logs "$name" >&2 || true
    exit 1
  fi
  sleep 2
done
