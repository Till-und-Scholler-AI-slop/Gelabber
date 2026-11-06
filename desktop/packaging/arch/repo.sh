#!/usr/bin/env bash
# The Gelabber pacman repository, built in an Arch container (CI).
#
#   repo.sh build <version> <tarball> <site> [source-url]
#     Builds and signs the package gelabber-desktop (native/PKGBUILD) from
#     the app tarball, then the signed repository database:
#     <site>/x86_64/gelabber.db and the package, <site>/gelabber.asc (public
#     key). Without a source URL the package records the local file (test
#     builds).
#   repo.sh test <site> <tarball> <tauri-pkgbuild> <tauri-tarball> <tauri-version>
#     As a user would: adds the repository and its key to pacman, installs
#     the Tauri app's package gelabber-desktop (PKGBUILD and tarball of a
#     release up to 0.8) and lets it run the WebView fixture page
#     (desktop/ui/tests/fixtures/webview, so python must be installed),
#     upgrades it with -Syu into the native app, checks what that took over,
#     starts it (../native/smoke.sh, so xvfb-run and a Vulkan driver must be
#     installed), upgrades to a newer pkgrel, removes with -Rns, and checks
#     what is left. Runs after build in the same container (same signing key)
#     and as root; the Tauri app's WebKitGTK store lands in
#     ./webview-fixture-arch.
#
# Signing key: ARCH_REPO_GPG_KEY (ASCII-armored private key without
# passphrase). Without it a throwaway key signs the test build.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
builder=builder
package=gelabber-desktop
id=io.github.till-und-scholler-ai-slop.gelabber
fixtures="$here/../../ui/tests/fixtures/webview"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/$id"

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

# Builds <pkgbuild> for one tarball in a directory of its own and prints
# that directory; the package and the regenerated .SRCINFO are in it.
make_package() {
  local pkgbuild="$1" version="$2" tarball="$3" source_url="$4" pkgrel="$5"
  local work
  work="$(mktemp -d)"
  cp "$pkgbuild" "$work/PKGBUILD"
  cp "$here/update-pkgbuild.sh" "$work/"
  chown -R "$builder" "$work"
  local name
  name="$(basename "$tarball")"
  cp "$tarball" "$work/$name"
  as_builder "$work/update-pkgbuild.sh" "$version" "${source_url:-$name}" "$work/$name" "$pkgrel" >&2
  # makepkg looks for the source under its target name first, so a release
  # build does not download what it already has.
  local target
  target="$(cd "$work" && bash -c 'source ./PKGBUILD; echo "$pkgname-$pkgver-$pkgrel.tar.gz"')"
  [ "$target" = "$name" ] || as_builder cp "$work/$name" "$work/$target"
  # The package lands next to the PKGBUILD (makepkg rejects a PKGDEST
  # elsewhere under fakeroot in the CI container).
  (cd "$work" && as_builder env GPGKEY="$fpr" PKGDEST="$work" makepkg --sign --nodeps --noconfirm >&2)
  echo "$work"
}

