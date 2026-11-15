// Blocking people (#565): the button and its question, the "Blockiert"
// settings area, a blocked DM's composer and folded messages on servers.
import { defineCatalog } from "../catalog.ts";

export const blocks = defineCatalog(
  "blocks",
  {
    "action.block": "Blockieren",
    "action.unblock": "Blockierung aufheben",
    "confirm.title": "{name} blockieren?",
    "confirm.message":
      "Die Person kann dir nicht mehr schreiben und keine Freundschaftsanfrage schicken. Du bekommst keine Benachrichtigungen mehr von ihr, und ihre Nachrichten auf gemeinsamen Servern werden eingeklappt. Sie erfährt nichts davon.",
    "block.failed": "Nicht blockiert.",
    "unblock.failed": "Blockierung nicht aufgehoben.",
    "area.hint":
      "Wen du blockierst, kann dir nicht schreiben und keine Freundschaftsanfrage schicken. Die Person erfährt es nicht.",
    "area.empty": "Du blockierst niemanden.",
    "area.loadFailed": "Die Liste konnte nicht geladen werden.",
    "area.since": "Seit {date}",
    "composer.blocked": "Du hast {name} blockiert.",
    "composer.closed":
      "Du kannst in dieser Unterhaltung keine Nachrichten senden.",
    folded: {
      one: "Blockierte Nachricht",
      other: "{count} blockierte Nachrichten",
    },
    "folded.show": "Anzeigen",
  },
  {
    "action.block": "Block",
    "action.unblock": "Unblock",
    "confirm.title": "Block {name}?",
    "confirm.message":
      "They can’t message you or send friend requests. You’ll stop getting their notifications, and their messages on shared servers are collapsed. They are not told.",
    "block.failed": "Not blocked.",
    "unblock.failed": "Not unblocked.",
    "area.hint":
      "People you block can’t message you or send friend requests. They are not told.",
    "area.empty": "You don’t block anyone.",
    "area.loadFailed": "Couldn’t load the list.",
    "area.since": "Since {date}",
    "composer.blocked": "You blocked {name}.",
    "composer.closed": "You can’t send messages in this conversation.",
    folded: {
      one: "Blocked message",
      other: "{count} blocked messages",
    },
    "folded.show": "Show",
  },
);
