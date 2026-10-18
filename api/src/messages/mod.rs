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

pub mod reactions;
pub mod validate;
mod workflows;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, patch};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::{info, warn};
use uuid::Uuid;

use crate::attachments::{self, Attachment};
use crate::auth::session::CurrentUser;
use crate::auth::user::User;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::{EventDraft, EventKind, delivery};
use crate::gifs::{self, Gif, GifPick};
use crate::json::Body;
use crate::path::Id;
use crate::previews::{self, LinkPreview, urls::preview_urls};
use crate::servers::channel::{self, ChannelKind};
use crate::servers::membership::{self, Membership};
use crate::servers::permissions::Permission;
use crate::state::AppState;

use self::validate::Cursor;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(reactions::router())
        .route(
            "/api/channels/{id}/messages",
            get(list_messages).post(create_message),
        )
        .route(
            "/api/messages/{id}",
            patch(update_message).delete(delete_message),
        )
        .route("/api/messages/{id}/previews", delete(suppress_previews))
        .merge(workflows::router())
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
    pub revision: i64,
    /// Immutable INSERT order for cross-device read boundaries.
    pub created_order: i64,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub reactions: Vec<reactions::Reaction>,
    /// A GIF message: empty content, no attachments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gif: Option<Gif>,
    /// Link previews (#254), filled in after sending by an edit event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previews: Vec<LinkPreview>,
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
    revision: i64,
    created_order: i64,
    reactions: sqlx::types::Json<Vec<reactions::Reaction>>,
    gif: Option<sqlx::types::Json<Gif>>,
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
            revision: row.revision,
            created_order: row.created_order,
            attachments: Vec::new(),
            reactions: row.reactions.0,
            gif: row.gif.map(|gif| gif.0),
            previews: Vec::new(),
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
    revision: i64,
    created_order: i64,
    gif: Option<sqlx::types::Json<Gif>>,
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
            revision: self.revision,
            created_order: self.created_order,
            attachments: Vec::new(),
            reactions: Vec::new(),
            gif: self.gif.map(|gif| gif.0),
            previews: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub attachment_ids: Vec<Uuid>,
    /// Post this GIF instead of text or files.
    #[serde(default)]
    pub gif: Option<GifPick>,
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
    previews::for_messages(&state.db, &mut page.messages).await?;
    Ok(Json(page))
}

async fn create_message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    headers: HeaderMap,
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
    // Resolving asks Klipy (up to its 8 s timeout) before the channel lock
    // is taken, so a slow provider holds only this request, not the channel.
    let gif = match &body.gif {
        Some(pick) => Some(resolve_gif(&state, user.id, pick, &content, &attachment_ids).await?),
        None => None,
    };
    if content.is_empty() && attachment_ids.is_empty() && gif.is_none() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "content", "required",
        )])));
    }
    if !attachment_ids.is_empty() {
        access.require_send_files()?;
    }

    let mut tx = state.db.begin().await?;
    delivery::lock_channel(&mut tx, channel_id).await?;
    let row = sqlx::query_as::<_, MessageInsert>(
        "INSERT INTO messages (channel_id, author_id, content, gif) VALUES ($1, $2, $3, $4) \
         RETURNING id, channel_id, content, created_at, edited_at, revision, created_order, gif",
    )
    .bind(channel_id)
    .bind(user.id)
    .bind(&content)
    .bind(gif.as_ref().map(sqlx::types::Json))
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
    .await;
    let bound = match bound {
        Ok(bound) => bound,
        Err(err) => {
            tx.rollback().await?;
            // Quarantine only this uploader's invalid objects, after rollback;
            // no physical delete can damage a rolled-back message transaction.
            if let Err(cleanup_error) = attachments::cleanup::discard_mismatches(
                &state,
                channel_id,
                user.id,
                &attachment_ids,
            )
            .await
            {
                warn!(error=?cleanup_error, "invalid upload quarantine deferred to expiry cleanup");
            }
            return Err(err);
        }
    };
    let mut message = row.into_message(&user);
    message.attachments = bound;
    persist_event(
        &mut tx,
        access.event_server_id(),
        EventKind::C,
        &mut message,
    )
    .await?;
    tx.commit().await?;
    info!(channel_id = %channel_id, message_id = %message.id, "message created");
    if let (Some(gif), Some(pick), Some(gifs)) = (&message.gif, &body.gif, &state.gifs) {
        gifs.shared(&gif.slug, pick.query.trim(), user.id);
    }
    let _ = delivery::deliver_pending(&state, 32).await;
    previews::enqueue(
        &state,
        user.id,
        previews::Job {
            message_id: message.id,
            channel_id,
            server_id: access.event_server_id(),
            urls: preview_urls(&message.content),
            own_host: previews::request_host(&headers),
        },
    );
    Ok((StatusCode::CREATED, Json(message)).into_response())
}

