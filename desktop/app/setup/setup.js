// Bundled setup page: checks and stores the server, then navigates the
// window there. Opened from the menu ("Server wechseln"), it carries the
// current server in ?server= and offers the way back to it.
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

const current = new URLSearchParams(location.search).get("server");
if (current) {
  input.value = current;
  back.querySelector("span").textContent = new URL(current).host;
  back.hidden = false;
  back.addEventListener("click", () => connect(current));
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  connect(input.value);
});
