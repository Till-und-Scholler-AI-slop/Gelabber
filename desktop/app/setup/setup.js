// Bundled setup page: checks and stores the server, then navigates the
// window there. On Windows it also switches the automatic update check and
// checks on request (src/update.rs); the notice of an update found is the
// app's own, over every page. Opened via "Server wechseln" (Ctrl+Shift+S or the web
// client's user menu), it carries the current server in ?server= and offers
// the way back to it; when that server did not answer at start, ?error= says
// why.
const form = document.getElementById("server-form");
const input = document.getElementById("server");
const submit = form.querySelector('button[type="submit"]');
const back = document.getElementById("back");
const error = document.getElementById("error");

async function connect(server) {
  error.hidden = true;
  submit.disabled = back.disabled = true;
  try {
    await window.__TAURI_INTERNALS__.invoke("set_server", { server });
  } catch (failure) {
    error.textContent = String(failure);
    error.hidden = false;
  } finally {
    submit.disabled = back.disabled = false;
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
}
if (failure) {
  error.textContent = failure;
  error.hidden = false;
  back.firstChild.textContent = "Erneut verbinden mit ";
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  connect(input.value);
});

const updates = document.getElementById("updates");
const autoCheck = document.getElementById("auto-check");
const checkNow = document.getElementById("check-now");
const updateStatus = document.getElementById("update-status");

function showUpdates(state) {
  autoCheck.checked = state.autoCheck;
  let line = `Installiert: ${state.current}.`;
  if (state.phase === "checking") line += " Suche läuft …";
  else if (state.available) line += ` Verfügbar: ${state.available}.`;
  else if (state.lastCheck) line += " Das ist die neueste Version.";
  if (state.error) line += ` Letzte Suche: ${state.error}`;
  updateStatus.textContent = line;
}

async function updateCommand(command, args) {
  checkNow.disabled = autoCheck.disabled = true;
  try {
    showUpdates(await window.__TAURI_INTERNALS__.invoke(command, args));
  } catch (failure) {
    updateStatus.textContent = String(failure);
  } finally {
    checkNow.disabled = autoCheck.disabled = false;
  }
}

window.__TAURI_INTERNALS__.invoke("update_state").then((state) => {
  // Linux: the package manager updates the app.
  if (!state.supported) return;
  showUpdates(state);
  updates.hidden = false;
}, () => {});
autoCheck.addEventListener("change", () =>
  updateCommand("update_set_auto_check", { enabled: autoCheck.checked }),
);
checkNow.addEventListener("click", () => updateCommand("update_check"));
