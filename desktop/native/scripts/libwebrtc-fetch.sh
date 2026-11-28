#!/usr/bin/env bash
# Fetches a stored Linux libwebrtc package, an asset of the libwebrtc-packages
# release, into a package directory, as a step output:
#
#   libwebrtc-fetch.sh [--or-build] <key> <package dir>    prints found=true or found=false
#
# Anyone with write access can replace an asset, so a package is taken only
# with the SHA-256 that desktop/native/libwebrtc-packages.sha256 lists for
# <key>.tar.zst. One that differs, or that does not unpack into a package,
# fails. A package that is not listed or not in the release fails too, unless
# --or-build: then found=false, and desktop-native.yml builds it.
# Needs gh and GITHUB_REPOSITORY (GH_TOKEN in CI).
set -euo pipefail
or_build=false
if [ "${1:-}" = --or-build ]; then
  or_build=true
  shift
fi
usage="usage: libwebrtc-fetch.sh [--or-build] <key> <package dir>"
key="${1:?$usage}"
package="${2:?$usage}"
: "${GITHUB_REPOSITORY:?the repository whose libwebrtc-packages release holds the package}"
asset="$key.tar.zst"
pins="$(cd "$(dirname "$0")/.." && pwd)/libwebrtc-packages.sha256"

unavailable() {
  if [ "$or_build" = true ]; then
    echo "$1, so the package is built." >&2
    echo "found=false"
    exit 0
  fi
  echo "$1: desktop-native.yml builds it, libwebrtc-store.yml stores it after a push to a version branch." >&2
  exit 1
}

pin=$(awk -v asset="$asset" '$2 == asset { print; exit }' "$pins")
if [ -z "$pin" ]; then
  unavailable "desktop/native/libwebrtc-packages.sha256 lists no $asset"
fi
mkdir -p "$package"
fetch=$(mktemp -d "$package/.fetch.XXXXXX")
trap 'rm -rf "$fetch"' EXIT
if ! gh release download libwebrtc-packages --repo "$GITHUB_REPOSITORY" --pattern "$asset" --dir "$fetch"; then
  unavailable "The libwebrtc-packages release holds no $asset"
fi
if ! (cd "$fetch" && printf '%s\n' "$pin" | sha256sum --check --strict >&2); then
  echo "The stored $asset is not the one desktop/native/libwebrtc-packages.sha256 lists." >&2
  exit 1
fi
tar --zstd -xf "$fetch/$asset" -C "$package"
if [ ! -f "$package/lib/libwebrtc.a" ]; then
  echo "The stored $asset holds no lib/libwebrtc.a." >&2
  exit 1
fi
echo "found=true"