# The app's package into the repository in <out>; its PKGBUILD and .SRCINFO
# land in <out>/native.
build_one() {
  local version="$1" tarball="$2" out="$3" source_url="$4" pkgrel="$5"
  local work
  work="$(make_package "$here/native/PKGBUILD" "$version" "$tarball" "$source_url" "$pkgrel")"
  mkdir -p "$out/x86_64" "$out/native"
  cp "$work"/*.pkg.tar.zst "$work"/*.pkg.tar.zst.sig "$out/x86_64/"
  cp "$work/PKGBUILD" "$work/.SRCINFO" "$out/native/"
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
      echo "left behind: $path" >&2
      exit 1
    fi
  done
}

# The Tauri app on the fixture page of 127.0.0.1:8124, then of 8123 (the
# server it keeps): its WebKitGTK storage as a user's, without an export.
tauri_fills_storage() {
  local store="${XDG_DATA_HOME:-$HOME/.local/share}/$id/localstorage" port server app
  mkdir -p "$config_dir"
  for port in 8124 8123; do
    printf '{"server":"http://127.0.0.1:%s"}' "$port" >"$config_dir/desktop.json"
    rm -f "/tmp/webview-done-$port"
    python3 -I "$fixtures/page/serve.py" "$port" "/tmp/webview-done-$port" 2>/dev/null &
    server=$!
    # As v0.8.9's smoke test started it; setsid, so the kill takes WebKit's processes too.
    WEBKIT_DISABLE_COMPOSITING_MODE=1 setsid xvfb-run -a /usr/bin/gelabber-desktop >"/tmp/tauri-$port.log" 2>&1 &
    app=$!
    for _ in $(seq 120); do [ -e "/tmp/webview-done-$port" ] && break; sleep 1; done
    sleep 2
    kill -TERM -- "-$app" 2>/dev/null || true
    wait "$app" || true
    kill "$server" 2>/dev/null || true
    wait "$server" || true
    if [ ! -e "/tmp/webview-done-$port" ]; then
      cat "/tmp/tauri-$port.log"
      echo "the Tauri app did not run the fixture page on port $port" >&2
      exit 1
    fi
  done
  test -f "$store/http_127.0.0.1_8123.localstorage"
  test ! -e "$config_dir/legacy-settings.json"
  mkdir -p webview-fixture-arch
  cp "$store"/http_127.0.0.1_812[34].localstorage* webview-fixture-arch/
  rm -f webview-fixture-arch/*-shm
}

# The native app's first start as the user who had the Tauri app (no
# GELABBER_CONFIG_DIR): it takes the WebView's settings, volumes and drafts.
native_takes_over() {
  local work status=0
  work="$(mktemp -d)"
  mkdir -m 700 "$work/run"
  timeout --kill-after=10 70 env -u GELABBER_CONFIG_DIR -u GELABBER_SERVER -u WAYLAND_DISPLAY \
    XDG_RUNTIME_DIR="$work/run" GELABBER_SMOKE=10 RUST_LOG=info \
    xvfb-run -a -s "-screen 0 1280x800x24" /usr/bin/gelabber --server https://gelabber.invalid \
    >"$work/log" 2>&1 || status=$?
  cat "$work/log"
  if [ "$status" -ne 0 ] || ! grep -q '^gelabber: took over .* from WebKitGTK$' "$work/log"; then
    echo "the native app's first start took nothing from WebKitGTK (exit status $status)" >&2
    exit 1
  fi
  rm -rf "$work"
  python3 -I "$fixtures/check_import.py" "$config_dir"
}

# An update shows up like a new release: same version, pkgrel 2.
upgrade() {
  local site="$1" tarball="$2"
  local version
  version="$(pacman -Q "$package" | awk '{print $2}')"
  version="${version%-*}"
  build_one "$version" "$tarball" "$site" "" 2
  pacman -Syu --noconfirm
  pacman -Q "$package" | grep -- '-2$'
}

case "${1:-}" in
  build)
    version="${2:?version}" tarball="$(realpath "${3:?tarball}")" site="${4:?site}"
    mkdir -p "$site"
    site="$(realpath "$site")"
    setup_builder
    build_one "$version" "$tarball" "$site" "${5:-}" 1
    ls -l "$site" "$site/x86_64"
    ;;
  test)
    site="$(realpath "${2:?site}")"
    test_tarball="$(realpath "${3:?tarball}")"
    tauri_pkgbuild="$(realpath "${4:?tauri-pkgbuild}")"
    tauri_tarball="$(realpath "${5:?tauri-tarball}")"
    tauri_version="${6:?tauri-version}"
    setup_builder
    use_site "$site"
    pacman -Syu --noconfirm
    # The Tauri app as its users have it, with its dependencies (WebKitGTK).
    tauri="$(make_package "$tauri_pkgbuild" "$tauri_version" "$tauri_tarball" "" 1)"
    pacman -U --noconfirm "$tauri"/*.pkg.tar.zst
    test -x /usr/bin/gelabber-desktop
    test -x /opt/gelabber-desktop/gelabber-desktop
    test -f /usr/share/applications/gelabber-desktop.desktop
    tauri_fills_storage
    old="$(pacman -Q "$package" | awk '{print $2}')"
    new="$(pacman -Sl gelabber | awk -v p="$package" '$2 == p {print $3}')"
    if [ "$(vercmp "$new" "$old")" -le 0 ]; then
      echo "the repository's $package $new is not newer than the Tauri package $old" >&2
      exit 1
    fi
    # The upgrade a user gets with -Syu: same name, a newer version.
    pacman -Syu --noconfirm
    pacman -Q "$package" | grep -qx "$package $new"
    pacman -Qi "$package"
    test -x /usr/bin/gelabber
    test "$(readlink -f /usr/bin/gelabber-desktop)" = /opt/gelabber/gelabber
    test -f /usr/share/applications/gelabber.desktop
    test -f /usr/share/icons/hicolor/256x256/apps/gelabber.png
    # The Tauri package's own files went with its version.
    nothing_left /opt/gelabber-desktop /usr/share/applications/gelabber-desktop.desktop \
      /usr/share/icons/hicolor/256x256/apps/gelabber-desktop.png
    libraries_found /opt/gelabber/gelabber /opt/gelabber/libgelabber_media.so
    native_takes_over
    # The installed app starts and paints (needs xvfb-run and a Vulkan
    # driver), through the command the Tauri package had.
    "$here/../native/smoke.sh" /usr/bin/gelabber-desktop
    upgrade "$site" "$test_tarball"
    pacman -Rns --noconfirm "$package"
    nothing_left /opt/gelabber /usr/bin/gelabber /usr/bin/gelabber-desktop \
      /usr/share/applications/gelabber.desktop /usr/share/icons/hicolor/256x256/apps/gelabber.png
    echo "upgrade from the Tauri package $old to $new, upgrade and removal OK"
    ;;
  *)
    echo "usage: repo.sh build <version> <tarball> <site> [source-url] | test <site> <tarball> <tauri-pkgbuild> <tauri-tarball> <tauri-version>" >&2
    exit 2
    ;;
esac
