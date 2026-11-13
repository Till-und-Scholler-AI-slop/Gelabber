//! The page "Feedback" of Betrieb (`/api/operator/feedback`): operators
//! list, read, answer, close, reopen and delete entries. Every route goes
//! through [`Operator`] and answers `404` while feedback is off; pending
//! entries (public form, not confirmed) are invisible to all of them.
//!
//! Every route but the image writes one audit row, as `operator::reports`
//! does: a read after it succeeded, a mutation inside its transaction. The
//! image is left out, because the list or read that showed it wrote one.

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use super::{Context, Enabled, Image, Kind, MESSAGES_MAX, notify};
use crate::auth::validate;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::operator::Operator;
use crate::operator::accounts::{PAGE, offset};
use crate::operator::audit::{self, Action, Targets};
use crate::path::Id;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/operator/feedback", get(list))
        .route(
            "/api/operator/feedback/{id}",
            get(read).delete(delete_entry),
        )
        .route("/api/operator/feedback/{id}/image", get(image))
        .route("/api/operator/feedback/{id}/messages", post(reply))
        .route("/api/operator/feedback/{id}/done", post(done))
        .route("/api/operator/feedback/{id}/reopen", post(reopen))
}

/// Someone named on an entry: an operator who replied or closed it, or the
/// author of a message.
#[derive(Debug, Clone, Serialize)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
}

/// Who sent an entry.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Sender {
    Account {
        id: Uuid,
        name: String,
        email: String,
        avatar_url: Option<String>,
    },
    Visitor {
        email: String,
        locale: String,
    },
    /// An account entry whose account is gone.
    Deleted,
}

/// One message of a thread as operators see it. `mail` is the state of an
/// operator reply to a visitor (`pending`, `sent`, `failed`), else `null`;
/// `author` is `null` once that account is gone.
#[derive(Debug, Clone, Serialize)]
pub struct OperatorMessage {
    pub id: Uuid,
    /// `person` or `operator`.
    pub from: String,
    pub author: Option<Person>,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub mail: Option<String>,
}

/// An entry as operators see it.
#[derive(Debug, Clone, Serialize)]
pub struct OperatorEntry {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub done_at: Option<DateTime<Utc>>,
    pub done_by: Option<Person>,
    pub sender: Sender,
    pub text: String,
    pub image: Option<Image>,
    pub context: Context,
    /// The last word is the sender's: no messages yet, or the last one is
    /// the person's.
    pub awaiting_reply: bool,
    pub messages: Vec<OperatorMessage>,
}

/// The columns of [`Row`] over `feedback f`, with sender and closer; a
/// macro so the queries stay `&'static str`.
macro_rules! entry_select {
    () => {
        "SELECT f.id, f.kind, f.status, f.body, f.created_at, f.last_activity_at, \
             f.done_at, f.done_by, d.name AS done_by_name, f.user_id, \
             u.name AS user_name, u.email AS user_email, u.avatar_url AS user_avatar_url, \
             f.visitor_email, f.visitor_locale, f.client, f.os, f.app_version, f.locale, \
             f.page, f.image_type, f.image_bytes \
         FROM feedback f \
         LEFT JOIN users u ON u.id = f.user_id \
         LEFT JOIN users d ON d.id = f.done_by "
    };
}

#[derive(Debug, Clone, FromRow)]
struct Row {
    id: Uuid,
    kind: String,
    status: String,
    body: String,
    created_at: DateTime<Utc>,
    last_activity_at: DateTime<Utc>,
    done_at: Option<DateTime<Utc>>,
    done_by: Option<Uuid>,
    done_by_name: Option<String>,
    user_id: Option<Uuid>,
    user_name: Option<String>,
    user_email: Option<String>,
    user_avatar_url: Option<String>,
    visitor_email: Option<String>,
    visitor_locale: Option<String>,
    client: String,
    os: Option<String>,
    app_version: Option<String>,
    locale: String,
    page: Option<String>,
    image_type: Option<String>,
    image_bytes: Option<i32>,
}

#[derive(Debug, Clone, FromRow)]
struct MessageRow {
    id: Uuid,
    feedback_id: Uuid,
    author_kind: String,
    author_id: Option<Uuid>,
    author_name: Option<String>,
    body: String,
    mail_state: Option<String>,
    created_at: DateTime<Utc>,
}

fn person(id: Option<Uuid>, name: Option<String>) -> Option<Person> {
    Some(Person {
        id: id?,
        name: name?,
    })
}

