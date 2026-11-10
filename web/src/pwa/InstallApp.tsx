import { ChevronIcon } from "../components/Icons.tsx";
import { useT } from "../i18n/catalog.ts";
import { pwa } from "../i18n/catalogs/pwa.ts";
import { DesktopApp } from "./DesktopApp.tsx";
import {
  promptInstallation,
  useInstallation,
  type InstallState,
} from "./install.ts";
import "./install.css";

/** Settings → App: the desktop app where there is one, then the web app. */
export function InstallApp() {
  return (
    <>
      <DesktopApp />
      <InstallPanel state={useInstallation()} />
    </>
  );
}

/** The same panel behind one line, where signing in comes first. */
export function InstallDisclosure() {
  return <InstallDisclosurePanel state={useInstallation()} />;
}

export function InstallDisclosurePanel({ state }: { state: InstallState }) {
  const t = useT(pwa);
  if (state.native) return null;
  return (
    <details className="pwa-install-disclosure">
      <summary>
        {t("install.disclosure")}
        <ChevronIcon size={14} />
      </summary>
      <InstallPanel state={state} />
    </details>
  );
}

export function InstallPanel({ state }: { state: InstallState }) {
  const t = useT(pwa);
  if (state.native) return null;
  const offered = state.available || state.pending;
  return (
    <section className="pwa-install" aria-label={t("install.label")}>
      <div className="pwa-install-heading">
        <img src="/icons/icon-192.png" alt="" width={48} height={48} />
        <div>
          <h2>{t("install.title")}</h2>
          <p>{state.mobile ? t("install.mobile") : t("install.desktop")}</p>
        </div>
      </div>
      {state.installed ? (
        <p role="status">{t("install.installed")}</p>
      ) : !state.secure ? (
        <p>{t("install.insecure")}</p>
      ) : (
        <>
          {offered ? (
            <button
              type="button"
              className="living-room-primary"
              disabled={state.pending}
              onClick={() => void promptInstallation()}
            >
              {state.pending ? t("install.opening") : t("install.button")}
            </button>
          ) : null}
          {state.accepted && <p role="status">{t("install.accepted")}</p>}
          {state.failed && <p role="alert">{t("install.failed")}</p>}
          {state.ios ? (
            <ol>
              <li>{t("ios.safari")}</li>
              <li>{t("ios.share")}</li>
              <li>{t("ios.add")}</li>
            </ol>
          ) : state.accepted ? null : state.mobile ? (
            <p>{offered ? t("mobile.alsoMenu") : t("mobile.menu")}</p>
          ) : offered ? null : (
            <p>{t("desktopBrowser.where")}</p>
          )}
          <p className="pwa-install-note">{t("install.online")}</p>
        </>
      )}
    </section>
  );
}
