#!/usr/bin/env bash
set -euo pipefail
# No mutable tags: source, upstream model, compiler image and result are pinned.
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BUILD=$(mktemp -d)
trap 'rm -rf "$BUILD"' EXIT
curl --fail --location --silent --show-error -o "$BUILD/source.tar.gz" https://codeload.github.com/xiph/rnnoise/tar.gz/904a876dce1f9ab8860c0a5000ed151f9f6eef58
printf '%s  %s\n' 975488a6b6ed404b176f9e7a72b418470e50b1e7596f875eb7799f5b785e3ebf "$BUILD/source.tar.gz" | sha256sum -c -
mkdir "$BUILD/source"
tar -xzf "$BUILD/source.tar.gz" --strip-components=1 -C "$BUILD/source"
curl --fail --location --silent --show-error -o "$BUILD/model.tar.gz" https://media.xiph.org/rnnoise/models/rnnoise_data-0b50c45.tar.gz
printf '%s  %s\n' 4ac81c5c0884ec4bd5907026aaae16209b7b76cd9d7f71af582094a2f98f4b43 "$BUILD/model.tar.gz" | sha256sum -c -
tar -xzf "$BUILD/model.tar.gz" -C "$BUILD/source"
# v0.2 scalar vec.h references this absent Opus header; it only needs OPUS_CLEAR.
# This compatibility macro does not change the model or algorithm.
printf '#include <string.h>\n#define OPUS_CLEAR(dst, n) memset((dst), 0, (n)*sizeof(*(dst)))\n' > "$BUILD/source/src/os_support.h"
docker run --rm --user "$(id -u):$(id -g)" -e EM_CACHE=/tmp/em-cache -v "$BUILD/source:/src" -w /src emscripten/emsdk@sha256:af45409f3199d88db4b1b03af0098532c8fb33a375ac257463eeb0a622870d06 \
  emcc -O3 -g1 -msimd128 -Iinclude -Isrc src/denoise.c src/rnn.c src/pitch.c src/kiss_fft.c src/celt_lpc.c src/nnet.c src/nnet_default.c src/parse_lpcnet_weights.c src/rnnoise_data.c src/rnnoise_tables.c \
  -s STANDALONE_WASM=1 -s STACK_SIZE=1048576 -s INITIAL_MEMORY=16777216 -s ALLOW_MEMORY_GROWTH=0 \
  -s EXPORTED_FUNCTIONS='["_rnnoise_create","_rnnoise_destroy","_rnnoise_process_frame","_rnnoise_get_frame_size","_malloc","_free"]' --no-entry -o rnnoise.wasm
printf '%s  %s\n' e66d0eaef35d3774e86377efa8b9897e5b226284fb53059f0c1444881888b71c "$BUILD/source/rnnoise.wasm" | sha256sum -c -
install -m644 "$BUILD/source/rnnoise.wasm" "$ROOT/public/audio/rnnoise.wasm"
install -m644 "$BUILD/source/COPYING" "$ROOT/public/audio/RNNOISE-LICENSE.txt"
