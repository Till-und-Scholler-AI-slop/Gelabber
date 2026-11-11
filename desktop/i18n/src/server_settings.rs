//! Server settings (web/src/i18n/catalogs/serverSettings.ts): name and
//! picture, roles, invites, members, bans, leaving and
//! deleting. The invite dialog the settings open and the member actions
//! keep the web's wording from its servers and members catalogs.

use crate::{Locale, get, plural};

crate::strings! {
    title = "Servereinstellungen" | "Server settings";
    load_failed = "Die Servereinstellungen konnten nicht geladen werden."
        | "The server settings couldn’t be loaded.";
    nav_label = "Abschnitte" | "Sections";

    // The section anchors; the ids stay German.
    section_general = "Allgemein" | "General";
    section_roles = "Rollen" | "Roles";
    section_invites = "Einladungen" | "Invites";
    section_members = "Mitglieder" | "Members";
    section_bans = "Sperren" | "Bans";
    section_danger = "Gefahrenzone" | "Danger zone";

    general_hint = "Ein neues Bild gilt sofort, den Namen übernimmst du mit Speichern."
        | "A new picture applies at once; the name applies when you save.";
    general_name_label = "Servername" | "Server name";
    general_saving = "Speichert…" | "Saving…";
    general_unchanged = "Keine Änderungen." | "No changes.";
    icon_upload = "Bild hochladen" | "Upload picture";
    icon_uploading = "Lädt hoch…" | "Uploading…";
    icon_removing = "Entfernt…" | "Removing…";
    icon_pick_label = "Serverbild auswählen" | "Choose server picture";

    permission_manage_server_label = "Server verwalten" | "Manage server";
    permission_manage_server_hint = "Umbenennen, Rollen ändern, Einladungen."
        | "Rename, change roles, invites.";
    permission_manage_channels_label = "Kanäle verwalten" | "Manage channels";
    permission_manage_channels_hint = "Kategorien und Kanäle anlegen, umbenennen, löschen."
        | "Create, rename and delete categories and channels.";
    permission_manage_messages_label = "Nachrichten löschen" | "Delete messages";
    permission_manage_messages_hint = "Fremde Nachrichten im Chat löschen."
        | "Delete other people’s messages in chat.";
    permission_mention_everyone_label = "@everyone und @here" | "@everyone and @here";
    permission_mention_everyone_hint = "Alle Mitglieder auf einmal erwähnen und benachrichtigen."
        | "Mention and notify every member at once.";
    permission_send_messages_label = "Nachrichten schreiben" | "Send messages";
    permission_send_messages_hint = "In Textkanälen schreiben." | "Write in text channels.";
    permission_send_files_label = "Dateien senden" | "Send files";
    permission_send_files_hint = "Anhänge hochladen." | "Upload attachments.";
    permission_join_voice_label = "Sprachkanälen beitreten" | "Join voice channels";
    permission_join_voice_hint = "In Sprachkanälen reden, zuhören und die Kamera zeigen."
        | "Talk, listen and show your camera in voice channels.";
    permission_go_live_label = "Live gehen" | "Go Live";
    permission_go_live_hint = "Im Sprachkanal einen Bildschirm oder ein Fenster für alle dort zeigen."
        | "Show a screen or window to everyone in the voice channel.";
    permission_moderate_members_label = "Mitglieder moderieren" | "Moderate members";
    permission_moderate_members_hint = "Mitglieder vom Server entfernen und sperren."
        | "Remove members from the server and ban them.";

    roles_hint = "Eine Rolle bündelt Rechte. Wer mehrere Rollen hat, bekommt die Rechte aller zusammen."
        | "A role bundles permissions. Whoever has several roles gets all of them together.";
    roles_new = "Neue Rolle" | "New role";
    roles_delete = "Rolle löschen" | "Delete role";
    roles_name_label = "Name" | "Name";
    roles_color_label = "Farbe" | "Colour";
    roles_permissions_label = "Rechte" | "Permissions";
    roles_owner_text = "Der Inhaber hat immer alle Rechte und sieht jeden Kanal."
        | "The owner always has every permission and sees every channel.";
    roles_delete_message = "Mitglieder verlieren die Rechte dieser Rolle. Kanäle, die nur für diese Rolle sind, sieht danach nur der Inhaber."
        | "Members lose this role’s permissions. Channels that are only for this role will then be seen by the owner alone.";
    roles_color_name_amber = "Bernstein" | "Amber";
    roles_color_name_blue = "Blau" | "Blue";
    roles_color_name_green = "Grün" | "Green";
    roles_color_name_pink = "Rosa" | "Pink";
    roles_color_name_violet = "Violett" | "Violet";
    roles_color_name_coral = "Koralle" | "Coral";

    invites_hint = "Alle aktiven Links. Jedes Mitglied kann Links erstellen."
        | "All active links. Any member can create links.";
    invites_create = "Neuen Link erstellen" | "Create new link";
    invites_loading = "Lade Einladungen…" | "Loading invites…";
    invites_load_failed = "Die Einladungen konnten nicht geladen werden." | "The invites couldn’t be loaded.";
    invites_empty = "Keine aktiven Einladungen." | "No active invites.";
    invites_revoke_title = "Link widerrufen?" | "Revoke link?";
    invites_revoke_message = "Der Link funktioniert danach nicht mehr. Bestehende Mitglieder bleiben."
        | "The link stops working. Existing members stay.";
    invites_revoke = "Widerrufen" | "Revoke";
    invites_revoked = "Link widerrufen." | "Link revoked.";

    members_owner = "Inhaber" | "Owner";
    members_hint = "Hier vergibst du Rollen. Was eine Rolle darf, stellst du unter „Rollen“ ein."
        | "Assign roles here. What a role may do is set under “Roles”.";
    members_manage_roles = "Rollen verwalten" | "Manage roles";
    members_no_roles = "Noch keine Rollen." | "No roles yet.";

    bans_title = "Gesperrte Konten" | "Banned accounts";
    bans_hint = "Gesperrte User können auch mit einem Einladungslink nicht wieder beitreten."
        | "Banned people can’t rejoin, not even with an invite link.";
    bans_loading = "Lade Sperren…" | "Loading bans…";
    bans_load_failed = "Die Sperren konnten nicht geladen werden." | "The bans couldn’t be loaded.";
    bans_empty = "Niemand ist gesperrt." | "Nobody is banned.";
    bans_unban_message = "Ein Beitritt per Einladungslink ist dann wieder möglich."
        | "They can then join again with an invite link.";
    bans_unban = "Entsperren" | "Unban";

    danger_owner_hint = "Löscht Kanäle, Kategorien, Nachrichten, Einladungen und Mitgliedschaften endgültig."
        | "Permanently deletes channels, categories, messages, invites and memberships.";
    danger_member_hint = "Wieder beitreten kannst du nur mit einem neuen Einladungslink."
        | "You can only rejoin with a new invite link.";
    danger_delete_message = "Kanäle, Nachrichten, Einladungen und Mitgliedschaften werden gelöscht."
        | "Channels, messages, invites and memberships will be deleted.";
    danger_delete = "Server löschen" | "Delete server";
    danger_leave_confirm = "Verlassen" | "Leave";
    danger_leave = "Server verlassen" | "Leave server";

    // Member actions (web members moderation.*).
    moderation_kick = "Vom Server entfernen" | "Remove from server";
    moderation_ban = "Sperren" | "Ban";
    moderation_kick_message = "Mit einem neuen Einladungslink ist ein erneuter Beitritt möglich."
        | "They can rejoin with a new invite link.";
    moderation_ban_message = "Sie können nicht wieder beitreten, auch nicht mit einem Einladungslink."
        | "They can’t rejoin, not even with an invite link.";

    // Failed actions (web servers error.*).
    error_invite_not_revoked = "Einladung nicht widerrufen." | "Invite not revoked.";
    error_member_not_banned = "Mitglied nicht gesperrt." | "Member not banned.";
    error_member_not_unbanned = "Mitglied nicht entsperrt." | "Member not unbanned.";

    // An invite in the list (web servers expiry.*).
    expiry_never = "läuft nie ab" | "never expires";
    expiry_expired = "abgelaufen" | "expired";

    // The invite dialog (web servers invite.* and terms.*).
    invite_making = "Link wird erstellt…" | "Creating link…";
    invite_copy = "Link kopieren" | "Copy link";
    invite_replace_failed = "Der neue Link konnte nicht erstellt werden. Der bisherige gilt weiter."
        | "The new link couldn’t be created. The previous one still works.";
    invite_terms_editor = "Ablauf & Limit ändern" | "Change expiry & limit";
    invite_expires_after = "Läuft ab nach" | "Expires after";
    invite_uses_label = "Wie oft nutzbar" | "How many uses";
    terms_expiry_hour = "1 Stunde" | "1 hour";
    terms_expiry_day = "24 Stunden" | "24 hours";
    terms_expiry_week = "7 Tage" | "7 days";
    terms_expiry_never = "Nie" | "Never";
    terms_uses_unlimited = "Unbegrenzt" | "Unlimited";
    terms_valid_forever = "unbegrenzt gültig" | "never expires";
    terms_uses_unlimited_summary = "unbegrenzt nutzbar" | "unlimited uses";

    // Handing the server on (web serverSettings transfer.*, #519).
    transfer_title = "Server übergeben" | "Transfer server";
    transfer_hint = "Ein anderes Mitglied wird Eigentümer. Du bleibst Mitglied."
        | "Another member becomes the owner. You stay a member.";
    transfer_no_member = "Lade zuerst jemanden ein, dem du den Server übergeben kannst."
        | "Invite someone first who you can hand the server to.";
    transfer_open = "Übergeben…" | "Transfer…";
    transfer_member = "Neuer Eigentümer" | "New owner";
    transfer_submit = "Übergeben" | "Transfer";
    transfer_pending = "Wird übergeben…" | "Transferring…";
    transfer_confirm_message = "Du kannst das nicht selbst rückgängig machen."
        | "You can’t undo this yourself.";

    // Verzeichnis (web serverSettings directory.* and joinRequests.*, #520).
    section_directory = "Verzeichnis" | "Directory";
    directory_hint = "Ob und wie dein Server unter Entdecken erscheint."
        | "Whether and how your server appears in Discover.";
    directory_loading = "Lade den Eintrag…" | "Loading the listing…";
    directory_load_failed = "Der Eintrag konnte nicht geladen werden."
        | "The listing couldn’t be loaded.";
    directory_delisted = "Der Betreiber hat diesen Server aus dem Verzeichnis genommen."
        | "The operator has removed this server from the directory.";
    directory_listed = "Im Verzeichnis zeigen" | "Show in the directory";
    directory_description = "Beschreibung" | "Description";
    directory_description_required = "Im Verzeichnis braucht der Server eine Beschreibung."
        | "A server in the directory needs a description.";
    directory_description_invalid = "Die Beschreibung enthält Zeichen, die nicht erlaubt sind."
        | "The description contains characters that aren’t allowed.";
    directory_description_too_long = "Die Beschreibung ist zu lang."
        | "The description is too long.";
    directory_language = "Sprache" | "Language";
    directory_language_pick = "Sprache wählen" | "Choose a language";
    directory_language_required = "Bitte eine Sprache wählen." | "Please choose a language.";
    directory_join_mode = "Beitritt" | "Joining";
    directory_join_open = "Offen" | "Open";
    directory_join_open_hint = "Jeder kann direkt beitreten." | "Anyone can join straight away.";
    directory_join_request = "Auf Anfrage" | "On request";
    directory_join_request_hint = "Du oder wer Mitglieder moderieren darf, nimmt Anfragen an."
        | "You or anyone who may moderate members accepts requests.";
    directory_preview = "Vorschau" | "Preview";
    directory_saving = "Speichert…" | "Saving…";
    directory_unchanged = "Keine Änderungen." | "No changes.";
    join_requests_title = "Beitrittsanfragen" | "Join requests";
    join_requests_loading = "Lade Anfragen…" | "Loading requests…";
    join_requests_load_failed = "Die Anfragen konnten nicht geladen werden."
        | "The requests couldn’t be loaded.";
    join_requests_empty = "Keine offenen Anfragen." | "No open requests.";
    join_requests_accept = "Annehmen" | "Accept";
    join_requests_decline = "Ablehnen" | "Decline";
}

