// The setup page as the desktop app draws it (desktop/ui/src/setup.rs), in
// the sign-in pages' frame: checks and stores the server, then opens it.
// Opened via "Server wechseln" (Ctrl+Shift+S or the user menu), it carries
// the current server in ?server= and offers the way back to it; when that
// server did not answer at start, ?error= says why in words and ?detail=
// what the system said. Where the app installs updates itself, it shows
// their state and installs one that is there.
const form = document.getElementById("server-form");
const input = document.getElementById("server");
const note = document.getElementById("server-note");
const detail = document.getElementById("detail");
const submit = form.querySelector('button[type="submit"]');
const back = document.getElementById("back");
const GUIDANCE = note.textContent.trim();

function showError(message, why) {
  input.setAttribute("aria-invalid", message ? "true" : "false");
  note.textContent = message || GUIDANCE;
  note.classList.toggle("error", Boolean(message));
  note.setAttribute("role", message ? "alert" : "none");
  detail.textContent = why ? `Details: ${why}` : "";
  detail.hidden = !why;
}

async function connect(server) {
  showError(null);
  submit.disabled = back.disabled = true;
  submit.textContent = "Verbinde…";
  try {
    await window.__TAURI_INTERNALS__.invoke("set_server", { server });
  } catch (failure) {
    showError("Der Server ist nicht erreichbar.", String(failure));
  } finally {
    submit.disabled = back.disabled = false;
    submit.textContent = "Verbinden";
  }
}

const params = new URLSearchParams(location.search);
const current = params.get("server");
const failure = params.get("error");
if (current) {
  input.value = current;
  back.querySelector("span").textContent = new URL(current).host;
  back.hidden = false;
  back.addEventListener("click", () => connect(current));
  if (!failure) document.getElementById("title").textContent = "Server wechseln";
}
if (failure) {
  showError(failure, params.get("detail"));
  back.firstChild.textContent = "Erneut verbinden mit ";
}
input.addEventListener("input", () => showError(null));

form.addEventListener("submit", (event) => {
  event.preventDefault();
  connect(input.value);
});

const updates = document.getElementById("updates");
const action = document.getElementById("update-action");
const updateStatus = document.getElementById("update-status");
let installable = false;

function showUpdates(state) {
  let line = `Installiert: ${state.current}.`;
  if (state.phase === "checking") line += " Suche läuft …";
  else if (state.available) line += ` Verfügbar: ${state.available}.`;
  else if (state.lastCheck) line += " Das ist die neueste Version.";
  if (state.error) line += ` Letzte Suche: ${state.error}`;
  updateStatus.textContent = line;
  installable = Boolean(state.available);
  action.textContent = installable ? "Jetzt installieren" : "Nach Updates suchen";
  action.className = installable ? "secondary" : "ghost";
}

async function updateCommand(command) {
  action.disabled = true;
  try {
    showUpdates(await window.__TAURI_INTERNALS__.invoke(command));
  } catch (failure) {
    updateStatus.textContent = String(failure);
  } finally {
    action.disabled = false;
  }
}

window.__TAURI_INTERNALS__.invoke("update_state").then((state) => {
  // Linux and the tarball: a package manager or the user updates the app.
  if (!state.supported) return;
  showUpdates(state);
  updates.hidden = false;
}, () => {});
action.addEventListener("click", () =>
  updateCommand(installable ? "update_install" : "update_check"),
);
