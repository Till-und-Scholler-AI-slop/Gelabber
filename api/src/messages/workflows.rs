//! Private account read positions and scoped PostgreSQL full-text search.

use axum::{
    Router,
    extract::{Query, State},
    response::Json,
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::{Bound, Message, MessagePage, MessageRow, messaging_channel, resolve_bound, validate};
use crate::{
    attachments,
    auth::session::CurrentUser,
    error::{ApiError, FieldErrors},
    gateway::{
        delivery,
        protocol::{ReadSync, UserFrame},
    },
    json::Body,
    path::{Id, Ids},
    servers::{channel::ChannelKind, membership},
    state::AppState,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/messages/unread", get(unread))
        .route("/api/channels/{id}/read", put(mark_read))
        .route("/api/servers/{id}/read", put(mark_server_read))
        .route("/api/messages/read-all", put(mark_all_read))
        .route("/api/channels/{id}/messages/search", get(search))
        .route(
            "/api/channels/{id}/messages/{message_id}/context",
            get(context),
        )
}

#[derive(Debug, Serialize, FromRow)]
pub(crate) struct ReadState {
    pub(crate) channel_id: Uuid,
    pub(crate) server_id: Option<Uuid>,
    pub(crate) read_message_id: Option<Uuid>,
    pub(crate) read_at: Option<DateTime<Utc>>,
    pub(crate) unread_count: i64,
    /// Unread messages that mention the user (`<@id>`, `@here` when they were
    /// online, `@everyone`). Bounded like `unread_count`.
    pub(crate) mention_count: i64,
    /// Effective notification level: the channel's, else its server's, else
    /// `all` (`notifications/settings.rs`).
    pub(crate) level: String,
    /// A mute of the channel or its server holds now.
    pub(crate) muted: bool,
    /// When the mute ends; none while muted means until turned back on.
    pub(crate) muted_until: Option<DateTime<Utc>>,
}

// One statement takes a consistent snapshot of membership, cursors and rows.
pub(crate) const UNREAD: &str = "WITH visible AS (
    SELECT c.id AS channel_id, c.server_id, sm.joined_at
    FROM channels c JOIN server_members sm ON sm.server_id=c.server_id AND sm.user_id=$1
    WHERE c.kind='text'
    UNION ALL
    SELECT c.id, c.server_id, cm.joined_at
    FROM channels c JOIN channel_members cm ON cm.channel_id=c.id AND cm.user_id=$1
    WHERE c.kind='dm'
), mutes AS (
    SELECT v.channel_id,
      cs.level AS channel_level, ss.level AS server_level,
      (cs.muted AND (cs.muted_until IS NULL OR cs.muted_until>now())) IS TRUE AS channel_muted,
      (ss.muted AND (ss.muted_until IS NULL OR ss.muted_until>now())) IS TRUE AS server_muted,
      cs.muted_until AS channel_until, ss.muted_until AS server_until
    FROM visible v
    LEFT JOIN notification_settings cs ON cs.user_id=$1 AND cs.scope_kind='channel' AND cs.scope_id=v.channel_id
    LEFT JOIN notification_settings ss ON ss.user_id=$1 AND ss.scope_kind='server' AND ss.scope_id=v.server_id
) SELECT v.channel_id, v.server_id, r.message_id AS read_message_id, r.message_at AS read_at,
    (SELECT count(*) FROM (SELECT 1 FROM messages m WHERE m.channel_id=v.channel_id AND m.author_id<>$1
      AND m.created_at>=v.joined_at
      AND (r.created_order IS NULL OR m.created_order>r.created_order) LIMIT 100) bounded) AS unread_count,
    (SELECT count(*) FROM (SELECT 1 FROM messages m WHERE m.channel_id=v.channel_id AND m.author_id<>$1
      AND m.created_at>=v.joined_at
      AND (r.created_order IS NULL OR m.created_order>r.created_order)
      AND ($1=ANY(m.mentions) OR m.mentions_everyone) LIMIT 100) bounded) AS mention_count,
    COALESCE(n.channel_level, n.server_level, 'all') AS level,
    (n.channel_muted OR n.server_muted) AS muted,
    CASE WHEN (n.channel_muted AND n.channel_until IS NULL) OR (n.server_muted AND n.server_until IS NULL)
      THEN NULL
      ELSE GREATEST(CASE WHEN n.channel_muted THEN n.channel_until END,
                    CASE WHEN n.server_muted THEN n.server_until END)
    END AS muted_until
  FROM visible v JOIN mutes n ON n.channel_id=v.channel_id
  LEFT JOIN channel_read_state r ON r.user_id=$1 AND r.channel_id=v.channel_id
    AND r.membership_at=v.joined_at
  WHERE ($2::uuid IS NULL OR v.channel_id=$2)";

