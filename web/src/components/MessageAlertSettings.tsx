import { isDesktopApp } from "../voice/native/bridge.ts";
import { useMediaSettings } from "../voice/settings.ts";
import { CloseToTray } from "./CloseToTray.tsx";
import { SettingsGroup } from "./SettingsGroup.tsx";
import { Switch } from "./ui/Switch.tsx";

/** How new messages reach this device: popups, system notifications, sound. */
export function MessageAlertSettings() {
  const settings = useMediaSettings();
  // The desktop app's notifications come from its window, not a browser tab.
  const desktop = isDesktopApp();
  return (
    <SettingsGroup title="Nachrichten" scope="device">
      <Switch
        id="message-toasts"
        className="settings-row"
        label="Popup bei neuen Nachrichten in anderen Chats"
        checked={settings.messageToasts}
        onChange={(event) =>
          settings.patch({ messageToasts: event.target.checked })
        }
      />
      <Switch
        id="desktop-notify"
        className="settings-row"
        label={
          desktop
            ? "Benachrichtigung, wenn das Gelabber-Fenster nicht im Vordergrund ist"
            : "Browser-Benachrichtigung, wenn der Tab nicht im Vordergrund ist"
        }
        checked={settings.desktopNotify}
        onChange={(event) => {
          const desktopNotify = event.target.checked;
          settings.patch({ desktopNotify });
          if (desktopNotify && typeof Notification !== "undefined") {
            void Notification.requestPermission();
          }
        }}
      />
      <Switch
        id="message-sound"
        className="settings-row"
        label="Ton bei Benachrichtigungen am Computer"
        description="Ein Ton für mehrere Nachrichten kurz hintereinander, keiner, während du taubgeschaltet bist. Auf dem Handy gilt die Geräteeinstellung."
        checked={settings.messageSound}
        disabled={!settings.desktopNotify}
        onChange={(event) =>
          settings.patch({ messageSound: event.target.checked })
        }
      />
      {desktop && <CloseToTray />}
    </SettingsGroup>
  );
}
