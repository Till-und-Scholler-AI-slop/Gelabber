#!/bin/sh
# Verify locked native-tool archives before any package installation; no Python needed.
set -eu

fail() { printf '%s\n' "$*" >&2; exit 1; }

select_records() {
    case "$media_debian_arch" in amd64|arm64) ;; *) fail "unsupported native Debian architecture" ;; esac
    awk -F '\t' -v arch="$media_debian_arch" '
        /^#/ || NF == 0 { next }
        NF != 6 || seen[$1 FS $2]++ { invalid = 1; exit }
        $1 == arch { print; count++ }
        END { if (invalid || !count) exit 1 }
    ' "$media_debian_lock" > "$media_debian_records" || fail "invalid or empty native Debian lock"
}

verify_archives() {
    media_tab=$(printf '\t')
    while IFS="$media_tab" read -r target package version archive_arch checksum filename; do
        case "$package" in ''|*[!a-z0-9+.-]*) fail "invalid locked package name" ;; esac
        case "$checksum" in ''|*[!0-9a-f]*) fail "invalid locked archive checksum" ;; esac
        [ "${#checksum}" -eq 64 ] || fail "invalid locked archive checksum"
        [ "$archive_arch" = "$target" ] || [ "$archive_arch" = all ] || fail "invalid locked archive architecture"
        archive="$media_debian_archives/$package.deb"
        [ -f "$archive" ] && [ ! -L "$archive" ] || fail "missing native build archive: $package"
        digest=$(sha256sum "$archive")
        digest=${digest%% *}
        [ "$digest" = "$checksum" ] || fail "native build archive checksum mismatch: $package"
        [ "$(dpkg-deb --field "$archive" Package)" = "$package" ] || fail "native build archive package mismatch: $package"
        [ "$(dpkg-deb --field "$archive" Version)" = "$version" ] || fail "native build archive version mismatch: $package"
        [ "$(dpkg-deb --field "$archive" Architecture)" = "$archive_arch" ] || fail "native build archive architecture mismatch: $package"
    done < "$media_debian_records"
}

if [ "${1:-}" = --verify-only ]; then
    [ "$#" -eq 4 ] || fail "usage: --verify-only LOCK ARCHIVE_DIRECTORY TARGET_ARCHITECTURE"
    media_debian_lock=$2
    media_debian_archives=$3
    media_debian_arch=$4
    media_debian_records=$(mktemp)
    trap 'rm -f "$media_debian_records"' 0
    select_records
    verify_archives
    exit 0
fi

[ "$#" -eq 1 ] || fail "usage: install-mediasoup-debian-build-tools.sh LOCK"
media_debian_lock=$1
. /etc/os-release
[ "$ID" = debian ] && [ "$VERSION_CODENAME" = trixie ] || fail "native archive installer requires Debian trixie"
media_debian_arch=$(dpkg --print-architecture)
media_debian_workspace=$(mktemp -d)
trap 'rm -rf "$media_debian_workspace"' 0
media_debian_records="$media_debian_workspace/records.tsv"
media_debian_archives="$media_debian_workspace/archives"
mkdir "$media_debian_archives"
select_records
# APT authenticates the configured Debian InRelease and package indexes. The
# independent lock also rejects a changed archive even if its version is reused.
apt-get update -o APT::Update::Error-Mode=any
media_tab=$(printf '\t')
while IFS="$media_tab" read -r target package version archive_arch checksum filename; do
    case "$package" in ''|*[!a-z0-9+.-]*) fail "invalid locked package name" ;; esac
    download_directory="$media_debian_workspace/$package"
    mkdir "$download_directory"
    (cd "$download_directory" && apt-get download "$package:$target=$version")
    set -- "$download_directory/"*.deb
    [ "$#" -eq 1 ] && [ -f "$1" ] || fail "unexpected downloaded archive count: $package"
    mv "$1" "$media_debian_archives/$package.deb"
done < "$media_debian_records"
verify_archives
set --
while IFS="$media_tab" read -r target package version archive_arch checksum filename; do
    set -- "$@" "$media_debian_archives/$package.deb"
done < "$media_debian_records"
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$@"