async fn unread(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<ReadState>>, ApiError> {
    Ok(Json(
        sqlx::query_as(UNREAD)
            .bind(user.id)
            .bind(None::<Uuid>)
            .fetch_all(&state.db)
            .await?,
    ))
}

#[derive(Deserialize)]
struct ReadBody {
    /// Absent: up to the newest message ("Als gelesen markieren").
    #[serde(default)]
    message_id: Option<Uuid>,
}

async fn mark_read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    Body(body): Body<ReadBody>,
) -> Result<Json<ReadState>, ApiError> {
    let access = messaging_channel(&state.db, channel_id, user.id).await?;
    let mut tx = state.db.begin().await?;
    if let super::MessagingChannel::Server { member } = &access {
        membership::lock_server_conn(&mut tx, member.server.id, false).await?;
    }
    delivery::lock_channel(&mut tx, channel_id).await?;
    let channel = crate::servers::channel::get(&mut *tx, channel_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let joined_at: DateTime<Utc> = if channel.kind == ChannelKind::Dm {
        sqlx::query_scalar(
            "SELECT joined_at FROM channel_members WHERE channel_id=$1 AND user_id=$2 FOR SHARE",
        )
        .bind(channel_id)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?
    } else if channel.kind == ChannelKind::Text {
        sqlx::query_scalar(
            "SELECT joined_at FROM server_members WHERE server_id=$1 AND user_id=$2 FOR SHARE",
        )
        .bind(channel.server_id)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?
    } else {
        return Err(ApiError::NotFound);
    };
    let newest: Option<(Uuid, DateTime<Utc>, i64)> = match body.message_id {
        Some(id) => Some(
            sqlx::query_as(
                "SELECT id,created_at,created_order FROM messages WHERE id=$1 AND channel_id=$2",
            )
            .bind(id)
            .bind(channel_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ApiError::NotFound)?,
        ),
        None => {
            sqlx::query_as(
                "SELECT id,created_at,created_order FROM messages WHERE channel_id=$1 \
                 AND created_at>=$2 ORDER BY created_order DESC LIMIT 1",
            )
            .bind(channel_id)
            .bind(joined_at)
            .fetch_optional(&mut *tx)
            .await?
        }
    };
    // Nothing to read in an empty conversation.
    let Some((message_id, at, created_order)) = newest else {
        let row: ReadState = sqlx::query_as(UNREAD)
            .bind(user.id)
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Json(row));
    };
    let moved = sqlx::query("INSERT INTO channel_read_state (user_id,channel_id,membership_at,message_at,message_id,created_order)
        VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (user_id,channel_id) DO UPDATE
        SET membership_at=EXCLUDED.membership_at,message_at=EXCLUDED.message_at,message_id=EXCLUDED.message_id,created_order=EXCLUDED.created_order
        WHERE channel_read_state.membership_at<>EXCLUDED.membership_at
           OR channel_read_state.created_order<EXCLUDED.created_order")
        .bind(user.id).bind(channel_id).bind(joined_at).bind(at).bind(message_id).bind(created_order).execute(&mut *tx).await?
        .rows_affected() > 0;
    let row: ReadState = sqlx::query_as(UNREAD)
        .bind(user.id)
        .bind(channel_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    if moved {
        announce_read(&state, user.id, std::slice::from_ref(&row)).await;
    }
    Ok(Json(row))
}

/// Tells the account's other sockets (other tabs, other devices). Best
/// effort: one that misses it catches up with its next unread snapshot.
async fn announce_read(state: &AppState, user: Uuid, rows: &[ReadState]) {
    if rows.is_empty() {
        return;
    }
    let frame = UserFrame::Read {
        d: rows
            .iter()
            .map(|row| ReadSync {
                channel_id: row.channel_id,
                server_id: row.server_id,
                read_message_id: row.read_message_id,
                read_at: row.read_at,
                unread_count: row.unread_count,
                mention_count: row.mention_count,
            })
            .collect(),
    };
    if let Err(error) = state.gateway.publish_user(user, &frame).await {
        tracing::debug!(error = error.code(), "read position not announced");
    }
}

/// Every visible conversation of `scope` read up to its newest message, in
/// one statement. Forward only, like `mark_read`: a cursor that is already
/// further (a newer mark that won the race) stays. Returns the conversations
/// whose cursor moved.
async fn read_up_to_latest(
    tx: &mut sqlx::PgConnection,
    user: Uuid,
    server: Option<Uuid>,
    dms: bool,
    servers: bool,
) -> Result<Vec<Uuid>, ApiError> {
    Ok(sqlx::query_scalar(
        "WITH visible AS (
            SELECT c.id AS channel_id, sm.joined_at
            FROM channels c JOIN server_members sm ON sm.server_id=c.server_id AND sm.user_id=$1
            WHERE c.kind='text' AND $4 AND ($2::uuid IS NULL OR c.server_id=$2)
            UNION ALL
            SELECT c.id, cm.joined_at
            FROM channels c JOIN channel_members cm ON cm.channel_id=c.id AND cm.user_id=$1
            WHERE c.kind='dm' AND $3 AND $2::uuid IS NULL
        )
        INSERT INTO channel_read_state (user_id,channel_id,membership_at,message_at,message_id,created_order)
        SELECT $1, v.channel_id, v.joined_at, l.created_at, l.id, l.created_order
        FROM visible v
        CROSS JOIN LATERAL (
            SELECT m.id, m.created_at, m.created_order FROM messages m
            WHERE m.channel_id=v.channel_id AND m.created_at>=v.joined_at
            ORDER BY m.created_order DESC LIMIT 1
        ) l
        ON CONFLICT (user_id,channel_id) DO UPDATE
        SET membership_at=EXCLUDED.membership_at,message_at=EXCLUDED.message_at,message_id=EXCLUDED.message_id,created_order=EXCLUDED.created_order
        WHERE channel_read_state.membership_at<>EXCLUDED.membership_at
           OR channel_read_state.created_order<EXCLUDED.created_order
        RETURNING channel_id",
    )
    .bind(user)
    .bind(server)
    .bind(dms)
    .bind(servers)
    .fetch_all(&mut *tx)
    .await?)
}

async fn finish_bulk_read(
    state: &AppState,
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    user: Uuid,
    moved: Vec<Uuid>,
) -> Result<Json<Vec<ReadState>>, ApiError> {
    let rows: Vec<ReadState> = sqlx::query_as(UNREAD)
        .bind(user)
        .bind(None::<Uuid>)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    let changed: Vec<ReadState> = rows
        .into_iter()
        .filter(|row| moved.contains(&row.channel_id))
        .collect();
    announce_read(state, user, &changed).await;
    Ok(Json(changed))
}

/// "Als gelesen markieren" on a server (issue #245): its text channels.
async fn mark_server_read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<Vec<ReadState>>, ApiError> {
    membership::load(&state.db, server_id, user.id).await?;
    let mut tx = state.db.begin().await?;
    membership::lock_server_conn(&mut tx, server_id, false).await?;
    let moved = read_up_to_latest(&mut tx, user.id, Some(server_id), false, true).await?;
    finish_bulk_read(&state, tx, user.id, moved).await
}

#[derive(Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ReadScope {
    #[default]
    All,
    Dms,
}

