import { useEffect, useState } from "react";

import { desktopShellInfo, setCloseToTray } from "../messages/desktopShell.ts";
import { Switch } from "./ui/Switch.tsx";

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
      <Switch
        id="close-to-tray"
        className="settings-row"
        label="Fenster schließen minimiert in den Infobereich"
        description="Ein laufender Anruf geht weiter. Beenden über das Symbol im Infobereich."
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
      {failed && (
        <p role="alert" className="ui-field-error">
          Die Einstellung konnte nicht gespeichert werden.
        </p>
      )}
    </>
  );
}
