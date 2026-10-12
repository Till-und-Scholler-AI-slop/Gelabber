import { useEffect, useState } from "react";

import {
  notificationPermissionText,
  type NotificationPermissionState,
} from "../messages/notify.ts";
import { useInstallation } from "../pwa/install.ts";

function permission(): NotificationPermissionState {
  return typeof Notification === "undefined"
    ? "unsupported"
    : Notification.permission;
}

export function NotificationPermissionStatus() {
  const [state, setState] = useState(permission);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const device = useInstallation();
  useEffect(() => {
    const refresh = () => setState(permission());
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, []);
  const request = async () => {
    if (state === "unsupported") return;
    setPending(true);
    setFailed(false);
    try {
      // Called straight from the click: browsers only ask on a user gesture.
      // Older Safari answers through a callback and returns nothing.
      setState((await Notification.requestPermission()) ?? permission());
    } catch {
      setFailed(true);
    } finally {
      setPending(false);
    }
  };
  return (
    <div className="rounded-lg border border-neutral-300 p-3 text-sm dark:border-neutral-700">
      <p role="status">
        {notificationPermissionText(state, {
          // The desktop app is not bound by the browser's HTTPS rule.
          secure: device.secure || device.native,
          ios: device.ios,
          standalone: device.standalone,
        })}
      </p>
      {device.mobile && (state === "granted" || state === "default") && (
        <p className="mt-2">
          Auf dem Handy kommen Benachrichtigungen nur an, solange Gelabber im
          Hintergrund noch läuft, zum Beispiel während eines Gesprächs. Ist die
          App geschlossen, kommt nichts an.
        </p>
      )}
      {state === "default" && (
        <button
          type="button"
          disabled={pending}
          onClick={() => void request()}
          className="mt-2 rounded border px-3 py-2 font-medium disabled:opacity-50"
        >
          Benachrichtigungen erlauben
        </button>
      )}
      {failed && (
        <p role="alert">
          Die Berechtigung konnte nicht abgefragt werden. Versuche es erneut.
        </p>
      )}
    </div>
  );
}
