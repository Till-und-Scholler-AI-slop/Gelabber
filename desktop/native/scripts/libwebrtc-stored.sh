#!/usr/bin/env bash
# Whether the libwebrtc-packages release holds a package, as a step output:
#
#   libwebrtc-stored.sh <asset name>    prints missing=true or missing=false
#
# missing=true makes desktop-native.yml hand the package to libwebrtc-store.yml.
# Without the release (or gh) nothing is stored: a warning and missing=false.
set -euo pipefail
asset="${1:?usage: libwebrtc-stored.sh <asset name>}"
if ! names=$(gh release view libwebrtc-packages --repo "$GITHUB_REPOSITORY" --json assets --jq '.assets[].name'); then
  echo "::warning::The libwebrtc-packages release is missing, so the package is not stored." >&2
  echo "missing=false"
elif printf '%s\n' "$names" | grep -qxF "$asset"; then
  echo "missing=false"
else
  echo "missing=true"
fi