async fn update_message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(message_id): Id,
    headers: HeaderMap,
    Body(body): Body<UpdateBody>,
) -> Result<Json<Message>, ApiError> {
    let (access, current) = message_for(&state.db, message_id, user.id).await?;
    if current.author.id != user.id {
        return Err(ApiError::Forbidden("You can only edit your own messages."));
    }
    access.require_send()?;
    if current.gif.is_some() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "gif",
            "not_editable",
        )])));
    }

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

    let mut tx = state.db.begin().await?;
    delivery::lock_channel(&mut tx, current.channel_id).await?;
    let row = sqlx::query_as::<_, MessageInsert>(
        "UPDATE messages SET content = $2, edited_at = now() WHERE id = $1 \
         RETURNING id, channel_id, content, created_at, edited_at, revision, created_order, gif",
    )
    .bind(message_id)
    .bind(&content)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    // Previews of links the edit removed go now; new links get theirs from
    // the worker, as on send.
    let urls = preview_urls(&row.content);
    let kept: Vec<String> = urls.iter().map(|url| url.as_str().to_owned()).collect();
    sqlx::query(
        "DELETE FROM message_previews mp USING link_previews lp \
         WHERE mp.preview_id = lp.id AND mp.message_id = $1 AND NOT (lp.url = ANY($2))",
    )
    .bind(message_id)
    .bind(&kept)
    .execute(&mut *tx)
    .await?;
    let suppressed: bool = sqlx::query_scalar("SELECT suppress_embeds FROM messages WHERE id = $1")
        .bind(message_id)
        .fetch_one(&mut *tx)
        .await?;
    let mut message = row.into_message(&user);
    message.attachments = current.attachments;
    persist_event(
        &mut tx,
        access.event_server_id(),
        EventKind::E,
        &mut message,
    )
    .await?;
    tx.commit().await?;
    let _ = delivery::deliver_pending(&state, 32).await;
    if !suppressed {
        previews::enqueue(
            &state,
            user.id,
            previews::Job {
                message_id,
                channel_id: message.channel_id,
                server_id: access.event_server_id(),
                urls,
                own_host: previews::request_host(&headers),
            },
        );
    }
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

    let mut tx = state.db.begin().await?;
    delivery::lock_channel(&mut tx, current.channel_id).await?;
    sqlx::query("DELETE FROM messages WHERE id = $1")
        .bind(message_id)
        .execute(&mut *tx)
        .await?;
    info!(
        channel_id = %current.channel_id,
        message_id = %message_id,
        "message deleted"
    );
    let revision = delivery::revision(&mut tx).await?;
    delivery::enqueue(
        &mut tx,
        revision,
        EventDraft {
            kind: EventKind::D,
            server_id: access.event_server_id(),
            channel_id: Some(current.channel_id),
            entity_id: Some(message_id),
            delta: None,
        },
    )
    .await?;
    tx.commit().await?;
    let _ = delivery::deliver_pending(&state, 32).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn persist_event(
    db: &mut sqlx::PgConnection,
    server: Uuid,
    kind: EventKind,
    message: &mut Message,
) -> Result<(), ApiError> {
    reactions::populate(db, message).await?;
    previews::for_messages(&mut *db, std::slice::from_mut(message)).await?;
    message.revision = delivery::revision(db).await?;
    sqlx::query("UPDATE messages SET revision=$2 WHERE id=$1")
        .bind(message.id)
        .bind(message.revision)
        .execute(&mut *db)
        .await?;
    delivery::enqueue(
        db,
        message.revision,
        EventDraft {
            kind,
            server_id: server,
            channel_id: Some(message.channel_id),
            entity_id: Some(message.id),
            delta: Some(
                serde_json::to_value(message).map_err(|err| ApiError::Internal(err.to_string()))?,
            ),
        },
    )
    .await
}

