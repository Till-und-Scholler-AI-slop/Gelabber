//! Push titles and bodies (issue #314), in the recipient's [`Locale`].
//!
//! The pages carry the same words (`web/src/messages/system.ts`,
//! `web/src/messages/notificationPrivacy.ts`, `web/src/messages/mentions.ts`,
//! `web/public/sw.js`).

use super::Locale;
use crate::messages::system::{self, SystemMessage};

/// Title of a hidden notification (#268): who wrote, nothing else.
pub fn new_message_from(locale: Locale, author: &str) -> String {
    match locale {
        Locale::De => format!("Neue Nachricht von {author}"),
        Locale::En => format!("New message from {author}"),
    }
}

/// What a recalled push says instead of the deleted message (#277).
pub fn message_deleted(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Nachricht gelöscht",
        Locale::En => "Message deleted",
    }
}

/// A channel whose name could not be read any more.
pub fn unnamed_channel(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Kanal",
        Locale::En => "Channel",
    }
}

/// A mention of somebody no longer around, without the `@`.
pub fn unknown_user(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Unbekannt",
        Locale::En => "Unknown",
    }
}

/// A message that is only a GIF.
pub fn gif(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "GIF",
        Locale::En => "GIF",
    }
}

/// A message that is only files.
pub fn file(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Datei",
        Locale::En => "File",
    }
}

/// A system message as a sentence; `author` is the person it is about.
/// The same wording as `systemText` in web/src/messages/system.ts.
pub fn system_text(locale: Locale, message: &SystemMessage, author: &str) -> String {
    match (locale, message.kind.as_str()) {
        (Locale::De, system::LIVE_STARTED) => {
            format!("{author} ist live in {}.", message.param("voice_name"))
        }
        (Locale::De, _) => format!("Systemnachricht von {author}"),
        (Locale::En, system::LIVE_STARTED) => {
            format!("{author} is live in {}.", message.param("voice_name"))
        }
        (Locale::En, _) => format!("System message from {author}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_texts() {
        let de = Locale::De;
        assert_eq!(new_message_from(de, "Ada"), "Neue Nachricht von Ada");
        let live = SystemMessage {
            kind: system::LIVE_STARTED.into(),
            params: serde_json::json!({ "voice_name": "Lounge" }),
        };
        assert_eq!(system_text(de, &live, "Ada"), "Ada ist live in Lounge.");
        let unknown = SystemMessage {
            kind: "later_kind".into(),
            params: serde_json::json!({}),
        };
        assert_eq!(system_text(de, &unknown, "Ada"), "Systemnachricht von Ada");
    }

    #[test]
    fn english_texts() {
        let en = Locale::En;
        assert_eq!(new_message_from(en, "Ada"), "New message from Ada");
        assert_eq!(message_deleted(en), "Message deleted");
        assert_eq!(unnamed_channel(en), "Channel");
        assert_eq!(unknown_user(en), "Unknown");
        assert_eq!(gif(en), "GIF");
        assert_eq!(file(en), "File");
        let live = SystemMessage {
            kind: system::LIVE_STARTED.into(),
            params: serde_json::json!({ "voice_name": "Lounge" }),
        };
        assert_eq!(system_text(en, &live, "Ada"), "Ada is live in Lounge.");
        let unknown = SystemMessage {
            kind: "later_kind".into(),
            params: serde_json::json!({}),
        };
        assert_eq!(system_text(en, &unknown, "Ada"), "System message from Ada");
    }
}
