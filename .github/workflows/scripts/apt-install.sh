#!/usr/bin/env bash
# Installs Ubuntu packages (without recommends) on a GitHub-hosted runner.
#
# The runner's first mirror, azure.archive.ubuntu.com, sometimes serves
# packages at 30-80 kB/s for a whole job, while Canonical's archive delivers
# them in seconds. apt moves on to the next mirror in its list only after an
# error, never because a download is slow. So every fetch here has a time
# limit; when one runs out, the next mirror goes first and the fetch resumes,
# keeping the files it already has.
#
# Usage: apt-install.sh PACKAGE...
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 PACKAGE..." >&2
  exit 2
fi

# The runner's own list (Azure first); then Canonical's archive, then
# kernel.org's mirror. Fetch limits per attempt in seconds: a healthy mirror
# needs a fraction (the desktop job's 90 MB took 7 s), and all attempts with
# their list updates stay within the callers' 15 minute step limit.
mirror_list=/etc/apt/apt-mirrors.txt
fallbacks=(https://archive.ubuntu.com/ubuntu/ https://mirrors.edge.kernel.org/ubuntu/)
limits=(90 240 240)
update_limit=60
options=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30
         -o DPkg::Lock::Timeout=120)
# The package lists are fetched once per job and again after a mirror switch.
updated="${RUNNER_TEMP:-/tmp}/apt-install-updated"

apt_get() {
  sudo env DEBIAN_FRONTEND=noninteractive apt-get "${options[@]}" "$@"
}

# apt_get with a time limit in seconds.
apt_get_within() {
  local limit=$1
  shift
  sudo env DEBIAN_FRONTEND=noninteractive timeout --kill-after=10 "$limit" \
    apt-get "${options[@]}" "$@"
}

# Puts the mirror first in the runner's list; the others stay behind it as
# apt's own fallback on errors.
use_mirror() {
  local mirror=$1
  if [ ! -f "$mirror_list" ]; then
    echo "apt-install: no $mirror_list, keeping the configured sources"
    return
  fi
  { echo "$mirror"; cut -f1 "$mirror_list" | grep -vxF "$mirror" || true; } \
    | awk 'NF { printf "%s\tpriority:%d\n", $1, NR }' \
    | sudo tee "$mirror_list.new" >/dev/null
  sudo mv "$mirror_list.new" "$mirror_list"
  rm -f "$updated"
}

for attempt in "${!limits[@]}"; do
  limit=${limits[attempt]}
  if [ "$attempt" -gt 0 ]; then
    mirror=${fallbacks[attempt - 1]}
    # A killed apt-get can leave its line open; the annotation needs its own.
    printf '\n'
    echo "::warning::apt-install: attempt $attempt failed or ran past its ${limits[attempt - 1]} s; switching to $mirror"
    use_mirror "$mirror"
  fi
  start=$SECONDS
  if [ ! -e "$updated" ]; then
    apt_get_within "$update_limit" update || continue
    touch "$updated"
  fi
  if apt_get_within "$limit" install -y --no-install-recommends --download-only "$@"; then
    echo "apt-install: fetched in $((SECONDS - start)) s (attempt $((attempt + 1)))"
    apt_get install -y --no-install-recommends --no-download "$@"
    exit 0
  fi
done

echo "::error::apt-install: no mirror delivered the packages within its limit" >&2
exit 1
