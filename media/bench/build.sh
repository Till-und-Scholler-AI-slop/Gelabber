#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
npm ci
npm run build
cargo build --locked --release --manifest-path current-probe/Cargo.toml
cargo build --locked --release --manifest-path mediasoup-probe/Cargo.toml
cargo build --locked --release --manifest-path rtp-source/Cargo.toml
docker build --tag gelabber-bench/janus:v1.4.2 janus
python3 -m unittest discover -s tests -v
node --test tests/*.test.mjs
node --check browser.mjs
node --check loadgen.mjs
node --check local-fixed-video.mjs
node --check native-video.mjs
node --check native-video-adapters.mjs
node --check native-video-browser.mjs
node --check native-video-pilot.mjs
node --check mediasoup-native-sdp.mjs
node --check janus-events.mjs
node --check janus-broker.mjs
node --check local-janus-events.mjs