/// "Mitglieder ({count})".
pub fn members_title(count: usize) -> String {
    let n = crate::intl::count(count as u64);
    match get() {
        Locale::De => format!("Mitglieder ({n})"),
        Locale::En => format!("Members ({n})"),
    }
}

/// "{name} wieder zulassen?"
pub fn bans_unban_title(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} wieder zulassen?"),
        Locale::En => format!("Let {name} back in?"),
    }
}

/// "Server „{name}“ endgültig löschen?"
pub fn danger_delete_title(name: &str) -> String {
    match get() {
        Locale::De => format!("Server „{name}“ endgültig löschen?"),
        Locale::En => format!("Delete server “{name}” for good?"),
    }
}

/// "Server „{name}“ verlassen?"
pub fn danger_leave_title(name: &str) -> String {
    match get() {
        Locale::De => format!("Server „{name}“ verlassen?"),
        Locale::En => format!("Leave server “{name}”?"),
    }
}

/// "Kopieren ging nicht. Der Code ist {code}."
pub fn invites_copy_failed(code: &str) -> String {
    match get() {
        Locale::De => format!("Kopieren ging nicht. Der Code ist {code}."),
        Locale::En => format!("Couldn’t copy. The code is {code}."),
    }
}

/// "Aktionen für {name}".
pub fn moderation_menu_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Aktionen für {name}"),
        Locale::En => format!("Actions for {name}"),
    }
}

