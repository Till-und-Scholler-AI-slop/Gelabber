#!/usr/bin/env bash
# The Gelabber pacman repository, built in an Arch container (CI).
#
#   repo.sh build <version> <tarball> <site> [source-url]
#     Builds and signs the package from the app tarball, then the signed
#     repository database: <site>/x86_64/gelabber.db and the package,
#     <site>/gelabber.asc (public key). Without a source URL the package
#     records the local file (test builds).
#   repo.sh build-native <version> <tarball> <site> [source-url]
#     The same for the native app's beta package (native/PKGBUILD,
#     gelabber-desktop-native) from its tarball, added to the same database.
#   repo.sh test <site> <tarball>
#     As a user would: adds the repository and its key to pacman, installs,
#     upgrades to a newer pkgrel, removes with -Rns, and checks what is left.
#     Runs after build in the same container (same signing key).
#   repo.sh test-native <site> <tarball>
#     After test: installs the Tauri app's package, replaces it with the
#     native one (conflicts=), starts it (../native/smoke.sh, so xvfb-run
#     and a Vulkan driver must be installed), upgrades and removes it.
#
# Signing key: ARCH_REPO_GPG_KEY (ASCII-armored private key without
# passphrase). Without it a throwaway key signs the test build.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
builder=builder

as_builder() {
  sudo -u "$builder" -H env GNUPGHOME=/home/$builder/.gnupg "$@"
}

secret_key() {
  as_builder gpg --batch --with-colons --list-secret-keys 2>/dev/null | awk -F: '/^fpr:/ {print $10; exit}'
}

setup_builder() {
  id "$builder" >/dev/null 2>&1 || useradd -m "$builder"
  as_builder mkdir -p -m 700 /home/$builder/.gnupg
  fpr="$(secret_key)"
  [ -n "$fpr" ] && return 0
  if [ -n "${ARCH_REPO_GPG_KEY:-}" ]; then
    printf '%s\n' "$ARCH_REPO_GPG_KEY" | as_builder gpg --batch --import
  else
    echo "No ARCH_REPO_GPG_KEY: signing with a throwaway test key."
    as_builder gpg --batch --passphrase '' --quick-gen-key \
      "Gelabber test repository <test@invalid>" ed25519 sign 1d
  fi
  fpr="$(secret_key)"
  test -n "$fpr"
}

