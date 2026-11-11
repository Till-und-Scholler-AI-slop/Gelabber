import { defineCatalog } from "../catalog.ts";

// Einstellungen → Sicherheit, from the account package (v0.10, #519): the
// signed-in devices. The section Anmeldung keeps its `settings` keys.
export const account = defineCatalog(
  "account",
  {
    "devices.title": "Angemeldete Geräte",
    "devices.loadFailed": "Deine Geräte konnten nicht geladen werden.",
    "devices.current": "Dieses Gerät",
    "devices.desktop": "Gelabber für {os}",
    "devices.desktopOnly": "Gelabber",
    "devices.browserOn": "{browser} unter {os}",
    "devices.browser": "Browser",
    "devices.unknown": "Unbekanntes Gerät",
    "devices.lastActive": "Zuletzt aktiv: {when}",
    "devices.lastActive.now": "gerade eben",
    "devices.lastActive.today": "heute",
    "devices.lastActive.yesterday": "gestern",
    "devices.lastActive.daysAgo": {
      one: "vor {count} Tag",
      other: "vor {count} Tagen",
    },
    "devices.signOut": "Abmelden",
    "devices.signingOut": "Abmelden…",
  },
  {
    "devices.title": "Signed-in devices",
    "devices.loadFailed": "Your devices could not be loaded.",
    "devices.current": "This device",
    "devices.desktop": "Gelabber for {os}",
    "devices.desktopOnly": "Gelabber",
    "devices.browserOn": "{browser} on {os}",
    "devices.browser": "Browser",
    "devices.unknown": "Unknown device",
    "devices.lastActive": "Last active: {when}",
    "devices.lastActive.now": "just now",
    "devices.lastActive.today": "today",
    "devices.lastActive.yesterday": "yesterday",
    "devices.lastActive.daysAgo": {
      one: "{count} day ago",
      other: "{count} days ago",
    },
    "devices.signOut": "Sign out",
    "devices.signingOut": "Signing out…",
  },
);
