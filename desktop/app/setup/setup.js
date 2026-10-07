// Bundled first-start page: stores the server and navigates the window there.
const form = document.getElementById("server-form");
const input = document.getElementById("server");
const error = document.getElementById("error");

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  error.hidden = true;
  try {
    await window.__TAURI_INTERNALS__.invoke("set_server", {
      server: input.value,
    });
  } catch (failure) {
    error.textContent = String(failure);
    error.hidden = false;
  }
});
