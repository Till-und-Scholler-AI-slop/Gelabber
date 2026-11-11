//! The app's frame (web/src/i18n/catalogs/shell.ts): the sidebars' frame,
//! server rail, user panel, quick switcher, shortcuts, row menus,
//! connection notices, the welcome page and the typing line. The client
//! crate's own lines (connection errors, a kick or ban) are here too.
//! Direct messages have `dms`, servers and channels `servers`.

use crate::{Locale, get, plural};

crate::strings! {
    sidebar_label = "Navigation" | "Navigation";
    sidebar_loading_label = "Navigation wird geladen" | "Loading navigation";

    logout_end_call_title = "Abmelden und Anruf beenden?" | "Sign out and end the call?";
    logout_end_call_message = "Du verlässt dabei den Sprachkanal."
        | "This makes you leave the voice channel.";
    logout_end_call_confirm = "Abmelden" | "Sign out";

    connection_offline = "Keine Verbindung – dein Gerät ist offline"
        | "No connection – your device is offline";
    connection_reconnecting = "Keine Verbindung – wird wiederhergestellt…"
        | "No connection – reconnecting…";
    connection_restored = "Verbindung wiederhergestellt" | "Connection restored";
    /// The desktop app's own: TLS turned the connection down.
    connection_certificate = "Keine Verbindung – das Zertifikat des Servers wird nicht akzeptiert"
        | "No connection – the server’s certificate is not accepted";
    /// The desktop app's own: the server turned the upgrade down.
    connection_refused = "Keine Verbindung – vom Server abgelehnt"
        | "No connection – refused by the server";

    user_panel_settings = "Einstellungen" | "Settings";
    user_panel_menu_label = "Benutzermenü" | "User menu";
    user_panel_edit_profile = "Profil bearbeiten" | "Edit profile";
    user_panel_sign_out = "Abmelden" | "Sign out";
    user_panel_status = "Status" | "Status";
    user_panel_saving = "Speichert…" | "Saving…";
    /// The desktop app's own menu item: back to the server setup.
    user_panel_switch_server = "Server wechseln …" | "Switch server…";

    row_menu_not_saved = "Nicht gespeichert." | "Not saved.";
    row_menu_mark_read = "Als gelesen markieren" | "Mark as read";
    row_menu_mark_all_dms_read = "Alle Direktnachrichten als gelesen markieren"
        | "Mark all direct messages as read";
    row_menu_not_marked_read = "Nicht als gelesen markiert." | "Not marked as read.";
    row_menu_unmute = "Stummschaltung aufheben" | "Unmute";
    row_menu_mute = "Stummschalten" | "Mute";
    row_menu_notifications = "Benachrichtigungen" | "Notifications";
    row_menu_notification_settings = "Benachrichtigungseinstellungen…" | "Notification settings…";
    row_menu_create_channel_here = "Kanal hier erstellen" | "Create channel here";
    row_menu_rename_category = "Kategorie umbenennen" | "Rename category";
    row_menu_edit_channel = "Kanal bearbeiten" | "Edit channel";

    switcher_dm_detail = "Direktnachricht" | "Direct message";
    switcher_server_detail = "Server" | "Server";
    switcher_title = "Wechseln zu" | "Switch to";
    switcher_label = "Server, Kanal oder Unterhaltung" | "Server, channel or conversation";
    switcher_placeholder = "Server, Kanal oder Unterhaltung suchen"
        | "Search servers, channels or conversations";
    switcher_hint = "↑ ↓ auswählen · Enter öffnen · Esc schließen"
        | "↑ ↓ select · Enter open · Esc close";

    keys_ctrl = "Strg" | "Ctrl";
    keys_alt = "Alt" | "Alt";
    keys_shift = "Umschalt" | "Shift";

    shortcuts_title = "Tastenkürzel" | "Keyboard shortcuts";
    shortcuts_general = "Allgemein" | "General";
    shortcuts_switcher = "Server, Kanal oder Unterhaltung öffnen"
        | "Open a server, channel or conversation";
    shortcuts_search = "In der Unterhaltung suchen" | "Search the conversation";
    shortcuts_channel = "Vorheriger oder nächster Kanal" | "Previous or next channel";
    shortcuts_unread_channel = "Vorheriger oder nächster ungelesener Kanal"
        | "Previous or next unread channel";
    shortcuts_server = "Vorheriger oder nächster Server" | "Previous or next server";
    shortcuts_escape = "Schließen, sonst Unterhaltung als gelesen markieren"
        | "Close, otherwise mark the conversation as read";
    shortcuts_server_read = "Ganzen Server als gelesen markieren" | "Mark the whole server as read";
    shortcuts_help = "Diese Übersicht" | "This overview";
    shortcuts_voice = "Im Sprachkanal" | "In a voice channel";
    shortcuts_mute = "Stummschaltung ein oder aus" | "Mute on or off";
    shortcuts_deafen = "Taubschaltung ein oder aus" | "Deafen on or off";
    shortcuts_push_to_talk = "Zum Sprechen gedrückt halten" | "Hold to talk";
    shortcuts_cinema = "Kino ein oder aus" | "Cinema on or off";
    shortcuts_fullscreen_mute_keys = "M / Leertaste" | "M / Space";
    shortcuts_fullscreen_mute = "Im Vollbild: Stummschaltung ein oder aus"
        | "In full screen: mute on or off";
    /// The desktop app's own shortcut, as "Server wechseln …".
    shortcuts_switch_server = "Mit einem anderen Gelabber-Server verbinden"
        | "Connect to another Gelabber server";

    someone = "Jemand" | "Someone";
    overview = "Übersicht" | "Overview";

    server_menu_muted = "Stummgeschaltet" | "Muted";
    server_menu_invite = "Einladen" | "Invite";
    server_menu_notifications = "Benachrichtigungen…" | "Notifications…";
    server_menu_create_channel = "Kanal erstellen" | "Create channel";
    server_menu_create_category = "Kategorie erstellen" | "Create category";
    server_menu_settings = "Servereinstellungen" | "Server settings";
    server_menu_members = "Mitglieder" | "Members";

    channels_empty = "Noch keine Kanäle." | "No channels yet.";
    channels_empty_category = "Leer" | "Empty";
    channels_key_hint_actions = "Pfeiltasten hoch und runter wechseln zwischen den Einträgen, links und rechts zwischen einem Eintrag und seinen Aktionen. Pos1 und Ende springen an den Anfang und das Ende der Zeile."
        | "The up and down arrow keys move between entries, left and right between an entry and its actions. Home and End jump to the start and end of the row.";
    channels_key_hint = "Pfeiltasten hoch und runter wechseln zwischen den Einträgen."
        | "The up and down arrow keys move between entries.";
    channels_delete_channel_message = "Alle Nachrichten darin werden gelöscht."
        | "All messages in it will be deleted.";
    channels_delete_channel_confirm = "Kanal löschen" | "Delete channel";
    channels_delete_category_message = "Die Kanäle darin bleiben erhalten." | "The channels in it stay.";
    channels_delete_category_confirm = "Kategorie löschen" | "Delete category";

    participant_deafened = "taub" | "deafened";
    participant_muted = "stumm" | "muted";
    participant_own_profile = "Dein Profil" | "Your profile";
    participant_profile = "Profil und Lautstärke" | "Profile and volume";
    participant_speaking = "spricht" | "speaking";

    channel_draft = "Entwurf" | "Draft";

    rail_add_server = "Server hinzufügen" | "Add server";
    rail_dms = "Direktnachrichten" | "Direct messages";
    rail_mentions_over99 = "Mehr als 99 Erwähnungen" | "More than 99 mentions";
    rail_unread = "Ungelesene Nachrichten" | "Unread messages";

    removed_banned = "Du wurdest vom Server gesperrt." | "You were banned from the server.";
    removed_kicked = "Du wurdest vom Server entfernt." | "You were removed from the server.";

    welcome_title = "Willkommen" | "Welcome";
    welcome_load_failed = "Die Server konnten nicht geladen werden." | "Couldn’t load the servers.";
    welcome_create_title = "Eigenen Server erstellen" | "Create your own server";
    welcome_create_lead = "Mit #allgemein und einem Sprachkanal."
        | "With #general and a voice channel.";
    welcome_create_action = "Server erstellen" | "Create server";
    welcome_join_title = "Server beitreten" | "Join a server";

    no_servers = "Keine Server. Erstelle einen über + in der Serverleiste oder öffne einen Einladungslink."
        | "No servers. Create one with + in the server bar or open an invite link.";
    server_gone = "Server nicht gefunden oder kein Zugriff."
        | "Server not found, or you have no access.";
    channels_load_failed = "Die Kanäle konnten nicht geladen werden." | "Couldn’t load the channels.";
    dms_load_failed = "Die Unterhaltungen konnten nicht geladen werden."
        | "Couldn’t load the conversations.";

    /// The desktop app's own: a stored notification level this version
    /// does not know cannot be saved back.
    notify_unknown_level = "Diese Stufe kennt diese Version von Gelabber nicht. Bitte wähle zuerst eine Stufe."
        | "This version of Gelabber doesn’t know this level. Please pick a level first.";

    /// The desktop app's own: an attachment upload the storage turned down.
    error_upload_failed = "Der Upload ist fehlgeschlagen." | "The upload failed.";
    /// The desktop app's own: an older page of history had nothing readable.
    error_unreadable_page = "keine lesbare Nachricht auf der älteren Seite"
        | "no readable message on the older page";
    user_panel_call_settings = "Anrufeinstellungen" | "Call settings";
}

