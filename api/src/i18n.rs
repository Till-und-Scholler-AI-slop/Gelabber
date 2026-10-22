//! Texts the server itself puts in front of people (issue #314): push
//! titles and bodies. Messages in the app are rendered by the client; a push
//! has no client code to run, so its words come from here, in the
//! recipient's [`Locale`].
//!
//! The pages carry the same German words (`web/src/messages/system.ts`,
//! `web/src/messages/notificationPrivacy.ts`, `web/src/messages/mentions.ts`,
//! `web/public/sw.js`).

use crate::messages::system::{self, SystemMessage};

/// The language a recipient reads. Gelabber speaks German only; a second
/// language adds a variant here and a per-account setting that
/// [`Locale::of_recipient`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    De,
}

impl Locale {
    /// Every account reads German until a second language ships.
    pub fn of_recipient(_user: uuid::Uuid) -> Self {
        // TODO(#356): read the account's locale once there is a second one.
        Self::De
    }
}

/// Title of a hidden notification (#268): who wrote, nothing else.
pub fn new_message_from(locale: Locale, author: &str) -> String {
    match locale {
        Locale::De => format!("Neue Nachricht von {author}"),
    }
}

/// What a recalled push says instead of the deleted message (#277).
pub fn message_deleted(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Nachricht gelöscht",
    }
}

/// A channel whose name could not be read any more.
pub fn unnamed_channel(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Kanal",
    }
}

/// A mention of somebody no longer around, without the `@`.
pub fn unknown_user(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Unbekannt",
    }
}

/// A message that is only a GIF.
pub fn gif(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "GIF",
    }
}

/// A message that is only files.
pub fn file(locale: Locale) -> &'static str {
    match locale {
        Locale::De => "Datei",
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_texts() {
        let de = Locale::De;
        assert_eq!(new_message_from(de, "Ada"), "Neue Nachricht von Ada");
        let live = SystemMessage::live_started(uuid::Uuid::new_v4(), "Lounge");
        assert_eq!(system_text(de, &live, "Ada"), "Ada ist live in Lounge.");
        let unknown = SystemMessage {
            kind: "later_kind".into(),
            params: serde_json::json!({}),
        };
        assert_eq!(system_text(de, &unknown, "Ada"), "Systemnachricht von Ada");
    }
}
