import { useEffect, useState } from "react";

import {
  notificationPermissionText,
  type NotificationPermissionState,
} from "../messages/notify.ts";
import { desktopNotifications } from "../messages/desktopNotifications.ts";
import { useInstallation } from "../pwa/install.ts";
import { syncPush, type PushResult } from "../pwa/push.ts";
import { Button } from "./ui/Button.tsx";

function permission(): NotificationPermissionState {
  return typeof Notification === "undefined"
    ? "unsupported"
    : Notification.permission;
}

/** In the desktop app the system shows the notifications, no browser
 * permission involved; an older app keeps the webview's. */
function useSystemNotifications(): boolean {
  const [system, setSystem] = useState(false);
  useEffect(() => {
    let live = true;
    void desktopNotifications().then((available) => {
      if (live) setSystem(available);
    });
    return () => {
      live = false;
    };
  }, []);
  return system;
}

export function NotificationPermissionStatus() {
  const system = useSystemNotifications();
  if (system) {
    return (
      <div className="rounded-lg border border-lr-border p-3 text-sm">
        <p role="status">
          Gelabber zeigt Benachrichtigungen über dein System an, wie andere Apps
          auch. Ob sie erscheinen, auf dem Sperrbildschirm oder während „Nicht
          stören“, stellst du in den Einstellungen deines Systems ein.
        </p>
      </div>
    );
  }
  return <BrowserNotificationPermission />;
}

function BrowserNotificationPermission() {
  const [state, setState] = useState(permission);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const device = useInstallation();
  const [push, setPush] = useState<PushResult | null>(null);
  useEffect(() => {
    if (state !== "granted") return;
    let live = true;
    syncPush()
      .catch((): PushResult => "unavailable")
      .then((result) => {
        if (live) setPush(result);
      });
    return () => {
      live = false;
    };
  }, [state]);
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
    <div className="settings-note">
      <p role="status">
        {notificationPermissionText(state, {
          // The desktop app is not bound by the browser's HTTPS rule.
          secure: device.secure || device.native,
          ios: device.ios,
          standalone: device.standalone,
        })}
      </p>
      {state === "granted" && push === "on" ? (
        <p className="mt-2">
          Direktnachrichten und @Erwähnungen kommen auch an, wenn Gelabber
          geschlossen ist.
        </p>
      ) : (
        device.mobile &&
        (state === "granted" || state === "default") &&
        (state === "default" || push !== null) && (
          <p className="mt-2">
            {state === "default"
              ? "Nach der Erlaubnis kommen Direktnachrichten und @Erwähnungen auch an, wenn Gelabber geschlossen ist, sofern der Server das anbietet."
              : "Auf diesem Gerät kommen Benachrichtigungen nur an, solange Gelabber im Hintergrund noch läuft, zum Beispiel während eines Gesprächs. Ist die App geschlossen, kommt nichts an."}
          </p>
        )
      )}
      {state === "default" && (
        <Button
          className="mt-2"
          pending={pending}
          onClick={() => void request()}
        >
          Benachrichtigungen erlauben
        </Button>
      )}
      {failed && (
        <p role="alert">
          Die Berechtigung konnte nicht abgefragt werden. Versuche es erneut.
        </p>
      )}
    </div>
  );
}
