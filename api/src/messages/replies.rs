//! Replies (issue #251). A message may name an earlier message of the same
//! channel. `reply_to` is stored as sent and never checked: an id that is
//! gone, never existed or belongs to another channel is quoted as deleted,
//! because the quote only joins this channel. So a reply whose original was
//! deleted meanwhile still goes through, and the answer says nothing about
//! messages elsewhere. Payloads carry a compact preview of the original, filled in
//! after the row query like attachments, so the history SQL stays as is.

use serde::Serialize;
use sqlx::{FromRow, PgExecutor};
use uuid::Uuid;

use super::Message;
use super::system::SystemMessage;
use crate::error::ApiError;

/// Characters of the original shown above the reply.
pub const EXCERPT_MAX: usize = 100;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReplyAuthor {
    pub id: Uuid,
    pub name: String,
}

/// The answered message as the reply header shows it. A deleted original
/// is only `{id, deleted: true}`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Reply {
    pub id: Uuid,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<ReplyAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_attachment: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub has_gif: bool,
    /// The original is a system message (#314); the client renders the
    /// excerpt from it, `excerpt` is then empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemMessage>,
}

#[derive(Debug, FromRow)]
struct ReplyRow {
    message_id: Uuid,
    reply_to: Uuid,
    original_id: Option<Uuid>,
    author_id: Option<Uuid>,
    author_name: Option<String>,
    content: Option<String>,
    has_attachment: bool,
    has_gif: bool,
    system_kind: Option<String>,
    system_params: Option<sqlx::types::Json<serde_json::Value>>,
}

impl From<ReplyRow> for Reply {
    fn from(row: ReplyRow) -> Self {
        let (Some(_), Some(id), Some(name), Some(content)) =
            (row.original_id, row.author_id, row.author_name, row.content)
        else {
            return Self::deleted(row.reply_to);
        };
        Self {
            id: row.reply_to,
            deleted: false,
            author: Some(ReplyAuthor { id, name }),
            excerpt: Some(excerpt(&content)),
            has_attachment: row.has_attachment,
            has_gif: row.has_gif,
            system: SystemMessage::from_columns(row.system_kind, row.system_params),
        }
    }
}

impl Reply {
    fn deleted(id: Uuid) -> Self {
        Self {
            id,
            deleted: true,
            author: None,
            excerpt: None,
            has_attachment: false,
            has_gif: false,
            system: None,
        }
    }
}

/// First [`EXCERPT_MAX`] characters on one line. Spoilers (`||text||`) stay
/// hidden, and the cut never splits a mention token (`<@id>`), so the
/// client can still render what it shows.
pub fn excerpt(content: &str) -> String {
    let line = mask_spoilers(&content.split_whitespace().collect::<Vec<_>>().join(" "));
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(EXCERPT_MAX).collect();
    if chars.next().is_some() {
        if let Some(start) = out.rfind("<@")
            && !out[start..].contains('>')
        {
            out.truncate(start);
        }
        out.push('…');
    }
    out
}

/// Every closed `||…||` pair becomes `▒▒▒`; an unpaired `||` stays as typed.
fn mask_spoilers(line: &str) -> String {
    let parts: Vec<&str> = line.split("||").collect();
    let mut out = String::with_capacity(line.len());
    for (index, part) in parts.iter().enumerate() {
        let inside = index % 2 == 1;
        if inside && index + 1 < parts.len() {
            out.push_str("▒▒▒");
        } else {
            if inside {
                out.push_str("||");
            }
            out.push_str(part);
        }
    }
    out
}

// The original must stay in the reply's channel; a row that says otherwise
// is answered as deleted rather than leaking another channel's text.
const REPLIES_SQL: &str = "SELECT m.id AS message_id, m.reply_to, o.id AS original_id, \
        o.author_id, u.name AS author_name, o.content, \
        EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = o.id) AS has_attachment, \
        o.gif IS NOT NULL AS has_gif, o.system_kind, o.system_params \
     FROM messages m \
     LEFT JOIN messages o ON o.id = m.reply_to AND o.channel_id = m.channel_id \
     LEFT JOIN users u ON u.id = o.author_id \
     WHERE m.id = ANY($1) AND m.reply_to IS NOT NULL";

/// Fill `reply` on every message that answers another one.
pub async fn for_messages(
    db: impl PgExecutor<'_>,
    messages: &mut [Message],
) -> Result<(), ApiError> {
    let ids: Vec<Uuid> = messages.iter().map(|m| m.id).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, ReplyRow>(REPLIES_SQL)
        .bind(&ids)
        .fetch_all(db)
        .await?;
    let mut by_message: std::collections::HashMap<Uuid, Reply> = rows
        .into_iter()
        .map(|row| (row.message_id, Reply::from(row)))
        .collect();
    for message in messages {
        message.reply = by_message.remove(&message.id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_is_one_short_line() {
        assert_eq!(excerpt("hallo\n  welt"), "hallo welt");
        let long = "a".repeat(EXCERPT_MAX + 5);
        let cut = excerpt(&long);
        assert_eq!(cut.chars().count(), EXCERPT_MAX + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(
            excerpt(&"ä".repeat(EXCERPT_MAX)).chars().count(),
            EXCERPT_MAX
        );
    }

    #[test]
    fn spoilers_stay_hidden_and_mentions_stay_whole() {
        assert_eq!(excerpt("sag ||Snape stirbt|| nicht"), "sag ▒▒▒ nicht");
        assert_eq!(excerpt("a || b"), "a || b");
        assert_eq!(excerpt("||x|| und ||y"), "▒▒▒ und ||y");
        let id = "<@00000000-0000-0000-0000-000000000001>";
        let cut = excerpt(&format!("{} {id} danach", "a".repeat(EXCERPT_MAX - 10)));
        assert!(!cut.contains("<@"), "{cut}");
        assert!(cut.ends_with('…'));
        assert_eq!(excerpt(&format!("hi {id}")), format!("hi {id}"));
    }
}