impl Row {
    fn sender(&self) -> Sender {
        if self.client == "form" {
            return Sender::Visitor {
                email: self.visitor_email.clone().unwrap_or_default(),
                locale: self.visitor_locale.clone().unwrap_or_default(),
            };
        }
        match (self.user_id, &self.user_name, &self.user_email) {
            (Some(id), Some(name), Some(email)) => Sender::Account {
                id,
                name: name.clone(),
                email: email.clone(),
                avatar_url: self.user_avatar_url.clone(),
            },
            _ => Sender::Deleted,
        }
    }

    /// The entry with `messages` of this entry in thread order.
    fn into_entry(self, messages: Vec<MessageRow>) -> OperatorEntry {
        let awaiting_reply = messages
            .last()
            .is_none_or(|message| message.author_kind == "person");
        let image = self
            .image_type
            .clone()
            .zip(self.image_bytes)
            .map(|(content_type, size)| Image {
                url: format!("/api/operator/feedback/{}/image", self.id),
                content_type,
                size,
            });
        OperatorEntry {
            sender: self.sender(),
            image,
            context: Context {
                client: self.client,
                os: self.os,
                app_version: self.app_version,
                locale: self.locale,
                page: self.page,
            },
            id: self.id,
            kind: self.kind,
            status: self.status,
            created_at: self.created_at,
            last_activity_at: self.last_activity_at,
            done_at: self.done_at,
            done_by: person(self.done_by, self.done_by_name),
            text: self.body,
            awaiting_reply,
            messages: messages.into_iter().map(OperatorMessage::from).collect(),
        }
    }
}

impl From<MessageRow> for OperatorMessage {
    fn from(row: MessageRow) -> Self {
        Self {
            id: row.id,
            from: row.author_kind,
            author: person(row.author_id, row.author_name),
            text: row.body,
            created_at: row.created_at,
            mail: row.mail_state,
        }
    }
}

/// The columns of [`MessageRow`] over `feedback_messages m`.
macro_rules! message_select {
    () => {
        "SELECT m.id, m.feedback_id, m.author_kind, m.author_id, a.name AS author_name, \
             m.body, m.mail_state, m.created_at \
         FROM feedback_messages m \
         LEFT JOIN users a ON a.id = m.author_id "
    };
}

