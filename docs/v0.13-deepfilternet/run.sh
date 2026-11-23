#!/usr/bin/env bash
# GEL-28: the whole measurement, from pinned downloads to results/.
# Needs curl, Node >= 22, Rust (cargo) and a C compiler. With
# GELABBER_LIBWEBRTC_DIR set, native RNNoise is built with the libwebrtc
# package's clang and sysroot, the compiler of the desktop app.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
cd "$HERE"
WORK="$HERE/work"
RESULTS="$HERE/results"
mkdir -p "$WORK/bin" "$WORK/out/rnnoise" "$WORK/out/rnnoise-native" "$WORK/out/dfn3" "$WORK/out/dfn3-lib" "$RESULTS"

./fetch.sh
npm ci --ignore-scripts --no-audit --no-fund

TARGET_DIR=${CARGO_TARGET_DIR:-$HERE/dfn-bench/target}
cargo build --release --locked --manifest-path dfn-bench/Cargo.toml --target-dir "$TARGET_DIR"
# The WebAssembly build needs `rustup target add wasm32-unknown-unknown`.
DFN_WASM=
if rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
  RUSTFLAGS="-C target-feature=+simd128" cargo build --release --locked --manifest-path dfn-wasm/Cargo.toml \
    --target wasm32-unknown-unknown --target-dir "$TARGET_DIR"
  DFN_WASM="$TARGET_DIR/wasm32-unknown-unknown/release/dfn_wasm.wasm"
else
  echo "wasm32-unknown-unknown not installed: skipping the DeepFilterNet3 WebAssembly timing" >&2
fi

R="$WORK/src/rnnoise"
RN_SRC=("$R"/src/{denoise,rnn,pitch,kiss_fft,celt_lpc,nnet,nnet_default,parse_lpcnet_weights,rnnoise_data,rnnoise_tables}.c)
# -O3 -DNDEBUG: CMake's Release flags, as desktop/core/build.rs builds it.
if [ -n "${GELABBER_LIBWEBRTC_DIR:-}" ]; then
  P="$GELABBER_LIBWEBRTC_DIR"
  CC_CMD=("$P/toolchain/bin/clang" --target=x86_64-linux-gnu "--sysroot=$P/sysroot" -fuse-ld=lld)
else
  CC_CMD=("${CC:-cc}")
fi
"${CC_CMD[@]}" -O3 -DNDEBUG -w -I"$R/include" -I"$R/src" rnnoise-native.c "${RN_SRC[@]}" -lm -o "$WORK/bin/rnnoise-native"

node mix.mjs
CLIPS=()
while read -r id; do CLIPS+=("$WORK/clips/$id.wav"); done < <(node -e \
  'for (const c of JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))) console.log(c.id)' \
  "$WORK/clips/manifest.json")

{
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "cpu: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
  echo "logical cpus: $(nproc)"
  echo "hypervisor: $(grep -qw hypervisor /proc/cpuinfo && echo yes || echo no)"
  echo "kernel: $(uname -sr)"
  echo "node: $(node --version)"
  echo "rustc: $(rustc --version)"
  echo "load average before engines: $(cut -d' ' -f1-3 /proc/loadavg)"
} > "$RESULTS/machine.txt"

# One engine at a time, each single-threaded.
node rnnoise-wasm.mjs > "$RESULTS/cpu-rnnoise-wasm.json"
"$WORK/bin/rnnoise-native" "$WORK/out/rnnoise-native" "${CLIPS[@]}" > "$RESULTS/cpu-rnnoise-native.json"
MODEL="$WORK/src/deepfilternet/models/DeepFilterNet3_onnx.tar.gz"
# Two DeepFilterNet3 settings that differ only in when libDF skips model
# stages: the deep-filter command line's thresholds (-15/35/35 dB local SNR),
# which ran the full model on every clip here, and the library defaults
# (-10/30/20 dB) that the LADSPA plugin and the C API start with.
DFN_PARAMS=100,-15,35,35 "$TARGET_DIR/release/dfn-bench" "$MODEL" "$WORK/out/dfn3" "${CLIPS[@]}" \
  > "$RESULTS/cpu-dfn3.json"
"$TARGET_DIR/release/dfn-bench" "$MODEL" "$WORK/out/dfn3-lib" "${CLIPS[@]}" > "$RESULTS/cpu-dfn3-lib.json"
if [ -n "$DFN_WASM" ]; then
  node dfn-wasm.mjs "$DFN_WASM" "$MODEL" > "$RESULTS/cpu-dfn3-wasm.json"
fi
echo "load average after engines: $(cut -d' ' -f1-3 /proc/loadavg)" >> "$RESULTS/machine.txt"

node score.mjs > /dev/null
echo "results in $RESULTS"
