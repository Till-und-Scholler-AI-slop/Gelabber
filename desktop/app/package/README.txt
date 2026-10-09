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
Hardware H264 encode: gst-plugins-bad (nvh264enc for NVIDIA, vah264enc for
Intel/AMD VA-API). Without it the app encodes in software.

Logs: GELABBER_MEDIA_LOG=1 ./gelabber-desktop ...
Force an H264 encoder: GELABBER_H264_ENCODER=nvh264enc|vah264enc|none

Licences: THIRD-PARTY-NOTICES.txt lists the third-party software in the two
binaries, with its licences and their texts, and what is still open. It is
next to this file in the tarball; the package installs it as
/usr/share/licenses/gelabber-desktop/THIRD-PARTY-NOTICES.txt.
libgelabber_media.so contains OpenH264 (H264 encoder) and FFmpeg decoders
from libwebrtc (LGPL; H264, AAC and others). How H264 licensing applies to
this build is not settled (issue #165); do not redistribute it until it is.