/// A picked GIF as stored: alone in its message, resolved by Klipy so its
/// URLs and size are Klipy's and not the client's. The lookup is a Klipy API
/// request, so it counts against the sender's `gif` bucket like a search,
/// on top of the `msg` bucket the gate already charged.
async fn resolve_gif(
    state: &AppState,
    user_id: Uuid,
    pick: &GifPick,
    content: &str,
    attachment_ids: &[Uuid],
) -> Result<Gif, ApiError> {
    let fail = |code: &'static str| ApiError::Validation(FieldErrors::from([("gif", code)]));
    if !content.is_empty() || !attachment_ids.is_empty() {
        return Err(fail("exclusive"));
    }
    let gifs = state.gifs.as_ref().ok_or_else(|| fail("unavailable"))?;
    let slug = pick.slug.trim();
    if !gifs::klipy::valid_slug(slug) || pick.query.chars().count() > gifs::QUERY_MAX {
        return Err(fail("invalid"));
    }
    gifs::rate_limit(state, user_id)?;
    gifs.resolve(slug).await?.ok_or_else(|| fail("invalid"))
}

/// `DELETE /api/messages/{id}/previews`: the author (or, on a server,
/// `manage_messages`) removes the message's link previews; later edits do
/// not bring them back.
async fn suppress_previews(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(message_id): Id,
) -> Result<StatusCode, ApiError> {
    let (access, current) = message_for(&state.db, message_id, user.id).await?;
    if current.author.id != user.id {
        access.require_manage_messages()?;
    }
    let mut tx = state.db.begin().await?;
    delivery::lock_channel(&mut tx, current.channel_id).await?;
    let changed = sqlx::query(
        "UPDATE messages SET suppress_embeds = true WHERE id = $1 AND NOT suppress_embeds",
    )
    .bind(message_id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    let removed = sqlx::query("DELETE FROM message_previews WHERE message_id = $1")
        .bind(message_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        > 0;
    if !changed && !removed {
        tx.rollback().await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    if removed {
        let mut message = current;
        persist_event(
            &mut tx,
            access.event_server_id(),
            EventKind::E,
            &mut message,
        )
        .await?;
    }
    tx.commit().await?;
    if removed {
        let _ = delivery::deliver_pending(&state, 32).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The worker's result for one message (`previews::process`): `ids` in link
/// order replace its previews, with an edit event, unless the message was
/// deleted, edited to other links or had its previews removed meanwhile.
pub(crate) async fn attach_previews(
    state: &AppState,
    job: &previews::Job,
    ids: &[Uuid],
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    delivery::lock_channel(&mut tx, job.channel_id).await?;
    let current: Option<(String, bool)> = sqlx::query_as(
        "SELECT content, suppress_embeds FROM messages WHERE id = $1 AND channel_id = $2",
    )
    .bind(job.message_id)
    .bind(job.channel_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((content, suppressed)) = current else {
        return Ok(());
    };
    if suppressed || preview_urls(&content) != job.urls {
        return Ok(());
    }
    let existing: Vec<Uuid> = sqlx::query_scalar(
        "SELECT preview_id FROM message_previews WHERE message_id = $1 ORDER BY position",
    )
    .bind(job.message_id)
    .fetch_all(&mut *tx)
    .await?;
    if existing == ids {
        return Ok(());
    }
    sqlx::query("DELETE FROM message_previews WHERE message_id = $1")
        .bind(job.message_id)
        .execute(&mut *tx)
        .await?;
    for (position, id) in ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO message_previews (message_id, preview_id, position) VALUES ($1, $2, $3)",
        )
        .bind(job.message_id)
        .bind(id)
        .bind(position as i16)
        .execute(&mut *tx)
        .await?;
    }
    let row = sqlx::query_as::<_, MessageRow>(MESSAGE_BY_ID_SQL)
        .bind(job.message_id)
        .fetch_one(&mut *tx)
        .await?;
    let mut message = Message::from(row);
    message.attachments = attachments::for_message(&mut *tx, job.message_id).await?;
    persist_event(&mut tx, job.server_id, EventKind::E, &mut message).await?;
    tx.commit().await?;
    let _ = delivery::deliver_pending(state, 32).await;
    Ok(())
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
    let result: Result<(),ApiError> = async {
        let mut tx=state.db.begin().await?;
        delivery::lock_channel(&mut tx,text_id).await?;
        let row=sqlx::query_as::<_,MessageInsert>("INSERT INTO messages (channel_id, author_id, content) VALUES ($1,$2,$3) RETURNING id,channel_id,content,created_at,edited_at,revision,created_order,gif").bind(text_id).bind(user.id).bind(content).fetch_one(&mut *tx).await?;
        let mut message=row.into_message(user);
        persist_event(&mut tx,server_id,EventKind::C,&mut message).await?;
        tx.commit().await?;
        Ok(())
    }.await;
    if let Err(err) = result {
        warn!(error = err.code(), "live hint persistence failed");
        return;
    }
    let _ = delivery::deliver_pending(state, 32).await;
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

const MESSAGE_BY_ID_SQL: &str = "SELECT m.id, m.channel_id, m.author_id, u.name AS author_name, \
                u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at, m.revision, m.created_order, m.gif, COALESCE((SELECT jsonb_agg(jsonb_build_object('emoji', r.emoji, 'user_ids', r.user_ids) ORDER BY r.emoji) FROM (SELECT emoji,jsonb_agg(user_id ORDER BY user_id) AS user_ids FROM message_reactions WHERE message_id=m.id GROUP BY emoji) r),'[]'::jsonb) AS reactions \
         FROM messages m JOIN users u ON u.id = m.author_id \
         WHERE m.id = $1";

async fn message_for(
    db: &PgPool,
    message_id: Uuid,
    user_id: Uuid,
) -> Result<(MessagingChannel, Message), ApiError> {
    let row = sqlx::query_as::<_, MessageRow>(MESSAGE_BY_ID_SQL)
        .bind(message_id)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::NotFound)?;
    let access = messaging_channel(db, row.channel_id, user_id).await?;
    let mut message = Message::from(row);
    message.attachments = attachments::for_message(db, message.id).await?;
    previews::for_messages(db, std::slice::from_mut(&mut message)).await?;
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
                    u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at, m.revision, m.created_order, m.gif, COALESCE((SELECT jsonb_agg(jsonb_build_object('emoji', r.emoji, 'user_ids', r.user_ids) ORDER BY r.emoji) FROM (SELECT emoji,jsonb_agg(user_id ORDER BY user_id) AS user_ids FROM message_reactions WHERE message_id=m.id GROUP BY emoji) r),'[]'::jsonb) AS reactions \
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
                    u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at, m.revision, m.created_order, m.gif, COALESCE((SELECT jsonb_agg(jsonb_build_object('emoji', r.emoji, 'user_ids', r.user_ids) ORDER BY r.emoji) FROM (SELECT emoji,jsonb_agg(user_id ORDER BY user_id) AS user_ids FROM message_reactions WHERE message_id=m.id GROUP BY emoji) r),'[]'::jsonb) AS reactions \
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
                    u.avatar_url AS author_avatar_url, m.content, m.created_at, m.edited_at, m.revision, m.created_order, m.gif, COALESCE((SELECT jsonb_agg(jsonb_build_object('emoji', r.emoji, 'user_ids', r.user_ids) ORDER BY r.emoji) FROM (SELECT emoji,jsonb_agg(user_id ORDER BY user_id) AS user_ids FROM message_reactions WHERE message_id=m.id GROUP BY emoji) r),'[]'::jsonb) AS reactions \
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
