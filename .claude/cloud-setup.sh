#!/bin/bash
# Prepares a Claude Code cloud container for Gelabber work: the Node version
# web/package.json asks for, web dependencies, the Rust toolchain with fmt and
# clippy, backend crates, and the test database. The environment's setup
# script runs it from the repository root; it is safe to run again.
set -euo pipefail
cd "$(dirname "$0")/.."

# Node: the minimum of web/package.json "engines", ahead of the image's Node.
node_version=$(python3 -c 'import json,re; print(re.sub(r"[^0-9.]", "", json.load(open("web/package.json"))["engines"]["node"]))')
node_dir=/opt/node-v$node_version
if [ ! -x "$node_dir/bin/node" ]; then
  archive=$(mktemp)
  curl -fsSL "https://nodejs.org/dist/v$node_version/node-v$node_version-linux-x64.tar.xz" -o "$archive"
  mkdir -p "$node_dir"
  tar -xJf "$archive" -C "$node_dir" --strip-components=1
  rm -f "$archive"
fi
mkdir -p "$HOME/.local/bin"
for tool in node npm npx; do ln -sf "$node_dir/bin/$tool" "$HOME/.local/bin/$tool"; done
export PATH="$HOME/.local/bin:$PATH"
(cd web && npm ci --no-audit --no-fund)

# Rust: rust-toolchain.toml picks the version; CI's lint job uses fmt and clippy.
rustup component add rustfmt clippy
cargo fetch --locked --manifest-path backend/Cargo.toml

# Test database as in backend/api/.env.example; sqlx::test needs CREATEDB.
pg_ctlcluster 16 main start 2>/dev/null || true
su postgres -c "psql -tAc \"SELECT 1 FROM pg_roles WHERE rolname = 'gelabber'\"" | grep -q 1 \
  || su postgres -c "psql -c \"CREATE ROLE gelabber LOGIN CREATEDB PASSWORD 'gelabber'\""
su postgres -c "psql -tAc \"SELECT 1 FROM pg_database WHERE datname = 'gelabber'\"" | grep -q 1 \
  || su postgres -c "createdb -O gelabber gelabber"
