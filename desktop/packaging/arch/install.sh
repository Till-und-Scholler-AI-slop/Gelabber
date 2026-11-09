#!/bin/sh
# Gelabber for Arch Linux: adds the signed [gelabber] pacman repository and
# installs gelabber-desktop from it. Updates then come with pacman -Syu
# (or yay -Syu). Running it again only installs or updates the package.
#
#   curl -fsSL https://till-und-scholler-ai-slop.github.io/Gelabber/install.sh | sudo sh
#
# Arguments go to pacman, e.g. `sudo sh -s -- --noconfirm`. What it does by
# hand: desktop/packaging/arch/README.md ("For users").
set -eu

repo_url=https://till-und-scholler-ai-slop.github.io/Gelabber
fingerprint=5992A8FD7578E86E36D45A4001C81EE2C2C0BBC5
conf=/etc/pacman.conf

fail() {
  echo "gelabber install: $*" >&2
  exit 1
}

# The primary keys' fingerprints in an armored key file, one per line.
key_fingerprints() {
  gpg --homedir "$1" --batch --with-colons --show-keys "$2" 2>/dev/null |
    awk -F: '/^pub:/ { primary = 1; next } /^fpr:/ && primary { print $10; primary = 0 }'
}

# All in one function: `curl | sh` has read the whole script before anything
# runs, so pacman cannot read the rest of it as input.
main() {
  [ "$(id -u)" = 0 ] || fail "needs root: curl -fsSL $repo_url/install.sh | sudo sh"
  command -v pacman-key >/dev/null 2>&1 || fail "needs pacman (Arch Linux or a derivative)"
  [ "$(uname -m)" = x86_64 ] || fail "the package is built for x86_64 only, not $(uname -m)"

  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT

  echo "==> Repository key $fingerprint"
  curl -fsSL "$repo_url/gelabber.asc" -o "$work/gelabber.asc" || fail "could not download $repo_url/gelabber.asc"
  mkdir -m 700 "$work/gnupg"
  found="$(key_fingerprints "$work/gnupg" "$work/gelabber.asc")"
  [ "$found" = "$fingerprint" ] || fail "gelabber.asc is not the key $fingerprint (it holds: ${found:-no key})"
  pacman-key --add "$work/gelabber.asc" >/dev/null
  pacman-key --lsign-key "$fingerprint" >/dev/null

  if grep -q '^\[gelabber\]' "$conf"; then
    echo "==> $conf already has [gelabber], left as it is"
  else
    echo "==> Adding [gelabber] to $conf"
    printf '\n[gelabber]\nSigLevel = Required DatabaseRequired\nServer = %s/$arch\n' "$repo_url" >>"$conf"
  fi

  # pacman asks on the terminal, not on the pipe that brought this script.
  echo "==> pacman -Syu gelabber-desktop (updates the rest of the system too)"
  if (: </dev/tty) 2>/dev/null; then
    pacman -Syu --needed "$@" gelabber-desktop </dev/tty
  else
    pacman -Syu --needed "$@" gelabber-desktop
  fi

  echo "==> Done: start it with 'gelabber' or from the app launcher. Updates come with pacman -Syu."
}

main "$@"