# <recipe>: the directory of the PKGBUILD below this one ("" or "native");
# its PKGBUILD and .SRCINFO land in the same place below <out>.
build_one() {
  local version="$1" tarball="$2" out="$3" source_url="$4" pkgrel="$5" recipe="${6:-}"
  local work
  work="$(mktemp -d)"
  cp "$here/${recipe:+$recipe/}PKGBUILD" "$here/update-pkgbuild.sh" "$work/"
  chown -R "$builder" "$work"
  local name
  name="$(basename "$tarball")"
  cp "$tarball" "$work/$name"
  as_builder "$work/update-pkgbuild.sh" "$version" "${source_url:-$name}" "$work/$name" "$pkgrel"
  # makepkg looks for the source under its target name first, so a release
  # build does not download what it already has.
  local target
  target="$(cd "$work" && bash -c 'source ./PKGBUILD; echo "$pkgname-$pkgver-$pkgrel.tar.gz"')"
  [ "$target" = "$name" ] || as_builder cp "$work/$name" "$work/$target"
  # The package lands next to the PKGBUILD (makepkg rejects a PKGDEST
  # elsewhere under fakeroot in the CI container).
  (cd "$work" && as_builder env GPGKEY="$fpr" PKGDEST="$work" makepkg --sign --nodeps --noconfirm)
  mkdir -p "$out/x86_64" "$out/${recipe:-.}"
  cp "$work"/*.pkg.tar.zst "$work"/*.pkg.tar.zst.sig "$out/x86_64/"
  cp "$work/PKGBUILD" "$work/.SRCINFO" "$out/${recipe:-.}/"
  chown -R "$builder" "$out"
  local pkg
  pkg="$(ls "$work"/*.pkg.tar.zst)"
  (cd "$out/x86_64" && as_builder repo-add --sign --key "$fpr" gelabber.db.tar.zst "$(basename "$pkg")")
  # Static hosting serves files, not symlinks.
  for link in "$out"/x86_64/gelabber.db "$out"/x86_64/gelabber.files \
    "$out"/x86_64/gelabber.db.sig "$out"/x86_64/gelabber.files.sig; do
    [ -L "$link" ] && cp --remove-destination "$(readlink -f "$link")" "$link"
  done
  as_builder gpg --batch --armor --export "$fpr" >"$out/gelabber.asc"
  echo "$fpr" >"$out/fingerprint.txt"
}

# pacman trusts the repository in <site> from here on.
use_site() {
  local site="$1"
  [ "$fpr" = "$(cat "$site/fingerprint.txt")" ]
  grep -q '^\[gelabber\]' /etc/pacman.conf && return 0
  pacman-key --init >/dev/null
  pacman-key --add "$site/gelabber.asc"
  pacman-key --lsign-key "$fpr"
  cat >>/etc/pacman.conf <<CONF

[gelabber]
SigLevel = Required DatabaseRequired
Server = file://$site/\$arch
CONF
}

# Every library the files need is there: depends=() is complete.
libraries_found() {
  if ldd "$@" | grep 'not found'; then
    echo "a dependency is missing from depends=()" >&2
    exit 1
  fi
}

nothing_left() {
  for path in "$@"; do
    if [ -e "$path" ] || [ -L "$path" ]; then
      echo "left behind after -Rns: $path" >&2
      exit 1
    fi
  done
}

# An update shows up like a new release: same version, pkgrel 2.
upgrade() {
  local package="$1" site="$2" tarball="$3" recipe="${4:-}"
  local version
  version="$(pacman -Q "$package" | awk '{print $2}')"
  version="${version%-*}"
  build_one "$version" "$tarball" "$site" "" 2 "$recipe"
  pacman -Syu --noconfirm
  pacman -Q "$package" | grep -- '-2$'
}

case "${1:-}" in
  build | build-native)
    recipe=""
    [ "$1" = build-native ] && recipe=native
    version="${2:?version}" tarball="$(realpath "${3:?tarball}")" site="${4:?site}"
    mkdir -p "$site"
    site="$(realpath "$site")"
    setup_builder
    build_one "$version" "$tarball" "$site" "${5:-}" 1 "$recipe"
    ls -l "$site" "$site/x86_64"
    ;;
  test)
    site="$(realpath "${2:?site}")"
    test_tarball="$(realpath "${3:?tarball}")"
    setup_builder
    use_site "$site"
    pacman -Syu --noconfirm gelabber-desktop
    pacman -Qi gelabber-desktop
    test -x /usr/bin/gelabber-desktop
    test -f /usr/share/applications/gelabber-desktop.desktop
    libraries_found /opt/gelabber-desktop/gelabber-desktop /opt/gelabber-desktop/libgelabber_media.so
    upgrade gelabber-desktop "$site" "$test_tarball"
    pacman -Rns --noconfirm gelabber-desktop
    nothing_left /opt/gelabber-desktop /usr/bin/gelabber-desktop \
      /usr/share/applications/gelabber-desktop.desktop \
      /usr/share/icons/hicolor/256x256/apps/gelabber-desktop.png
    echo "install, upgrade and removal OK"
    ;;
  test-native)
    site="$(realpath "${2:?site}")"
    test_tarball="$(realpath "${3:?tarball}")"
    setup_builder
    use_site "$site"
    # From the Tauri app, as a user switches: pacman asks to remove the
    # conflicting package, and --ask 4 says yes to that question.
    pacman -Syu --noconfirm gelabber-desktop
    pacman -S --noconfirm --ask 4 gelabber-desktop-native
    if pacman -Q gelabber-desktop 2>/dev/null; then
      echo "gelabber-desktop is still installed next to gelabber-desktop-native" >&2
      exit 1
    fi
    pacman -Qi gelabber-desktop-native
    test -x /usr/bin/gelabber
    test -f /usr/share/applications/gelabber.desktop
    test -f /usr/share/icons/hicolor/256x256/apps/gelabber.png
    libraries_found /opt/gelabber/gelabber /opt/gelabber/libgelabber_media.so
    # The installed app starts and paints (needs xvfb-run and a Vulkan driver).
    "$here/../native/smoke.sh" /usr/bin/gelabber
    upgrade gelabber-desktop-native "$site" "$test_tarball" native
    pacman -Rns --noconfirm gelabber-desktop-native
    nothing_left /opt/gelabber /usr/bin/gelabber /usr/share/applications/gelabber.desktop \
      /usr/share/icons/hicolor/256x256/apps/gelabber.png
    echo "native: switch from the Tauri app, upgrade and removal OK"
    ;;
  *)
    echo "usage: repo.sh build|build-native <version> <tarball> <site> [source-url] | test|test-native <site> <tarball>" >&2
    exit 2
    ;;
esac
