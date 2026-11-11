// Server settings (`/s/$serverId/settings`): name and picture, roles,
// invites, the directory listing and its join requests, members, bans,
// leaving and deleting.
import { defineCatalog } from "../catalog.ts";

export const serverSettings = defineCatalog(
  "serverSettings",
  {
    title: "Servereinstellungen",
    loadFailed: "Die Servereinstellungen konnten nicht geladen werden.",
    navLabel: "Abschnitte",

    // The section anchors; the URL fragments stay German.
    "section.general": "Allgemein",
    "section.roles": "Rollen",
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

    "permission.manageServer.label": "Server verwalten",
    "permission.manageServer.hint": "Umbenennen, Rollen ändern, Einladungen.",
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
    "permission.moderateMembers.label": "Mitglieder moderieren",
    "permission.moderateMembers.hint":
      "Mitglieder vom Server entfernen und sperren.",

    "roles.hint":
      "Eine Rolle bündelt Rechte. Wer mehrere Rollen hat, bekommt die Rechte aller zusammen.",
    "roles.new": "Neue Rolle",
    "roles.delete": "Rolle löschen",
    "roles.nameLabel": "Name",
    "roles.colorLabel": "Farbe",
    "roles.permissionsLabel": "Rechte",
    "roles.inherited": "über „{role}“",
    "roles.ownerText":
      "Der Inhaber hat immer alle Rechte und sieht jeden Kanal.",
    "roles.deleteTitle": "Rolle „{name}“ löschen?",
    "roles.deleteMessage":
      "Mitglieder verlieren die Rechte dieser Rolle. Kanäle, die nur für diese Rolle sind, sieht danach nur der Inhaber.",
    "roles.colorName.amber": "Bernstein",
    "roles.colorName.blue": "Blau",
    "roles.colorName.green": "Grün",
    "roles.colorName.pink": "Rosa",
    "roles.colorName.violet": "Violett",
    "roles.colorName.coral": "Koralle",

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
    "members.hint":
      "Hier vergibst du Rollen. Was eine Rolle darf, stellst du unter „Rollen“ ein.",
    "members.rolesFor": "Rollen für {name}",
    "members.manageRoles": "Rollen verwalten",
    "members.noRoles": "Noch keine Rollen.",

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

    // Verzeichnis (discovery): the owner's listing and the join requests.
    "section.directory": "Verzeichnis",
    "directory.hint": "Ob und wie dein Server unter Entdecken erscheint.",
    "directory.loading": "Lade den Eintrag…",
    "directory.loadFailed": "Der Eintrag konnte nicht geladen werden.",
    "directory.delisted":
      "Der Betreiber hat diesen Server aus dem Verzeichnis genommen.",
    "directory.listed": "Im Verzeichnis zeigen",
    "directory.description": "Beschreibung",
    "directory.descriptionCount": "{length} / {max} Zeichen",
    "directory.descriptionRequired":
      "Im Verzeichnis braucht der Server eine Beschreibung.",
    "directory.descriptionInvalid":
      "Die Beschreibung enthält Zeichen, die nicht erlaubt sind.",
    "directory.descriptionTooLong": "Die Beschreibung ist zu lang.",
    "directory.language": "Sprache",
    "directory.languagePick": "Sprache wählen",
    "directory.languageRequired": "Bitte eine Sprache wählen.",
    "directory.joinMode": "Beitritt",
    "directory.joinOpen": "Offen",
    "directory.joinOpenHint": "Jeder kann direkt beitreten.",
    "directory.joinRequest": "Auf Anfrage",
    "directory.joinRequestHint":
      "Du oder wer Mitglieder moderieren darf, nimmt Anfragen an.",
    "directory.preview": "Vorschau",
    "directory.saving": "Speichert…",
    "directory.unchanged": "Keine Änderungen.",
    "joinRequests.title": "Beitrittsanfragen",
    "joinRequests.loading": "Lade Anfragen…",
    "joinRequests.loadFailed": "Die Anfragen konnten nicht geladen werden.",
    "joinRequests.empty": "Keine offenen Anfragen.",
    "joinRequests.accept": "Annehmen",
    "joinRequests.decline": "Ablehnen",
    "joinRequests.count": {
      one: "{count} offene Anfrage",
      other: "{count} offene Anfragen",
    },

    "transfer.title": "Server übergeben",
    "transfer.hint":
      "Ein anderes Mitglied wird Eigentümer. Du bleibst Mitglied.",
    "transfer.noMember":
      "Lade zuerst jemanden ein, dem du den Server übergeben kannst.",
    "transfer.open": "Übergeben…",
    "transfer.member": "Neuer Eigentümer",
    "transfer.submit": "Übergeben",
    "transfer.pending": "Wird übergeben…",
    "transfer.confirmTitle": "„{server}“ an {name} übergeben?",
    "transfer.confirmMessage": "Du kannst das nicht selbst rückgängig machen.",
  },
  {
    title: "Server settings",
    loadFailed: "The server settings couldn’t be loaded.",
    navLabel: "Sections",

    "section.general": "General",
    "section.roles": "Roles",
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

    "permission.manageServer.label": "Manage server",
    "permission.manageServer.hint": "Rename, change roles, invites.",
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
    "permission.moderateMembers.label": "Moderate members",
    "permission.moderateMembers.hint":
      "Remove members from the server and ban them.",

    "roles.hint":
      "A role bundles permissions. Whoever has several roles gets all of them together.",
    "roles.new": "New role",
    "roles.delete": "Delete role",
    "roles.nameLabel": "Name",
    "roles.colorLabel": "Colour",
    "roles.permissionsLabel": "Permissions",
    "roles.inherited": "via “{role}”",
    "roles.ownerText":
      "The owner always has every permission and sees every channel.",
    "roles.deleteTitle": "Delete role “{name}”?",
    "roles.deleteMessage":
      "Members lose this role’s permissions. Channels that are only for this role will then be seen by the owner alone.",
    "roles.colorName.amber": "Amber",
    "roles.colorName.blue": "Blue",
    "roles.colorName.green": "Green",
    "roles.colorName.pink": "Pink",
    "roles.colorName.violet": "Violet",
    "roles.colorName.coral": "Coral",

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
    "members.hint":
      "Assign roles here. What a role may do is set under “Roles”.",
    "members.rolesFor": "Roles for {name}",
    "members.manageRoles": "Manage roles",
    "members.noRoles": "No roles yet.",

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

    "section.directory": "Directory",
    "directory.hint": "Whether and how your server appears in Discover.",
    "directory.loading": "Loading the listing…",
    "directory.loadFailed": "The listing couldn’t be loaded.",
    "directory.delisted":
      "The operator has removed this server from the directory.",
    "directory.listed": "Show in the directory",
    "directory.description": "Description",
    "directory.descriptionCount": "{length} / {max} characters",
    "directory.descriptionRequired":
      "A server in the directory needs a description.",
    "directory.descriptionInvalid":
      "The description contains characters that aren’t allowed.",
    "directory.descriptionTooLong": "The description is too long.",
    "directory.language": "Language",
    "directory.languagePick": "Choose a language",
    "directory.languageRequired": "Please choose a language.",
    "directory.joinMode": "Joining",
    "directory.joinOpen": "Open",
    "directory.joinOpenHint": "Anyone can join straight away.",
    "directory.joinRequest": "On request",
    "directory.joinRequestHint":
      "You or anyone who may moderate members accepts requests.",
    "directory.preview": "Preview",
    "directory.saving": "Saving…",
    "directory.unchanged": "No changes.",
    "joinRequests.title": "Join requests",
    "joinRequests.loading": "Loading requests…",
    "joinRequests.loadFailed": "The requests couldn’t be loaded.",
    "joinRequests.empty": "No open requests.",
    "joinRequests.accept": "Accept",
    "joinRequests.decline": "Decline",
    "joinRequests.count": {
      one: "{count} open request",
      other: "{count} open requests",
    },

    "transfer.title": "Transfer server",
    "transfer.hint": "Another member becomes the owner. You stay a member.",
    "transfer.noMember": "Invite someone first who you can hand the server to.",
    "transfer.open": "Transfer…",
    "transfer.member": "New owner",
    "transfer.submit": "Transfer",
    "transfer.pending": "Transferring…",
    "transfer.confirmTitle": "Transfer “{server}” to {name}?",
    "transfer.confirmMessage": "You can’t undo this yourself.",
  },
);
