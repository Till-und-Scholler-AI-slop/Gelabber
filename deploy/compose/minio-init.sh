#!/bin/sh
# One-shot before the API: bucket plus a MinIO user that may only use this
# bucket (#196). Root credentials stay in this container and in minio; the
# API gets MINIO_APP_USER / MINIO_APP_PASSWORD. Idempotent: every start
# re-applies the policy and the password from .env, objects stay untouched.
set -eu

bucket="${MINIO_BUCKET:-gelabber}"
user="${MINIO_APP_USER:-gelabber-api}"
pass="${MINIO_APP_PASSWORD:-}"
policy="gelabber-app-${bucket}"

if [ "${#pass}" -lt 8 ]; then
  echo "MINIO_APP_PASSWORD must have at least 8 characters (openssl rand -hex 24)." >&2
  exit 1
fi
if [ "$user" = "${MINIO_ROOT_USER:-}" ]; then
  echo "MINIO_APP_USER must differ from MINIO_ROOT_USER." >&2
  exit 1
fi
if [ "$pass" = "${MINIO_ROOT_PASSWORD:-}" ]; then
  echo "MINIO_APP_PASSWORD must differ from MINIO_ROOT_PASSWORD." >&2
  exit 1
fi

# mc takes both passwords as arguments, so they show in `ps` on the host for
# the moment each call runs. Only root on the host sees other containers'
# processes; the values are in `docker inspect` there anyway.

export MC_CONFIG_DIR=/tmp/mc
mc --quiet alias set local "${MINIO_INIT_ENDPOINT:-http://minio:9000}" \
  "${MINIO_ROOT_USER:?}" "${MINIO_ROOT_PASSWORD:?}" >/dev/null
mc --quiet mb --ignore-existing "local/${bucket}" >/dev/null

# Objects plus HEAD on the bucket. CreateBucket on this name only, because
# the API checks the bucket at startup (409 = already there).
cat > /tmp/policy.json <<JSON
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:CreateBucket", "s3:GetBucketLocation", "s3:ListBucket"],
      "Resource": ["arn:aws:s3:::${bucket}"]
    },
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
      "Resource": ["arn:aws:s3:::${bucket}/*"]
    }
  ]
}
JSON
mc --quiet admin policy create local "$policy" /tmp/policy.json >/dev/null
mc --quiet admin user add local "$user" "$pass" >/dev/null
mc --quiet admin policy attach local "$policy" --user "$user" >/dev/null
echo "MinIO user ${user} may use bucket ${bucket} only."