#[derive(Deserialize)]
struct ReadAllBody {
    #[serde(default)]
    scope: ReadScope,
}

/// Everything read, or only the direct messages.
async fn mark_all_read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<ReadAllBody>,
) -> Result<Json<Vec<ReadState>>, ApiError> {
    let mut tx = state.db.begin().await?;
    let moved =
        read_up_to_latest(&mut tx, user.id, None, true, body.scope == ReadScope::All).await?;
    finish_bulk_read(&state, tx, user.id, moved).await
}

#[derive(Deserialize)]
struct SearchParams {
    q: String,
    before: Option<String>,
    limit: Option<i64>,
}

async fn search(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    Query(params): Query<SearchParams>,
) -> Result<Json<MessagePage>, ApiError> {
    messaging_channel(&state.db, channel_id, user.id).await?;
    let mut fields = FieldErrors::new();
    let q = params.q.trim();
    if q.is_empty() {
        fields.insert("q", "required");
    }
    if q.chars().count() > 200 {
        fields.insert("q", "too_long");
    }
    let limit = validate::limit(params.limit, &mut fields);
    let before = validate::cursor(params.before.as_deref(), "before", &mut fields);
    validate::finish(fields)?;
    let limit = limit.expect("validated limit");
    let bound = if let Some(cursor) = before {
        Some(resolve_bound(&state.db, channel_id, cursor, Bound::Before, "before").await?)
    } else {
        None
    };
    let rows: Vec<MessageRow> = sqlx::query_as("SELECT m.id,m.channel_id,m.author_id,u.name AS author_name,
        u.avatar_url AS author_avatar_url,m.content,m.created_at,m.edited_at,m.revision,m.created_order,m.gif,m.mentions,m.mentions_everyone, COALESCE((SELECT jsonb_agg(jsonb_build_object('emoji', r.emoji, 'user_ids', r.user_ids) ORDER BY r.emoji) FROM (SELECT emoji,jsonb_agg(user_id ORDER BY user_id) AS user_ids FROM message_reactions WHERE message_id=m.id GROUP BY emoji) r),'[]'::jsonb) AS reactions
        FROM messages m JOIN users u ON u.id=m.author_id
        WHERE m.channel_id=$1 AND to_tsvector('simple',m.content) @@ websearch_to_tsquery('simple',$2)
          AND ($3::timestamptz IS NULL OR (m.created_at,m.id)<($3,$4))
        ORDER BY m.created_at DESC,m.id DESC LIMIT $5")
        .bind(channel_id).bind(q).bind(bound.map(|b| b.0)).bind(bound.map(|b| b.1))
        .bind(limit+1).fetch_all(&state.db).await?;
    let has_more = rows.len() as i64 > limit;
    let mut messages: Vec<Message> = rows
        .into_iter()
        .take(limit as usize)
        .map(Message::from)
        .collect();
    messages.reverse();
    attachments::for_messages(&state.db, &mut messages).await?;
    Ok(Json(MessagePage { messages, has_more }))
}

#[derive(Serialize)]
struct MessageContext {
    target_id: Uuid,
    messages: Vec<Message>,
    before: String,
    after: String,
}

/// One bounded snapshot, independent of how far back the target is. Composite
/// cursors retain the normal history contract even when neighbouring rows vanish.
async fn context(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Ids(channel_id, message_id): Ids,
) -> Result<Json<MessageContext>, ApiError> {
    messaging_channel(&state.db, channel_id, user.id).await?;
    let rows: Vec<MessageRow> = sqlx::query_as(
        "WITH target AS (SELECT created_at,id FROM messages WHERE channel_id=$1 AND id=$2),
        selected AS (
          (SELECT m.id FROM messages m,target t WHERE m.channel_id=$1
             AND (m.created_at,m.id)<(t.created_at,t.id)
           ORDER BY m.created_at DESC,m.id DESC LIMIT 30)
          UNION ALL
          (SELECT m.id FROM messages m,target t WHERE m.channel_id=$1
             AND (m.created_at,m.id)>=(t.created_at,t.id)
           ORDER BY m.created_at,m.id LIMIT 31)
        ) SELECT m.id,m.channel_id,m.author_id,u.name AS author_name,
            u.avatar_url AS author_avatar_url,m.content,m.created_at,m.edited_at,m.revision,m.created_order,m.gif,m.mentions,m.mentions_everyone, COALESCE((SELECT jsonb_agg(jsonb_build_object('emoji', r.emoji, 'user_ids', r.user_ids) ORDER BY r.emoji) FROM (SELECT emoji,jsonb_agg(user_id ORDER BY user_id) AS user_ids FROM message_reactions WHERE message_id=m.id GROUP BY emoji) r),'[]'::jsonb) AS reactions
          FROM selected JOIN messages m ON m.id=selected.id JOIN users u ON u.id=m.author_id
          ORDER BY m.created_at,m.id",
    )
    .bind(channel_id)
    .bind(message_id)
    .fetch_all(&state.db)
    .await?;
    let mut messages: Vec<Message> = rows.into_iter().map(Message::from).collect();
    let first = messages.first().ok_or(ApiError::NotFound)?;
    let before = format!("{}|{}", first.created_at.to_rfc3339(), first.id);
    let last = messages.last().expect("nonempty snapshot");
    let after = format!("{}|{}", last.created_at.to_rfc3339(), last.id);
    attachments::for_messages(&state.db, &mut messages).await?;
    Ok(Json(MessageContext {
        target_id: message_id,
        messages,
        before,
        after,
    }))
}
