// "Server wechseln …" in the user menu: the item the web client showed inside
// the Tauri app (removed in 0.9) before "Abmelden", which the native app keeps
// as its own (desktop/ui/src/screens/shell/user_panel.rs). The reference of
// the parity scenes with `desktop` (web/scripts/parity/shots.mjs).
(() => {
  if (window.top !== window) return;
  const label = () =>
    document.documentElement.lang.startsWith("en") ? "Switch server…" : "Server wechseln …";
  const place = () => {
    const nav = document.querySelector("details.user-panel-menu > nav");
    if (!nav) return;
    let item = nav.querySelector(":scope > [data-switch-server]");
    if (!item) {
      item = document.createElement("button");
      item.type = "button";
      item.dataset.switchServer = "";
      // "Abmelden" is always the menu's last item.
      nav.insertBefore(item, nav.lastElementChild);
    }
    if (item.textContent !== label()) item.textContent = label();
  };
  new MutationObserver(place).observe(document, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["lang"],
  });
})();
