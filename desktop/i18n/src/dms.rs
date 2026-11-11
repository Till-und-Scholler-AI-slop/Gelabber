//! Direct messages (web/src/i18n/catalogs/dms.ts): their sidebar, the list
//! page and the "Neue Nachricht" dialog.

use crate::{Locale, get};

crate::strings! {
    sidebar_label = "Direktnachrichten" | "Direct messages";
    sidebar_title = "Nachrichten" | "Messages";
    sidebar_empty = "Noch keine Unterhaltungen." | "No conversations yet.";
    list_label = "Unterhaltungen" | "Conversations";
    new_message = "Neue Nachricht" | "New message";
    new_message_person = "Person" | "Person";
    new_message_placeholder = "Name suchen" | "Search for a name";
    new_message_continue = "Unterhaltung fortsetzen" | "Continue conversation";
    new_message_nobody = "Keine Personen. Direktnachrichten gehen an Mitglieder deiner Server."
        | "Nobody here. Direct messages go to members of your servers.";
    open_failed = "Unterhaltung nicht geöffnet." | "Conversation not opened.";
    index_load_failed = "Die Direktnachrichten konnten nicht geladen werden."
        | "Couldn’t load your direct messages.";
    index_pick = "Wähle eine Unterhaltung." | "Pick a conversation.";
    index_empty = "Keine Unterhaltungen" | "No conversations";
}

/// "Niemand gefunden für „{query}“."
pub fn new_message_not_found(query: &str) -> String {
    match get() {
        Locale::De => format!("Niemand gefunden für „{query}“."),
        Locale::En => format!("Nobody found for “{query}”."),
    }
}
