import { useT } from "../i18n/catalog.ts";
import { notifications } from "../i18n/catalogs/notifications.ts";
import { useMediaSettings } from "../voice/settings.ts";
import { NotificationPermissionStatus } from "./NotificationPermission.tsx";
import { SettingsGroup } from "./SettingsGroup.tsx";
import { Switch } from "./ui/Switch.tsx";

/** How new messages reach this device: popups, system notifications, sound. */
export function MessageAlertSettings() {
  const t = useT(notifications);
  const settings = useMediaSettings();
  return (
    <SettingsGroup title={t("alerts.title")} scope="device">
      <Switch
        id="message-toasts"
        className="settings-row"
        label={t("alerts.toasts")}
        checked={settings.messageToasts}
        onChange={(event) =>
          settings.patch({ messageToasts: event.target.checked })
        }
      />
      {/* What the browser allows sits under the switch it decides. */}
      <div className="settings-row-noted">
        <Switch
          id="desktop-notify"
          className="settings-row"
          label={t("alerts.desktop")}
          aria-describedby="desktop-notify-note"
          checked={settings.desktopNotify}
          onChange={(event) => {
            const desktopNotify = event.target.checked;
            settings.patch({ desktopNotify });
            if (desktopNotify && typeof Notification !== "undefined") {
              void Notification.requestPermission();
            }
          }}
        />
        <NotificationPermissionStatus id="desktop-notify-note" />
      </div>
      <Switch
        id="message-sound"
        className="settings-row"
        label={t("alerts.sound")}
        description={t("alerts.soundHint")}
        checked={settings.messageSound}
        disabled={!settings.desktopNotify}
        onChange={(event) =>
          settings.patch({ messageSound: event.target.checked })
        }
      />
    </SettingsGroup>
  );
}
