// Runs before React: index.html's inline script already chose the language
// (stored, else the browser's) and set <html lang>. The store starts from
// that choice, so the first render uses the right catalog.
import { isLocale, setLocale } from "./locale.ts";

const chosen = document.documentElement.lang;
if (isLocale(chosen)) setLocale(chosen, { persist: false });
