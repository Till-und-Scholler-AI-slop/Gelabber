//! Every mail of the feedback package. Each send runs after the response,
//! in a spawned task, and never holds up the route that triggers it.
//!
//! - [`new_entry`] and [`person_reply`] send [`Mail::FeedbackNotice`] to
//!   every operator with a confirmed, enabled account, in that account's
//!   language, from the `Notice` budget. The sender is not skipped: an
//!   operator who writes feedback hears that it arrived.
//! - [`operator_reply`] sends [`Mail::FeedbackAnswered`] to the sender's
//!   account once per unread stretch (`Notice` budget), or the reply itself
//!   to a visitor ([`Mail::FeedbackReply`], no budget: operators are
//!   trusted, and a dropped reply would be silent) and records the outcome
//!   in `feedback_messages.mail_state`.
//!
//! Nothing is retried. Failures log the entry and the mail's kind, never an
//! address.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use tracing::{info, warn};
use uuid::Uuid;

use super::Kind;
use crate::auth;
use crate::error::ApiError;
use crate::i18n::Locale;
use crate::limits::{self, MailBudget};
use crate::mail::Mail;
use crate::operator::access::is_operator_address;
use crate::state::AppState;

/// Lines of the text an operator notice quotes.
const EXCERPT_LINES: usize = 3;
/// Characters of the text an operator notice quotes, before the `…`.
const EXCERPT_CHARS: usize = 300;

/// A new entry reached the operators (created, or a visitor's confirmed).
pub fn new_entry(state: &AppState, feedback_id: Uuid) {
    if state.mail.is_none() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(err) = notice(&state, feedback_id, None).await {
            warn!(%feedback_id, kind = "feedback_notice", error = ?err, "feedback mail failed");
        }
    });
}

/// The sender replied `text` in the thread of `feedback_id`. The notice
/// quotes that reply, not whichever is newest when the task runs.
pub fn person_reply(state: &AppState, feedback_id: Uuid, text: String) {
    if state.mail.is_none() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(err) = notice(&state, feedback_id, Some(text)).await {
            warn!(%feedback_id, kind = "feedback_notice", error = ?err, "feedback mail failed");
        }
    });
}

/// An operator replied with `message_id`. `first_unread` is true when the
/// reply set the sender's unread mark, so one mail goes out per unread
/// stretch.
pub fn operator_reply(state: &AppState, message_id: Uuid, first_unread: bool) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(err) = answer(&state, message_id, first_unread).await {
            warn!(%message_id, error = ?err, "feedback reply mail failed");
        }
    });
}

#[derive(FromRow)]
struct NoticeRow {
    kind: String,
    body: String,
    client: String,
    visitor_email: Option<String>,
    sender_name: Option<String>,
    sender_email: Option<String>,
}

/// Mails every operator about the entry, or with `reply` about that reply
/// of its sender.
async fn notice(
    state: &AppState,
    feedback_id: Uuid,
    reply: Option<String>,
) -> Result<(), ApiError> {
    let Some(mailer) = state.mail.clone() else {
        return Ok(());
    };
    let row: Option<NoticeRow> = sqlx::query_as(
        "SELECT f.kind, f.body, f.client, f.visitor_email, \
                u.name AS sender_name, u.email AS sender_email \
         FROM feedback f LEFT JOIN users u ON u.id = f.user_id \
         WHERE f.id = $1 AND f.status <> 'pending'",
    )
    .bind(feedback_id)
    .fetch_optional(&state.db)
    .await?;
    // Deleted meanwhile.
    let Some(row) = row else {
        return Ok(());
    };
    let Some(kind) = Kind::parse(&row.kind) else {
        return Err(ApiError::Internal(format!("feedback kind {}", row.kind)));
    };
    let sender = match (
        row.client.as_str(),
        row.visitor_email,
        row.sender_name,
        row.sender_email,
    ) {
        ("form", Some(email), _, _) => email,
        (_, _, Some(name), Some(email)) => format!("{name} ({email})"),
        // The sender's account is gone: nobody is left to name.
        _ => return Ok(()),
    };
    let is_reply = reply.is_some();
    let text = reply.unwrap_or(row.body);
    let mail = Mail::FeedbackNotice {
        kind,
        reply: is_reply,
        sender,
        excerpt: excerpt(&text),
        link: format!(
            "{}/betrieb/feedback?entry={feedback_id}",
            mailer.public_url()
        ),
    };
    let canonical: Vec<String> = state.operators.iter().cloned().collect();
    let recipients: Vec<(Uuid, String, Locale)> = sqlx::query_as(
        "SELECT id, email, locale FROM users \
         WHERE lower(email) = ANY($1) AND email_verified_at IS NOT NULL \
           AND disabled_at IS NULL AND deleted_at IS NULL",
    )
    .bind(&canonical)
    .fetch_all(&state.db)
    .await?;
    for (user_id, email, locale) in recipients {
        if !is_operator_address(state, &email, true) {
            continue;
        }
        if limits::mail_allowed(state, MailBudget::Notice, &email) {
            info!(%feedback_id, %user_id, kind = mail.kind(), "feedback mail");
            auth::send_mail_to(state, &email, locale, mail.clone(), MailBudget::Notice);
        }
    }
    Ok(())
}

