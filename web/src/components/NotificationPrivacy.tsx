import { useState } from "react";

import {
  useChangeNotificationPrivacy,
  useNotificationPrivacy,
} from "../messages/notificationPrivacy.ts";
import { useT } from "../i18n/catalog.ts";
import { notifications } from "../i18n/catalogs/notifications.ts";
import { SettingsGroup } from "./SettingsGroup.tsx";
import { Switch } from "./ui/Switch.tsx";

/** The account's switch for notifications without the message (#268). */
export function NotificationPrivacySetting() {
  const t = useT(notifications);
  const { data, isPending } = useNotificationPrivacy();
  const change = useChangeNotificationPrivacy();
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const toggle = async (hide_content: boolean) => {
    setSaving(true);
    setFailed(false);
    try {
      await change({ hide_content });
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };
  return (
    <SettingsGroup title={t("privacy.title")} scope="account">
      <Switch
        className="settings-row"
        label={t("privacy.hideContent")}
        description={t("privacy.hideContentHint")}
        checked={data?.hide_content ?? false}
        disabled={isPending || saving}
        onChange={(event) => void toggle(event.target.checked)}
      />
      {failed && (
        <p role="alert" className="ui-field-error">
          {t("privacy.failed")}
        </p>
      )}
    </SettingsGroup>
  );
}
