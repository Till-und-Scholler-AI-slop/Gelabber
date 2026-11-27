#!/bin/sh
# Installs the pinned cargo-nextest into Cargo's bin folder, for ci.yml and
# desktop-native.yml alike: one version and one checksum.
set -eu
version=0.9.149
sha256=d65c7f14bcb5c1566d1ecd534c5da8dedb81ba58586e478b0ab81c877d2bd8ad
archive=$(mktemp)
trap 'rm -f "$archive"' EXIT
curl -fsSL --retry 3 --retry-all-errors \
  "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${version}/cargo-nextest-${version}-x86_64-unknown-linux-musl.tar.gz" \
  -o "$archive"
echo "${sha256}  ${archive}" | sha256sum --check
bin=${CARGO_HOME:-$HOME/.cargo}/bin
mkdir -p "$bin"
tar -xzf "$archive" -C "$bin" cargo-nextest
