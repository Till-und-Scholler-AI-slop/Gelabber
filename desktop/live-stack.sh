#!/usr/bin/env bash
# The desktop's live tests on a seeded parity stack (desktop/README.md
# "Tests"; desktop-live in e2e-extended.yml): starts the API, the media server
# and Vite, seeds them, and runs live-tests.sh.
#
#   live-stack.sh <logs>                 API and media server built from this
#                                        tree (backend/target/debug), and its
#                                        web client; the fixtures are checked
#   live-stack.sh <logs> <tag> <web>     the published images of release <tag>
#                                        and that release's web/ in <web>
#
# Postgres, Redis and MinIO run already. The API's and media server's
# settings come from the environment (API_ADDR, MEDIA_ADDR, ...);
# GELABBER_TEST_SERVER is Vite's origin. Logs land in <logs>.
set -euo pipefail
logs=${1:?usage: live-stack.sh <logs> [<tag> <web>]}
tag=${2:-}
root="$(cd "$(dirname "$0")/.." && pwd)"
web=${3:-$root/web}
: "${GELABBER_TEST_SERVER:?the web origin of the stack, e.g. http://127.0.0.1:5180}"
: "${API_ADDR:?the address the API listens on}"
mkdir -p "$logs"
port=${GELABBER_TEST_SERVER##*:}
port=${port%%/*}
pids=()
stop() {
  if [ -n "$tag" ]; then
    docker logs gelabber-previous-api >"$logs/api.log" 2>&1 || true
    docker logs gelabber-previous-media >"$logs/media.log" 2>&1 || true
    docker rm -f gelabber-previous-api gelabber-previous-media >/dev/null 2>&1 || true
  fi
  kill "${pids[@]}" 2>/dev/null || true
}
trap stop EXIT

if [ -n "$tag" ]; then
  # The released images, on the host's network like the services.
  images="ghcr.io/${GITHUB_REPOSITORY:-till-und-scholler-ai-slop/gelabber}"
  images=${images,,}
  api_env=()
  for name in DATABASE_URL REDIS_URL MINIO_ENDPOINT MINIO_PUBLIC_ENDPOINT MINIO_ROOT_USER \
      MINIO_ROOT_PASSWORD MINIO_BUCKET API_ADDR API_OPERATOR_EMAILS API_DIRECTORY APP_PUBLIC_URL \
      OIDC_GOOGLE_CLIENT_ID OIDC_GOOGLE_CLIENT_SECRET API_RATE_AUTH_PER_MIN; do
    api_env+=(-e "$name")
  done
  docker run -d --name gelabber-previous-api --network host "${api_env[@]}" "$images/api:$tag" >/dev/null
  docker run -d --name gelabber-previous-media --network host \
    -e REDIS_URL -e MEDIA_ADDR -e MEDIA_ICE_BIND -e MEDIA_ADVERTISED_IP "$images/media:$tag" >/dev/null
else
  "$root/backend/target/debug/gelabber-api" >"$logs/api.log" 2>&1 &
  pids+=($!)
  "$root/backend/target/debug/gelabber-media" >"$logs/media.log" 2>&1 &
  pids+=($!)
fi
(cd "$web" && exec node node_modules/vite/bin/vite.js --host 127.0.0.1 --port "$port" --strictPort) \
  >"$logs/vite.log" 2>&1 &
pids+=($!)

deadline=$((SECONDS + 120))
until curl -fsS "http://$API_ADDR/ready" >/dev/null 2>&1 && curl -fsS "$GELABBER_TEST_SERVER/" >/dev/null 2>&1; do
  if [ "$SECONDS" -ge "$deadline" ]; then
    echo "live-stack.sh: the API or Vite did not come up within 120 s" >&2
    tail -n 50 "$logs"/*.log >&2
    exit 1
  fi
  sleep 2
done

# The seed of that server's version, into the file the desktop tests read.
(cd "$web" && node scripts/parity/seed.mjs "$GELABBER_TEST_SERVER" "$root/web/scripts/parity/fixture.json")
status=0
# The recorded fixtures stand for this tree's API (desktop/client/tests/models.rs).
if [ -z "$tag" ]; then
  node "$root/desktop/client/tests/record-fixtures.mjs" "$GELABBER_TEST_SERVER" --check || status=1
fi
"$root/desktop/live-tests.sh" || status=1
if [ "$status" -ne 0 ]; then
  tail -n 100 "$logs"/*.log >&2
fi
exit "$status"
