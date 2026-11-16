// Friends, friend codes and message requests (#563): the friend list, the
// requests tab, the friend code, the profile card's button and a request's
// bar in a conversation.
import { defineCatalog } from "../catalog.ts";

export const friends = defineCatalog(
  "friends",
  {
    "nav.friends": "Freunde",
    "nav.requests": "Anfragen",
    "nav.label": "Freunde und Anfragen",
    "page.title": "Freunde",
    "tab.all": "Alle",
    "tab.pending": "Ausstehend",
    "tab.add": "Hinzufügen",
    "tabs.label": "Freundesliste",
    "list.empty":
      "Noch keine Freunde. Schick eine Anfrage per Freundescode oder über das Profil von jemandem.",
    "list.loadFailed": "Die Freundesliste konnte nicht geladen werden.",
    "status.online": "Online",
    "status.idle": "Abwesend",
    "pending.empty": "Keine offenen Anfragen.",
    "pending.incoming": "Eingegangen",
    "pending.outgoing": "Gesendet",
    "action.message": "Nachricht",
    "action.remove": "Entfernen",
    "action.removeConfirm": "{name} aus deinen Freunden entfernen?",
    "action.removeHint":
      "Eure Unterhaltung bleibt erhalten. Ihr könnt euch weiter schreiben.",
    "action.accept": "Annehmen",
    "action.decline": "Ablehnen",
    "action.withdraw": "Zurückziehen",
    "action.failed": "Das hat nicht geklappt.",
    "add.title": "Freund hinzufügen",
    "add.hint":
      "Gib den Freundescode einer Person ein, etwa K7QX-M2PD. Sie sieht deine Anfrage unter „Freunde“.",
    "add.label": "Freundescode",
    "add.submit": "Anfrage senden",
    "add.sent": "Anfrage an {name} gesendet.",
    "add.friends": "Du und {name} seid jetzt befreundet.",
    "send.failed": "Die Anfrage wurde nicht gesendet.",
    "code.title": "Dein Freundescode",
    "code.hint":
      "Wer diesen Code hat, kann dir eine Freundschaftsanfrage schicken.",
    "code.copy": "Kopieren",
    "code.copied": "Freundescode kopiert.",
    "code.copyFailed":
      "Kopieren ging nicht. Markiere den Code und kopiere ihn selbst.",
    "code.regenerate": "Neuen Code erstellen",
    "code.regenerateConfirm": "Neuen Freundescode erstellen?",
    "code.regenerateHint":
      "Der alte Code funktioniert danach nicht mehr. Schon gesendete Anfragen bleiben.",
    "code.regenerateFailed": "Kein neuer Code erstellt.",
    "code.loading": "Code wird geladen …",
    "card.add": "Freund hinzufügen",
    "card.pending": "Anfrage gesendet",
    "card.incoming": "Anfrage annehmen",
    "card.friends": "Befreundet",
    "requests.title": "Nachrichtenanfragen",
    "requests.hint":
      "Leute von deinen Servern, die dir schreiben möchten. Erst wenn du annimmst, landen sie bei deinen Nachrichten.",
    "requests.empty": "Keine Nachrichtenanfragen.",
    "requests.loadFailed": "Die Anfragen konnten nicht geladen werden.",
    "requests.open": "Ansehen",
    "bar.incoming":
      "{name} möchte dir schreiben. Wenn du annimmst, könnt ihr euch frei schreiben. Befreundet seid ihr dadurch nicht.",
    "bar.outgoing": {
      one: "Nachrichtenanfrage: Bis {name} annimmt, geht noch {count} Nachricht, nur Text ohne Links.",
      other:
        "Nachrichtenanfrage: Bis {name} annimmt, gehen noch {count} Nachrichten, nur Text ohne Links.",
    },
    "bar.outgoingFull":
      "Nachrichtenanfrage: Warte, bis {name} annimmt. Mehr Nachrichten gehen erst danach.",
    "newMessage.request": "Nachrichtenanfrage · {servers}",
    "newMessage.friend": "Freund",
  },
  {
    "nav.friends": "Friends",
    "nav.requests": "Requests",
    "nav.label": "Friends and requests",
    "page.title": "Friends",
    "tab.all": "All",
    "tab.pending": "Pending",
    "tab.add": "Add",
    "tabs.label": "Friend list",
    "list.empty":
      "No friends yet. Send a request with a friend code or from someone’s profile.",
    "list.loadFailed": "Couldn’t load your friends.",
    "status.online": "Online",
    "status.idle": "Away",
    "pending.empty": "No open requests.",
    "pending.incoming": "Received",
    "pending.outgoing": "Sent",
    "action.message": "Message",
    "action.remove": "Remove",
    "action.removeConfirm": "Remove {name} from your friends?",
    "action.removeHint":
      "Your conversation stays. You can keep writing to each other.",
    "action.accept": "Accept",
    "action.decline": "Decline",
    "action.withdraw": "Withdraw",
    "action.failed": "That didn’t work.",
    "add.title": "Add a friend",
    "add.hint":
      "Enter someone’s friend code, like K7QX-M2PD. They see your request under “Friends”.",
    "add.label": "Friend code",
    "add.submit": "Send request",
    "add.sent": "Request sent to {name}.",
    "add.friends": "You and {name} are friends now.",
    "send.failed": "The request wasn’t sent.",
    "code.title": "Your friend code",
    "code.hint": "Anyone with this code can send you a friend request.",
    "code.copy": "Copy",
    "code.copied": "Friend code copied.",
    "code.copyFailed": "Couldn’t copy. Select the code and copy it yourself.",
    "code.regenerate": "Create a new code",
    "code.regenerateConfirm": "Create a new friend code?",
    "code.regenerateHint":
      "The old code stops working. Requests already sent stay.",
    "code.regenerateFailed": "No new code created.",
    "code.loading": "Loading code …",
    "card.add": "Add friend",
    "card.pending": "Request sent",
    "card.incoming": "Accept request",
    "card.friends": "Friends",
    "requests.title": "Message requests",
    "requests.hint":
      "People from your servers who want to message you. They join your messages only once you accept.",
    "requests.empty": "No message requests.",
    "requests.loadFailed": "Couldn’t load the requests.",
    "requests.open": "View",
    "bar.incoming":
      "{name} wants to message you. If you accept, you can write freely. It doesn’t make you friends.",
    "bar.outgoing": {
      one: "Message request: until {name} accepts, {count} more message can go, text only without links.",
      other:
        "Message request: until {name} accepts, {count} more messages can go, text only without links.",
    },
    "bar.outgoingFull":
      "Message request: wait until {name} accepts. More messages can go after that.",
    "newMessage.request": "Message request · {servers}",
    "newMessage.friend": "Friend",
  },
);
