//! `/api/channels/{id}/messages` and `/api/messages/{id}` (issue #5 + #15).
//!
//! REST carries history and write rights. The client sends optimistic and
//! virtualises the list. After a successful write this module calls
//! [`publish_channel`]; Redis failure is logged and does not fail the request.
//!
//! Rights: any member may read a text channel's history. `send_messages` is
//! required to post or edit. The author may always delete their own row;
//! `manage_messages` deletes someone else's on a server. DMs (`kind = dm`)
//! use these same routes; both participants may write. Voice channels
//! answer `404` — they have no message resource. A foreign/unknown channel
//! is the same `404`.

pub mod validate;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, patch};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::{info, warn};
use uuid::Uuid;

use crate::attachments::{self, Attachment};
use crate::auth::session::CurrentUser;
use crate::auth::user::User;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::{EventKind, publish_channel};
use crate::json::Body;
use crate::path::Id;
use crate::servers::channel::{self, ChannelKind};
use crate::servers::membership::{self, Membership};
use crate::servers::permissions::Permission;
use crate::state::AppState;

use self::validate::Cursor;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/channels/{id}/messages",
            get(list_messages).post(create_message),
        )
        .route(
            "/api/messages/{id}",
            patch(update_message).delete(delete_message),
        )
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Author {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Message {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author: Author,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MessagePage {
    /// Oldest → newest within this page, so the client can append/prepend
    /// without reversing.
    pub messages: Vec<Message>,
    /// More rows exist in the direction we paged: older for the default /
    /// `before` page, newer for `after`.
    pub has_more: bool,
}

#[derive(Debug, FromRow)]
struct MessageRow {
    id: Uuid,
    channel_id: Uuid,
    author_id: Uuid,
    author_name: String,
    author_avatar_url: Option<String>,
    content: String,
    created_at: DateTime<Utc>,
    edited_at: Option<DateTime<Utc>>,
}

impl From<MessageRow> for Message {
    fn from(row: MessageRow) -> Self {
        Self {
            id: row.id,
            channel_id: row.channel_id,
            author: Author {
                id: row.author_id,
                name: row.author_name,
                avatar_url: row.author_avatar_url,
            },
            content: row.content,
            created_at: row.created_at,
            edited_at: row.edited_at,
            attachments: Vec::new(),
        }
    }
}

#[derive(Debug, FromRow)]
struct MessageInsert {
    id: Uuid,
    channel_id: Uuid,
    content: String,
    created_at: DateTime<Utc>,
    edited_at: Option<DateTime<Utc>>,
}

impl MessageInsert {
    fn into_message(self, user: &User) -> Message {
        Message {
            id: self.id,
            channel_id: self.channel_id,
            author: Author {
                id: user.id,
                name: user.name.clone(),
                avatar_url: user.avatar_url.clone(),
            },
            content: self.content,
            created_at: self.created_at,
            edited_at: self.edited_at,
            attachments: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub attachment_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    pub before: Option<String>,
    pub after: Option<String>,
    pub limit: Option<i64>,
}

async fn list_messages(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    Query(params): Query<ListParams>,
) -> Result<Json<MessagePage>, ApiError> {
    messaging_channel(&state.db, channel_id, user.id).await?;

    let mut errors = FieldErrors::new();
    let before = validate::cursor(params.before.as_deref(), "before", &mut errors);
    let after = validate::cursor(params.after.as_deref(), "after", &mut errors);
    let limit = validate::limit(params.limit, &mut errors);
    validate::finish(errors)?;
    validate::both_bounds_rejected(before, after)?;
    let limit = limit.expect("validated");

    let mut page = load_page(&state.db, channel_id, before, after, limit).await?;
    attachments::for_messages(&state.db, &mut page.messages).await?;
    Ok(Json(page))
}

async fn create_message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    Body(body): Body<CreateBody>,
) -> Result<Response, ApiError> {
    let access = messaging_channel(&state.db, channel_id, user.id).await?;
    access.require_send()?;

    let mut errors = FieldErrors::new();
    let content = validate::content_optional(&body.content, &mut errors);
    let attachment_ids =
        crate::attachments::validate::attachment_ids(&body.attachment_ids, &mut errors);
    validate::finish(errors)?;
    let content = content.expect("validated");
    let attachment_ids = attachment_ids.expect("validated");
    if content.is_empty() && attachment_ids.is_empty() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "content", "required",
        )])));
    }
    if !attachment_ids.is_empty() {
        access.require_send_files()?;
    }

    let mut tx = state.db.begin().await?;
    let row = sqlx::query_as::<_, MessageInsert>(
        "INSERT INTO messages (channel_id, author_id, content) VALUES ($1, $2, $3) \
         RETURNING id, channel_id, content, created_at, edited_at",
    )
    .bind(channel_id)
    .bind(user.id)
    .bind(&content)
    .fetch_one(&mut *tx)
    .await?;
    let bound = attachments::bind_to_message(
        &mut tx,
        &state.store,
        row.id,
        channel_id,
        user.id,
        &attachment_ids,
    )
    .await?;
    tx.commit().await?;
    let mut message = row.into_message(&user);
    message.attachments = bound;
    info!(channel_id = %channel_id, message_id = %message.id, "message created");
    fanout(
        &state,
        access.event_server_id(),
        channel_id,
        EventKind::C,
        Some(message.id),
        serde_json::to_value(&message).ok(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(message)).into_response())
}

