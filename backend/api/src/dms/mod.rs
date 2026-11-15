//! `/api/dms` — 1:1 direct messages as a channel kind (issue #9).
//!
//! A DM is a `channels` row (`kind = 'dm'`) with exactly two
//! `channel_members`. Message history, send, edit, delete and the WS
//! topic are the same paths as a text channel. No group DM.
//!
//! Who may open one is the relation (#563): friends open an ordinary DM,
//! people who share a server a message request ([`request`]), anyone else
//! gets `403 dm_not_allowed` and can only send a friend request by code
//! (`crate::friends`). A DM that exists stays usable: every DM from before
//! #563 counts as accepted, and removing a friend does not close one.
//! Incoming requests are not in `GET /api/dms` but in
//! `GET /api/dms/requests`, answered with `accept` or `decline`.
//!
//! The protocol `s` field for a DM is the channel id itself, so subscribe
//! / typing / events stay on the existing frames.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgPool, Postgres, Transaction};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::auth::user::{self, User};
use crate::error::{ApiError, FieldErrors};
use crate::gateway::protocol::UserFrame;
use crate::json::Body;
use crate::path::Id;
use crate::servers::channel::{Channel, ChannelKind};
use crate::state::AppState;

pub mod request;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/dms", get(list_dms).post(open_dm))
        .route("/api/dms/requests", get(list_requests))
        .route("/api/dms/{id}", get(get_dm))
        .route("/api/dms/{id}/accept", post(accept_request))
        .route("/api/dms/{id}/decline", post(decline_request))
}

/// Public DM as one of the two participants sees it. `peer` is the other
/// person — the stored channel name is empty.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DirectMessage {
    pub id: Uuid,
    pub kind: ChannelKind,
    pub created_at: DateTime<Utc>,
    pub peer: Peer,
    /// Set while the DM is a message request (#563); absent once accepted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<DmRequest>,
}

/// A pending message request as one side sees it.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct DmRequest {
    /// The other person started it: accept or decline. Otherwise the caller
    /// is the sender, who may write `remaining` more messages.
    pub incoming: bool,
    pub remaining: i64,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Peer {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, FromRow)]
struct DmRow {
    id: Uuid,
    created_at: DateTime<Utc>,
    peer_id: Uuid,
    peer_name: String,
    peer_avatar_url: Option<String>,
    request_by: Option<Uuid>,
    /// Messages of the request's current round (0 once a decline cooled).
    request_sent: i64,
}

impl From<DmRow> for DirectMessage {
    fn from(row: DmRow) -> Self {
        Self {
            id: row.id,
            kind: ChannelKind::Dm,
            created_at: row.created_at,
            request: row.request_by.map(|by| DmRequest {
                incoming: by == row.peer_id,
                remaining: (request::MAX_MESSAGES - row.request_sent).max(0),
            }),
            peer: Peer {
                id: row.peer_id,
                name: row.peer_name,
                avatar_url: row.peer_avatar_url,
            },
        }
    }
}

/// The columns of [`DmRow`] after the peer's; `c` is the channel.
macro_rules! request_columns {
    () => {
        ", c.dm_request_by AS request_by, \
         CASE WHEN c.dm_request_by IS NULL \
                OR c.dm_declined_at <= now() - interval '1 hour' THEN 0 \
              ELSE (SELECT count(*) FROM messages m WHERE m.channel_id = c.id \
                    AND m.author_id = c.dm_request_by AND m.created_at >= c.dm_request_since) \
         END AS request_sent "
    };
}

#[derive(Debug, Deserialize)]
pub struct OpenDmBody {
    #[serde(default)]
    pub user_id: String,
}

