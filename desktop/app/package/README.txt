Gelabber desktop (Linux x64): test build from CI, not a release.

Run:
  ./gelabber-desktop --server https://your-gelabber-server
(or start without --server and enter the address; it is remembered.)
libgelabber_media.so must stay next to the binary.

Needs (Arch/Omarchy package names):
  webkit2gtk-4.1 gtk3 libsoup3 gst-plugins-base gst-plugins-good
  pipewire pipewire-pulse xdg-desktop-portal xdg-desktop-portal-hyprland
  vulkan-icd-loader plus the GPU's Vulkan driver (nvidia-utils, vulkan-intel)
Hardware H264 encode: gst-plugins-bad (nvh264enc for NVIDIA, vah264enc for
Intel/AMD VA-API). Without it the app encodes in software.

Logs: GELABBER_MEDIA_LOG=1 ./gelabber-desktop ...
Force an H264 encoder: GELABBER_H264_ENCODER=nvh264enc|vah264enc|none

The build contains libwebrtc's H264 decoder (FFmpeg); do not redistribute
it until the H264 licensing question is settled.