async fn update_message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(message_id): Id,
    Body(body): Body<UpdateBody>,
) -> Result<Json<Message>, ApiError> {
    let (access, current) = message_for(&state.db, message_id, user.id).await?;
    if current.author.id != user.id {
        return Err(ApiError::Forbidden("You can only edit your own messages."));
    }
    access.require_send()?;

    let mut errors = FieldErrors::new();
    let content = validate::content_optional(&body.content, &mut errors);
    validate::finish(errors)?;
    let content = content.expect("validated");

    if content.is_empty() && current.attachments.is_empty() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "content", "required",
        )])));
    }

    if content == current.content {
        return Ok(Json(current));
    }

    let row = sqlx::query_as::<_, MessageInsert>(
        "UPDATE messages SET content = $2, edited_at = now() WHERE id = $1 \
         RETURNING id, channel_id, content, created_at, edited_at",
    )
    .bind(message_id)
    .bind(&content)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let mut message = row.into_message(&user);
    message.attachments = attachments::for_message(&state.db, message.id).await?;
    fanout(
        &state,
        access.event_server_id(),
        message.channel_id,
        EventKind::E,
        Some(message.id),
        serde_json::to_value(&message).ok(),
    )
    .await;
    Ok(Json(message))
}

async fn delete_message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(message_id): Id,
) -> Result<StatusCode, ApiError> {
    let (access, current) = message_for(&state.db, message_id, user.id).await?;
    if current.author.id != user.id {
        access.require_manage_messages()?;
    }

    attachments::drop_objects(&state.store, message_id, &state.db).await;
    sqlx::query("DELETE FROM messages WHERE id = $1")
        .bind(message_id)
        .execute(&state.db)
        .await?;
    info!(
        channel_id = %current.channel_id,
        message_id = %message_id,
        "message deleted"
    );
    fanout(
        &state,
        access.event_server_id(),
        current.channel_id,
        EventKind::D,
        Some(message_id),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Redis fan-out after a successful write. A down or dummy Redis URL
/// (`tests` use `127.0.0.1:1`) must not fail the HTTP response — the row
/// is already persisted.
async fn fanout(
    state: &AppState,
    server_id: Uuid,
    channel_id: Uuid,
    kind: EventKind,
    entity_id: Option<Uuid>,
    delta: Option<serde_json::Value>,
) {
    if let Err(err) = publish_channel(state, server_id, channel_id, kind, entity_id, delta).await {
        warn!(
            error = err.code(),
            %channel_id,
            "gateway publish after message write failed"
        );
    }
}

/// Best-effort notice in the server's oldest text channel when Go Live starts.
/// The badge is already local; this must not fail the signaling frame.
pub(crate) async fn post_live_hint(state: &AppState, user: &User, server_id: Uuid, voice_id: Uuid) {
    let voice_name = match sqlx::query_scalar::<_, String>(
        "SELECT name FROM channels WHERE id = $1 AND server_id = $2 AND kind = 'voice'",
    )
    .bind(voice_id)
    .bind(server_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(name) => name,
        Err(err) => {
            warn!(error = %err, "live hint: voice name");
            return;
        }
    };
    let Some(voice_name) = voice_name else {
        return;
    };
    let text_id = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM channels WHERE server_id = $1 AND kind = 'text' \
         ORDER BY created_at ASC LIMIT 1",
    )
    .bind(server_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(id) => id,
        Err(err) => {
            warn!(error = %err, "live hint: text channel");
            return;
        }
    };
    let Some(text_id) = text_id else {
        return;
    };
    let content = format!("{} ist live in {voice_name}.", user.name);
    let row = match sqlx::query_as::<_, MessageInsert>(
        "INSERT INTO messages (channel_id, author_id, content) VALUES ($1, $2, $3) \
         RETURNING id, channel_id, content, created_at, edited_at",
    )
    .bind(text_id)
    .bind(user.id)
    .bind(&content)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            warn!(error = %err, "live hint: insert");
            return;
        }
    };
    let message = row.into_message(user);
    fanout(
        state,
        server_id,
        text_id,
        EventKind::C,
        Some(message.id),
        serde_json::to_value(&message).ok(),
    )
    .await;
}