/// The desktop app's own: the server turned the upgrade down, with why
/// ("HTTP 403").
pub fn connection_refused_with(reason: &str) -> String {
    match get() {
        Locale::De => format!("Keine Verbindung – vom Server abgelehnt ({reason})"),
        Locale::En => format!("No connection – refused by the server ({reason})"),
    }
}

/// "Kategorie {name}": a category's row menu.
pub fn row_menu_category_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Kategorie {name}"),
        Locale::En => format!("Category {name}"),
    }
}

/// "Nichts gefunden für „{query}“."
pub fn switcher_empty(query: &str) -> String {
    match get() {
        Locale::De => format!("Nichts gefunden für „{query}“."),
        Locale::En => format!("Nothing found for “{query}”."),
    }
}

/// "Kanal „{name}“ löschen?"
pub fn channels_delete_channel_title(name: &str) -> String {
    match get() {
        Locale::De => format!("Kanal „{name}“ löschen?"),
        Locale::En => format!("Delete channel “{name}”?"),
    }
}

/// "Kategorie „{name}“ löschen?"
pub fn channels_delete_category_title(name: &str) -> String {
    match get() {
        Locale::De => format!("Kategorie „{name}“ löschen?"),
        Locale::En => format!("Delete category “{name}”?"),
    }
}

