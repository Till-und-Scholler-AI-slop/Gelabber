// The language itself: the "Sprache" settings section, and the texts the
// static pages show before the app runs (index.html's splash,
// public/offline.html). Those pages carry each text twice, in blocks tagged
// lang="de" and lang="en", and CSS shows the one <html lang> names, so the
// German one shows without script; src/bootSplash.test.mjs holds them
// equal to this catalog.
import { defineCatalog } from "../catalog.ts";

export const language = defineCatalog(
  "language",
  {
    "section.title": "Sprache",
    // Each language is named in its own language on its option.
    "option.de": "Deutsch",
    "option.en": "English",
    "name.de": "Deutsch",
    "name.en": "Englisch",
    hint: "Gilt auf allen deinen Geräten, auch für Benachrichtigungen und E-Mails von Gelabber. Nachrichten anderer werden nicht übersetzt.",
    "splash.connecting": "Verbinde…",
    "splash.failed": "Gelabber konnte nicht geladen werden.",
    "splash.reload": "Seite neu laden",
    "offline.title": "Keine Verbindung · Gelabber",
    "offline.heading": "Gerade keine Verbindung",
    "offline.body":
      "Für Nachrichten und Gespräche brauchst du eine Internetverbindung. Sobald du wieder online bist, lädt Gelabber diese Seite von selbst neu.",
    "offline.retry": "Erneut verbinden",
  },
  {
    "section.title": "Language",
    "option.de": "Deutsch",
    "option.en": "English",
    "name.de": "German",
    "name.en": "English",
    hint: "Applies on all your devices, including notifications and emails from Gelabber. Other people’s messages are not translated.",
    "splash.connecting": "Connecting…",
    "splash.failed": "Gelabber could not be loaded.",
    "splash.reload": "Reload page",
    "offline.title": "No connection · Gelabber",
    "offline.heading": "No connection right now",
    "offline.body":
      "Messages and calls need an internet connection. As soon as you are back online, Gelabber reloads this page by itself.",
    "offline.retry": "Reconnect",
  },
);
