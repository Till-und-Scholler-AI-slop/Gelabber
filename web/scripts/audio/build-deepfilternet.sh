#!/usr/bin/env bash
# DeepFilterNet3 for "Enhanced voice: Strong" (GEL-28): builds
# public/audio/deepfilternet.wasm and installs the model and the licences.
# Source and model are pinned by SHA-256, the crates by Cargo.lock, the
# compiler by version; the result must match the hash below. Needs curl, Node,
# rustup with Rust 1.100.0 and its wasm32-unknown-unknown target. No Docker.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERE="$ROOT/scripts/audio"
TOOLCHAIN=1.100.0
# Main at the commit that put the model weights under MIT/Apache-2.0, as
# measured in docs/v0.13-deepfilternet (its fetch.sh has the same pins).
DFN_COMMIT=7b14b895fe7fc42cab72b4a4c1e49c982bd6404b
DFN_ARCHIVE_SHA256=60ddfcd8a4b62b78e95c59707a9c9bd6df74f09d72db41abad8987c67f2a680a
MODEL_SHA256=c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616
WASM_SHA256=18929d4fed09a64173cd26e958a712ac6466d821b3e327035d8eaa7d2e5fbb33

# A fixed directory, not mktemp: two builds from different directories gave
# different modules, so the build directory is part of the pin.
BUILD=/tmp/gelabber-deepfilternet-build
rm -rf "$BUILD"
mkdir "$BUILD"
trap 'rm -rf "$BUILD"' EXIT
curl --fail --location --silent --show-error -o "$BUILD/source.tar.gz" \
  "https://codeload.github.com/Rikorose/DeepFilterNet/tar.gz/$DFN_COMMIT"
printf '%s  %s\n' "$DFN_ARCHIVE_SHA256" "$BUILD/source.tar.gz" | sha256sum -c -
mkdir "$BUILD/deepfilternet"
tar -xzf "$BUILD/source.tar.gz" --strip-components=1 -C "$BUILD/deepfilternet"
MODEL="$BUILD/deepfilternet/models/DeepFilterNet3_onnx.tar.gz"
printf '%s  %s\n' "$MODEL_SHA256" "$MODEL" | sha256sum -c -

# The crate sits next to the source, where its Cargo.toml expects libDF.
mkdir "$BUILD/crate"
cp -r "$HERE/deepfilternet/Cargo.toml" "$HERE/deepfilternet/Cargo.lock" "$HERE/deepfilternet/src" "$BUILD/crate/"
# A fresh CARGO_HOME: no user cargo config, and every path the compiler sees
# lies under $BUILD, which the remap turns into /build.
export CARGO_HOME="$BUILD/cargo" RUSTUP_TOOLCHAIN="$TOOLCHAIN"
export RUSTFLAGS="-C target-feature=+simd128 --remap-path-prefix=$BUILD=/build"
cargo "+$TOOLCHAIN" build --release --locked --target wasm32-unknown-unknown \
  --manifest-path "$BUILD/crate/Cargo.toml" --target-dir "$BUILD/target"
WASM="$BUILD/target/wasm32-unknown-unknown/release/gelabber_deepfilternet.wasm"
sha256sum "$WASM"
printf '%s  %s\n' "$WASM_SHA256" "$WASM" | sha256sum -c -

cargo "+$TOOLCHAIN" metadata --format-version 1 --locked --filter-platform wasm32-unknown-unknown \
  --manifest-path "$BUILD/crate/Cargo.toml" > "$BUILD/metadata.json"
cat > "$BUILD/header.txt" <<EOF
Third-party licences for the "Enhanced voice: Strong" noise filter
(GEL-28). Provenance: DEEPFILTERNET-PROVENANCE.json.

deepfilternet3-model.bin is the DeepFilterNet3 model (models/DeepFilterNet3_onnx.tar.gz)
from DeepFilterNet, https://github.com/Rikorose/DeepFilterNet, commit
$DFN_COMMIT, unchanged. At that commit the model weights are licensed
like the code: MIT OR Apache-2.0 (both texts below, under deep_filter).

deepfilternet.wasm is built by web/scripts/audio/build-deepfilternet.sh from
libDF (crate deep_filter) at the same commit, tract and the crates listed
below, and Gelabber's own glue (MIT, see the repository's LICENSE).
EOF
node "$HERE/deepfilternet-licenses.mjs" "$BUILD/metadata.json" "$BUILD/LICENSE.txt" "$BUILD/header.txt"

install -m644 "$WASM" "$ROOT/public/audio/deepfilternet.wasm"
# Not .tar.gz: Vite's dev server sends a .gz file with Content-Encoding:
# gzip, the browser unpacks it, and the hash check fails.
install -m644 "$MODEL" "$ROOT/public/audio/deepfilternet3-model.bin"
install -m644 "$BUILD/LICENSE.txt" "$ROOT/public/audio/DEEPFILTERNET-LICENSE.txt"
