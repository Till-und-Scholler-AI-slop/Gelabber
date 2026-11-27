#!/bin/sh
# The release a server upgrades from in one step (GEL-364: the newest tag of
# the previous minor version) and the highest migration it shipped, printed
# as "TAG NUMBER". The upgrade test starts from that schema (GEL-413).
#
#   previous_release.sh [--fetch]
#     The version line comes from the refs in BASE_REF and REF_NAME, as
#     GitHub names them (github.base_ref, github.ref_name); without them,
#     from the checked-out branch:
#     - a version branch vX.Y as BASE_REF or REF_NAME, or the one of a merge
#       queue (gh-readonly-queue/vX.Y/...), is the line;
#     - main, release/* and hotfix/* take the workspace version in
#       backend/Cargo.toml (the release branch has bumped it);
#     - any other branch belongs to the development line, the newest vX.Y
#       branch on origin: v0.15 is developed while Cargo still says 0.14.x.
#     --fetch first fetches the previous minor's tags (CI checks out none).
#     A line whose previous minor has no release yet fails here, on
#     purpose: skipping would also skip --check, and servers still run that
#     minor's predecessor. Cut a version branch after the release before it.
#
#   previous_release.sh --own [--fetch]
#     The newest release of the line itself, as "TAG NUMBER", or nothing
#     when the line has none yet. Servers run it too, so CI checks the
#     migrations against it as well (GEL-444).
#
#   previous_release.sh --check TAG NUMBER
#     Fails when a migration TAG shipped was edited, deleted or renamed
#     (servers refuse to start on a checksum mismatch), or when one numbered
#     at or below NUMBER was added after it (a server would run it after its
#     rows exist, out of order).
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd -P)
migrations=backend/api/migrations

version_line() {
  for ref in "${BASE_REF:-}" "${REF_NAME:-}"; do
    case "$ref" in
      v[0-9]*.[0-9]*)
        if echo "$ref" | grep -Eq '^v[0-9]+\.[0-9]+$'; then echo "$ref"; return; fi ;;
      gh-readonly-queue/v*)
        queued=$(echo "$ref" | sed -n 's#^gh-readonly-queue/\(v[0-9][0-9]*\.[0-9][0-9]*\)/.*#\1#p')
        if [ -n "$queued" ]; then echo "$queued"; return; fi ;;
    esac
  done
  ref=${REF_NAME:-}
  if [ -z "${BASE_REF:-}" ] && [ -z "$ref" ]; then
    ref=$(git -C "$root" rev-parse --abbrev-ref HEAD)
  fi
  case "${BASE_REF:-}:$ref" in
    main:* | :main | *:release/* | *:hotfix/* | release/*:* | hotfix/*:*)
      sed -n 's/^version = "\([0-9]*\.[0-9]*\)\..*"/v\1/p' "$root/backend/Cargo.toml" | head -1
      return ;;
  esac
  # The development line: the newest version branch on origin.
  { git -C "$root" ls-remote --heads origin 'v*' 2>/dev/null \
      || git -C "$root" for-each-ref --format='refs/heads/%(refname:lstrip=3)' 'refs/remotes/origin/v*'; } \
    | sed -n 's#.*refs/heads/\(v[0-9][0-9]*\.[0-9][0-9]*\)$#\1#p' | sort -t. -k1,1 -k2,2n | tail -1
}

check() {
  tag=$1
  last=$2
  changed=$(git -C "$root" diff --name-only --diff-filter=DMR "$tag" HEAD -- "$migrations/")
  if [ -n "$changed" ]; then
    echo "released migrations changed since $tag:" $changed >&2
    exit 1
  fi
  inserted=$(git -C "$root" diff --name-only --diff-filter=A "$tag" HEAD -- "$migrations/" \
    | sed -n 's#.*/\(0*\)\([0-9][0-9]*\)_[^/]*\.sql$#\2 &#p' \
    | while read -r number file; do if [ "$number" -le "$last" ]; then echo "$file"; fi; done)
  if [ -n "$inserted" ]; then
    echo "migrations added at or below $tag's $last, which a server would run out of order:" $inserted >&2
    exit 1
  fi
}

own=false
fetch=false
case "${1:-}" in
  --check)
    check "$2" "$3"
    exit 0 ;;
  --own) own=true; [ "${2:-}" = --fetch ] && fetch=true ;;
  --fetch) fetch=true ;;
  '') ;;
  *) echo "unknown argument: $1" >&2; exit 2 ;;
esac

# The newest tag vSERIES.*, fetched first with --fetch; empty without one.
newest_tag() {
  if [ "$fetch" = true ]; then
    if [ -z "$(git -C "$root" ls-remote --tags origin "refs/tags/v$1.*")" ]; then
      return 0
    fi
    git -C "$root" fetch --quiet --depth=1 --no-tags origin "+refs/tags/v$1.*:refs/tags/v$1.*"
  fi
  git -C "$root" tag --list "v$1.*" --sort=-version:refname | head -1
}

# "TAG NUMBER": the tag and the highest migration it shipped.
boundary() {
  last=$(git -C "$root" ls-tree --name-only "$1" "$migrations/" \
    | sed -n 's#.*/0*\([0-9][0-9]*\)_[^/]*\.sql$#\1#p' | sort -n | tail -1)
  if [ -z "$last" ]; then
    echo "$1 has no migrations" >&2
    exit 1
  fi
  echo "$1 $last"
}

line=$(version_line)
major=$(echo "$line" | sed -n 's/^v\([0-9]*\)\.[0-9]*$/\1/p')
minor=$(echo "$line" | sed -n 's/^v[0-9]*\.\([0-9]*\)$/\1/p')
if [ -z "$major" ] || [ -z "$minor" ]; then
  echo "no version line found (BASE_REF='${BASE_REF:-}', REF_NAME='${REF_NAME:-}')" >&2
  exit 1
fi
if [ "$own" = true ]; then
  tag=$(newest_tag "$major.$minor")
  if [ -n "$tag" ]; then boundary "$tag"; fi
  exit 0
fi
previous="$major.$((minor - 1))"
tag=$(newest_tag "$previous")
if [ -z "$tag" ] && [ "$fetch" = true ]; then
  echo "$line follows v$previous, which has no release yet (no tag v$previous.* on origin)" >&2
  exit 1
fi
if [ -z "$tag" ]; then
  echo "no tag v$previous.* in this checkout; fetch the tags or run with --fetch" >&2
  exit 1
fi
boundary "$tag"
