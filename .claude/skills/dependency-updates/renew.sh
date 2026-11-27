#!/bin/sh
# Renews the files that Dependabot's updates make stale (skill
# dependency-updates). Run from the repository root:
#   sh .claude/skills/dependency-updates/renew.sh [caddy|node|notices|pip]...
# Without arguments, all four. Each part writes nothing when nothing changed
# and stops the run when it cannot work. None runs code of the updated
# dependencies: cargo tree and metadata only read the crates, gh asks GitHub
# for a release asset's digest, Python reads PyPI's file list.
set -eu
here=$(dirname "$0")

# ci.yml (job deploy, "Install Caddy") downloads the version that
# deploy/compose/compose.yaml pins and checks its SHA-256, GitHub's digest of
# the release asset (test_ci_caddy_uses_compose_pin_...).
caddy() {
  v=$(sed -n 's/^ *image: caddy:\([0-9][0-9.]*\)-alpine$/\1/p' deploy/compose/compose.yaml)
  [ -n "$v" ] || { echo "renew: no Caddy pin in deploy/compose/compose.yaml" >&2; exit 1; }
  grep -q '^ *CADDY_VERSION: ' .github/workflows/ci.yml \
    || { echo "renew: no CADDY_VERSION in .github/workflows/ci.yml" >&2; exit 1; }
  grep -q "^ *CADDY_VERSION: $v\$" .github/workflows/ci.yml && return 0
  digest=$(gh api "repos/caddyserver/caddy/releases/tags/v$v" \
    --jq ".assets[] | select(.name == \"caddy_${v}_linux_amd64.tar.gz\") | .digest")
  expr "x$digest" : 'xsha256:[0-9a-f]\{64\}$' > /dev/null \
    || { echo "renew: no SHA-256 digest for Caddy $v: '$digest'" >&2; exit 1; }
  sed -i -E "s/^( *CADDY_VERSION: ).*/\1$v/; s/^( *CADDY_SHA256: ).*/\1${digest#sha256:}/" .github/workflows/ci.yml
  grep -q "^ *CADDY_SHA256: ${digest#sha256:}\$" .github/workflows/ci.yml \
    || { echo "renew: no CADDY_SHA256 in .github/workflows/ci.yml" >&2; exit 1; }
}

# The workflows' node-version follows the web image.
node() {
  n=$(sed -n 's/^FROM node:\([0-9][0-9.]*\)-.*/\1/p' web/Dockerfile | head -1)
  [ -n "$n" ] || { echo "renew: no Node version in web/Dockerfile" >&2; exit 1; }
  sed -i -E "s/^( *node-version: )[0-9.]+\$/\1$n/" .github/workflows/ci.yml .github/workflows/e2e-extended.yml
}

# The desktop app's third-party notices, from desktop/Cargo.lock (CI job
# "Desktop / Third-party notices"). Needs cargo and the toolchain of
# rust-toolchain.toml; fetches the crates of the lock file.
notices() {
  python3 desktop/packaging/third-party-notices.py
}

# Dependabot writes requirements.txt's hashes; every file PyPI has for a
# pinned release must be hashed, so any platform installs it.
pip() {
  python3 -I "$here/pip_hashes.py" deploy/compose/tests/requirements.txt
}

[ $# -gt 0 ] || set -- caddy node notices pip
for part in "$@"; do
  case "$part" in
    caddy|node|notices|pip) "$part" ;;
    *) echo "renew: unknown part $part" >&2; exit 2 ;;
  esac
done