/// "{name} vom Server entfernen?"
pub fn moderation_kick_title(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} vom Server entfernen?"),
        Locale::En => format!("Remove {name} from the server?"),
    }
}

/// "{name} sperren?"
pub fn moderation_ban_title(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} sperren?"),
        Locale::En => format!("Ban {name}?"),
    }
}

/// An invite's uses and expiry: "{uses}/{max} benutzt · {expiry}".
pub fn expiry_uses(uses: &str, max: &str, expiry: &str) -> String {
    match get() {
        Locale::De => format!("{uses}/{max} benutzt · {expiry}"),
        Locale::En => format!("{uses}/{max} used · {expiry}"),
    }
}

/// "läuft in 5 Minuten ab".
pub fn expiry_in_minutes(count: i64) -> String {
    let n = count.unsigned_abs();
    match get() {
        Locale::De => format!("läuft in {count} {} ab", plural(n, "Minute", "Minuten")),
        Locale::En => format!("expires in {count} {}", plural(n, "minute", "minutes")),
    }
}

/// "läuft in 3 Stunden ab".
pub fn expiry_in_hours(count: i64) -> String {
    let n = count.unsigned_abs();
    match get() {
        Locale::De => format!("läuft in {count} {} ab", plural(n, "Stunde", "Stunden")),
        Locale::En => format!("expires in {count} {}", plural(n, "hour", "hours")),
    }
}

