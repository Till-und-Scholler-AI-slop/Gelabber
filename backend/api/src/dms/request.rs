//! Message requests (#563): a DM started with someone who shares a server
//! but is not a friend. Until the recipient accepts, the sender may write
//! [`MAX_MESSAGES`] messages of plain text: no links, files, GIFs or
//! mentions, refused here rather than shown unclickable, so phishing text
//! never lands in an inbox. The recipient cannot write before accepting.
//!
//! The limit counts every message sent into the request
//! (`channels.dm_request_sent`), deleted ones too, so deleting and sending
//! again neither passes it nor repeats the push. A declined request is
//! hidden from the recipient and says nothing to the sender: what is left
//! of the limit can still be written, stored without a push, and the
//! sender's view is the same as for a request nobody answers.

use sqlx::{FromRow, PgConnection, PgExecutor};
use uuid::Uuid;

use crate::error::{ApiError, FieldErrors};

pub const MAX_MESSAGES: i64 = 3;

/// What a write into a DM may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// An ordinary DM.
    Open,
    /// A message request from the writer to `to`. `announce`: the first
    /// message, which pushes "X wants to message you" and shows the
    /// request; `silent`: written after a decline.
    Request {
        to: Uuid,
        announce: bool,
        silent: bool,
    },
}

/// The parts of a message the request rules look at.
pub struct Draft<'a> {
    pub content: &'a str,
    pub files: bool,
    pub gif: bool,
}

#[derive(FromRow)]
struct Row {
    request_by: Option<Uuid>,
    sent: i16,
    declined: bool,
}

async fn row<'e, E: PgExecutor<'e>>(db: E, channel_id: Uuid) -> Result<Row, ApiError> {
    sqlx::query_as::<_, Row>(
        "SELECT dm_request_by AS request_by, dm_request_sent AS sent, \
                dm_declined_at IS NOT NULL AS declined \
         FROM channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

/// The early, read-only check before uploads and mentions are looked at:
/// refuses what [`gate`] would refuse on the draft alone.
pub async fn precheck<'e, E: PgExecutor<'e>>(
    db: E,
    channel_id: Uuid,
    user_id: Uuid,
    draft: &Draft<'_>,
) -> Result<(), ApiError> {
    match row(db, channel_id).await?.request_by {
        None => Ok(()),
        Some(by) if by != user_id => Err(ApiError::MessageRequestPending),
        Some(_) => refuse(draft),
    }
}

/// Under the channel lock (`messages::lock_access`), right before the
/// insert: the authoritative check, which also counts the message.
pub async fn gate(
    tx: &mut PgConnection,
    channel_id: Uuid,
    user_id: Uuid,
    draft: &Draft<'_>,
) -> Result<Gate, ApiError> {
    let current = row(&mut *tx, channel_id).await?;
    let Some(by) = current.request_by else {
        return Ok(Gate::Open);
    };
    if by != user_id {
        return Err(ApiError::MessageRequestPending);
    }
    refuse(draft)?;
    let sent = i64::from(current.sent);
    if sent >= MAX_MESSAGES {
        return Err(ApiError::MessageRequestLimit);
    }
    // In the insert's transaction: a send that fails later takes it back.
    sqlx::query("UPDATE channels SET dm_request_sent = dm_request_sent + 1 WHERE id = $1")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    let to: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM channel_members WHERE channel_id = $1 AND user_id <> $2",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(Gate::Request {
        to,
        announce: !current.declined && sent == 0,
        silent: current.declined,
    })
}

/// An edit by the sender of a pending request follows the same text rules.
pub async fn check_edit(
    tx: &mut PgConnection,
    channel_id: Uuid,
    user_id: Uuid,
    content: &str,
) -> Result<(), ApiError> {
    match row(&mut *tx, channel_id).await?.request_by {
        Some(by) if by == user_id => refuse(&Draft {
            content,
            files: false,
            gif: false,
        }),
        _ => Ok(()),
    }
}

