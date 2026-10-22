import { useState } from "react";

import {
  useChangeNotificationPrivacy,
  useNotificationPrivacy,
} from "../messages/notificationPrivacy.ts";
import { SettingsGroup } from "./SettingsGroup.tsx";
import { Switch } from "./ui/Switch.tsx";

/** The account's switch for notifications without the message (#268). */
export function NotificationPrivacySetting() {
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
    <SettingsGroup title="Sperrbildschirm" scope="account">
      <Switch
        className="settings-row"
        label="Inhalt in Benachrichtigungen verbergen"
        description="Benachrichtigungen zeigen nur „Neue Nachricht von …“, ohne Text und ohne Chat. Gilt für alle Geräte deines Accounts."
        checked={data?.hide_content ?? false}
        disabled={isPending || saving}
        onChange={(event) => void toggle(event.target.checked)}
      />
      {failed && (
        <p role="alert" className="ui-field-error">
          Die Einstellung konnte nicht gespeichert werden. Versuche es erneut.
        </p>
      )}
    </SettingsGroup>
  );
}
