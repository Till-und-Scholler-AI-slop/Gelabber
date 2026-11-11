import { defineCatalog } from "../catalog.ts";

// Einstellungen → Sicherheit, from the account package (v0.10, #519): the
// signed-in devices, the data export and deleting the account. The section
// Anmeldung keeps its `settings` keys.
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

    "data.title": "Kontodaten",
    "export.title": "Daten exportieren",
    "export.loadFailed":
      "Der Stand deines Exports konnte nicht geladen werden.",
    "export.hint.none": "Deine Nachrichten, Dateien und Kontodaten als Archiv.",
    "export.hint.mail": "Du bekommst eine E-Mail, sobald es bereitsteht.",
    "export.hint.noMail":
      "Du kannst es hier herunterladen, sobald es bereitsteht.",
    "export.hint.building": "Wird erstellt. Das kann eine Weile dauern.",
    "export.hint.ready": "Bereit bis {date}, {size}.",
    "export.hint.failed": "Der Export ist fehlgeschlagen.",
    "export.request": "Export anfordern",
    "export.building": "Wird erstellt…",
    "export.download": "Herunterladen",
    "export.retry": "Erneut anfordern",
    "export.tooSoon": "Einen neuen Export kannst du {when} anfordern.",

    "delete.title": "Konto löschen",
    "delete.hint":
      "Endgültig. Server, die dir gehören, übergibst oder löschst du vorher.",
    "delete.owned.lead":
      "Dir gehören noch diese Server. Übergib sie an ein anderes Mitglied oder lösche sie, bevor du dein Konto löschst.",
    "delete.owned.members": {
      one: "{count} Mitglied",
      other: "{count} Mitglieder",
    },
    "delete.owned.settings": "Servereinstellungen",
    "delete.confirm.title": "Konto endgültig löschen?",
    "delete.confirm.lead":
      "Deine Nachrichten, Dateien, Direktnachrichten und Reaktionen werden gelöscht. Du verlässt alle Server. Das lässt sich nicht rückgängig machen.",
    "delete.confirm.exportHint":
      "Exportiere deine Daten vorher, wenn du sie behalten willst.",
    "delete.password": "Passwort",
    "delete.email": "E-Mail-Adresse",
    "delete.emailHint":
      "Dein Konto hat kein Passwort. Gib zur Bestätigung seine E-Mail-Adresse ein.",
    "delete.emailWrong": "Das ist nicht die Adresse deines Kontos.",
    "delete.deleting": "Löschen…",
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

    "data.title": "Account data",
    "export.title": "Export data",
    "export.loadFailed": "The state of your export could not be loaded.",
    "export.hint.none": "Your messages, files and account data as an archive.",
    "export.hint.mail": "You will get an email as soon as it is ready.",
    "export.hint.noMail": "You can download it here as soon as it is ready.",
    "export.hint.building": "Being prepared. This can take a while.",
    "export.hint.ready": "Ready until {date}, {size}.",
    "export.hint.failed": "The export failed.",
    "export.request": "Request export",
    "export.building": "Being prepared…",
    "export.download": "Download",
    "export.retry": "Request again",
    "export.tooSoon": "You can request a new export {when}.",

    "delete.title": "Delete account",
    "delete.hint":
      "Final. Hand over or delete the servers you own before you do this.",
    "delete.owned.lead":
      "You still own these servers. Hand them over to another member or delete them before you delete your account.",
    "delete.owned.members": {
      one: "{count} member",
      other: "{count} members",
    },
    "delete.owned.settings": "Server settings",
    "delete.confirm.title": "Delete your account for good?",
    "delete.confirm.lead":
      "Your messages, files, direct messages and reactions will be deleted. You will leave every server. This cannot be undone.",
    "delete.confirm.exportHint":
      "Export your data first if you want to keep it.",
    "delete.password": "Password",
    "delete.email": "Email address",
    "delete.emailHint":
      "Your account has no password. Enter its email address to confirm.",
    "delete.emailWrong": "That is not your account’s address.",
    "delete.deleting": "Deleting…",
  },
);
