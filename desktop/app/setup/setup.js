// Bundled setup page: checks and stores the server, then navigates the
// window there. Opened via "Server wechseln" (Ctrl+Shift+S or the web
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
