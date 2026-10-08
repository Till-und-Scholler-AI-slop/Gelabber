# Gelabber for Arch Linux (pacman repository)

The AUR is closed to new accounts, so Gelabber ships its own pacman
repository on GitHub Pages. `yay` (and `pacman`) install from it like from
any other repository: `yay -S` once, updates with `yay -Syu`, removal with
`yay -Rns`. The package is a binary package of the release's
`gelabber-desktop-linux-x64.tar.gz`; nothing is compiled on the user's
machine.

Status: **test setup**. CI builds and checks everything on every desktop
change, but nothing is public until a maintainer turns publishing on (see
below). The build contains libwebrtc's H264 decoder (FFmpeg); do not turn
publishing on before the H264 licensing question in #165 is settled.

## For users

One time, as root:

```sh
# 1. The repository's signing key. Check the fingerprint against the one in
#    the release notes before trusting it.
curl -fsSLO https://till-und-scholler-ai-slop.github.io/Gelabber/gelabber.asc
sudo pacman-key --add gelabber.asc
sudo pacman-key --lsign-key <FINGERPRINT>

# 2. The repository, at the end of /etc/pacman.conf:
#    [gelabber]
#    SigLevel = Required DatabaseRequired
#    Server = https://till-und-scholler-ai-slop.github.io/Gelabber/$arch
```

Then:

```sh
yay -S gelabber-desktop      # install (or: sudo pacman -Syu gelabber-desktop)
yay -Syu                     # updates, together with the rest of the system
yay -Rns gelabber-desktop    # removal
```

Removal deletes every file the package installed. Your login and server
choice stay in `~/.config/io.github.till-und-scholler-ai-slop.gelabber` and
`~/.local/share/io.github.till-und-scholler-ai-slop.gelabber` (pacman never
touches home directories); delete them by hand for a clean slate.

Pre-releases (alpha, beta, rc) are published too and sort before the final
release: `0.5.0alpha.1 < 0.5.0beta.1 < 0.5.0rc.1 < 0.5.0`. The repository
always holds only the newest release.

## How it works

- `PKGBUILD` / `.SRCINFO`: the package. `update-pkgbuild.sh` sets version,
  download URL and SHA-256 for one build and regenerates `.SRCINFO`; CI
  checks that the committed `.SRCINFO` matches the `PKGBUILD`.
- `repo.sh build`: builds and signs the package, then the signed repository
  database (`repo-add --sign`), plus `gelabber.asc` with the public key.
- `repo.sh test`: adds that repository to pacman with `SigLevel = Required`,
  installs, upgrades to a newer build with `-Syu`, removes with `-Rns` and
  fails if a library is missing or a file is left behind.
- `.github/workflows/desktop-native.yml`:
  - every run: job "Arch package and repository" builds with a throwaway
    key and runs the test; the repository is the `arch-repo` artifact.
  - on a published GitHub release (pre-releases included; releases made by
    `release.yml` dispatch this workflow on their tag instead): attaches
    `gelabber-desktop-linux-x64.tar.gz` to the release, builds the
    repository with the real key, and, only if publishing is on, deploys it
    to GitHub Pages.

`yay` itself is not run in CI (it refuses to run as root); for packages from
a pacman repository it calls pacman, which the test covers.

## Maintainer setup (once, before the first public release)

1. **Signing key** (no passphrase; the secret store protects it):
   ```sh
   gpg --quick-gen-key "Gelabber packages <packages@example.org>" ed25519 sign never
   gpg --armor --export-secret-keys <FINGERPRINT>   # -> secret ARCH_REPO_GPG_KEY
   gpg --armor --export <FINGERPRINT>               # keep; publish the fingerprint
   ```
   Keep an offline backup. A lost key means every user has to trust a new
   one.
2. **Secret** (Settings → Secrets and variables → Actions → Secrets):
   `ARCH_REPO_GPG_KEY` = the exported private key.
3. **GitHub Pages** (Settings → Pages): Source "GitHub Actions". The site
   is public, whatever the repository's visibility.
4. **Approval** (Settings → Environments → `github-pages`): add yourself as
   required reviewer, and allow deployments from tags `v*` (the default
   only allows the default branch).
5. **Turn publishing on** (Settings → Secrets and variables → Actions →
   Variables): `ARCH_REPO_PUBLISH` = `true`. Until then releases only get
   the tarball attached and the repository as a workflow artifact.
6. Put the key fingerprint in the release notes and in this README.

Releases themselves follow AGENTS.md: cut from `main` after the version PR,
never from a feature branch.

## If the AUR opens again

The build also leaves `PKGBUILD` and `.SRCINFO` with the release's version,
URL and checksum in the `arch-repo` artifact. Publishing them as
`gelabber-desktop-bin` needs an AUR account, an SSH key registered there
(secret for CI), and a push to `ssh://aur@aur.archlinux.org/gelabber-desktop-bin.git`.
The package then has to be renamed to `gelabber-desktop-bin`; it already
conflicts with that name so both cannot be installed together.
