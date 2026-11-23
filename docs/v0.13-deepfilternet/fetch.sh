#!/usr/bin/env bash
# Downloads every input of the GEL-28 measurement into work/ and checks each
# file against the SHA-256 recorded here. Sources are pinned by commit; a
# changed upstream file fails the check instead of changing the results.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
WORK="$HERE/work"
mkdir -p "$WORK/dl" "$WORK/src"

get() { # url sha256 destination
  if [ ! -f "$3" ] || ! printf '%s  %s\n' "$2" "$3" | sha256sum -c --status -; then
    curl --fail --location --silent --show-error -o "$3.part" "$1"
    mv "$3.part" "$3"
  fi
  printf '%s  %s\n' "$2" "$3" | sha256sum -c -
}

# DeepFilterNet, main at the commit that put the model weights under
# MIT/Apache-2.0 (the last release, v0.5.6, is from 2023).
DFN_COMMIT=7b14b895fe7fc42cab72b4a4c1e49c982bd6404b
get "https://codeload.github.com/Rikorose/DeepFilterNet/tar.gz/$DFN_COMMIT" \
  60ddfcd8a4b62b78e95c59707a9c9bd6df74f09d72db41abad8987c67f2a680a "$WORK/dl/deepfilternet.tar.gz"
if [ ! -d "$WORK/src/deepfilternet" ]; then
  mkdir "$WORK/src/deepfilternet"
  tar -xzf "$WORK/dl/deepfilternet.tar.gz" --strip-components=1 -C "$WORK/src/deepfilternet"
fi
# The model and the simulated room impulse response come from that tarball.
printf '%s  %s\n' \
  c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616 "$WORK/src/deepfilternet/models/DeepFilterNet3_onnx.tar.gz" \
  227db737f46bca264378e1c55c25f033ada513dbfbed928edf6783e0d13ba89d "$WORK/src/deepfilternet/assets/rir_sim_1001_w11.7_l2.6_h2.5_rt60_0.7919.wav" |
  sha256sum -c -

# RNNoise: the same source commit and model as web/scripts/audio/build-rnnoise.sh
# and desktop/native/CMakeLists.txt, for the native CPU benchmark.
get https://codeload.github.com/xiph/rnnoise/tar.gz/904a876dce1f9ab8860c0a5000ed151f9f6eef58 \
  975488a6b6ed404b176f9e7a72b418470e50b1e7596f875eb7799f5b785e3ebf "$WORK/dl/rnnoise.tar.gz"
get https://media.xiph.org/rnnoise/models/rnnoise_data-0b50c45.tar.gz \
  4ac81c5c0884ec4bd5907026aaae16209b7b76cd9d7f71af582094a2f98f4b43 "$WORK/dl/rnnoise_data.tar.gz"
if [ ! -d "$WORK/src/rnnoise" ]; then
  mkdir "$WORK/src/rnnoise"
  tar -xzf "$WORK/dl/rnnoise.tar.gz" --strip-components=1 -C "$WORK/src/rnnoise"
  tar -xzf "$WORK/dl/rnnoise_data.tar.gz" -C "$WORK/src/rnnoise"
  # Same compatibility shim as build-rnnoise.sh: v0.2's scalar vec.h includes
  # an absent Opus header and only needs OPUS_CLEAR.
  printf '#include <string.h>\n#define OPUS_CLEAR(dst, n) memset((dst), 0, (n)*sizeof(*(dst)))\n' \
    > "$WORK/src/rnnoise/src/os_support.h"
fi

# DNSMOS P.835 (Microsoft DNS Challenge, CC BY 4.0): the standard model and
# the personalized one, which also counts an interfering talker as noise.
DNS=https://raw.githubusercontent.com/microsoft/DNS-Challenge/785ab12275d5cf9341c5e946b0eee9f8825ece3b/DNSMOS
mkdir -p "$WORK/dnsmos"
get "$DNS/DNSMOS/sig_bak_ovr.onnx" \
  269fbebdb513aa23cddfbb593542ecc540284a91849ac50516870e1ac78f6edd "$WORK/dnsmos/sig_bak_ovr.onnx"
get "$DNS/pDNSMOS/sig_bak_ovr.onnx" \
  9e3a197449ca2177f0997afec3bd6b890117ce2f17b89d6eea7fa0d47272c81c "$WORK/dnsmos/p_sig_bak_ovr.onnx"
# Reference implementation, kept for comparing dnsmos.mjs against it.
get "$DNS/dnsmos_local.py" \
  1ab566afe006daab32ac7073296a5d0ef99f8b82f91c7266f3ccf26113d7a28b "$WORK/dnsmos/dnsmos_local.py"

# MS-SNSD test clips (repository MIT; speech from PTDB-TUG/Edinburgh, noise
# from Freesound CC0). 16 kHz mono.
SNSD=https://raw.githubusercontent.com/microsoft/MS-SNSD/fe61c4ba0d9ac8dd7e23d719cc79f8947e1dc742
mkdir -p "$WORK/snsd"
while read -r sha path; do
  get "$SNSD/$path" "$sha" "$WORK/snsd/$(basename "$path")"
done <<'EOF'
be5c2c6578f085ac3e95bd42fb0cbda8f6f2bbda0d03bb5b19fbe60d67e079f7 clean_test/clnsp0.wav
1e82b4cc442db91c9f3e4eff2cd06b2bf7def7078cf90cf02b1a0eb6eadb3b4f clean_test/clnsp2.wav
a6eb64b70f1b55c768fa6fc133154ff1b9ee73c1add8e69f32bd7bbdd3c582ed clean_test/clnsp6.wav
264a2511c10a395c6b60b7dbb5e9d30c7412b28f1976f09df6c703e7c4fa3dd8 clean_test/clnsp8.wav
0e5c70ee4491c6739dffe5e4c3baef5eeb2f9ba96457275bcd196367f0a319b5 clean_test/clnsp12.wav
0ce20201462a78f8db3cf960d93ebc57c9f9203422814ce989aa09b7099abd17 clean_test/clnsp13.wav
47980660888977fb45e5cdfd4a14d6585139e90267c0e61a01eeadb4a5891222 noise_test/Typing_1.wav
3fe55d7035d7ef2599841738f818dc4463f5ef983eb4b5fefeee153d8caa2110 noise_test/AirConditioner_1.wav
EOF
