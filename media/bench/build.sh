#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
npm ci
npm run build
cargo build --locked --release --manifest-path current-probe/Cargo.toml
cargo build --locked --release --manifest-path mediasoup-probe/Cargo.toml
docker build --tag gelabber-bench/janus:v1.4.2 janus
python3 -m unittest discover -s tests -v
node --test tests/*.test.mjs
node --check browser.mjs
node --check loadgen.mjs