async fn list_dms(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<DirectMessage>>, ApiError> {
    Ok(Json(load_list(&state.db, user.id).await?))
}

async fn get_dm(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
) -> Result<Json<DirectMessage>, ApiError> {
    Ok(Json(load_one(&state.db, channel_id, user.id).await?))
}

/// `POST /api/dms`: the DM with `user_id`, opened if need be. An existing
/// DM is returned whatever the relation is now. A new one is an ordinary
/// DM between friends, a message request between people who share a
/// server, and refused (`403 dm_not_allowed`) otherwise.
async fn open_dm(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<OpenDmBody>,
) -> Result<Response, ApiError> {
    let mut errors = FieldErrors::new();
    let peer_id = parse_user_id(&body.user_id, &mut errors);
    crate::servers::validate::finish(errors)?;
    let peer_id = peer_id.expect("validated");
    if peer_id == user.id {
        return Err(ApiError::Validation(FieldErrors::from([(
            "user_id", "invalid",
        )])));
    }

    let peer = user::by_id(&state.db, peer_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let (dm, created) = open_or_create(&state.db, user.id, &peer).await?;
    if created {
        crate::gateway::delivery::wake(&state);
        info!(
            channel_id = %dm.id,
            user_id = %user.id,
            peer_id = %peer.id,
            request = dm.request.is_some(),
            "dm opened"
        );
        Ok((StatusCode::CREATED, Json(dm)).into_response())
    } else {
        Ok(Json(dm).into_response())
    }
}

/// `GET /api/dms/requests`: message requests to the caller that have a
/// message and are not declined, newest first.
async fn list_requests(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<DirectMessage>>, ApiError> {
    let rows = sqlx::query_as::<_, DmRow>(concat!(
        "SELECT c.id, c.created_at, \
                p.user_id AS peer_id, u.name AS peer_name, u.avatar_url AS peer_avatar_url",
        request_columns!(),
        "FROM channel_members me \
         JOIN channels c ON c.id = me.channel_id AND c.kind = 'dm' \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> me.user_id \
         JOIN users u ON u.id = p.user_id \
         WHERE me.user_id = $1 AND c.dm_request_by = p.user_id AND c.dm_declined_at IS NULL \
           AND EXISTS (SELECT 1 FROM messages m WHERE m.channel_id = c.id) \
         ORDER BY (SELECT MAX(m.created_at) FROM messages m WHERE m.channel_id = c.id) DESC, c.id"
    ))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(DirectMessage::from).collect()))
}

/// The request the caller received in DM `channel_id`, locked: `None` when
/// there is none (an ordinary DM, the caller's own request, not theirs).
async fn lock_incoming(
    tx: &mut Transaction<'_, Postgres>,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<Option<Uuid>, ApiError> {
    crate::gateway::delivery::lock_channel(&mut *tx, channel_id).await?;
    Ok(sqlx::query_scalar(
        "SELECT c.dm_request_by FROM channels c \
         JOIN channel_members me ON me.channel_id = c.id AND me.user_id = $2 \
         WHERE c.id = $1 AND c.kind = 'dm' AND c.dm_request_by IS NOT NULL \
           AND c.dm_request_by <> $2 \
         FOR UPDATE OF c",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?)
}

/// `POST /api/dms/{id}/accept`: the request becomes an ordinary DM. It does
/// not make the two friends. Also after a decline.
async fn accept_request(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
) -> Result<Json<DirectMessage>, ApiError> {
    let mut tx = state.db.begin().await?;
    let sender = lock_incoming(&mut tx, channel_id, user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    accept_in(&mut tx, channel_id, &[user.id, sender]).await?;
    tx.commit().await?;
    crate::gateway::delivery::wake(&state);
    publish(&state, user.id, UserFrame::MessageRequests).await;
    info!(channel_id = %channel_id, user_id = %user.id, "message request accepted");
    Ok(Json(load_one(&state.db, channel_id, user.id).await?))
}

/// `POST /api/dms/{id}/decline`: hides the request. The sender is not told;
/// they may start a new round an hour later.
async fn decline_request(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    lock_incoming(&mut tx, channel_id, user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    sqlx::query("UPDATE channels SET dm_declined_at = now() WHERE id = $1")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    publish(&state, user.id, UserFrame::MessageRequests).await;
    info!(channel_id = %channel_id, user_id = %user.id, "message request declined");
    Ok(StatusCode::NO_CONTENT)
}

/// Turns DM `channel_id` into an ordinary DM and tells both sides' lists
/// (`op: "dm"`). The caller holds the channel lock.
async fn accept_in(
    tx: &mut Transaction<'_, Postgres>,
    channel_id: Uuid,
    users: &[Uuid],
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE channels SET dm_request_by = NULL, dm_request_since = NULL, dm_declined_at = NULL \
         WHERE id = $1",
    )
    .bind(channel_id)
    .execute(&mut **tx)
    .await?;
    for user in users {
        crate::gateway::delivery::enqueue_dm(&mut *tx, *user, channel_id).await?;
    }
    Ok(())
}

/// Two people became friends: a pending request between them is accepted
/// with it. Returns whether there was one; the caller wakes delivery after
/// its commit.
pub(crate) async fn accept_between(
    tx: &mut Transaction<'_, Postgres>,
    a: Uuid,
    b: Uuid,
) -> Result<bool, ApiError> {
    let key = pair_key(a, b);
    let channel: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM channels WHERE kind = 'dm' AND pair_key = $1 AND dm_request_by IS NOT NULL",
    )
    .bind(&key)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(channel) = channel else {
        return Ok(false);
    };
    crate::gateway::delivery::lock_channel(&mut *tx, channel).await?;
    accept_in(tx, channel, &[a, b]).await?;
    Ok(true)
}

/// Best effort after a commit: a socket that misses it catches up on its
/// next fetch.
pub(crate) async fn publish(state: &AppState, user: Uuid, frame: UserFrame) {
    if let Err(error) = state.gateway.publish_user(user, &frame).await {
        tracing::debug!(error = error.code(), "user frame not published");
    }
}

/// Sorted pair so A↔B is one channel regardless of who opens it.
pub fn pair_key(a: Uuid, b: Uuid) -> String {
    let (lo, hi) = if a.as_u128() <= b.as_u128() {
        (a, b)
    } else {
        (b, a)
    };
    format!("{lo}:{hi}")
}

fn parse_user_id(raw: &str, errors: &mut FieldErrors) -> Option<Uuid> {
    let value = raw.trim();
    if value.is_empty() {
        errors.insert("user_id", "required");
        return None;
    }
    match value.parse::<Uuid>() {
        Ok(id) => Some(id),
        Err(_) => {
            errors.insert("user_id", "invalid");
            None
        }
    }
}

/// How two people may start a conversation (#563).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Friends,
    SharedServer,
    Strangers,
}

pub async fn relation<'e, E: PgExecutor<'e>>(
    db: E,
    a: Uuid,
    b: Uuid,
) -> Result<Relation, ApiError> {
    let (friends, shared): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM friendships \
                        WHERE user_low = LEAST($1, $2) AND user_high = GREATEST($1, $2)), \
                EXISTS (SELECT 1 FROM server_members x \
                        JOIN server_members y ON y.server_id = x.server_id \
                        WHERE x.user_id = $1 AND y.user_id = $2)",
    )
    .bind(a)
    .bind(b)
    .fetch_one(db)
    .await?;
    Ok(if friends {
        Relation::Friends
    } else if shared {
        Relation::SharedServer
    } else {
        Relation::Strangers
    })
}