/// A category's "…" button.
pub fn category_actions_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Aktionen für Kategorie „{name}“"),
        Locale::En => format!("Actions for category “{name}”"),
    }
}

/// A category's "+" button.
pub fn category_create_channel_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Kanal in „{name}“ erstellen"),
        Locale::En => format!("Create channel in “{name}”"),
    }
}

/// The voice room you sit in: "{name} · verbunden".
pub fn channel_connected(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} · verbunden"),
        Locale::En => format!("{name} · connected"),
    }
}

/// A channel row's (or a conversation's) "…" button.
pub fn channel_actions_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Aktionen für {name}"),
        Locale::En => format!("Actions for {name}"),
    }
}

/// A channel row's pencil.
pub fn channel_edit_label(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} bearbeiten"),
        Locale::En => format!("Edit {name}"),
    }
}

/// A server icon's mention badge.
pub fn rail_mentions(count: u64) -> String {
    match get() {
        Locale::De => plural(
            count,
            format!("{count} Erwähnung"),
            format!("{count} Erwähnungen"),
        ),
        Locale::En => plural(
            count,
            format!("{count} mention"),
            format!("{count} mentions"),
        ),
    }
}

/// "{name} tippt…"
pub fn typing_one(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} tippt…"),
        Locale::En => format!("{name} is typing…"),
    }
}

/// "{first} und {second} tippen…"
pub fn typing_two(first: &str, second: &str) -> String {
    match get() {
        Locale::De => format!("{first} und {second} tippen…"),
        Locale::En => format!("{first} and {second} are typing…"),
    }
}

/// "{first}, {second} und {count} weitere tippen…"
pub fn typing_more(first: &str, second: &str, count: u64) -> String {
    match get() {
        Locale::De => format!("{first}, {second} und {count} weitere tippen…"),
        Locale::En => plural(
            count,
            format!("{first}, {second} and {count} other are typing…"),
            format!("{first}, {second} and {count} others are typing…"),
        ),
    }
}

/// "Willkommen, {name}"
pub fn welcome_title_named(name: &str) -> String {
    match get() {
        Locale::De => format!("Willkommen, {name}"),
        Locale::En => format!("Welcome, {name}"),
    }
}

/// A message toast's accessible name (web chat `toast.openLabel`).
pub fn toast_open_label(place: &str, author: &str, preview: &str) -> String {
    match get() {
        Locale::De => format!("Öffnen: {place}, {author}: {preview}"),
        Locale::En => format!("Open: {place}, {author}: {preview}"),
    }
}

/// A toast for a mention (web chat `toast.openMentionedLabel`).
pub fn toast_open_mentioned_label(place: &str, author: &str, preview: &str) -> String {
    match get() {
        Locale::De => format!("Öffnen: {place}, {author} hat dich erwähnt: {preview}"),
        Locale::En => format!("Open: {place}, {author} mentioned you: {preview}"),
    }
}

/// A Live's toast (web chat `toast.openLiveLabel`).
pub fn toast_open_live_label(place: &str, preview: &str) -> String {
    match get() {
        Locale::De => format!("Öffnen: {place}, {preview}"),
        Locale::En => format!("Open: {place}, {preview}"),
    }
}

/// The desktop app's own: a request that did not reach the server.
pub fn error_transport(detail: &str) -> String {
    match get() {
        Locale::De => format!("Keine Verbindung zum Server ({detail})"),
        Locale::En => format!("No connection to the server ({detail})"),
    }
}

/// The desktop app's own: an answer the app could not read.
pub fn error_decode(detail: &str) -> String {
    match get() {
        Locale::De => format!("Unerwartete Antwort ({detail})"),
        Locale::En => format!("Unexpected answer ({detail})"),
    }
}

/// The desktop app's own: an error answer without a message.
pub fn error_status(status: u16) -> String {
    match get() {
        Locale::De => format!("Fehler {status}"),
        Locale::En => format!("Error {status}"),
    }
}

/// The desktop app's own: a download over its size cap.
pub fn error_too_large(max: usize) -> String {
    match get() {
        Locale::De => format!("mehr als {max} Bytes"),
        Locale::En => format!("more than {max} bytes"),
    }
}
