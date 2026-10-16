import { useEffect, useState } from "react";

import { desktopShellInfo, setCloseToTray } from "../messages/desktopShell.ts";

/** Desktop app with a tray only: closing the window hides it there. */
export function CloseToTray() {
  const [state, setState] = useState<{ tray: boolean; on: boolean } | null>(
    null,
  );
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    void desktopShellInfo().then((info) => {
      if (active && info) setState({ tray: info.tray, on: info.close_to_tray });
    });
    return () => {
      active = false;
    };
  }, []);
  if (!state?.tray) return null;
  return (
    <>
      <label
        htmlFor="close-to-tray"
        className="flex items-center gap-2 text-sm"
      >
        <input
          id="close-to-tray"
          type="checkbox"
          checked={state.on}
          onChange={(event) => {
            const on = event.target.checked;
            setFailed(false);
            setState({ ...state, on });
            void setCloseToTray(on).then((ok) => {
              if (ok) return;
              setFailed(true);
              setState({ ...state, on: !on });
            });
          }}
        />
        Fenster schließen minimiert in den Infobereich
      </label>
      <p className="text-xs text-neutral-600 dark:text-neutral-400">
        Ein laufender Anruf geht weiter. Beenden über das Symbol im Infobereich.
      </p>
      {failed && (
        <p role="alert" className="text-xs">
          Die Einstellung konnte nicht gespeichert werden.
        </p>
      )}
    </>
  );
}
