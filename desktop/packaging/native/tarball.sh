#!/usr/bin/env bash
# The Linux tarball of the native app, as CI attaches it to a release.
#
#   tarball.sh [--binary <gelabber>] <libgelabber_media.so> [<out-dir>]
#
# Builds desktop/gelabber in release against the given media core (unless
# --binary names a built one), strips both, checks them as the Tauri app's
# package is checked (no shared C++ runtime, only gm_* exported, the core
# found next to the binary, no WebKitGTK, libsoup or GTK) and writes
# <out-dir>/gelabber-native-linux-x64.tar.gz (default: the current directory).
# Respects CARGO_TARGET_DIR and CARGO_BUILD_JOBS.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
name=gelabber-native-linux-x64

binary=""
if [ "${1:-}" = "--binary" ]; then
  binary="$(realpath "${2:?--binary needs a path}")"
  shift 2
fi
lib="$(realpath "${1:?usage: tarball.sh [--binary <gelabber>] <libgelabber_media.so> [<out-dir>]}")"
out="${2:-.}"
test -f "$lib" || { echo "no media core at $lib" >&2; exit 1; }
notices="$here/THIRD-PARTY-NOTICES.txt"
test -f "$notices" || {
  echo "$notices is missing: run desktop/packaging/third-party-notices.py --app native" >&2
  exit 1
}

if [ -z "$binary" ]; then
  # The core's directory: its build script links it from there.
  GELABBER_MEDIA_LIB_DIR="$(dirname "$lib")" \
    cargo build --release --locked --manifest-path "$repo/desktop/Cargo.toml" -p gelabber
  binary="${CARGO_TARGET_DIR:-$repo/desktop/target}/release/gelabber"
fi
test -x "$binary" || { echo "no binary at $binary" >&2; exit 1; }

mkdir -p "$out"
out="$(realpath "$out")"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
pkg="$stage/$name"
mkdir "$pkg"
strip --strip-unneeded -o "$pkg/gelabber" "$binary"
strip --strip-unneeded -o "$pkg/libgelabber_media.so" "$lib"
install -m644 "$here/README.txt" "$notices" "$here/gelabber.desktop" "$pkg/"
install -m644 "$repo/desktop/app/icons/icon.png" "$pkg/gelabber.png"

fail() {
  echo "tarball: $*" >&2
  exit 1
}
needed() {
  readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'
}

# Chromium's libc++ is linked into the core and hidden; neither file may
# bring the system's C++ runtime in.
for file in "$pkg/gelabber" "$pkg/libgelabber_media.so"; do
  if needed "$file" | grep -E '^lib(stdc|c)\+\+'; then
    fail "$(basename "$file") depends on a shared C++ runtime"
  fi
done
foreign="$(nm -D --defined-only "$pkg/libgelabber_media.so" | awk '{print $3}' | grep -v '^gm_' || true)"
[ -z "$foreign" ] || fail "libgelabber_media.so exports more than gm_*: $foreign"
if needed "$pkg/gelabber" | grep -E '^lib(webkit2gtk|javascriptcoregtk|soup|gtk|gdk)-'; then
  fail "gelabber depends on WebKitGTK, libsoup or GTK"
fi
readelf -d "$pkg/gelabber" | grep -E '\((RUNPATH|RPATH)\)' | grep -qF '$ORIGIN' ||
  fail "gelabber has no \$ORIGIN rpath and would not find libgelabber_media.so"
if needed "$pkg/gelabber" | grep -qx libgelabber_media.so; then
  found="$(env -u LD_LIBRARY_PATH ldd "$pkg/gelabber" | awk '$1 == "libgelabber_media.so" {print $3}')"
  [ "$found" = "$pkg/libgelabber_media.so" ] ||
    fail "gelabber loads libgelabber_media.so from ${found:-nowhere}, not from next to itself"
else
  # Until the call engine is linked in, nothing of the core is used.
  echo "note: gelabber does not load libgelabber_media.so (no gm_* call in the binary)"
fi

tarball="$out/$name.tar.gz"
tar --sort=name --owner=0 --group=0 --numeric-owner -C "$stage" -czf "$tarball" "$name"
tar -tvzf "$tarball"
echo "dynamic dependencies of gelabber: $(needed "$pkg/gelabber" | tr '\n' ' ')"
sha256sum "$tarball"
