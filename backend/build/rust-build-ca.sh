#!/bin/sh
# Prepare rustup's cargo to fetch crates.io inside rust:*-slim images.
# rustup ships a vendored libcurl that may miss Debian's CA bundle; Docker
# Desktop / TLS-inspecting proxies then fail with OpenSSL verify 19
# (self-signed certificate in certificate chain). Trusting the chain the
# server presents is opt-in (TRUST_PRESENTED_CRATES_CA=true, a build arg):
# otherwise anyone on the path could feed the build.
set -eu

apt-get update
apt-get install -y --no-install-recommends ca-certificates openssl
update-ca-certificates

bundle=/etc/ssl/certs/ca-certificates.crt
presented=/usr/local/share/ca-certificates/crates-io-presented.crt

if openssl s_client -verify_return_error \
    -servername index.crates.io \
    -connect index.crates.io:443 </dev/null \
    >/tmp/crates-tls.out 2>/tmp/crates-tls.err; then
  echo "crates.io TLS verify ok"
elif [ "${TRUST_PRESENTED_CRATES_CA:-false}" != true ]; then
  cat /tmp/crates-tls.err >&2 || true
  echo "crates.io TLS verify failed. Behind a TLS-inspecting proxy, build with --build-arg TRUST_PRESENTED_CRATES_CA=true to trust the chain it presents." >&2
  exit 1
else
  echo "crates.io TLS verify failed; trusting presented chain for cargo fetch (TRUST_PRESENTED_CRATES_CA=true)" >&2
  cat /tmp/crates-tls.err >&2 || true
  openssl s_client -showcerts \
    -servername index.crates.io \
    -connect index.crates.io:443 </dev/null 2>/dev/null \
    | awk '/-----BEGIN CERTIFICATE-----/,/-----END CERTIFICATE-----/' \
    > "$presented"
  if [ ! -s "$presented" ]; then
    echo "could not capture crates.io certificate chain" >&2
    exit 1
  fi
  update-ca-certificates
fi

mkdir -p "${CARGO_HOME:-/usr/local/cargo}"
printf '%s\n' \
  '[http]' \
  "cainfo = \"$bundle\"" \
  'multiplexing = false' \
  '[net]' \
  'retry = 3' \
  > "${CARGO_HOME:-/usr/local/cargo}/config.toml"

rm -rf /var/lib/apt/lists/* /tmp/crates-tls.out /tmp/crates-tls.err
