#!/usr/bin/env bash
# The desktop app's version, the first `version = "…"` line of desktop/Cargo.toml
# ([workspace.package]), on stdout:
#
#   version.sh [--tag <release tag>] [<Cargo.toml>]
#
# With a non-empty tag, it fails unless the tag is v<version>. Carriage
# returns (a Windows checkout) are ignored.
set -euo pipefail
tag=""
if [ "${1:-}" = "--tag" ]; then
  tag="${2?--tag needs a value}"
  shift 2
fi
manifest="${1:-$(cd "$(dirname "$0")/../../.." && pwd)/desktop/Cargo.toml}"
version="$(sed -n 's/^version = "\(.*\)"[[:space:]]*$/\1/p' "$manifest" | head -n1)"
error() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then echo "::error::$*" >&2; else echo "$*" >&2; fi
  exit 1
}
[ -n "$version" ] || error "$manifest has no version line"
if [ -n "$tag" ] && [ "v$version" != "$tag" ]; then
  error "release $tag: desktop/Cargo.toml, and with it the app, says $version; the version there (and in desktop/Cargo.lock) comes first"
fi
echo "$version"
