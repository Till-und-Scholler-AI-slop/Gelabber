import { useState } from "react";

import {
  useChangeNotificationPrivacy,
  useNotificationPrivacy,
} from "../messages/notificationPrivacy.ts";

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
    <fieldset className="settings-group">
      <legend>Sperrbildschirm</legend>
      <label className="settings-preference">
        <span>
          <strong>Inhalt in Benachrichtigungen verbergen</strong>
          <small>
            Benachrichtigungen zeigen nur „Neue Nachricht von …“, ohne Text und
            ohne Chat. Gilt für alle Geräte deines Accounts.
          </small>
        </span>
        <input
          type="checkbox"
          role="switch"
          checked={data?.hide_content ?? false}
          disabled={isPending || saving}
          onChange={(event) => void toggle(event.target.checked)}
        />
      </label>
      {failed && (
        <p role="alert" className="text-sm text-red-600 dark:text-red-400">
          Die Einstellung konnte nicht gespeichert werden. Versuche es erneut.
        </p>
      )}
    </fieldset>
  );
}
