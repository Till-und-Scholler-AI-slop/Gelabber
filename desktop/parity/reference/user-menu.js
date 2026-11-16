// "Server wechseln …" in the account menu: the entry the web client showed
// inside the Tauri app (removed in 0.9) before the line above "Abmelden",
// which the native app keeps as its own
// (desktop/ui/src/screens/shell/user_panel.rs). The reference of the parity
// scenes with `desktop` (web/scripts/parity/shots.mjs).
(() => {
  if (window.top !== window) return;
  const label = () =>
    document.documentElement.lang.startsWith("en") ? "Switch server…" : "Server wechseln …";
  const place = () => {
    const menu = document.querySelector(".user-account-menu");
    if (!menu) return;
    let item = menu.querySelector(":scope > [data-switch-server]");
    if (!item) {
      item = document.createElement("button");
      item.type = "button";
      item.role = "menuitem";
      item.className = "shell-menu-item";
      item.dataset.switchServer = "";
      // "Abmelden" is always the menu's last entry, after its line.
      menu.insertBefore(item, menu.lastElementChild.previousElementSibling);
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