/// Text channel (via server membership) or 1:1 DM (via channel_members).
/// Voice is not a message resource.
pub(crate) enum MessagingChannel {
    Server { member: Membership },
    Dm { channel: channel::Channel },
}

impl MessagingChannel {
    /// Protocol `s`: the real server, or the DM channel id.
    pub(crate) fn event_server_id(&self) -> Uuid {
        match self {
            Self::Server { member, .. } => member.server.id,
            Self::Dm { channel } => channel.id,
        }
    }

    pub(crate) fn require_send(&self) -> Result<(), ApiError> {
        match self {
            Self::Server { member, .. } => member.require(Permission::SendMessages),
            Self::Dm { .. } => Ok(()),
        }
    }

    pub(crate) fn require_send_files(&self) -> Result<(), ApiError> {
        match self {
            Self::Server { member, .. } => member.require(Permission::SendFiles),
            Self::Dm { .. } => Ok(()),
        }
    }

    pub(crate) fn require_manage_messages(&self) -> Result<(), ApiError> {
        match self {
            Self::Server { member, .. } => member.require(Permission::ManageMessages),
            Self::Dm { .. } => Err(ApiError::Forbidden(
                "You can only delete your own messages.",
            )),
        }
    }
}

pub(crate) async fn messaging_channel(
    db: &PgPool,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<MessagingChannel, ApiError> {
    let channel = channel::get(db, channel_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !channel.kind.is_messaging() {
        return Err(ApiError::NotFound);
    }
    if channel.kind == ChannelKind::Dm {
        channel::require_participant(db, channel.id, user_id).await?;
        return Ok(MessagingChannel::Dm { channel });
    }
    let server_id = channel.server_id.ok_or(ApiError::NotFound)?;
    let member = membership::load(db, server_id, user_id).await?;
    Ok(MessagingChannel::Server { member })
}

async fn message_for(
    db: &PgPool,
    message_id: Uuid,
    user_id: Uuid,
) -> Result<(MessagingChannel, Message), ApiError> {
    let row = sqlx::query_as::<_, MessageRow>(
        "SELECT m.id, m.channel_id, m.author_id, u.name AS author_name, \
                u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at \
         FROM messages m JOIN users u ON u.id = m.author_id \
         WHERE m.id = $1",
    )
    .bind(message_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let access = messaging_channel(db, row.channel_id, user_id).await?;
    let mut message = Message::from(row);
    message.attachments = attachments::for_message(db, message.id).await?;
    Ok((access, message))
}

async fn load_page(
    db: &PgPool,
    channel_id: Uuid,
    before: Option<Cursor>,
    after: Option<Cursor>,
    limit: i64,
) -> Result<MessagePage, ApiError> {
    let take = limit + 1;
    let rows = if let Some(cursor) = before {
        let (at, id) = resolve_bound(db, channel_id, cursor, Bound::Before, "before").await?;
        sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.channel_id, m.author_id, u.name AS author_name, \
                    u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at \
             FROM messages m JOIN users u ON u.id = m.author_id \
             WHERE m.channel_id = $1 AND (m.created_at, m.id) < ($2, $3) \
             ORDER BY m.created_at DESC, m.id DESC \
             LIMIT $4",
        )
        .bind(channel_id)
        .bind(at)
        .bind(id)
        .bind(take)
        .fetch_all(db)
        .await?
    } else if let Some(cursor) = after {
        let (at, id) = resolve_bound(db, channel_id, cursor, Bound::After, "after").await?;
        let mut newer = sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.channel_id, m.author_id, u.name AS author_name, \
                    u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at \
             FROM messages m JOIN users u ON u.id = m.author_id \
             WHERE m.channel_id = $1 AND (m.created_at, m.id) > ($2, $3) \
             ORDER BY m.created_at ASC, m.id ASC \
             LIMIT $4",
        )
        .bind(channel_id)
        .bind(at)
        .bind(id)
        .bind(take)
        .fetch_all(db)
        .await?;
        // `has_more` is computed on this order; then flip to oldest → newest
        // (already newest-last).
        let has_more = newer.len() as i64 > limit;
        if has_more {
            newer.truncate(limit as usize);
        }
        return Ok(MessagePage {
            messages: newer.into_iter().map(Message::from).collect(),
            has_more,
        });
    } else {
        sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.channel_id, m.author_id, u.name AS author_name, \
                    u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at \
             FROM messages m JOIN users u ON u.id = m.author_id \
             WHERE m.channel_id = $1 \
             ORDER BY m.created_at DESC, m.id DESC \
             LIMIT $2",
        )
        .bind(channel_id)
        .bind(take)
        .fetch_all(db)
        .await?
    };

    let has_more = rows.len() as i64 > limit;
    let mut rows = rows;
    if has_more {
        rows.truncate(limit as usize);
    }
    rows.reverse();
    Ok(MessagePage {
        messages: rows.into_iter().map(Message::from).collect(),
        has_more,
    })
}

