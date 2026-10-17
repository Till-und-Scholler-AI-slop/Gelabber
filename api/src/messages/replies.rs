//! Replies (issue #251). A message may name an earlier message of the same
//! channel. Payloads carry a compact preview of the original, filled in
//! after the row query like attachments, so the history SQL stays as is.

use serde::Serialize;
use sqlx::{FromRow, PgConnection, PgExecutor};
use uuid::Uuid;

use super::Message;
use crate::error::{ApiError, FieldErrors};

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
        }
    }
}

/// First [`EXCERPT_MAX`] characters on one line.
pub fn excerpt(content: &str) -> String {
    let line = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = line.chars();
    let mut out: String = chars.by_ref().take(EXCERPT_MAX).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

// The original must stay in the reply's channel; a row that says otherwise
// is answered as deleted rather than leaking another channel's text.
const REPLIES_SQL: &str = "SELECT m.id AS message_id, m.reply_to, o.id AS original_id, \
        o.author_id, u.name AS author_name, o.content, \
        EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = o.id) AS has_attachment, \
        o.gif IS NOT NULL AS has_gif \
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

/// A new reply's target: a message of this channel. Run under the channel
/// lock, which deletes take too, so the original cannot vanish before the
/// insert commits.
pub async fn check_target(
    db: &mut PgConnection,
    channel_id: Uuid,
    reply_to: Uuid,
) -> Result<(), ApiError> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM messages WHERE id = $1 AND channel_id = $2")
            .bind(reply_to)
            .bind(channel_id)
            .fetch_optional(db)
            .await?;
    found
        .map(|_| ())
        .ok_or_else(|| ApiError::Validation(FieldErrors::from([("reply_to", "invalid")])))
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
}
