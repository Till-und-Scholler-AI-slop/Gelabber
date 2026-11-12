// The emoji picker (composer and reactions). The emoji's own names and
// search words come from CLDR (src/messages/emoji-data.json, and
// emoji-names.en.json in English), not from here.
import { defineCatalog } from "../catalog.ts";

export const emoji = defineCatalog(
  "emoji",
  {
    "picker.search": "Emoji suchen",
    "picker.group": "Kategorie",
    "picker.allGroups": "Alle Kategorien",
    "picker.found": {
      one: "{count} Emojis gefunden",
      other: "{count} Emojis gefunden",
    },
    "picker.gridLabel": "Emoji-Auswahl",
    "picker.more": "Weitere Emojis anzeigen",
    "picker.licence": "Unicode {version} / CLDR {cldr} – Lizenz",
    "picker.loading": "Emoji-Auswahl wird geladen…",
    "picker.loadFailed": "Die Emoji-Auswahl konnte nicht geladen werden.",
  },
  {
    "picker.search": "Search emoji",
    "picker.group": "Category",
    "picker.allGroups": "All categories",
    "picker.found": {
      one: "{count} emoji found",
      other: "{count} emoji found",
    },
    "picker.gridLabel": "Emoji picker",
    "picker.more": "Show more emoji",
    "picker.licence": "Unicode {version} / CLDR {cldr} – licence",
    "picker.loading": "Loading emoji picker…",
    "picker.loadFailed": "Couldn’t load the emoji picker.",
  },
);
