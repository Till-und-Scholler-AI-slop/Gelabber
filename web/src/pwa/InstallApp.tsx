import { promptInstallation, useInstallation } from "./install.ts";
import "./install.css";

export function InstallApp() {
  const state = useInstallation();
  if (state.native) return null;
  return (
    <section className="pwa-install" aria-label="Gelabber installieren">
      <div className="pwa-install-heading">
        <img src="/icons/icon-192.png" alt="" width={48} height={48} />
        <div>
          <h2>Gelabber als App</h2>
          <p>Direkt vom Startbildschirm öffnen.</p>
        </div>
      </div>
      {state.installed ? (
        <p role="status">
          Gelabber ist bereits als App geöffnet oder installiert.
        </p>
      ) : !state.secure ? (
        <p>
          Öffne Gelabber über HTTPS, damit du es als App installieren kannst.
        </p>
      ) : (
        <>
          {state.available || state.pending ? (
            <button
              type="button"
              className="living-room-primary"
              disabled={state.pending}
              onClick={() => void promptInstallation()}
            >
              {state.pending ? "Installation öffnen…" : "App installieren"}
            </button>
          ) : null}
          {state.accepted && (
            <p role="status">
              Installation bestätigt. Dein Browser richtet die App ein.
            </p>
          )}
          {state.failed && (
            <p role="alert">
              Die Installation konnte nicht geöffnet werden. Nutze das Menü
              deines Browsers oder versuche es später erneut.
            </p>
          )}
          {state.ios ? (
            <ol>
              <li>Öffne Gelabber in Safari.</li>
              <li>Tippe auf „Teilen“, dann auf „Zum Home-Bildschirm“.</li>
              <li>
                Aktiviere „Als Web-App öffnen“, falls angezeigt, und tippe auf
                „Hinzufügen“.
              </li>
            </ol>
          ) : (
            <p>
              Du kannst auch im Browsermenü „App installieren“ oder „Zum
              Startbildschirm hinzufügen“ wählen. Falls dein Browser das nicht
              anbietet, öffne Gelabber in Chrome auf Android oder Safari auf dem
              iPhone/iPad.
            </p>
          )}
          <p className="pwa-install-note">
            Für Chat und Gespräche brauchst du weiterhin eine
            Internetverbindung.
          </p>
        </>
      )}
    </section>
  );
}
