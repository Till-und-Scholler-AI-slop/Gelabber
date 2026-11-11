//! Servers (web/src/i18n/catalogs/servers.ts): adding, creating and joining
//! one, the category and channel dialogs, invite links, the server's
//! overview, the sidebar's voice heading and the toasts of failed server
//! writes. The overview's voice room card keeps the wording of the web's
//! voice catalog (`room.*`, `participant.*`).

use crate::{Locale, get, plural};

crate::strings! {
    /// The uncategorised voice rooms' heading. A category named after it in
    /// either language takes the rooms instead ([`voice_headings`]).
    voice_heading = "Sprachkanäle" | "Voice channels";

    add_title = "Server hinzufügen" | "Add a server";
    create_title = "Server erstellen" | "Create a server";
    create_hint = "Mit #allgemein und einem Sprachkanal." | "With #general and a voice channel.";
    join_title = "Server beitreten" | "Join a server";
    join_hint = "Mit einem Einladungslink oder Code." | "With an invite link or code.";
    join_input_label = "Einladungslink oder Code" | "Invite link or code";
    join_input_placeholder = "Link oder Code einfügen" | "Paste a link or code";
    join_error_empty = "Bitte einen Einladungslink oder Code eingeben."
        | "Enter an invite link or code.";
    join_error_foreign = "Dieser Link gehört zu einem anderen Gelabber-Server."
        | "This link belongs to another Gelabber server.";
    join_error_invalid = "Das sieht nicht wie ein Einladungslink aus."
        | "That doesn’t look like an invite link.";

    form_name_label = "Name" | "Name";
    form_create = "Erstellen" | "Create";
    form_creating = "Erstellen…" | "Creating…";

    category_create_title = "Kategorie erstellen" | "Create category";
    category_rename_title = "Kategorie umbenennen" | "Rename category";

    channel_create_title = "Kanal erstellen" | "Create channel";
    channel_edit_title = "Kanal bearbeiten" | "Edit channel";
    channel_kind_legend = "Kanaltyp" | "Channel type";
    channel_kind_text = "Text" | "Text";
    channel_kind_text_hint = "Nachrichten, Dateien" | "Messages, files";
    channel_kind_voice = "Sprache" | "Voice";
    channel_kind_voice_hint = "Reden, Kamera, Live" | "Talk, camera, Live";
    channel_category_label = "Kategorie" | "Category";
    channel_no_category = "Ohne Kategorie" | "No category";

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
    terms_uses_unlimited_note = "unbegrenzt nutzbar" | "unlimited uses";

    overview_label = "Übersicht" | "Overview";
    overview_alone = "Noch keine weiteren Mitglieder." | "No other members yet.";
    overview_invite = "Einladen" | "Invite";
    overview_no_room = "Kein Sprachkanal" | "No voice channel";
    overview_no_room_hint = "Erstelle einen Sprachkanal in der Seitenleiste."
        | "Create a voice channel in the sidebar.";
    overview_unread = "Ungelesen" | "Unread";
    overview_draft = "Entwurf" | "Draft";
    overview_attachment = "Anhang" | "Attachment";
    overview_you = "Du" | "You";
    overview_yesterday = "Gestern" | "Yesterday";

    // The overview's voice room card (web voice `room.*`, `participant.*`).
    room_connected = "Verbunden" | "Connected";
    room_deafened = "Taub" | "Deafened";
    room_muted = "Stumm" | "Muted";
    room_speaking = "Spricht" | "Speaking";
    room_present = "Im Raum" | "In the room";
    room_self = "(du)" | "(you)";
    room_camera_on = "Kamera an" | "Camera on";
    room_own_profile = "Dein Profil" | "Your profile";
    room_profile = "Profil und Lautstärke" | "Profile and volume";
    room_switch_note_unnamed = "deinen Sprachkanal" | "your voice channel";
    room_join = "Beitreten" | "Join";
    room_switch_here = "Hierher wechseln" | "Switch here";
    room_no_permission = "Keine Berechtigung für Sprachkanäle." | "No permission for voice channels.";
    room_test_microphone = "Mikrofon testen" | "Test microphone";
    room_call_settings = "Anrufeinstellungen" | "Call settings";

    load_server = "Der Server konnte nicht geladen werden." | "The server couldn’t be loaded.";

    error_server_not_saved = "Server nicht gespeichert." | "Server not saved.";
    error_server_not_deleted = "Server nicht gelöscht." | "Server not deleted.";
    error_server_not_left = "Server nicht verlassen." | "Couldn’t leave the server.";
    error_category_not_created = "Kategorie nicht erstellt." | "Category not created.";
    error_category_not_saved = "Kategorie nicht gespeichert." | "Category not saved.";
    error_category_not_deleted = "Kategorie nicht gelöscht." | "Category not deleted.";
    error_channel_not_created = "Kanal nicht erstellt." | "Channel not created.";
    error_channel_not_saved = "Kanal nicht gespeichert." | "Channel not saved.";
    error_channel_not_deleted = "Kanal nicht gelöscht." | "Channel not deleted.";
    error_invite_not_revoked = "Einladung nicht widerrufen." | "Invite not revoked.";
    error_member_not_kicked = "Mitglied nicht entfernt." | "Member not removed.";
    error_member_not_banned = "Mitglied nicht gesperrt." | "Member not banned.";
    error_member_not_unbanned = "Mitglied nicht entsperrt." | "Member not unbanned.";
}