#[derive(Clone, Copy)]
enum Bound {
    Before,
    After,
}

/// Turns a cursor into the exclusive `(created_at, id)` pair used in SQL.
/// A `{time}|{id}` bound is used as-is — the row may already be gone.
/// A bare id that does not exist is a field error, not an empty page
/// with `has_more: false` (that would stop channel-local history).
async fn resolve_bound(
    db: &PgPool,
    channel_id: Uuid,
    cursor: Cursor,
    bound: Bound,
    field: &'static str,
) -> Result<(DateTime<Utc>, Uuid), ApiError> {
    match cursor {
        Cursor::Bound { at, id } => Ok((at, id)),
        Cursor::Id(id) => {
            let row: Option<(DateTime<Utc>, Uuid)> = sqlx::query_as(
                "SELECT created_at, id FROM messages WHERE id = $1 AND channel_id = $2",
            )
            .bind(id)
            .bind(channel_id)
            .fetch_optional(db)
            .await?;
            row.ok_or_else(|| ApiError::Validation(FieldErrors::from([(field, "invalid")])))
        }
        Cursor::Time(at) => Ok(match bound {
            // Exclusive of everything at `at`: pad with nil / max id.
            Bound::Before => (at, Uuid::nil()),
            Bound::After => (at, Uuid::from_u128(u128::MAX)),
        }),
    }
}