async fn open_or_create(
    db: &PgPool,
    user_id: Uuid,
    peer: &User,
) -> Result<(DirectMessage, bool), ApiError> {
    let key = pair_key(user_id, peer.id);
    if let Some(existing) = find_by_pair(db, user_id, &key).await? {
        return Ok((existing, false));
    }

    let mut tx = db.begin().await?;
    // Serialise concurrent opens of the same pair on this transaction.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1), 0)")
        .bind(&key)
        .execute(&mut *tx)
        .await?;
    if let Some(existing) = find_by_pair(&mut *tx, user_id, &key).await? {
        tx.commit().await?;
        return Ok((existing, false));
    }
    let request = match relation(&mut *tx, user_id, peer.id).await? {
        Relation::Friends => false,
        Relation::SharedServer => true,
        Relation::Strangers => return Err(ApiError::DmNotAllowed),
    };

    let channel = insert_dm(&mut tx, &key, request.then_some(user_id)).await?;
    insert_member(&mut tx, channel.id, user_id).await?;
    insert_member(&mut tx, channel.id, peer.id).await?;
    crate::gateway::delivery::lock_channel(&mut tx, channel.id).await?;
    crate::gateway::delivery::enqueue_dm(&mut tx, user_id, channel.id).await?;
    // The recipient hears of a request with its first message (`mr`).
    if !request {
        crate::gateway::delivery::enqueue_dm(&mut tx, peer.id, channel.id).await?;
    }
    tx.commit().await?;

    Ok((
        DirectMessage {
            id: channel.id,
            kind: ChannelKind::Dm,
            created_at: channel.created_at,
            peer: Peer {
                id: peer.id,
                name: peer.name.clone(),
                avatar_url: peer.avatar_url.clone(),
            },
            request: request.then_some(DmRequest {
                incoming: false,
                remaining: request::MAX_MESSAGES,
            }),
        },
        true,
    ))
}

