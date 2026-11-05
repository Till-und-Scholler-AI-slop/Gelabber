# Native app: the desktop around the window

`desktop/ui/src/platform/` does for the egui app what the Tauri app
(`desktop/app/src`) does outside its page. The binary (`desktop/gelabber`)
owns one `Platform` and calls `logic` every pass (also while the window is
hidden in the tray) and `paint` after the page. Parity runs construct
`GelabberApp` without it, so nothing here changes a parity scene.

## Files in the config directory

The Tauri app's `app_config_dir()`:
`$XDG_CONFIG_HOME` or `~/.config`, `%APPDATA%`, or
`~/Library/Application Support`, plus
`io.github.till-und-scholler-ai-slop.gelabber/`. `GELABBER_CONFIG_DIR`
replaces the whole path. Every write goes to a partial file first and is
renamed, so a crash never leaves half a file.

| File | Content | Shared with the Tauri app |
|---|---|---|
| `desktop.json` | `server`, `close_to_tray`; unknown keys are kept | yes |
| `update.json` | `autoCheck` (read only) | yes |
| `settings.json` | device settings (`screens/settings/prefs.rs`) | no |
| `session.json` (0600) | `gelabber_session` cookie per server origin | no |
| `native-state.json` (0600) | per account: last place, last channel per server, last DM, drafts (7 days), per-person volumes; window size, position, maximized | no |

Server choice, first match wins: `--server <url>` / `--server=<url>`,
`GELABBER_SERVER`, `desktop.json`, else the setup screen. The cookie is not
in an OS keyring: the maintained crates pull a second D-Bus stack and crypto
in on Linux. The file is the user's only, as the browser's cookie store.

## Notifications (`notify/`)

The rules are the web's (`messages/useMessageToasts.ts`, `notify.ts`,
`notifyLevel.ts`, `notificationSettings.ts`, `notificationPrivacy.ts`):
chat creates of others, level and mute from the unread row or the settings,
only while the window is not in front (unfocused, minimized or in the tray),
the device setting "desktopNotify", live messages only (30 s on the server
clock), one per conversation in 5 s, each message once. The title is
`Anna · #kanal` (`hat dich erwähnt` for a mention), or only
`Neue Nachricht von Anna` while the account hides content (unknown counts as
hidden). Tags are `gelabber:<user>:<channel>`: reading the conversation,
signing out and switching on "Inhalt verbergen" take them down. A click
shows the window and opens `/s/…/c/…` or `/d/…`. A direct message while the
window is away asks for attention (taskbar flash, urgent hint). The message
sound goes through `Call::play_sound(Sound::Message)`, once per burst, not
while deafened, only with "messageSound".

Linux uses `org.freedesktop.Notifications` through zbus, Windows toasts
through WinRT (the Tauri app's code). Without a service the app runs
without notifications; the settings page then shows the browser texts.

## Tray, close-to-tray, one instance, window

- Tray: a StatusNotifierItem through ksni on Linux, tray-icon on Windows.
  "Gelabber öffnen" / "Beenden", a left click shows the window, a red dot
  for counted unread (mentions, DMs), a light dot for other unread, tooltip
  `Gelabber: 2 neue Nachrichten`.
- Close-to-tray: with a visible tray and `close_to_tray`, closing hides the
  window; a call keeps running. The switch "Fenster schließen minimiert in
  den Infobereich" appears in Benachrichtigungen only with a tray.
- One instance: a Unix socket in `$XDG_RUNTIME_DIR` (0600), a loopback port
  with a secret on Windows. A second start shows the first window and ends.
- Window: title "Gelabber", app id `gelabber`, icon `icon-192.png`, 1280×800,
  at least 480×360 (tauri.conf.json), dark title bar through the window
  theme, size and position restored.

## Push-to-talk (`ptt/`)

While a call runs, the voice input is "ptt" and a key is set, the key is
bound globally and its presses go to `Call::push_to_talk(pressed)`.
`platform::ptt_status(ctx)` says whether the global key works
(`Bound { trigger }`) or why not (`Failed { reason }`); voice code that
wants its own binding can use `platform::ptt::bind(key, report)`. Linux uses
the GlobalShortcuts portal (KDE, GNOME 48+, Hyprland:
`bind = , F13, global, gelabber:push-to-talk`), Windows a low-level hook.

## Updates (`update.rs`)

15 s after the start and every 6 hours while `autoCheck` holds, the
`latest.json` of the latest GitHub release (`GELABBER_UPDATE_ENDPOINT`
replaces it) is read. A newer version shows "Update verfügbar: Gelabber
x.y.z ist da." with "Herunterladen" (the release page) and "Später", never
during a call. `verify` checks a package as the Tauri updater does
(minisign, `desktop/app/updater.pub` or `GELABBER_UPDATER_PUBKEY` at build
time, signed for the announced version). Downloading and installing belong
to packaging.

## Tested headless, and what needs a desktop

`cargo test -p gelabber-ui platform` covers the rules, the files, the
cookie round trip, the single instance (socket and loopback), the
notification service and the portal against fakes on a private
`dbus-daemon` (skipped where it is missing), signatures and the update
check against a local endpoint, and a pass of `Platform` without a
desktop. `GELABBER_TEST_SERVER=http://127.0.0.1:5180 cargo test -p
gelabber-ui signed_in_after_a_restart -- --ignored` signs in against the
seeded stack, restarts the client from the stored cookie and signs out.

Only a desktop session shows: the tray icon and its menu on a real
StatusNotifier host, a notification click raising the window (on Wayland
the compositor may refuse focus without the activation token, which egui
cannot pass on), close-to-tray and the window coming back, the portal's
key dialog, the window theme and the restored position (Wayland gives
none). The Windows paths (toasts, tray-icon, keyboard hook) are copies of
the Tauri app's but are not compiled on Linux.
