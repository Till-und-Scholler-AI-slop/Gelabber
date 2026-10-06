#!/bin/sh
# Generate a dependency-only workspace. Never modify or build the real source here.
set -eu

input=${1:?input manifest directory required}
output=${2:?fresh output directory required}
test ! -e "$output"
mkdir -p "$output"

cp "$input/rust-toolchain.toml" "$output/rust-toolchain.toml"
awk '
  /^\[/ { section = $0 }
  section == "[workspace.package]" && /^version[[:space:]]*=/ {
    print "version = \"0.0.0\""; versions++; next
  }
  { print }
  END { if (versions != 1) exit 1 }
' "$input/Cargo.toml" > "$output/Cargo.toml"

# Only these local package versions vary with the application release. Preserve
# every external package, checksum, dependency edge and lockfile format verbatim.
awk '
  /^\[\[package\]\]/ { local_package = 0 }
  /^name = "gelabber-(api|media|shared)"$/ { local_package = 1; names++ }
  local_package && /^source[[:space:]]*=/ { exit 1 }
  local_package && /^version[[:space:]]*=/ {
    print "version = \"0.0.0\""; versions++; next
  }
  { print }
  END { if (names != 3 || versions != 3) exit 1 }
' "$input/Cargo.lock" > "$output/Cargo.lock"

for member in api media shared; do
  mkdir -p "$output/$member/src"
  cp "$input/$member/Cargo.toml" "$output/$member/Cargo.toml"
  printf '%s\n' '// Dependency-cache placeholder. Never shipped.' > "$output/$member/src/lib.rs"
done
for member in api media; do
  printf '%s\n' 'fn main() {}' > "$output/$member/src/main.rs"
done
