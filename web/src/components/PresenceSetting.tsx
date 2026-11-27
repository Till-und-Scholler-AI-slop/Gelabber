import { useT } from "../i18n/catalog.ts";
import { settings } from "../i18n/catalogs/settings.ts";
import {
  disableSystemIdle,
  enableSystemIdle,
  systemIdleSupported,
  useSystemIdle,
} from "../presence/systemIdle.ts";
import { SettingsGroup } from "./SettingsGroup.tsx";
import { Switch } from "./ui/Switch.tsx";

/** When the browser counts the user as away (GEL-105), and the switch for
 * the system's idle detection where the browser has one. */
export function PresenceSetting() {
  const t = useT(settings);
  const { enabled, denied } = useSystemIdle();
  if (!systemIdleSupported()) {
    return (
      <SettingsGroup title={t("presence.title")} scope="device">
        <p className="settings-hint">{t("presence.browserOnly")}</p>
      </SettingsGroup>
    );
  }
  return (
    <SettingsGroup title={t("presence.title")} scope="device">
      <Switch
        className="settings-row"
        label={t("presence.system")}
        description={t("presence.systemHint")}
        checked={enabled}
        onChange={(event) => {
          // The permission prompt needs this click.
          if (event.target.checked) void enableSystemIdle();
          else disableSystemIdle();
        }}
      />
      {denied ? (
        <p role="alert" className="ui-field-error">
          {t("presence.denied")}
        </p>
      ) : enabled ? null : (
        <p className="settings-hint">{t("presence.withoutSystem")}</p>
      )}
    </SettingsGroup>
  );
}
