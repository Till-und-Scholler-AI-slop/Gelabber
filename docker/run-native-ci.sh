#!/bin/sh
# Cargo's PR gate uses the same verified native tools as the product image.
# The optional Ubuntu fault diagnostic uses its own default target directory.
set -eu

workspace=${GITHUB_WORKSPACE:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)}
workspace=$(CDPATH= cd -- "$workspace" && pwd -P)
test -f "$workspace/Cargo.toml"
cargo_cache=${CARGO_HOME:-"$HOME/.cargo"}
mkdir -p "$cargo_cache" "$workspace/target/native-debian"
cargo_cache=$(CDPATH= cd -- "$cargo_cache" && pwd -P)
case "$workspace:$cargo_cache" in
  *'
'*) echo 'native CI bind paths must not contain newlines' >&2; exit 1 ;;
esac
case "$workspace" in *:*) echo 'native CI workspace must not contain colons' >&2; exit 1 ;; esac
case "$cargo_cache" in *:*) echo 'native CI cargo cache must not contain colons' >&2; exit 1 ;; esac

image=$(docker image inspect --format '{{.Id}}' gelabber-native-ci-toolchain:local)
case "$image" in sha256:*) digest=${image#sha256:} ;; *) echo 'native CI image ID missing' >&2; exit 1 ;; esac
case "$digest" in *[!a-f0-9]*|'') echo 'native CI image ID invalid' >&2; exit 1 ;; esac
test "${#digest}" -eq 64

# Preserve the checkout's absolute path and writable cache ownership. The image
# supplies rustup and PATH; host Cargo binaries do not replace its Rust tools.
exec docker run --rm --init --network host \
  --user "$(id -u):$(id -g)" \
  --volume "$workspace:$workspace" \
  --volume "$cargo_cache:/cargo" \
  --workdir "$workspace" \
  --env CARGO_HOME=/cargo \
  --env RUSTUP_HOME=/usr/local/rustup \
  --env CARGO_TARGET_DIR="$workspace/target/native-debian" \
  --env CARGO_TERM_COLOR \
  --env DATABASE_URL --env REDIS_URL \
  --env MINIO_ENDPOINT --env MINIO_ROOT_USER --env MINIO_ROOT_PASSWORD \
  "$image" cargo "$@"
