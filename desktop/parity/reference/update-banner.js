// The update notice the Tauri app (removed in 0.9) ran in every page: the
// reference of the parity scenes with `update` (web/scripts/parity/shots.mjs).
(() => {
  if (window.top !== window) return;
  const invoke = (command) => window.__TAURI_INTERNALS__.invoke(command);
  // Polls every 30 s; after a failed call later and later (up to 10 min), and
  // not at all once the app says this page may not ask.
  const EVERY = 30000, AT_MOST = 600000;
  let host, text, bar, restart, later, shown = null, wait = EVERY;
  // The page's theme tokens (custom properties reach into the shadow root).
  const button = (label, primary) => {
    const element = document.createElement("button");
    element.type = "button";
    element.textContent = label;
    element.style.cssText = "flex:none;padding:6px 12px;border-radius:8px;font:inherit;cursor:pointer;"
      + (primary ? "border:0;background:var(--lr-accent);color:var(--lr-accent-ink);font-weight:600"
                 : "border:1px solid var(--lr-border-strong);background:transparent;color:inherit");
    return element;
  };
  // While the update is at work the buttons stay, visibly unavailable.
  const busy = (on) => {
    for (const element of [restart, later]) {
      element.disabled = on;
      element.style.opacity = on ? "0.5" : "";
      element.style.cursor = on ? "default" : "pointer";
    }
  };
  // The thin bar under the line: the share downloaded, hidden otherwise.
  const fill = (state) => {
    const share = state.phase === "downloading" && state.total ? state.downloaded / state.total
      : state.phase === "installing" ? 1 : null;
    bar.parentElement.style.display = share === null ? "none" : "block";
    bar.style.width = `${Math.min(100, Math.max(0, 100 * (share ?? 0)))}%`;
  };
  // What a click on "Neu starten" is at, from the app's state.
  const progress = (state) => {
    if (state.phase === "confirming") return "Bitte im Fenster von Gelabber bestätigen …";
    if (state.phase === "installing") return "Update wird installiert, Gelabber startet neu …";
    if (state.phase !== "downloading") return null;
    const percent = state.total ? ` ${Math.floor((100 * state.downloaded) / state.total)} %` : "";
    return `Update wird geladen …${percent}`;
  };
  const build = () => {
    host = document.createElement("div");
    const root = host.attachShadow({ mode: "closed" });
    const box = document.createElement("div");
    box.setAttribute("role", "status");
    // Top centre, 12px under the 52px header: clear of its title and search,
    // and of the composer and the call dock at the bottom.
    box.style.cssText = "position:fixed;top:64px;left:50%;transform:translateX(-50%);"
      + "z-index:2147483647;display:flex;gap:10px;align-items:center;"
      + "width:max-content;max-width:calc(100vw - 32px);box-sizing:border-box;"
      + "padding:10px 12px;border-radius:12px;border:1px solid var(--lr-border);"
      + "background:var(--lr-panel);color:var(--lr-text);box-shadow:0 6px 24px rgba(0,0,0,.35);"
      + "font:14px/1.4 var(--lr-font)";
    const line = document.createElement("span");
    line.style.cssText = "flex:1;position:relative";
    text = document.createElement("span");
    const track = document.createElement("span");
    track.style.cssText = "display:none;position:absolute;left:0;right:0;top:calc(100% + 3px);"
      + "height:3px;border-radius:2px;background:var(--lr-border)";
    bar = document.createElement("span");
    bar.style.cssText = "display:block;height:100%;width:0;border-radius:2px;background:var(--lr-accent)";
    track.append(bar);
    line.append(text, track);
    restart = button("Neu starten", true);
    later = button("Später", false);
    restart.addEventListener("click", async () => {
      busy(true);
      text.textContent = "Bitte im Fenster von Gelabber bestätigen …";
      const watching = setInterval(async () => {
        const state = await invoke("update_state").catch(() => ({}));
        const message = progress(state);
        if (message) text.textContent = message;
        fill(state);
      }, 1000);
      try {
        // Asks natively first; ends the app when it works, and the
        // installer starts it again.
        await invoke("update_install");
      } catch (error) {
        text.textContent = String(error);
        busy(false);
        fill({});
      } finally {
        clearInterval(watching);
      }
    });
    later.addEventListener("click", () => {
      invoke("update_dismiss").catch(() => {});
      hide();
    });
    box.append(line, restart, later);
    root.append(box);
  };
  const show = (version) => {
    if (!host) build();
    if (shown !== version) {
      text.textContent = `Gelabber ${version} ist da. Zum Installieren startet die App neu.`;
      busy(false);
      fill({});
      shown = version;
    }
    if (!host.isConnected) document.documentElement.append(host);
  };
  const hide = () => {
    host?.remove();
    shown = null;
  };
  const poll = async () => {
    let state;
    try {
      state = await invoke("update_state");
      wait = EVERY;
    } catch (error) {
      // A page without the permission: Tauri's refusal names the command.
      if (/not allowed|denied/i.test(String(error))) return;
      wait = Math.min(wait * 2, AT_MOST);
      setTimeout(poll, wait);
      return;
    }
    if (!state.supported) return;
    if (state.offer) show(state.available);
    // Not while the click on "Neu starten" is at work.
    else if (state.phase === "idle") hide();
    setTimeout(poll, wait);
  };
  if (!window.__TAURI_INTERNALS__) return;
  if (document.readyState === "loading") addEventListener("DOMContentLoaded", poll, { once: true });
  else poll();
})();