async fn insert_dm<'e>(
    tx: &mut Transaction<'e, Postgres>,
    pair_key: &str,
    request_by: Option<Uuid>,
) -> Result<Channel, ApiError> {
    Ok(sqlx::query_as::<_, Channel>(concat!(
        "INSERT INTO channels AS c (name, kind, pair_key, dm_request_by, dm_request_since) \
         VALUES ('', 'dm', $1, $2, CASE WHEN $2::uuid IS NULL THEN NULL ELSE now() END) RETURNING ",
        crate::servers::channel::channel_columns!()
    ))
    .bind(pair_key)
    .bind(request_by)
    .fetch_one(&mut **tx)
    .await?)
}

async fn insert_member<'e>(
    tx: &mut Transaction<'e, Postgres>,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO channel_members (channel_id, user_id) VALUES ($1, $2)")
        .bind(channel_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The caller's DMs, without requests sent to them (those are
/// [`list_requests`]).
async fn load_list(db: &PgPool, user_id: Uuid) -> Result<Vec<DirectMessage>, ApiError> {
    let rows = sqlx::query_as::<_, DmRow>(concat!(
        "SELECT c.id, c.created_at, \
                p.user_id AS peer_id, u.name AS peer_name, u.avatar_url AS peer_avatar_url",
        request_columns!(),
        "FROM channel_members me \
         JOIN channels c ON c.id = me.channel_id AND c.kind = 'dm' \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> me.user_id \
         JOIN users u ON u.id = p.user_id \
         WHERE me.user_id = $1 AND (c.dm_request_by IS NULL OR c.dm_request_by = $1) \
         ORDER BY COALESCE(\
            (SELECT MAX(m.created_at) FROM messages m WHERE m.channel_id = c.id), \
            c.created_at\
         ) DESC, c.id"
    ))
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(DirectMessage::from).collect())
}

async fn load_one(db: &PgPool, channel_id: Uuid, user_id: Uuid) -> Result<DirectMessage, ApiError> {
    let row = sqlx::query_as::<_, DmRow>(concat!(
        "SELECT c.id, c.created_at, \
                p.user_id AS peer_id, u.name AS peer_name, u.avatar_url AS peer_avatar_url",
        request_columns!(),
        "FROM channel_members me \
         JOIN channels c ON c.id = me.channel_id AND c.kind = 'dm' \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> me.user_id \
         JOIN users u ON u.id = p.user_id \
         WHERE me.user_id = $1 AND c.id = $2"
    ))
    .bind(user_id)
    .bind(channel_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(row.into())
}

async fn find_by_pair<'e, E>(
    db: E,
    user_id: Uuid,
    key: &str,
) -> Result<Option<DirectMessage>, ApiError>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as::<_, DmRow>(concat!(
        "SELECT c.id, c.created_at, \
                p.user_id AS peer_id, u.name AS peer_name, u.avatar_url AS peer_avatar_url",
        request_columns!(),
        "FROM channels c \
         JOIN channel_members me ON me.channel_id = c.id AND me.user_id = $1 \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> $1 \
         JOIN users u ON u.id = p.user_id \
         WHERE c.kind = 'dm' AND c.pair_key = $2"
    ))
    .bind(user_id)
    .bind(key)
    .fetch_optional(db)
    .await?;
    Ok(row.map(DirectMessage::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_key_is_order_independent() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(pair_key(a, b), pair_key(b, a));
        assert_ne!(pair_key(a, b), pair_key(a, Uuid::from_u128(3)));
        assert!(pair_key(a, b).starts_with("00000000-0000-0000-0000-000000000001:"));
    }
}
