#!/usr/bin/env bash
# Points the PKGBUILD at one build of the app and regenerates .SRCINFO.
#
#   update-pkgbuild.sh <version> <source> <tarball> [pkgrel]
#
# <version>: a release tag or Cargo version (v0.5.0-alpha.1, 0.5.0-dev).
#   pacman versions cannot contain "-", so it becomes 0.5.0alpha.1, which
#   vercmp orders alpha < beta < rc < release like the tags.
# <source>: the download URL the package records, or a local file name.
# <tarball>: the same file on disk, for the checksum.
# Runs makepkg --printsrcinfo, so it needs an Arch system (CI container).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
version="${1:?version}"
source_ref="${2:?source}"
tarball="${3:?tarball}"
pkgrel="${4:-1}"

pkgver="${version#v}"
pkgver="${pkgver//-/}"
if [[ ! "$pkgver" =~ ^[0-9][0-9A-Za-z.+_]*$ ]]; then
  echo "cannot turn '$version' into a pacman version" >&2
  exit 1
fi
sha="$(sha256sum "$tarball" | cut -d' ' -f1)"

pkgbuild="$here/PKGBUILD"
sed -i \
  -e "s|^pkgver=.*|pkgver=${pkgver}|" \
  -e "s|^pkgrel=.*|pkgrel=${pkgrel}|" \
  -e "s|^source=.*|source=(\"\${pkgname}-\${pkgver}-\${pkgrel}.tar.gz::${source_ref}\")|" \
  -e "s|^sha256sums=.*|sha256sums=('${sha}')|" \
  "$pkgbuild"
(cd "$here" && makepkg --printsrcinfo > .SRCINFO)
echo "PKGBUILD: ${pkgver}-${pkgrel}, ${source_ref}, sha256 ${sha}"