#[derive(FromRow)]
struct AnswerRow {
    feedback_id: Uuid,
    body: String,
    kind: String,
    created_at: DateTime<Utc>,
    client: String,
    user_id: Option<Uuid>,
    visitor_email: Option<String>,
    visitor_locale: Option<Locale>,
}

/// The mail an operator's reply sends: "there is an answer" to an account,
/// the reply itself to a visitor.
async fn answer(state: &AppState, message_id: Uuid, first_unread: bool) -> Result<(), ApiError> {
    let row: Option<AnswerRow> = sqlx::query_as(
        "SELECT m.feedback_id, m.body, f.kind, f.created_at, f.client, f.user_id, \
                f.visitor_email, f.visitor_locale \
         FROM feedback_messages m JOIN feedback f ON f.id = m.feedback_id \
         WHERE m.id = $1",
    )
    .bind(message_id)
    .fetch_optional(&state.db)
    .await?;
    // Deleted meanwhile.
    let Some(row) = row else {
        return Ok(());
    };
    let feedback_id = row.feedback_id;
    if row.client == "form" {
        let sent = visitor_reply(state, row).await;
        sqlx::query(
            "UPDATE feedback_messages SET mail_state = $2 \
             WHERE id = $1 AND mail_state = 'pending'",
        )
        .bind(message_id)
        .bind(if sent { "sent" } else { "failed" })
        .execute(&state.db)
        .await?;
        return Ok(());
    }
    if !first_unread || state.mail.is_none() {
        return Ok(());
    }
    let Some(user_id) = row.user_id else {
        return Ok(());
    };
    let account: Option<(String, Locale)> = sqlx::query_as(
        "SELECT email, locale FROM users \
         WHERE id = $1 AND email_verified_at IS NOT NULL \
           AND disabled_at IS NULL AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((email, locale)) = account else {
        return Ok(());
    };
    if limits::mail_allowed(state, MailBudget::Notice, &email) {
        let mail = Mail::FeedbackAnswered;
        info!(%feedback_id, %user_id, kind = mail.kind(), "feedback mail");
        auth::send_mail_to(state, &email, locale, mail, MailBudget::Notice);
    }
    Ok(())
}

/// Sends an operator's reply to the visitor; `false` when it did not go
/// out (mail off, or the relay refused it).
async fn visitor_reply(state: &AppState, row: AnswerRow) -> bool {
    let feedback_id = row.feedback_id;
    let Some(mailer) = state.mail.clone() else {
        return false;
    };
    let (Some(email), Some(kind)) = (row.visitor_email, Kind::parse(&row.kind)) else {
        return false;
    };
    let mail = Mail::FeedbackReply {
        kind,
        sent_on: row.created_at,
        text: row.body,
    };
    let locale = row.visitor_locale.unwrap_or_default();
    match mailer.send(&email, &mail, locale).await {
        Ok(()) => {
            info!(%feedback_id, kind = mail.kind(), "feedback mail sent");
            true
        }
        Err(err) => {
            warn!(%feedback_id, kind = mail.kind(), error = %err, "feedback mail failed");
            false
        }
    }
}

/// The start of `text` an operator notice quotes: its first three lines, at
/// most 300 characters, with `…` when anything was cut.
pub fn excerpt(text: &str) -> String {
    let mut lines = text.lines();
    let kept: Vec<&str> = lines.by_ref().take(EXCERPT_LINES).collect();
    let mut cut = lines.next().is_some();
    let joined = kept.join("\n");
    let mut out: String = joined.chars().take(EXCERPT_CHARS).collect();
    if out.len() < joined.len() {
        cut = true;
    }
    if cut {
        out = out.trim_end().to_owned();
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_excerpt_keeps_three_lines_and_300_characters() {
        assert_eq!(excerpt("kurz"), "kurz");
        assert_eq!(excerpt("a\nb\nc"), "a\nb\nc");
        assert_eq!(excerpt("a\nb\nc\nd"), "a\nb\nc…");
        let long = "ä".repeat(301);
        assert_eq!(excerpt(&long), format!("{}…", "ä".repeat(300)));
        assert_eq!(excerpt(&"ä".repeat(300)), "ä".repeat(300));
        assert_eq!(excerpt("a\r\nb"), "a\nb");
    }
}
