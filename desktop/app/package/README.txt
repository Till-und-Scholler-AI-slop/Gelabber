Gelabber desktop (Linux x64)

Released as gelabber-desktop-linux-x64.tar.gz on the release page
(https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases) and as the
pacman package gelabber-desktop (Arch/Omarchy), which installs the same
files.

Run from the tarball:
  ./gelabber-desktop --server https://your-gelabber-server
(or start without --server and enter the address; it is remembered.)
libgelabber_media.so must stay next to the binary.
Installed from the package: "Gelabber" in the launcher, or gelabber-desktop
with the same arguments.

Needs (Arch/Omarchy package names; the package depends on them or lists
them as optional):
  webkit2gtk-4.1 gtk3 libsoup3 gst-plugins-base gst-plugins-good
  pipewire pipewire-pulse xdg-desktop-portal xdg-desktop-portal-hyprland
  vulkan-icd-loader plus the GPU's Vulkan driver (nvidia-utils, vulkan-intel)
Camera, screen and Go Live are sent as VP8, encoded in software. No
package adds hardware encoding: the app does not use the H264 encoders of
gst-plugins-bad.

Logs: GELABBER_MEDIA_LOG=1 ./gelabber-desktop ...
Small video: when a video tile says "Geringe Bildqualität" and, with the
pointer over it, names the server's Content-Security-Policy, that policy
keeps the app from fetching pictures the fast way. Whoever runs the server
has to allow ipc: and http://ipc.localhost in connect-src; see
deploy/README.md in the Gelabber repository.

Licences: THIRD-PARTY-NOTICES.txt lists the third-party software in the two
binaries, with its licences and their texts, and what is still open. It is
next to this file in the tarball; the package installs it as
/usr/share/licenses/gelabber-desktop/THIRD-PARTY-NOTICES.txt.
libgelabber_media.so contains OpenH264 (H264 encoder) and FFmpeg decoders
from libwebrtc (LGPL; H264, AAC and others). How H264 licensing applies to
this build is not settled (issue #165); do not redistribute it until it is.
