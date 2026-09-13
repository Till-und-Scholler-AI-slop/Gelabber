//! `/api/dms` — 1:1 direct messages as a channel kind (issue #9).
//!
//! A DM is a `channels` row (`kind = 'dm'`) with exactly two
//! `channel_members`. Message history, send, edit, delete and the WS
//! topic are the same paths as a text channel. No group DM, no friends
//! list, no message requests.
//!
//! The protocol `s` field for a DM is the channel id itself, so subscribe
//! / typing / events stay on the existing frames.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgPool, Postgres, Transaction};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::auth::user::{self, User};
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::Id;
use crate::servers::channel::{Channel, ChannelKind};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/dms", get(list_dms).post(open_dm))
        .route("/api/dms/{id}", get(get_dm))
}

/// Public DM as one of the two participants sees it. `peer` is the other
/// person — the stored channel name is empty.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DirectMessage {
    pub id: Uuid,
    pub kind: ChannelKind,
    pub created_at: DateTime<Utc>,
    pub peer: Peer,
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
}

impl From<DmRow> for DirectMessage {
    fn from(row: DmRow) -> Self {
        Self {
            id: row.id,
            kind: ChannelKind::Dm,
            created_at: row.created_at,
            peer: Peer {
                id: row.peer_id,
                name: row.peer_name,
                avatar_url: row.peer_avatar_url,
            },
        }
    }
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
        info!(
            channel_id = %dm.id,
            user_id = %user.id,
            peer_id = %peer.id,
            "dm opened"
        );
        Ok((StatusCode::CREATED, Json(dm)).into_response())
    } else {
        Ok(Json(dm).into_response())
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

    let channel = insert_dm(&mut tx, &key).await?;
    insert_member(&mut tx, channel.id, user_id).await?;
    insert_member(&mut tx, channel.id, peer.id).await?;
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
        },
        true,
    ))
}

async fn insert_dm<'e>(
    tx: &mut Transaction<'e, Postgres>,
    pair_key: &str,
) -> Result<Channel, ApiError> {
    Ok(sqlx::query_as::<_, Channel>(
        "INSERT INTO channels (name, kind, pair_key) VALUES ('', 'dm', $1) \
         RETURNING id, server_id, category_id, name, kind, created_at",
    )
    .bind(pair_key)
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

async fn load_list(db: &PgPool, user_id: Uuid) -> Result<Vec<DirectMessage>, ApiError> {
    let rows = sqlx::query_as::<_, DmRow>(
        "SELECT c.id, c.created_at, \
                p.user_id AS peer_id, u.name AS peer_name, u.avatar_url AS peer_avatar_url \
         FROM channel_members me \
         JOIN channels c ON c.id = me.channel_id AND c.kind = 'dm' \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> me.user_id \
         JOIN users u ON u.id = p.user_id \
         WHERE me.user_id = $1 \
         ORDER BY COALESCE(\
            (SELECT MAX(m.created_at) FROM messages m WHERE m.channel_id = c.id), \
            c.created_at\
         ) DESC, c.id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(DirectMessage::from).collect())
}

async fn load_one(db: &PgPool, channel_id: Uuid, user_id: Uuid) -> Result<DirectMessage, ApiError> {
    let row = sqlx::query_as::<_, DmRow>(
        "SELECT c.id, c.created_at, \
                p.user_id AS peer_id, u.name AS peer_name, u.avatar_url AS peer_avatar_url \
         FROM channel_members me \
         JOIN channels c ON c.id = me.channel_id AND c.kind = 'dm' \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> me.user_id \
         JOIN users u ON u.id = p.user_id \
         WHERE me.user_id = $1 AND c.id = $2",
    )
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
    let row = sqlx::query_as::<_, DmRow>(
        "SELECT c.id, c.created_at, \
                p.user_id AS peer_id, u.name AS peer_name, u.avatar_url AS peer_avatar_url \
         FROM channels c \
         JOIN channel_members me ON me.channel_id = c.id AND me.user_id = $1 \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> $1 \
         JOIN users u ON u.id = p.user_id \
         WHERE c.kind = 'dm' AND c.pair_key = $2",
    )
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