fn refuse(draft: &Draft<'_>) -> Result<(), ApiError> {
    let fail = |field: &'static str, code: &'static str| {
        Err(ApiError::Validation(FieldErrors::from([(field, code)])))
    };
    if draft.files {
        return fail("attachment_ids", "request_text_only");
    }
    if draft.gif {
        return fail("gif", "request_text_only");
    }
    if has_mention(draft.content) {
        return fail("content", "request_mention");
    }
    if has_link(draft.content) {
        return fail("content", "request_link");
    }
    Ok(())
}

/// A user token (`<@id>`), `@everyone` or `@here`, wherever it stands: in a
/// request even a mention in code is refused.
pub fn has_mention(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("<@") || lower.contains("@everyone") || lower.contains("@here")
}

/// Anything that reads as a link, stricter than the clients' linkifier: a
/// scheme (`https://`, `mailto:` …), `www.`, a host name like `evil.example`
/// or `bit.ly/x`, or an IPv4 address. A dotted word without spaces such as
/// `ende.Neu` counts too; that is the price of not letting `evil.com` through.
/// Invisible format characters and markdown marks are dropped first, so
/// `evil\u{200B}.com` or `evil**.**com`, which read as `evil.com`, count.
pub fn has_link(content: &str) -> bool {
    let lower: String = content
        .chars()
        .filter(|c| !invisible(*c) && !matches!(c, '*' | '_' | '~' | '`' | '|'))
        .collect::<String>()
        .to_lowercase();
    if lower.contains("://") || lower.contains("www.") || lower.contains("mailto:") {
        return true;
    }
    lower
        .split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '-' || c == '_'))
        .any(host_like)
}

/// Format characters (Unicode `Cf`) a text can hide in: soft hyphen, zero
/// width and joiners, direction marks and isolates, word joiner, BOM.
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}' | '\u{FEFF}')
}

fn host_like(token: &str) -> bool {
    let token = token.trim_matches(|c| c == '.' || c == '-' || c == '_');
    let labels: Vec<&str> = token.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| label.is_empty()) {
        return false;
    }
    let last = labels[labels.len() - 1];
    if last.chars().count() >= 2 && last.chars().all(char::is_alphabetic) {
        return true;
    }
    labels.len() == 4
        && labels
            .iter()
            .all(|label| label.len() <= 3 && label.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_in_any_shape_are_found() {
        for text in [
            "https://evil.example/login",
            "see HTTP://x.y",
            "www.example",
            "mailto:me",
            "go to evil.com now",
            "bit.ly/abc",
            "(paypal.de)",
            "[click](evil.com)",
            "10.0.0.1",
            "münchen.de",
            "Ende.Neu",
            "evil\u{200B}.com",
            "evil\u{00AD}.com",
            "evil**.**com",
            "`evil`.`com`",
            "ev_il._com_",
        ] {
            assert!(has_link(text), "{text}");
        }
        for text in [
            "Hallo, wie geht's?",
            "z.B. morgen",
            "v0.5 ist raus",
            "1.5 Stunden",
            "Ende. Neuer Satz",
            "...",
            "a.b",
        ] {
            assert!(!has_link(text), "{text}");
        }
    }

    #[test]
    fn mentions_in_any_shape_are_found() {
        assert!(has_mention("hi <@00000000-0000-0000-0000-000000000001>"));
        assert!(has_mention("@everyone look"));
        assert!(has_mention("`@HERE`"));
        assert!(!has_mention("mail me at home"));
    }

    #[test]
    fn drafts_are_refused_by_kind() {
        let code = |draft: Draft<'_>| match refuse(&draft) {
            Err(ApiError::Validation(fields)) => fields.into_iter().next().unwrap(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            code(Draft {
                content: "hi",
                files: true,
                gif: false
            }),
            ("attachment_ids", "request_text_only")
        );
        assert_eq!(
            code(Draft {
                content: "",
                files: false,
                gif: true
            }),
            ("gif", "request_text_only")
        );
        assert_eq!(
            code(Draft {
                content: "evil.com",
                files: false,
                gif: false
            }),
            ("content", "request_link")
        );
        assert!(
            refuse(&Draft {
                content: "Hallo!",
                files: false,
                gif: false
            })
            .is_ok()
        );
    }
}
