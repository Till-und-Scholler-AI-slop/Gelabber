#!/bin/sh
# The release a server upgrades from in one step (GEL-364: the newest tag of
# the previous minor version) and the highest migration it shipped, printed
# as "TAG NUMBER". The upgrade test starts from that schema (GEL-413).
# --fetch first fetches that minor's tags (CI checks out without tags).
set -eu
root=$(git rev-parse --show-toplevel)
version=$(sed -n 's/^version = "\([0-9]*\)\.\([0-9]*\)\..*"/\1 \2/p' "$root/backend/Cargo.toml" | head -1)
major=${version% *}
minor=${version#* }
previous="$major.$((minor - 1))"
if [ "${1:-}" = --fetch ]; then
  git -C "$root" fetch --quiet --depth=1 --no-tags origin "+refs/tags/v$previous.*:refs/tags/v$previous.*"
fi
tag=$(git -C "$root" tag --list "v$previous.*" --sort=-version:refname | head -1)
if [ -z "$tag" ]; then
  echo "no tag v$previous.* in this checkout; fetch it, or run with --fetch" >&2
  exit 1
fi
last=$(git -C "$root" ls-tree --name-only "$tag" backend/api/migrations/ \
  | sed -n 's#.*/0*\([0-9][0-9]*\)_[^/]*\.sql$#\1#p' | sort -n | tail -1)
if [ -z "$last" ]; then
  echo "$tag has no migrations" >&2
  exit 1
fi
echo "$tag $last"
