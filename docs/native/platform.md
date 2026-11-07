# Native app: the desktop around the window

`desktop/ui/src/platform/` does for the egui app what the Tauri app (up to
0.8, removed in 0.9) did outside its page. The binary (`desktop/gelabber`)
owns one `Platform` and calls `logic` every pass (also while the window is
hidden in the tray) and `paint` after the page. Parity runs construct
`GelabberApp` without it, so nothing here changes a parity scene.

## Files in the config directory

The Tauri app's `app_config_dir()`, kept so that its installs keep their
settings:
`$XDG_CONFIG_HOME` or `~/.config`, `%APPDATA%`, or
`~/Library/Application Support`, plus
`io.github.till-und-scholler-ai-slop.gelabber/`. `GELABBER_CONFIG_DIR`
replaces the whole path. Every write goes to a partial file first and is
renamed, so a crash never leaves half a file.

| File | Content | Written by the Tauri app too |
|---|---|---|
| `desktop.json` | `server`, `close_to_tray`; unknown keys are kept | yes |
| `update.json` | `autoCheck` (read only) | yes |
| `settings.json` | device settings (`screens/settings/prefs.rs`) | no |
| `session.json` (0600) | `gelabber_session` cookie per server origin, only where the keyring did not take it (see Sign-in) | no |
| `native-state.json` (0600) | per account: last place, last channel per server, last DM, drafts (7 days), per-person volumes; window size, position, maximized | no |
| `legacy-settings.json` | the Tauri app's export of the web client's localStorage (read once, never written) | written by it |

First start (`legacy.rs`): while `settings.json` is missing, the app takes
over `legacy-settings.json` (`{"version": 1, "localStorage": {key: value}}`:
`gelabber.media`, `gelabber.media.source-audio-share`, `gelabber.interface`
into `settings.json`, `gelabber.user-volumes:<account>` and
`gelabber:chat-draft:<account>:<channel>` into `native-state.json`). Without
a readable `legacy-settings.json` (0.8.9 and older) it reads the same entries
from the Tauri app's WebView storage (`webview.rs`: WebView2's LevelDB on
Windows, WebKitGTK's SQLite on Linux); a failure that may pass (an I/O or
sharing error, a busy database) leaves `settings.json` unwritten, so the next
start tries again, while an unknown format or a damaged store counts as none. It also takes the
files of native builds before 0.9 (`<config base>/gelabber/settings.json`
and `…/server`, the latter only where `desktop.json` has no server). It
overwrites nothing (a `desktop.json` or `native-state.json` it cannot read
stays as it is) and leaves the sources where they are; `settings.json` is
written even when empty, so it happens once.

Server choice, first match wins: `--server <url>` / `--server=<url>`,
`GELABBER_SERVER`, `desktop.json`, else the setup screen.

