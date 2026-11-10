// The app's frame: the shared controls in components/ui, the sidebars,
// server rail, user panel, quick switcher, shortcuts, toasts, row menus,
// connection notices and the workspace pages. Direct messages have `dms`.
import { defineCatalog } from "../catalog.ts";

export const shell = defineCatalog(
  "shell",
  {
    "password.show": "Passwort anzeigen",
    "password.hide": "Passwort verbergen",
    "password.capsLock": "Feststelltaste ist an",
  },
  {
    "password.show": "Show password",
    "password.hide": "Hide password",
    "password.capsLock": "Caps Lock is on",
  },
);
