import { useEffect, useState } from "react";

function permission(): NotificationPermission | "unsupported" {
  return typeof Notification === "undefined"
    ? "unsupported"
    : Notification.permission;
}

export function NotificationPermissionStatus() {
  const [state, setState] = useState(permission);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
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
      setState(await Notification.requestPermission());
    } catch {
      setFailed(true);
    } finally {
      setPending(false);
    }
  };
  return (
    <div className="rounded-lg border border-neutral-300 p-3 text-sm dark:border-neutral-700">
      <p role="status">
        {state === "granted"
          ? "Browser-Benachrichtigungen sind erlaubt."
          : state === "denied"
            ? "Browser-Benachrichtigungen sind blockiert. Du kannst sie in den Website-Einstellungen deines Browsers erlauben."
            : state === "unsupported"
              ? "Dieser Browser unterstützt hier keine Desktop-Benachrichtigungen."
              : "Der Browser benötigt noch deine Erlaubnis für Benachrichtigungen."}
      </p>
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
