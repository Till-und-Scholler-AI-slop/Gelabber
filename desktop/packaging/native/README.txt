Gelabber, desktop app (Linux x64)

Released as gelabber-native-linux-x64.tar.gz on the release page
(https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases) and as the
pacman package gelabber-desktop (Arch/Omarchy), which installs the same
files. It replaces the Tauri app of 0.8 and before: the package of the same
name upgrades into it (pacman -Syu), and it uses the same settings
directory.

Run from the tarball:
  ./gelabber --server https://your-gelabber-server
(or start without --server and enter the address; it is remembered.)
libgelabber_media.so must stay next to the binary.
Installed from the package: "Gelabber" in the launcher, or gelabber (also
gelabber-desktop, the Tauri app's command) with the same arguments.

Needs (Arch/Omarchy package names; the package depends on them or lists
them as optional):
  vulkan-icd-loader plus the GPU's Vulkan driver (nvidia-utils,
  vulkan-intel, vulkan-radeon), wayland or libx11, libxkbcommon,
  gst-plugins-base gst-plugins-good pipewire pipewire-pulse
  xdg-desktop-portal xdg-desktop-portal-hyprland
No WebKitGTK, libsoup or GTK: the window is drawn with egui on Vulkan.
Video on the GPU (optional): H264 comes from the
system's GStreamer (gst-plugins-bad with NVIDIA's nvcodec, gst-plugin-va for
Intel and AMD, gst-libav for decoding on the CPU); without them calls use
VP8.

Settings: ~/.config/io.github.till-und-scholler-ai-slop.gelabber/, the
Tauri app's directory (desktop.json with the server is shared). On the first
start the app takes over the settings a Tauri app from 0.8.10 on exported
(legacy-settings.json there) and those of earlier native builds
(~/.config/gelabber/); the sign-in is not taken over.

Updates: shortly after the start and every 6 hours the app looks for a
newer release. From the tarball it shows "Gelabber <version> ist da." with
"Herunterladen" (the release page); unpack the new tarball over this one.
Installed from the package it says that pacman updates it
(pacman -Syu); the app does not replace its own files.

Push-to-talk: choose "Push-to-Talk" in the voice settings. In a call the
app takes the key through the desktop's GlobalShortcuts portal. On Hyprland
bind it yourself in hyprland.conf:
  bind = , F13, global, gelabber:push-to-talk

Logs: RUST_LOG=info GELABBER_MEDIA_LOG=1 ./gelabber ...

Licences: THIRD-PARTY-NOTICES.txt lists the third-party software in the two
binaries and the fonts and data built into the app, with their licences and
texts. It is next to this file in the tarball; the package installs it as
/usr/share/licenses/gelabber-desktop/THIRD-PARTY-NOTICES.txt.
libgelabber_media.so contains no H264 codec (no OpenH264, no FFmpeg).