/// "läuft in 6 Tagen ab".
pub fn expiry_in_days(count: i64) -> String {
    let n = count.unsigned_abs();
    match get() {
        Locale::De => format!("läuft in {count} {} ab", plural(n, "Tag", "Tagen")),
        Locale::En => format!("expires in {count} {}", plural(n, "day", "days")),
    }
}

/// "Zu {server} einladen".
pub fn invite_title(server: &str) -> String {
    match get() {
        Locale::De => format!("Zu {server} einladen"),
        Locale::En => format!("Invite to {server}"),
    }
}

/// "Neuer Link · {terms}".
pub fn invite_new_link(terms: &str) -> String {
    match get() {
        Locale::De => format!("Neuer Link · {terms}"),
        Locale::En => format!("New link · {terms}"),
    }
}

/// "Link kopiert · {terms}".
pub fn invite_copied(terms: &str) -> String {
    match get() {
        Locale::De => format!("Link kopiert · {terms}"),
        Locale::En => format!("Link copied · {terms}"),
    }
}

/// "Der Einladungslink konnte nicht erstellt werden. {reason}".
pub fn invite_create_failed(reason: &str) -> String {
    match get() {
        Locale::De => format!("Der Einladungslink konnte nicht erstellt werden. {reason}"),
        Locale::En => format!("The invite link couldn’t be created. {reason}"),
    }
}

/// "gültig {duration}".
pub fn terms_valid_for(duration: &str) -> String {
    match get() {
        Locale::De => format!("gültig {duration}"),
        Locale::En => format!("valid for {duration}"),
    }
}

/// "{count}-mal nutzbar".
pub fn terms_uses_limited(count: u32) -> String {
    match get() {
        Locale::De => format!("{count}-mal nutzbar"),
        Locale::En => format!("{count} {}", plural(u64::from(count), "use", "uses")),
    }
}

/// "über „{role}“": a flag a custom role shows on through another role.
pub fn roles_inherited(role: &str) -> String {
    match get() {
        Locale::De => format!("über „{role}“"),
        Locale::En => format!("via “{role}”"),
    }
}

/// "Rolle „{name}“ löschen?"
pub fn roles_delete_title(name: &str) -> String {
    match get() {
        Locale::De => format!("Rolle „{name}“ löschen?"),
        Locale::En => format!("Delete role “{name}”?"),
    }
}

/// "Rollen für {name}": a member's role picker.
pub fn members_roles_for(name: &str) -> String {
    match get() {
        Locale::De => format!("Rollen für {name}"),
        Locale::En => format!("Roles for {name}"),
    }
}

/// "„{server}“ an {name} übergeben?"
pub fn transfer_confirm_title(server: &str, name: &str) -> String {
    match get() {
        Locale::De => format!("„{server}“ an {name} übergeben?"),
        Locale::En => format!("Transfer “{server}” to {name}?"),
    }
}

/// `directory.descriptionCount`: "31 / 300 Zeichen"; counts stay
/// unformatted, as on the profile.
pub fn directory_description_count(length: usize, max: usize) -> String {
    match get() {
        Locale::De => format!("{length} / {max} Zeichen"),
        Locale::En => format!("{length} / {max} characters"),
    }
}

/// `joinRequests.count`: "1 offene Anfrage", the anchor's count for screen
/// readers.
pub fn join_requests_count(count: u64) -> String {
    match get() {
        Locale::De => format!(
            "{count} {}",
            plural(count, "offene Anfrage", "offene Anfragen")
        ),
        Locale::En => format!("{count} {}", plural(count, "open request", "open requests")),
    }
}
