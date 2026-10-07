#!/usr/bin/env bash
# Build the pinned libwebrtc for Linux x86_64 and package it for the desktop
# media core.
#
#   desktop/native/scripts/build-libwebrtc-linux.sh <work-dir> <package-dir>
#
# Output layout (<package-dir>):
#   lib/libwebrtc.a        all libwebrtc objects plus Chromium's libc++/libc++abi
#   include/               libwebrtc headers (incl. third_party/abseil-cpp, libc++)
#   toolchain/             Chromium clang/lld used to build libwebrtc
#   sysroot/               Debian sysroot libwebrtc was built against
#   VERSIONS, args.gn      provenance
#
# libwebrtc is built with Chromium's own clang, sysroot and libc++ (the
# supported configuration). Everything that links it must use the same libc++,
# which is why the media core is a separate shared library exporting only a C
# ABI (see desktop/native/CMakeLists.txt).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../libwebrtc.env
source "$here/../libwebrtc.env"

work="$(realpath -m "${1:?work dir}")"
package="$(realpath -m "${2:?package dir}")"
mkdir -p "$work"
cd "$work"

if [ ! -d depot_tools ]; then
  git clone --depth 1 https://chromium.googlesource.com/chromium/tools/depot_tools.git
fi
export PATH="$work/depot_tools:$PATH"
# First run bootstraps depot_tools (CIPD python, vpython); runhooks fails without it.
gclient >/dev/null

cat > .gclient <<EOF
solutions = [{
  "name": "src",
  "url": "https://webrtc.googlesource.com/src.git@${WEBRTC_COMMIT}",
  "deps_file": "DEPS",
  "managed": False,
  "custom_deps": {},
}]
target_os = ["linux"]
EOF

gclient sync --no-history --shallow --nohooks -r "src@${WEBRTC_COMMIT}" -j 8
gclient runhooks

src="$work/src"
cd "$src"
test "$(git rev-parse HEAD)" = "$WEBRTC_COMMIT"

for patch in "$here"/../patches/*.patch; do
  [ -e "$patch" ] || continue
  if git apply --check "$patch" 2>/dev/null; then
    git apply "$patch"
  else
    # A rerun on a cached checkout already carries the patch.
    git apply --check --reverse "$patch"
  fi
done

out="out/gelabber"
gn_args=(
  'target_os="linux"'
  'target_cpu="x64"'
  is_debug=false
  is_component_build=false
  symbol_level=0
  treat_warnings_as_errors=false
  rtc_include_tests=false
  rtc_build_examples=false
  rtc_build_tools=false
  rtc_enable_protobuf=false
  rtc_use_perfetto=false
  use_rtti=true
  # Screen capture on Wayland goes through xdg-desktop-portal + PipeWire.
  # PipeWire is loaded at runtime (no link-time dependency).
  rtc_use_pipewire=true
  rtc_link_pipewire=false
  # Omarchy/Hyprland is Wayland-only; no X11 capture or link dependency.
  rtc_use_x11=false
  # Software H264 fallback (OpenH264 encoder and decoder). Hardware
  # encoders come from the core's own encoder factory.
  rtc_use_h264=true
  enable_rust=false
  enable_rust_cxx=false
  enable_chromium_prelude=false
  rtc_rusty_base64=false
  use_debug_fission=false
)
gn gen "$out" --args="${gn_args[*]}"
ninja -C "$out" :default buildtools/third_party/libc++ buildtools/third_party/libc++abi

# Empty the package dir in place: CI pre-creates it under a root-owned /mnt.
mkdir -p "$package"
find "$package" -mindepth 1 -delete
mkdir -p "$package/lib" "$package/include" "$package/toolchain/bin" "$package/toolchain/lib"

# One archive with every object, like the upstream packagers do: libwebrtc's
# own complete static library does not carry desktop_capture or libc++.
llvm_ar="$src/third_party/llvm-build/Release+Asserts/bin/llvm-ar"
# Quick-append (q), not replace (r): many objects share a basename (utils.o,
# ...) and r would keep only the last one.
(cd "$out/obj" && find . -name '*.o' -print0 | sort -z | xargs -0 "$llvm_ar" qc "$package/lib/libwebrtc.a")
"$llvm_ar" s "$package/lib/libwebrtc.a"

rsync -a --prune-empty-dirs --exclude='out/' --include='*/' \
  --include='*.h' --include='*.hpp' --include='*.inc' --include='*.def' \
  --exclude='*' ./ "$package/include/"
# libc++ configuration headers that are not *.h.
rsync -a ./third_party/libc++/src/include/ "$package/include/third_party/libc++/src/include/"
rsync -a ./buildtools/third_party/libc++/ "$package/include/buildtools/third_party/libc++/"
cp "$out/args.gn" "$package/args.gn"

llvm="$src/third_party/llvm-build/Release+Asserts"
cp -a "$llvm/bin/clang" "$llvm/bin/clang++" "$llvm/bin/lld" "$llvm/bin/ld.lld" \
  "$llvm/bin/llvm-ar" "$llvm/bin/llvm-nm" "$llvm/bin/llvm-strip" "$package/toolchain/bin/" 2>/dev/null || true
for tool in clang clang++ ld.lld; do test -e "$package/toolchain/bin/$tool"; done
cp -a "$llvm/lib/clang" "$package/toolchain/lib/"

sysroot="$(find build/linux -maxdepth 1 -type d -name 'debian_*amd64-sysroot' | head -n1)"
test -n "$sysroot"
rsync -a "$sysroot/" "$package/sysroot/"

{
  echo "WEBRTC_BRANCH=$WEBRTC_BRANCH"
  echo "WEBRTC_COMMIT=$WEBRTC_COMMIT"
  echo "LIBWEBRTC_PACKAGE_REVISION=$LIBWEBRTC_PACKAGE_REVISION"
  echo "CLANG_REVISION=$("$llvm/bin/clang" --version | head -n1)"
  echo "SYSROOT=$(basename "$sysroot")"
} > "$package/VERSIONS"

du -sh "$package/lib/libwebrtc.a" "$package/include" "$package/toolchain" "$package/sysroot"
