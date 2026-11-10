// Direct messages: their sidebar, the list page, a conversation's page and
// the "Neue Nachricht" dialog.
import { defineCatalog } from "../catalog.ts";

export const dms = defineCatalog(
  "dms",
  {
    "sidebar.label": "Direktnachrichten",
    "sidebar.title": "Nachrichten",
    "sidebar.empty": "Noch keine Unterhaltungen.",
    "list.label": "Unterhaltungen",
    newMessage: "Neue Nachricht",
    "newMessage.person": "Person",
    "newMessage.placeholder": "Name suchen",
    "newMessage.continue": "Unterhaltung fortsetzen",
    "newMessage.nobody":
      "Keine Personen. Direktnachrichten gehen an Mitglieder deiner Server.",
    "newMessage.notFound": "Niemand gefunden für „{query}“.",
    "open.failed": "Unterhaltung nicht geöffnet.",
  },
  {
    "sidebar.label": "Direct messages",
    "sidebar.title": "Messages",
    "sidebar.empty": "No conversations yet.",
    "list.label": "Conversations",
    newMessage: "New message",
    "newMessage.person": "Person",
    "newMessage.placeholder": "Search for a name",
    "newMessage.continue": "Continue conversation",
    "newMessage.nobody":
      "Nobody here. Direct messages go to members of your servers.",
    "newMessage.notFound": "Nobody found for “{query}”.",
    "open.failed": "Conversation not opened.",
  },
);
