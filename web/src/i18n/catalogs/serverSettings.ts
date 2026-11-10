// Server settings (`/s/$serverId/settings`): name and picture, member
// permissions, invites, members, bans, leaving and deleting.
import { defineCatalog } from "../catalog.ts";

export const serverSettings = defineCatalog(
  "serverSettings",
  {
    title: "Servereinstellungen",
    loadFailed: "Die Servereinstellungen konnten nicht geladen werden.",
    navLabel: "Abschnitte",

    // The section anchors; the URL fragments stay German.
    "section.general": "Allgemein",
    "section.permissions": "Rechte",
    "section.invites": "Einladungen",
    "section.members": "Mitglieder",
    "section.bans": "Sperren",
    "section.danger": "Gefahrenzone",

    "general.hint":
      "Ein neues Bild gilt sofort, den Namen übernimmst du mit Speichern.",
    "general.nameLabel": "Servername",
    "general.saving": "Speichert…",
    "general.unchanged": "Keine Änderungen.",
    "icon.upload": "Bild hochladen",
    "icon.uploading": "Lädt hoch…",
    "icon.removing": "Entfernt…",
    "icon.pickLabel": "Serverbild auswählen",

    "permissions.title": "Rechte für Mitglieder",
    "permissions.hint":
      "Gilt für alle Mitglieder außer dem Inhaber. Änderungen wirken sofort.",
    "permission.manageServer.label": "Server verwalten",
    "permission.manageServer.hint":
      "Umbenennen, Rechte ändern, Einladungen, Kick und Ban.",
    "permission.manageChannels.label": "Kanäle verwalten",
    "permission.manageChannels.hint":
      "Kategorien und Kanäle anlegen, umbenennen, löschen.",
    "permission.manageMessages.label": "Nachrichten löschen",
    "permission.manageMessages.hint": "Fremde Nachrichten im Chat löschen.",
    "permission.mentionEveryone.label": "@everyone und @here",
    "permission.mentionEveryone.hint":
      "Alle Mitglieder auf einmal erwähnen und benachrichtigen.",
    "permission.sendMessages.label": "Nachrichten schreiben",
    "permission.sendMessages.hint": "In Textkanälen schreiben.",
    "permission.sendFiles.label": "Dateien senden",
    "permission.sendFiles.hint": "Anhänge hochladen.",
    "permission.joinVoice.label": "Sprachkanälen beitreten",
    "permission.joinVoice.hint":
      "In Sprachkanälen reden, zuhören und die Kamera zeigen.",
    "permission.goLive.label": "Live gehen",
    "permission.goLive.hint":
      "Im Sprachkanal einen Bildschirm oder ein Fenster für alle dort zeigen.",

    "invites.hint": "Alle aktiven Links. Jedes Mitglied kann Links erstellen.",
    "invites.create": "Neuen Link erstellen",
    "invites.copyFailed": "Kopieren ging nicht. Der Code ist {code}.",
    "invites.loading": "Lade Einladungen…",
    "invites.loadFailed": "Die Einladungen konnten nicht geladen werden.",
    "invites.empty": "Keine aktiven Einladungen.",
    "invites.revokeTitle": "Link widerrufen?",
    "invites.revokeMessage":
      "Der Link funktioniert danach nicht mehr. Bestehende Mitglieder bleiben.",
    "invites.revoke": "Widerrufen",
    "invites.revoked": "Link widerrufen.",

    "members.title": "Mitglieder ({count})",
    "members.owner": "Inhaber",
    "members.member": "Mitglied",

    "bans.title": "Gesperrte Konten",
    "bans.hint":
      "Gesperrte User können auch mit einem Einladungslink nicht wieder beitreten.",
    "bans.loading": "Lade Sperren…",
    "bans.loadFailed": "Die Sperren konnten nicht geladen werden.",
    "bans.empty": "Niemand ist gesperrt.",
    "bans.unbanTitle": "{name} wieder zulassen?",
    "bans.unbanMessage":
      "Ein Beitritt per Einladungslink ist dann wieder möglich.",
    "bans.unban": "Entsperren",

    "danger.ownerHint":
      "Löscht Kanäle, Kategorien, Nachrichten, Einladungen und Mitgliedschaften endgültig.",
    "danger.memberHint":
      "Wieder beitreten kannst du nur mit einem neuen Einladungslink.",
    "danger.deleteTitle": "Server „{name}“ endgültig löschen?",
    "danger.deleteMessage":
      "Kanäle, Nachrichten, Einladungen und Mitgliedschaften werden gelöscht.",
    "danger.delete": "Server löschen",
    "danger.leaveTitle": "Server „{name}“ verlassen?",
    "danger.leaveConfirm": "Verlassen",
    "danger.leave": "Server verlassen",
  },
  {
    title: "Server settings",
    loadFailed: "The server settings couldn’t be loaded.",
    navLabel: "Sections",

    "section.general": "General",
    "section.permissions": "Permissions",
    "section.invites": "Invites",
    "section.members": "Members",
    "section.bans": "Bans",
    "section.danger": "Danger zone",

    "general.hint":
      "A new picture applies at once; the name applies when you save.",
    "general.nameLabel": "Server name",
    "general.saving": "Saving…",
    "general.unchanged": "No changes.",
    "icon.upload": "Upload picture",
    "icon.uploading": "Uploading…",
    "icon.removing": "Removing…",
    "icon.pickLabel": "Choose server picture",

    "permissions.title": "Member permissions",
    "permissions.hint":
      "Applies to every member except the owner. Changes take effect at once.",
    "permission.manageServer.label": "Manage server",
    "permission.manageServer.hint":
      "Rename, change permissions, invites, kick and ban.",
    "permission.manageChannels.label": "Manage channels",
    "permission.manageChannels.hint":
      "Create, rename and delete categories and channels.",
    "permission.manageMessages.label": "Delete messages",
    "permission.manageMessages.hint": "Delete other people’s messages in chat.",
    "permission.mentionEveryone.label": "@everyone and @here",
    "permission.mentionEveryone.hint":
      "Mention and notify every member at once.",
    "permission.sendMessages.label": "Send messages",
    "permission.sendMessages.hint": "Write in text channels.",
    "permission.sendFiles.label": "Send files",
    "permission.sendFiles.hint": "Upload attachments.",
    "permission.joinVoice.label": "Join voice channels",
    "permission.joinVoice.hint":
      "Talk, listen and show your camera in voice channels.",
    "permission.goLive.label": "Go Live",
    "permission.goLive.hint":
      "Show a screen or window to everyone in the voice channel.",

    "invites.hint": "All active links. Any member can create links.",
    "invites.create": "Create new link",
    "invites.copyFailed": "Couldn’t copy. The code is {code}.",
    "invites.loading": "Loading invites…",
    "invites.loadFailed": "The invites couldn’t be loaded.",
    "invites.empty": "No active invites.",
    "invites.revokeTitle": "Revoke link?",
    "invites.revokeMessage": "The link stops working. Existing members stay.",
    "invites.revoke": "Revoke",
    "invites.revoked": "Link revoked.",

    "members.title": "Members ({count})",
    "members.owner": "Owner",
    "members.member": "Member",

    "bans.title": "Banned accounts",
    "bans.hint": "Banned people can’t rejoin, not even with an invite link.",
    "bans.loading": "Loading bans…",
    "bans.loadFailed": "The bans couldn’t be loaded.",
    "bans.empty": "Nobody is banned.",
    "bans.unbanTitle": "Let {name} back in?",
    "bans.unbanMessage": "They can then join again with an invite link.",
    "bans.unban": "Unban",

    "danger.ownerHint":
      "Permanently deletes channels, categories, messages, invites and memberships.",
    "danger.memberHint": "You can only rejoin with a new invite link.",
    "danger.deleteTitle": "Delete server “{name}” for good?",
    "danger.deleteMessage":
      "Channels, messages, invites and memberships will be deleted.",
    "danger.delete": "Delete server",
    "danger.leaveTitle": "Leave server “{name}”?",
    "danger.leaveConfirm": "Leave",
    "danger.leave": "Leave server",
  },
);
