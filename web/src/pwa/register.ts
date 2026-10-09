/** Register only the production browser build. No forced activation or reload. */
export function registerServiceWorker(): void {
  if (!window.isSecureContext || !("serviceWorker" in navigator)) return;
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