Sign-in (`session.rs`): the `gelabber_session` cookie per server origin goes
into the system keyring first. On Linux that is the Secret Service over the
app's own zbus (item "Gelabber-Anmeldung (<origin>)", attributes
`application` = `gelabber` and `origin`), on Windows the Credential Manager
(target `Gelabber/<origin>`). Without a keyring, or while it is locked, the
cookie goes into `session.json` (0600); the app never unlocks a keyring,
because that would prompt in the middle of a frame. A cookie the keyring
takes is removed from the file. Reading takes the file first, since an entry
there is only ever newer. The Tauri app's WebView cookie is not taken over,
so everyone signs in once after the switch.

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
through WinRT (from the Tauri app's code). Without a service the app runs
without notifications; the settings page then shows the browser texts.

## Tray, close-to-tray, one instance, window

- Tray: a StatusNotifierItem through ksni on Linux, tray-icon on Windows.
  "Gelabber öffnen" / "Beenden", a left click shows the window, a red dot
  for counted unread (mentions, DMs), a light dot for other unread, tooltip
  `Gelabber: 2 neue Nachrichten`.
- Close-to-tray: with a visible tray and `close_to_tray`, closing hides the
  window; a call keeps running. The switch "Fenster schließen minimiert in
  den Infobereich" appears in Benachrichtigungen only with a tray. Not on
  Wayland: winit 0.30 can neither hide the window there nor raise it again,
  so closing ends the app (the tray still shows the dot and "Beenden").
- One instance: a Unix socket in `$XDG_RUNTIME_DIR` (0600), a loopback port
  with a secret on Windows. A second start shows the first window and ends.
- Window: title "Gelabber", app id `gelabber`, icon `icon-192.png`, 1280×800,
  at least 480×360 (as the Tauri window), dark title bar through the window
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
`latest-native.json` of the latest GitHub release (`GELABBER_UPDATE_ENDPOINT`
replaces it) is read. desktop-release.yml writes the same content into
`latest.json`, which the installed Tauri apps (0.8) read, so their updater
installs this app; `latest.json` of 0.8 releases names the Tauri installer.
A newer version shows the notice of the Tauri app's `BANNER`
(`desktop/parity/reference/update-banner.js`: same texts, colours and place,
top centre); what it offers depends on the install:

| Install | Detected by | Notice |
|---|---|---|
| Windows setup | `uninstall.exe` next to the binary, and a `windows-x86_64` entry | "Gelabber x.y.z ist da. Zum Installieren startet die App neu." with "Neu starten" and "Später" |
| pacman package | binary under `/opt/gelabber` (PKGBUILD) | "Gelabber x.y.z ist da. Updates kommen über pacman (Paket gelabber-desktop)." with "Später" |
| anything else (tarball) | | "Gelabber x.y.z ist da." with "Herunterladen" (that version's release page) and "Später" |

"Neu starten" downloads the installer the manifest names (no time limit as
a whole, 30 s to connect, 60 s per read), shows "Update wird geladen … N %",
and keeps it only when `verify` passes: minisign by `desktop/packaging/updater.pub`
(or `GELABBER_UPDATER_PUBKEY` at build time), the trusted comment naming the
announced version (`requireSignedVersion`). A failure stands in the notice
and the buttons work again. Then the window closes and, after
`run_native` returned (state written, single instance released), `main.rs`
starts the setup with ShellExecuteW as the updater plugin does:
`/P /UPDATE /R /ARGS <the app's arguments> /D=<its folder>`, so it installs
passively into the same folder, keeps the shortcuts with their
AppUserModelID and starts the new version with the same arguments.

The call lock works as the Tauri app's `Media::busy` did: during a call (joining,
connected, reconnecting, a channel held) or the microphone test, and for a
minute after, no notice shows. An installer downloaded when a call began
waits and installs once the lock lifts. "Später" hides the version until the
next start.

## AppUserModelID (`aumid.rs`)

On Windows `main.rs` sets the process's id to the bundle id
`io.github.till-und-scholler-ai-slop.gelabber` before the first window: the
same id installer.nsi writes into the shortcuts and toast.rs registers, so
the window groups with a pinned shortcut and the toasts belong to the app.

## Tested headless, and what needs a desktop

`cargo test -p gelabber-ui platform` covers the rules, the files, the
cookie round trip, the single instance (socket and loopback), the
notification service and the portal against fakes on a private
`dbus-daemon` (skipped where it is missing), signatures (throwaway keys:
wrong key, wrong or missing version, tampered file), the manifest, the
call lock, the update check and the installer download against a local
HTTP server, the setup's command line, the notice's texts and buttons,
and a pass of `Platform` without a desktop. The parity scenes
`update-notice` and `update-downloading` compare the notice with the
Tauri app's BANNER, kept in `desktop/parity/reference/` (shots.mjs answers
`update_state` from the scene). `GELABBER_TEST_SERVER=http://127.0.0.1:5180 cargo test -p
gelabber-ui signed_in_after_a_restart -- --ignored` signs in against the
seeded stack, restarts the client from the stored cookie and signs out.

Only a desktop session shows: the tray icon and its menu on a real
StatusNotifier host, a notification click raising the window (on Wayland
the compositor may refuse focus without the activation token, which egui
cannot pass on), close-to-tray and the window coming back, the portal's
key dialog, the window theme and the restored position (Wayland gives
none). The Windows paths (toasts, tray-icon, keyboard hook, the
AppUserModelID, starting the setup) come from the Tauri app's code; they are
linted for Windows in CI's `core-windows`, and the installer over a Tauri
install is tested there after merge. An update end to end from a running
app needs a signed release.
