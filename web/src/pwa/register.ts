let registered = false;

/** Register only the production browser build. No forced activation or reload. */
export function registerServiceWorker(): void {
  if (!window.isSecureContext || !("serviceWorker" in navigator)) return;
  registered = true;
  const register = () => {
    void navigator.serviceWorker
      .register("/sw.js", { scope: "/", updateViaCache: "none" })
      .catch(() => {
        // Offline/unsupported registration must not prevent login or calls.
        console.warn(
          "Gelabber: Die Offline-Seite konnte nicht eingerichtet werden.",
        );
      });
  };
  if (document.readyState === "complete") register();
  else window.addEventListener("load", register, { once: true });
}

/**
 * The worker this page registered, once it is active. Vite dev and the desktop
 * app never register one and get `undefined` without asking the browser, so
 * everything built on the worker keeps its previous behaviour there.
 */
export async function activeServiceWorker(): Promise<
  ServiceWorkerRegistration | undefined
> {
  if (!registered) return undefined;
  try {
    const registration = await navigator.serviceWorker.getRegistration();
    return registration?.active ? registration : undefined;
  } catch {
    return undefined;
  }
}
