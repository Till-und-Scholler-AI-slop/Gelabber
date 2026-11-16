//! Blocking people (#565, web/src/i18n/catalogs/blocks.ts): the button and
//! its question, the "Blockiert" settings area, a blocked DM's composer and
//! folded messages on servers.

use crate::{Locale, get, plural};

crate::strings! {
    action_block = "Blockieren" | "Block";
    action_unblock = "Blockierung aufheben" | "Unblock";
    confirm_message = "Die Person kann dir nicht mehr schreiben und keine Freundschaftsanfrage schicken. Du bekommst keine Benachrichtigungen mehr von ihr, und ihre Nachrichten auf gemeinsamen Servern werden eingeklappt. Sie erfährt nichts davon."
        | "They can’t message you or send friend requests. You’ll stop getting their notifications, and their messages on shared servers are collapsed. They are not told.";
    block_failed = "Nicht blockiert." | "Not blocked.";
    unblock_failed = "Blockierung nicht aufgehoben." | "Not unblocked.";
    area_hint = "Wen du blockierst, kann dir nicht schreiben und keine Freundschaftsanfrage schicken. Die Person erfährt es nicht."
        | "People you block can’t message you or send friend requests. They are not told.";
    area_empty = "Du blockierst niemanden." | "You don’t block anyone.";
    area_load_failed = "Die Liste konnte nicht geladen werden." | "Couldn’t load the list.";
    composer_closed = "Du kannst in dieser Unterhaltung keine Nachrichten senden."
        | "You can’t send messages in this conversation.";
    folded_show = "Anzeigen" | "Show";
}

/// "{name} blockieren?"
pub fn confirm_title(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} blockieren?"),
        Locale::En => format!("Block {name}?"),
    }
}

/// "Seit {date}" beside a blocked person.
pub fn area_since(date: &str) -> String {
    match get() {
        Locale::De => format!("Seit {date}"),
        Locale::En => format!("Since {date}"),
    }
}

/// "Du hast {name} blockiert." in place of a blocked DM's composer.
pub fn composer_blocked(name: &str) -> String {
    match get() {
        Locale::De => format!("Du hast {name} blockiert."),
        Locale::En => format!("You blocked {name}."),
    }
}

/// "Blockierte Nachricht" / "{count} blockierte Nachrichten".
pub fn folded(count: u64) -> String {
    match get() {
        Locale::De => plural(
            count,
            "Blockierte Nachricht".to_owned(),
            format!("{count} blockierte Nachrichten"),
        ),
        Locale::En => plural(
            count,
            "Blocked message".to_owned(),
            format!("{count} blocked messages"),
        ),
    }
}