/// The threads of `ids`, each in order.
async fn threads(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<MessageRow>>, ApiError> {
    let mut threads: HashMap<Uuid, Vec<MessageRow>> = HashMap::new();
    if ids.is_empty() {
        return Ok(threads);
    }
    let rows: Vec<MessageRow> = sqlx::query_as(concat!(
        message_select!(),
        "WHERE m.feedback_id = ANY($1) ORDER BY m.created_at, m.id"
    ))
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    for row in rows {
        threads.entry(row.feedback_id).or_default().push(row);
    }
    Ok(threads)
}

/// The visible entry `id` with its thread, or `404`.
async fn load(conn: &mut PgConnection, id: Uuid) -> Result<OperatorEntry, ApiError> {
    let row: Row = sqlx::query_as(concat!(
        entry_select!(),
        "WHERE f.id = $1 AND f.status <> 'pending'"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ApiError::NotFound)?;
    let messages = threads(conn, &[id]).await?.remove(&id).unwrap_or_default();
    Ok(row.into_entry(messages))
}

/// The audit targets of an entry: itself, and the sender's account if any.
fn targets(id: Uuid, user: Option<Uuid>) -> Targets {
    Targets {
        feedback: Some(id),
        user,
        ..Targets::default()
    }
}

fn sender_id(entry: &OperatorEntry) -> Option<Uuid> {
    match entry.sender {
        Sender::Account { id, .. } => Some(id),
        _ => None,
    }
}

/// Which entries the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Open,
    Done,
}

impl Status {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("open") {
            "open" => Some(Self::Open),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    // Strings, so that a bad value is a field error, not axum's plain 400.
    status: Option<String>,
    kind: Option<String>,
    offset: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EntryPage {
    pub entries: Vec<OperatorEntry>,
    pub total: i64,
}

/// `GET /api/operator/feedback?status=open|done&kind=&offset=`: latest
/// activity first, [`PAGE`] at a time. Without `kind`, every kind.
async fn list(
    _: Enabled,
    State(state): State<AppState>,
    op: Operator,
    Query(params): Query<ListParams>,
) -> Result<Json<EntryPage>, ApiError> {
    let mut errors = FieldErrors::new();
    let status = Status::parse(params.status.as_deref()).or_else(|| {
        errors.insert("status", "invalid");
        None
    });
    let kind = match params.kind.as_deref() {
        None => None,
        Some(raw) => {
            let kind = Kind::parse(raw);
            if kind.is_none() {
                errors.insert("kind", "invalid");
            }
            kind
        }
    };
    let offset = offset(params.offset.as_deref(), &mut errors);
    validate::finish(errors)?;
    let status = status.expect("validated");
    let kind = kind.map(Kind::as_str);

    let mut conn = state.db.acquire().await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM feedback \
         WHERE status = $1 AND ($2::text IS NULL OR kind = $2)",
    )
    .bind(status.as_str())
    .bind(kind)
    .fetch_one(&mut *conn)
    .await?;
    let rows: Vec<Row> = sqlx::query_as(concat!(
        entry_select!(),
        "WHERE f.status = $1 AND ($2::text IS NULL OR f.kind = $2) \
         ORDER BY f.last_activity_at DESC, f.id DESC LIMIT $3 OFFSET $4"
    ))
    .bind(status.as_str())
    .bind(kind)
    .bind(PAGE)
    .bind(offset)
    .fetch_all(&mut *conn)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let mut threads = threads(&mut conn, &ids).await?;
    let entries = rows
        .into_iter()
        .map(|row| {
            let messages = threads.remove(&row.id).unwrap_or_default();
            row.into_entry(messages)
        })
        .collect();

    audit::record(
        &mut conn,
        &op,
        Action::FeedbackList,
        Targets::default(),
        json!({ "status": status.as_str(), "kind": kind, "offset": offset }),
    )
    .await?;
    Ok(Json(EntryPage { entries, total }))
}

/// `GET /api/operator/feedback/{id}`: one entry with its thread.
async fn read(
    _: Enabled,
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<OperatorEntry>, ApiError> {
    let mut conn = state.db.acquire().await?;
    let entry = load(&mut conn, id).await?;
    audit::record(
        &mut conn,
        &op,
        Action::FeedbackRead,
        targets(id, sender_id(&entry)),
        json!({}),
    )
    .await?;
    Ok(Json(entry))
}

/// `GET /api/operator/feedback/{id}/image`: the entry's image, served as
/// the sender's route serves it. No audit row (see the module comment).
async fn image(
    _: Enabled,
    State(state): State<AppState>,
    _op: Operator,
    Id(id): Id,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (key, content_type, size): (String, String, i32) = sqlx::query_as(
        "SELECT image_key, image_type, image_bytes FROM feedback \
         WHERE id = $1 AND status <> 'pending' AND image_key IS NOT NULL",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    super::serve_image(&state, &key, &content_type, size, &headers).await
}

#[derive(Debug, Deserialize)]
pub struct ReplyBody {
    #[serde(default)]
    text: Option<String>,
}

/// `POST /api/operator/feedback/{id}/messages`: an operator answers. A
/// reply to a visitor goes out by mail (`mail_state` starts `pending`);
/// one to an account marks the entry unread for its sender, and only the
/// reply that sets the mark sends the "there is an answer" mail. A reply
/// on a done entry leaves it done.
async fn reply(
    _: Enabled,
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
    Body(body): Body<ReplyBody>,
) -> Result<(StatusCode, Json<OperatorMessage>), ApiError> {
    let mut errors = FieldErrors::new();
    let text = super::text(body.text.as_deref(), &mut errors);
    validate::finish(errors)?;
    let Some(text) = text else {
        return Err(ApiError::Internal(
            "feedback: validated text missing".into(),
        ));
    };
    let mut tx = state.db.begin().await?;
    let (client, user_id, unread_was_null): (String, Option<Uuid>, bool) = sqlx::query_as(
        "SELECT client, user_id, person_unread_since IS NULL FROM feedback \
         WHERE id = $1 AND status <> 'pending' FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM feedback_messages WHERE feedback_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if count >= MESSAGES_MAX {
        let mut errors = FieldErrors::new();
        errors.insert("text", "too_many");
        return Err(ApiError::Validation(errors));
    }
    let visitor = client == "form";
    let message: MessageRow = sqlx::query_as(
        "WITH m AS (INSERT INTO feedback_messages (feedback_id, author_kind, author_id, body, \
             mail_state) \
         VALUES ($1, 'operator', $2, $3, $4) \
         RETURNING id, feedback_id, author_kind, author_id, body, mail_state, created_at) \
         SELECT m.id, m.feedback_id, m.author_kind, m.author_id, a.name AS author_name, \
             m.body, m.mail_state, m.created_at \
         FROM m LEFT JOIN users a ON a.id = m.author_id",
    )
    .bind(id)
    .bind(op.user.id)
    .bind(&text)
    .bind(visitor.then_some("pending"))
    .fetch_one(&mut *tx)
    .await?;
    // Only an account that still exists has someone to read the reply.
    let first_unread = user_id.is_some() && unread_was_null;
    sqlx::query(
        "UPDATE feedback SET last_activity_at = now(), \
             person_unread_since = CASE WHEN user_id IS NULL THEN person_unread_since \
                 ELSE COALESCE(person_unread_since, now()) END \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let mail = visitor && state.mail.is_some();
    audit::record(
        &mut tx,
        &op,
        Action::FeedbackReply,
        targets(id, user_id),
        json!({ "chars": text.chars().count(), "mail": mail }),
    )
    .await?;
    tx.commit().await?;
    notify::operator_reply(&state, message.id, first_unread);
    Ok((StatusCode::CREATED, Json(message.into())))
}

/// `POST /api/operator/feedback/{id}/done`: closes the entry. Done already,
/// it changes nothing and says so in the audit row.
async fn done(
    _: Enabled,
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<OperatorEntry>, ApiError> {
    set_status(&state, &op, id, true).await.map(Json)
}

/// `POST /api/operator/feedback/{id}/reopen`: opens a done entry again.
async fn reopen(
    _: Enabled,
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<OperatorEntry>, ApiError> {
    set_status(&state, &op, id, false).await.map(Json)
}

/// Marks the entry done or open, in one transaction with its audit row.
/// `last_activity_at` stays: closing is no activity in the thread.
async fn set_status(
    state: &AppState,
    op: &Operator,
    id: Uuid,
    to_done: bool,
) -> Result<OperatorEntry, ApiError> {
    let mut tx = state.db.begin().await?;
    let (status, user_id): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status, user_id FROM feedback WHERE id = $1 AND status <> 'pending' FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    let changed = (status == "done") != to_done;
    if changed {
        if to_done {
            sqlx::query(
                "UPDATE feedback SET status = 'done', done_at = now(), done_by = $2 \
                 WHERE id = $1",
            )
            .bind(id)
            .bind(op.user.id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                "UPDATE feedback SET status = 'open', done_at = NULL, done_by = NULL \
                 WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    }
    let action = if to_done {
        Action::FeedbackDone
    } else {
        Action::FeedbackReopen
    };
    audit::record(
        &mut tx,
        op,
        action,
        targets(id, user_id),
        json!({ "changed": changed }),
    )
    .await?;
    let entry = load(&mut tx, id).await?;
    tx.commit().await?;
    Ok(entry)
}

/// `DELETE /api/operator/feedback/{id}`: the entry goes with its thread.
/// Messages cascade, and the `feedback_image_cleanup` trigger queues the
/// image. The audit row keeps no text and no address.
async fn delete_entry(
    _: Enabled,
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    let (kind, client, user_id, had_image): (String, String, Option<Uuid>, bool) = sqlx::query_as(
        "SELECT kind, client, user_id, image_key IS NOT NULL FROM feedback \
             WHERE id = $1 AND status <> 'pending' FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    let messages: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM feedback_messages WHERE feedback_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("DELETE FROM feedback WHERE id = $1 AND status <> 'pending'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        &op,
        Action::FeedbackDelete,
        targets(id, user_id),
        json!({
            "kind": kind,
            "client": client,
            "had_image": had_image,
            "messages": messages,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_open_and_done() {
        assert_eq!(Status::parse(None), Some(Status::Open));
        assert_eq!(Status::parse(Some("done")), Some(Status::Done));
        assert_eq!(Status::parse(Some("pending")), None);
        assert_eq!(Status::parse(Some("closed")), None);
    }

    #[test]
    fn senders_are_tagged_by_type() {
        assert_eq!(
            serde_json::to_value(Sender::Deleted).unwrap(),
            json!({ "type": "deleted" })
        );
        assert_eq!(
            serde_json::to_value(Sender::Visitor {
                email: "ada@example.org".into(),
                locale: "de".into(),
            })
            .unwrap(),
            json!({ "type": "visitor", "email": "ada@example.org", "locale": "de" })
        );
    }
}
