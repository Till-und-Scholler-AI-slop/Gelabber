#!/bin/bash
# Starts the services the backend tests need (Postgres, Redis, MinIO) on
# their default ports, if they are not running yet, and prints the
# environment for them (values from backend/api/.env.example).
# Usage: eval "$(.claude/skills/test-services/services.sh)"
set -euo pipefail
cd "$(dirname "$0")/../../.."
note() { echo "$*" >&2; }

if ! pg_isready -q -h 127.0.0.1 -p 5432 2>/dev/null; then
  if command -v pg_ctlcluster >/dev/null; then
    pg_ctlcluster "$(pg_lsclusters -h | awk 'NR==1{print $1}')" main start
  else
    note "Postgres is not running on 5432; start it (role gelabber with CREATEDB)."
  fi
fi

if ! redis-cli -p 6379 ping >/dev/null 2>&1; then
  # From /tmp, so a snapshot (dump.rdb) never lands in the repository.
  (cd /tmp && redis-server --daemonize yes >/dev/null)
fi

# The pinned MinIO image of the deployment.
minio_image=$(grep -o 'ghcr.io/[^}" ]*/minio:[^}" ]*' deploy/compose/compose.yaml | head -1)
if ! curl -fs http://127.0.0.1:9000/minio/health/live >/dev/null 2>&1; then
  if command -v docker >/dev/null; then
    docker info >/dev/null 2>&1 || { (dockerd >/tmp/dockerd.log 2>&1 &); sleep 5; }
    docker rm -f gelabber-test-minio >/dev/null 2>&1 || true
    env_value() { sed -n "s/^$1=//p" backend/api/.env.example | head -1; }
    docker run -d --name gelabber-test-minio -p 127.0.0.1:9000:9000 \
      -e MINIO_ROOT_USER="$(env_value MINIO_ROOT_USER)" -e MINIO_ROOT_PASSWORD="$(env_value MINIO_ROOT_PASSWORD)" \
      "$minio_image" server /data >/dev/null
    for _ in $(seq 30); do curl -fs http://127.0.0.1:9000/minio/health/live >/dev/null 2>&1 && break; sleep 1; done
  else
    note "MinIO is not running and Docker is missing: storage and avatar tests will fail."
  fi
fi

grep -E '^(DATABASE_URL|REDIS_URL|MINIO_ENDPOINT|MINIO_PUBLIC_ENDPOINT|MINIO_ROOT_USER|MINIO_ROOT_PASSWORD|MINIO_BUCKET)=' \
  backend/api/.env.example | sed 's/^/export /'