/// [`voice_heading`] in both languages, lower case: a server keeps the
/// names its creator's language gave it, and every viewer merges them.
pub fn voice_headings() -> [String; 2] {
    let (_, de, en) = ALL
        .iter()
        .find(|(key, _, _)| *key == "voice_heading")
        .copied()
        .unwrap_or(("", "", ""));
    [de.to_lowercase(), en.to_lowercase()]
}

/// "Wird gespeichert als #{name}".
pub fn channel_saved_as(name: &str) -> String {
    match get() {
        Locale::De => format!("Wird gespeichert als #{name}"),
        Locale::En => format!("Will be saved as #{name}"),
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
pub fn terms_uses_limited(count: u64) -> String {
    match get() {
        Locale::De => format!("{count}-mal nutzbar"),
        Locale::En => plural(count, format!("{count} use"), format!("{count} uses")),
    }
}

/// "{count} Mitglied(er)".
pub fn overview_members(count: u64) -> String {
    match get() {
        Locale::De => plural(
            count,
            format!("{count} Mitglied"),
            format!("{count} Mitglieder"),
        ),
        Locale::En => plural(count, format!("{count} member"), format!("{count} members")),
    }
}

/// "{count} im Sprachkanal".
pub fn overview_in_voice(count: u64) -> String {
    match get() {
        Locale::De => format!("{count} im Sprachkanal"),
        Locale::En => format!("{count} in a voice channel"),
    }
}

/// "{count} in Sprachkanälen".
pub fn overview_in_voice_rooms(count: u64) -> String {
    match get() {
        Locale::De => format!("{count} in Sprachkanälen"),
        Locale::En => format!("{count} in voice channels"),
    }
}

/// A person's tile line with the camera on: "{status} · Kamera an".
pub fn room_status_camera(status: &str) -> String {
    match get() {
        Locale::De => format!("{status} · Kamera an"),
        Locale::En => format!("{status} · Camera on"),
    }
}

/// "Du verlässt dabei {channel}." (web voice `room.switchNote`).
pub fn room_switch_note(channel: &str) -> String {
    match get() {
        Locale::De => format!("Du verlässt dabei {channel}."),
        Locale::En => format!("This leaves {channel}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_voice_heading_matches_in_both_languages() {
        assert_eq!(voice_headings(), ["sprachkanäle", "voice channels"]);
    }
}
