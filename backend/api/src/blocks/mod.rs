//! `/api/users/me/blocks` — one person blocks another (issue #565).
//!
//! While A blocks B, every private path between the two is closed in both
//! directions: opening a DM, writing into theirs, a message request and a
//! friend request. B gets the error an unreachable person gives
//! (`dm_not_allowed`, `not_found`, `dm_closed`), never "blocked"; A gets
//! `blocked`. Blocking ends their friendship, drops friend requests between
//! them and declines a pending message request, in one transaction. B's
//! pushes and mention counts do not reach A (`crate::push`,
//! `crate::messages::workflows`); on shared servers the clients fold B's
//! messages and typing for A. Unblocking leaves an accepted DM usable and
//! restores no friendship.
//!
//! Only A's sockets are told (`op: "bl"`); B's friend list reloads (`fr`)
//! when a friendship or request went.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgConnection, PgExecutor};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::dms;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::protocol::UserFrame;
use crate::path::Id;
use crate::state::AppState;

/// How many people one account may block.
pub const MAX_BLOCKS: i64 = 1000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users/me/blocks", get(list))
        .route(
            "/api/users/me/blocks/{id}",
            axum::routing::put(block).delete(unblock),
        )
}

#[derive(Debug, Serialize, FromRow)]
pub struct Blocked {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Who blocks whom between the caller and someone else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    None,
    /// The caller blocks them (whatever they do).
    Mine,
    /// They block the caller.
    Theirs,
}

impl Block {
    /// Refuses a private path: `blocked` to the blocker, `neutral` (what an
    /// unreachable person gives) to the one blocked.
    pub fn refuse(self, neutral: ApiError) -> Result<(), ApiError> {
        match self {
            Self::None => Ok(()),
            Self::Mine => Err(ApiError::Blocked),
            Self::Theirs => Err(neutral),
        }
    }
}

pub async fn between<'e, E: PgExecutor<'e>>(
    db: E,
    me: Uuid,
    them: Uuid,
) -> Result<Block, ApiError> {
    let (mine, theirs): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2), \
                EXISTS (SELECT 1 FROM user_blocks WHERE blocker_id = $2 AND blocked_id = $1)",
    )
    .bind(me)
    .bind(them)
    .fetch_one(db)
    .await?;
    Ok(if mine {
        Block::Mine
    } else if theirs {
        Block::Theirs
    } else {
        Block::None
    })
}

/// The block between the caller and the other member of DM `channel`, for
/// a write into it.
pub async fn in_dm<'e, E: PgExecutor<'e>>(
    db: E,
    channel: Uuid,
    me: Uuid,
) -> Result<Block, ApiError> {
    let (mine, theirs): (bool, bool) = sqlx::query_as(
        "SELECT COALESCE(bool_or(b.blocker_id = $2), false), \
                COALESCE(bool_or(b.blocked_id = $2), false) \
         FROM channels c \
         JOIN channel_members p ON p.channel_id = c.id AND p.user_id <> $2 \
         JOIN user_blocks b ON (b.blocker_id = $2 AND b.blocked_id = p.user_id) \
                            OR (b.blocker_id = p.user_id AND b.blocked_id = $2) \
         WHERE c.id = $1 AND c.kind = 'dm'",
    )
    .bind(channel)
    .bind(me)
    .fetch_one(db)
    .await?;
    Ok(if mine {
        Block::Mine
    } else if theirs {
        Block::Theirs
    } else {
        Block::None
    })
}

/// `GET /api/users/me/blocks`: newest first.
async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<Blocked>>, ApiError> {
    Ok(Json(blocked_by(&state.db, user.id).await?))
}

/// Whom `user` blocks, newest first (also for the data export).
pub async fn blocked_by<'e, E: PgExecutor<'e>>(
    db: E,
    user: Uuid,
) -> Result<Vec<Blocked>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.name, u.avatar_url, b.created_at \
         FROM user_blocks b JOIN users u ON u.id = b.blocked_id \
         WHERE b.blocker_id = $1 AND u.deleted_at IS NULL \
         ORDER BY b.created_at DESC, u.id",
    )
    .bind(user)
    .fetch_all(db)
    .await?)
}

/// `PUT /api/users/me/blocks/{id}`: idempotent, `204`.
async fn block(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(them): Id,
) -> Result<StatusCode, ApiError> {
    if them == user.id {
        return Err(ApiError::Validation(FieldErrors::from([(
            "user_id", "self",
        )])));
    }
    let mut tx = state.db.begin().await?;
    crate::friends::lock_pair(&mut tx, user.id, them).await?;
    if !crate::servers::account_alive(&mut tx, them).await? {
        return Err(ApiError::NotFound);
    }
    // Serialise this account's blocks so two at once cannot pass the cap;
    // `NO KEY UPDATE` does not conflict with `account_alive`'s `KEY SHARE`.
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let known: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2)",
    )
    .bind(user.id)
    .bind(them)
    .fetch_one(&mut *tx)
    .await?;
    if known {
        return Ok(StatusCode::NO_CONTENT);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_blocks WHERE blocker_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    if count >= MAX_BLOCKS {
        return Err(ApiError::BlocksFull);
    }
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
        .bind(user.id)
        .bind(them)
        .execute(&mut *tx)
        .await?;
    let cut = cut_ties(&mut tx, user.id, them).await?;
    tx.commit().await?;
    info!(user_id = %user.id, blocked = %them, "user blocked");
    dms::publish(&state, user.id, UserFrame::Blocks).await;
    if cut.friends {
        for who in [user.id, them] {
            dms::publish(&state, who, UserFrame::Friends).await;
        }
    }
    if cut.request {
        for who in [user.id, them] {
            dms::publish(&state, who, UserFrame::MessageRequests).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// What blocking took away between the two.
struct Cut {
    /// A friendship or a friend request.
    friends: bool,
    /// A pending message request.
    request: bool,
}

/// Ends the friendship, deletes friend requests both ways and declines a
/// pending message request (its messages stay for the record, as a silent
/// decline keeps them).
async fn cut_ties(db: &mut PgConnection, a: Uuid, b: Uuid) -> Result<Cut, ApiError> {
    let friendship = sqlx::query(
        "DELETE FROM friendships WHERE user_low = LEAST($1, $2) AND user_high = GREATEST($1, $2)",
    )
    .bind(a)
    .bind(b)
    .execute(&mut *db)
    .await?
    .rows_affected();
    let requests = sqlx::query(
        "DELETE FROM friend_requests WHERE (sender_id = $1 AND recipient_id = $2) \
                                        OR (sender_id = $2 AND recipient_id = $1)",
    )
    .bind(a)
    .bind(b)
    .execute(&mut *db)
    .await?
    .rows_affected();
    let request = sqlx::query(
        "UPDATE channels SET dm_declined_at = now() \
         WHERE kind = 'dm' AND pair_key = $1 AND dm_request_by IS NOT NULL \
           AND dm_declined_at IS NULL",
    )
    .bind(dms::pair_key(a, b))
    .execute(&mut *db)
    .await?
    .rows_affected();
    Ok(Cut {
        friends: friendship + requests > 0,
        request: request > 0,
    })
}

/// `DELETE /api/users/me/blocks/{id}`: idempotent, `204`.
async fn unblock(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(them): Id,
) -> Result<StatusCode, ApiError> {
    let gone = sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
        .bind(user.id)
        .bind(them)
        .execute(&state.db)
        .await?
        .rows_affected();
    if gone > 0 {
        info!(user_id = %user.id, unblocked = %them, "user unblocked");
        dms::publish(&state, user.id, UserFrame::Blocks).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
