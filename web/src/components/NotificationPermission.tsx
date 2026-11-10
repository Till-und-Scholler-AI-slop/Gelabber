import { useEffect, useState } from "react";

import {
  notificationPermissionText,
  type NotificationPermissionState,
} from "../messages/notify.ts";
import { useT } from "../i18n/catalog.ts";
import { notifications } from "../i18n/catalogs/notifications.ts";
import { useInstallation } from "../pwa/install.ts";
import { syncPush, type PushResult } from "../pwa/push.ts";
import { Button } from "./ui/Button.tsx";

function permission(): NotificationPermissionState {
  return typeof Notification === "undefined"
    ? "unsupported"
    : Notification.permission;
}

/** `id` names the status line, for the switch it describes. */
export function NotificationPermissionStatus({ id }: { id?: string }) {
  const t = useT(notifications);
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
    <div className="settings-row-note">
      <p id={id} role="status">
        {notificationPermissionText(state, {
          // The Tauri app is not bound by the browser's HTTPS rule.
          secure: device.secure || device.native,
          ios: device.ios,
          standalone: device.standalone,
        })}
      </p>
      {state === "granted" && push === "on" ? (
        <p className="mt-2">{t("permission.pushOn")}</p>
      ) : (
        device.mobile &&
        (state === "granted" || state === "default") &&
        (state === "default" || push !== null) && (
          <p className="mt-2">
            {state === "default"
              ? t("permission.pushAfterAllow")
              : t("permission.whileRunning")}
          </p>
        )
      )}
      {state === "default" && (
        <Button
          size="sm"
          className="mt-2"
          pending={pending}
          onClick={() => void request()}
        >
          {t("permission.allow")}
        </Button>
      )}
      {failed && <p role="alert">{t("permission.failed")}</p>}
    </div>
  );
}
