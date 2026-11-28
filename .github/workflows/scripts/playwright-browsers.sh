#!/usr/bin/env bash
# Installs Playwright's browsers for web/ and their system packages, the
# packages through apt-install.sh, which reads only Ubuntu's sources;
# --with-deps would read every source (GEL-404):
#
#   playwright-browsers.sh <browser>...    after npm ci in web/
set -euo pipefail
[ "$#" -gt 0 ] || { echo "usage: playwright-browsers.sh <browser>..." >&2; exit 2; }
root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$root/web"
# The dry run lists the missing packages indented by two spaces, one per
# line, and then exits 1; any other failure stops here.
status=0
listing=$(npx playwright install-deps --dry-run "$@" 2>&1) || status=$?
if [ "$status" -ne 0 ] && ! grep -q '^Missing system dependencies' <<<"$listing"; then
  printf '%s\n' "$listing" >&2
  exit "$status"
fi
mapfile -t deps < <(printf '%s\n' "$listing" | sed -n 's/^  \([^ ]\+\)$/\1/p')
echo "System packages for $*: ${deps[*]:-none}"
if [ "${#deps[@]}" -gt 0 ]; then
  "$root/.github/workflows/scripts/apt-install.sh" "${deps[@]}"
fi
# Again after the install: it exits 1 while a package is still missing.
npx playwright install-deps --dry-run "$@"
npx playwright install "$@"
