//! Servers (web/src/i18n/catalogs/servers.ts): adding, creating and joining
//! one, the category and channel dialogs, invite links, the server's
//! overview, the sidebar's voice heading and the toasts of failed server
//! writes.

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
    channel_access_label = "Zugriff" | "Access";
    channel_access_hint = "Nur diese Rollen sehen den Kanal. Mit „Alle Mitglieder“ sieht ihn jeder auf dem Server."
        | "Only these roles see the channel. With “All members” everyone on the server sees it.";
    channel_access_no_roles = "Rollen legst du in den Servereinstellungen an."
        | "You create roles in the server settings.";

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
    overview_invite = "Einladen" | "Invite";
    overview_alone = "Noch keine weiteren Mitglieder." | "No other members yet.";
    /// The "@" on a "Verpasst" card.
    overview_mentioned = "Du wurdest erwähnt" | "You were mentioned";
    /// The header's search field look-alike, which opens the quick switcher.
    overview_search = "Springen zu …" | "Jump to …";
    overview_missed = "Verpasst" | "Missed";
    overview_shared = "Geteilt" | "Shared";
    /// A "Verpasst" card: mark the channel read.
    overview_mark_read = "Gelesen" | "Mark read";
    overview_open = "Öffnen" | "Open";
    /// Beside the greeting when nothing was missed.
    overview_all_read = "Alles gelesen" | "All caught up";
    /// A Live card's button.
    overview_watch_short = "Zuschauen" | "Watch";
    /// The own stream's name and the own messages' author.
    overview_you = "Du" | "You";
    /// "Im Call": who sits in which voice channel.
    overview_calls = "Im Call" | "In a call";
    /// A voice channel's row in "Im Call": into that call.
    overview_join = "Beitreten" | "Join";
    /// The row of the call the user is in.
    overview_call_here = "Du bist hier" | "You’re here";
    overview_own_screen = "Dein Bildschirm" | "Your screen";

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
    error_role_not_created = "Rolle nicht erstellt." | "Role not created.";
    error_role_not_saved = "Rolle nicht gespeichert." | "Role not saved.";
    error_role_not_deleted = "Rolle nicht gelöscht." | "Role not deleted.";
    error_member_roles_not_saved = "Rollen nicht gespeichert." | "Roles not saved.";
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

/// "Guten Morgen, {name}" (05:00–10:59).
pub fn overview_greeting_morning(name: &str) -> String {
    match get() {
        Locale::De => format!("Guten Morgen, {name}"),
        Locale::En => format!("Good morning, {name}"),
    }
}

/// "Guten Tag, {name}" (11:00–17:59).
pub fn overview_greeting_day(name: &str) -> String {
    match get() {
        Locale::De => format!("Guten Tag, {name}"),
        Locale::En => format!("Good afternoon, {name}"),
    }
}

/// "Guten Abend, {name}" (18:00–04:59).
pub fn overview_greeting_evening(name: &str) -> String {
    match get() {
        Locale::De => format!("Guten Abend, {name}"),
        Locale::En => format!("Good evening, {name}"),
    }
}

/// A "Verpasst" card's unread count: "{count} neu".
pub fn overview_new(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} neu"),
        Locale::En => format!("{n} new"),
    }
}

/// The unread messages a card does not show: "{count} weitere".
pub fn overview_more(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} weitere"),
        Locale::En => format!("{n} more"),
    }
}

/// The same with only one channel unread: "{count} ältere anzeigen".
pub fn overview_older(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} ältere anzeigen"),
        Locale::En => format!("Show {n} older"),
    }
}

/// The unread channels past the shown cards: "{count} weitere Kanäle".
pub fn overview_more_channels(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => plural(
            count,
            format!("{n} weiterer Kanal"),
            format!("{n} weitere Kanäle"),
        ),
        Locale::En => plural(
            count,
            format!("{n} more channel"),
            format!("{n} more channels"),
        ),
    }
}

/// The first card's reply field: "Antworten in #{channel}".
pub fn overview_reply(channel: &str) -> String {
    match get() {
        Locale::De => format!("Antworten in #{channel}"),
        Locale::En => format!("Reply in #{channel}"),
    }
}

/// A stream: "Bildschirm von {name} ansehen".
pub fn overview_watch(name: &str) -> String {
    match get() {
        Locale::De => format!("Bildschirm von {name} ansehen"),
        Locale::En => format!("Watch {name}’s screen"),
    }
}

/// A voice channel's row in "Im Call": "{count} Personen".
pub fn overview_call_people(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => plural(count, format!("{n} Person"), format!("{n} Personen")),
        Locale::En => plural(count, format!("{n} person"), format!("{n} people")),
    }
}

/// The people of a row past the shown ones: "+{count}".
pub fn overview_call_more(count: u64) -> String {
    format!("+{}", crate::intl::count(count))
}

/// A stream's viewers: "{count} Zuschauer".
pub fn overview_viewers(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} Zuschauer"),
        Locale::En => plural(count, format!("{n} viewer"), format!("{n} viewers")),
    }
}

/// A shared picture without alt text: "Bild von {name}".
pub fn overview_picture_by(name: &str) -> String {
    match get() {
        Locale::De => format!("Bild von {name}"),
        Locale::En => format!("Picture from {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overview_counts_in_both_languages() {
        let _de = crate::on_this_thread(Locale::De);
        assert_eq!(overview_new(5), "5 neu");
        assert_eq!(overview_more_channels(1), "1 weiterer Kanal");
        assert_eq!(overview_more_channels(3), "3 weitere Kanäle");
        assert_eq!(overview_older(2), "2 ältere anzeigen");
        assert_eq!(overview_reply("allgemein"), "Antworten in #allgemein");
        let _en = crate::on_this_thread(Locale::En);
        assert_eq!(overview_more_channels(1), "1 more channel");
        assert_eq!(overview_older(2), "Show 2 older");
    }
}
