#!/bin/sh
# Renews the files that Dependabot's updates make stale (skill
# dependency-updates, step 5). Run from the repository root; each part writes
# nothing when nothing changed, and any part that cannot work stops the run.
# Runs no code of the updated dependencies: cargo tree/metadata only read
# the crates, and gh asks GitHub for a release asset's digest.
set -eu

# Caddy: ci.yml (job deploy, "Install Caddy") downloads the version that
# deploy/compose/compose.yaml pins and checks its SHA-256, GitHub's digest of
# the release asset (test_ci_caddy_uses_compose_pin_...).
v=$(sed -n 's/^ *image: caddy:\([0-9][0-9.]*\)-alpine$/\1/p' deploy/compose/compose.yaml)
[ -n "$v" ] || { echo "renew: no Caddy pin in deploy/compose/compose.yaml" >&2; exit 1; }
if ! grep -q "^ *CADDY_VERSION: $v\$" .github/workflows/ci.yml; then
  digest=$(gh api "repos/caddyserver/caddy/releases/tags/v$v" \
    --jq ".assets[] | select(.name == \"caddy_${v}_linux_amd64.tar.gz\") | .digest")
  printf '%s' "$digest" | grep -Eq '^sha256:[0-9a-f]{64}$' \
    || { echo "renew: no SHA-256 digest for Caddy $v: '$digest'" >&2; exit 1; }
  sed -i -E "s/^( *CADDY_VERSION: ).*/\1$v/; s/^( *CADDY_SHA256: ).*/\1${digest#sha256:}/" .github/workflows/ci.yml
  grep -q "^ *CADDY_SHA256: ${digest#sha256:}\$" .github/workflows/ci.yml \
    || { echo "renew: CADDY_SHA256 not found in ci.yml" >&2; exit 1; }
fi

# Node: the workflows' node-version follows the web image.
n=$(sed -n 's/^FROM node:\([0-9][0-9.]*\)-.*/\1/p' web/Dockerfile | head -1)
[ -n "$n" ] || { echo "renew: no Node version in web/Dockerfile" >&2; exit 1; }
sed -i -E "s/^( *node-version: )[0-9.]+\$/\1$n/" .github/workflows/ci.yml .github/workflows/e2e-extended.yml

# The desktop app's third-party notices, from desktop/Cargo.lock (CI job
# "Desktop / Third-party notices"). Needs cargo and the toolchain of
# rust-toolchain.toml; fetches the crates of the lock file.
python3 desktop/packaging/third-party-notices.py

# The deployment tests' requirements.txt: Dependabot writes the hashes; every
# file PyPI has for a pinned release must be hashed, so any platform installs.
python3 -I - <<'EOF'
import json, re, sys, urllib.request
text = open('deploy/compose/tests/requirements.txt').read()
pins = re.findall(r'^([A-Za-z0-9_.-]+)==(\S+)((?:[ \t]*\\\n[ \t]*--hash=sha256:[0-9a-f]{64})+)', text, re.M)
if not pins:
    sys.exit('renew: no hashed pin in deploy/compose/tests/requirements.txt')
for name, version, block in pins:
    have = set(re.findall(r'sha256:([0-9a-f]{64})', block))
    with urllib.request.urlopen(f'https://pypi.org/pypi/{name}/{version}/json') as r:
        want = {u['digests']['sha256'] for u in json.load(r)['urls']}
    if have != want:
        sys.exit(f'renew: {name} {version}: {len(have)} hashes, {len(want)} files on PyPI')
EOF
