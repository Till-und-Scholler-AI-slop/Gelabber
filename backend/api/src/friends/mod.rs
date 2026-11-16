//! `/api/friends` and `/api/me/friend-code` (issue #563).
//!
//! A friend request goes by friend code (anyone) or by user id (someone
//! whose profile card the sender can see: a shared server or a DM). The
//! recipient accepts or declines; a decline keeps the row for
//! [`COOLDOWN_SECS`], during which the same sender cannot ask again. A
//! withdrawal keeps it as long: asking again within that time is quiet (no
//! second push), so send and withdraw cannot ping someone over and over.
//! A request to someone who already asked the sender accepts theirs.
//! Friends may open an ordinary DM (`crate::dms`); becoming friends also
//! accepts a pending message request between the two.
//!
//! Every change goes to both accounts' sockets (`op: "fr"`), which fetch
//! the list again. The list shows a friend's online status when presence
//! has one (`gateway::live`); it is read on fetch, not pushed.

pub mod code;

use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection, PgPool};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::auth::user::{self, User};
use crate::dms;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::PresenceStatus;
use crate::gateway::protocol::UserFrame;
use crate::json::Body;
use crate::path::Id;
use crate::state::AppState;

/// Outgoing pending requests one account may have at once.
pub const MAX_OUTGOING: i64 = 50;
pub const COOLDOWN_SECS: i64 = 3600;
/// Friend-code lookups per account and hour (limiter bucket `friend_code`),
/// so codes cannot be guessed by trying.
pub const CODE_LOOKUPS_PER_HOUR: u32 = 20;
const HOUR: Duration = Duration::from_secs(3600);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/friends", get(list))
        .route("/api/friends/{id}", delete(remove))
        .route("/api/friends/requests", post(send_request))
        .route("/api/friends/requests/{id}", delete(withdraw))
        .route("/api/friends/requests/{id}/accept", post(accept))
        .route("/api/friends/requests/{id}/decline", post(decline))
        .route("/api/me/friend-code", get(get_code).post(regenerate_code))
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Friend {
    pub user: Person,
    pub since: DateTime<Utc>,
    /// `o` online, `i` idle; absent when offline or unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PresenceStatus>,
}

#[derive(Debug, Serialize)]
pub struct FriendRequest {
    pub user: Person,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct FriendList {
    pub friends: Vec<Friend>,
    pub incoming: Vec<FriendRequest>,
    pub outgoing: Vec<FriendRequest>,
}

#[derive(FromRow)]
struct Row {
    id: Uuid,
    name: String,
    avatar_url: Option<String>,
    at: DateTime<Utc>,
}

impl Row {
    fn person(&self) -> Person {
        Person {
            id: self.id,
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
        }
    }
}

/// `GET /api/friends`: friends by name, pending requests newest first.
async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<FriendList>, ApiError> {
    let friends = sqlx::query_as::<_, Row>(
        "SELECT u.id, u.name, u.avatar_url, f.created_at AS at FROM friendships f \
         JOIN users u ON u.id = CASE WHEN f.user_low = $1 THEN f.user_high ELSE f.user_low END \
         WHERE (f.user_low = $1 OR f.user_high = $1) AND u.deleted_at IS NULL \
         ORDER BY lower(u.name), u.id",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let pending = |sql: &'static str| {
        sqlx::query_as::<_, Row>(sql)
            .bind(user.id)
            .fetch_all(&state.db)
    };
    let incoming = pending(
        "SELECT u.id, u.name, u.avatar_url, r.created_at AS at FROM friend_requests r \
         JOIN users u ON u.id = r.sender_id \
         WHERE r.recipient_id = $1 AND r.status = 'pending' AND u.deleted_at IS NULL \
         ORDER BY r.created_at DESC, u.id",
    )
    .await?;
    let outgoing = pending(
        "SELECT u.id, u.name, u.avatar_url, r.created_at AS at FROM friend_requests r \
         JOIN users u ON u.id = r.recipient_id \
         WHERE r.sender_id = $1 AND r.status = 'pending' AND u.deleted_at IS NULL \
         ORDER BY r.created_at DESC, u.id",
    )
    .await?;
    let ids: Vec<Uuid> = friends.iter().map(|row| row.id).collect();
    // Without Redis nobody shows a status.
    let statuses = state.gateway.statuses_of(&ids).await.unwrap_or_default();
    let requests = |rows: Vec<Row>| {
        rows.into_iter()
            .map(|row| FriendRequest {
                user: row.person(),
                created_at: row.at,
            })
            .collect()
    };
    Ok(Json(FriendList {
        friends: friends
            .into_iter()
            .map(|row| Friend {
                status: statuses
                    .get(&row.id)
                    .copied()
                    .filter(|st| *st != PresenceStatus::Offline),
                user: row.person(),
                since: row.at,
            })
            .collect(),
        incoming: requests(incoming),
        outgoing: requests(outgoing),
    }))
}

#[derive(Debug, Default, Deserialize)]
pub struct SendBody {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The request waits for an answer.
    Pending,
    /// The other person had asked already: now friends.
    Friends,
}

#[derive(Debug, Serialize)]
pub struct SendAnswer {
    pub status: Outcome,
    /// Who the request went to (`peer`, as on a DM: a top-level `user` is
    /// the signed-in account elsewhere in the API).
    pub peer: Person,
}

/// `POST /api/friends/requests` with `user_id` or `code`. `201` for a new
/// request, `200` when it was pending already or made the two friends.
async fn send_request(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<SendBody>,
) -> Result<Response, ApiError> {
    let by_id = body
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let by_code = body
        .code
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let target = match (by_id, by_code) {
        (Some(_), Some(_)) => {
            return Err(ApiError::Validation(FieldErrors::from([(
                "code",
                "exclusive",
            )])));
        }
        (None, None) => {
            return Err(ApiError::Validation(FieldErrors::from([(
                "user_id", "required",
            )])));
        }
        (Some(raw), None) => {
            let id: Uuid = raw
                .parse()
                .map_err(|_| ApiError::Validation(FieldErrors::from([("user_id", "invalid")])))?;
            if id == user.id {
                return Err(ApiError::Validation(FieldErrors::from([(
                    "user_id", "self",
                )])));
            }
            // Someone the sender can see; a stranger is as unknown as a
            // missing id.
            if !user::card_visible(&state.db, user.id, id).await? {
                return Err(ApiError::NotFound);
            }
            id
        }
        (None, Some(raw)) => {
            state
                .limiter
                .hit(
                    "friend_code",
                    &user.id.to_string(),
                    CODE_LOOKUPS_PER_HOUR,
                    HOUR,
                )
                .map_err(|retry_after| {
                    state.metrics.rate_limited("friend_code");
                    ApiError::RateLimited { retry_after }
                })?;
            let code = code::normalize(raw)
                .ok_or_else(|| ApiError::Validation(FieldErrors::from([("code", "invalid")])))?;
            let id: Uuid = sqlx::query_scalar(
                "SELECT id FROM users WHERE friend_code = $1 AND deleted_at IS NULL",
            )
            .bind(&code)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound)?;
            if id == user.id {
                return Err(ApiError::Validation(FieldErrors::from([("code", "self")])));
            }
            id
        }
    };

    let Requested {
        outcome,
        created,
        push,
        dm,
    } = request(&state.db, user.id, target).await?;
    let person: Person = sqlx::query_as("SELECT id, name, avatar_url FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(&state.db)
        .await?;
    announce(&state, user.id, target, dm).await;
    if created {
        info!(user_id = %user.id, to = %target, "friend request sent");
    }
    if push {
        crate::push::invite(&state, target, &user.name, crate::push::Invite::Friend);
    }
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(SendAnswer {
            status: outcome,
            peer: person,
        }),
    )
        .into_response())
}

/// What [`request`] did.
struct Requested {
    outcome: Outcome,
    /// A request is pending now that was not before.
    created: bool,
    /// It is new, not one taken back within the cooldown: push it.
    push: bool,
    /// A message request was accepted on the way.
    dm: bool,
}

/// The friend-request write, in one transaction under the pair's lock.
async fn request(db: &PgPool, me: Uuid, them: Uuid) -> Result<Requested, ApiError> {
    let mut tx = db.begin().await?;
    lock_pair(&mut tx, me, them).await?;
    if !crate::servers::account_alive(&mut tx, them).await? {
        return Err(ApiError::NotFound);
    }
    if are_friends(&mut tx, me, them).await? {
        return Err(ApiError::AlreadyFriends);
    }
    let theirs: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM friend_requests \
                        WHERE sender_id = $1 AND recipient_id = $2 AND status = 'pending')",
    )
    .bind(them)
    .bind(me)
    .fetch_one(&mut *tx)
    .await?;
    if theirs {
        let dm = befriend(&mut tx, me, them).await?;
        tx.commit().await?;
        info!(user_id = %me, with = %them, "friend request answered by asking back");
        return Ok(Requested {
            outcome: Outcome::Friends,
            created: false,
            push: false,
            dm,
        });
    }
    let mine: Option<(String, i64)> = sqlx::query_as(
        "SELECT status, \
                GREATEST(0, CEIL(EXTRACT(EPOCH FROM \
                    COALESCE(closed_at, now()) + make_interval(secs => $3) - now())))::bigint \
         FROM friend_requests WHERE sender_id = $1 AND recipient_id = $2",
    )
    .bind(me)
    .bind(them)
    .bind(COOLDOWN_SECS as f64)
    .fetch_optional(&mut *tx)
    .await?;
    let mut push = true;
    match mine.as_ref().map(|(status, left)| (status.as_str(), *left)) {
        Some(("pending", _)) => {
            tx.commit().await?;
            return Ok(Requested {
                outcome: Outcome::Pending,
                created: false,
                push: false,
                dm: false,
            });
        }
        Some(("declined", left)) if left > 0 => {
            return Err(ApiError::FriendRequestDeclined {
                retry_after: left as u64,
            });
        }
        Some(("withdrawn", left)) if left > 0 => push = false,
        _ => {}
    }
    // Serialise this sender's requests so two at once cannot pass the cap.
    // `NO KEY UPDATE` does not conflict with the `KEY SHARE` that
    // `account_alive` takes on a recipient's row, so crossing requests
    // cannot deadlock.
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(me)
        .execute(&mut *tx)
        .await?;
    let outgoing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM friend_requests WHERE sender_id = $1 AND status = 'pending'",
    )
    .bind(me)
    .fetch_one(&mut *tx)
    .await?;
    if outgoing >= MAX_OUTGOING {
        return Err(ApiError::FriendRequestsFull);
    }
    sqlx::query(
        "INSERT INTO friend_requests (sender_id, recipient_id) VALUES ($1, $2) \
         ON CONFLICT (sender_id, recipient_id) DO UPDATE \
         SET status = 'pending', created_at = now(), closed_at = NULL",
    )
    .bind(me)
    .bind(them)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Requested {
        outcome: Outcome::Pending,
        created: true,
        push,
        dm: false,
    })
}

/// `POST /api/friends/requests/{id}/accept`: `{id}` asked the caller.
async fn accept(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(sender): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    lock_pair(&mut tx, user.id, sender).await?;
    if !crate::servers::account_alive(&mut tx, sender).await? {
        return Err(ApiError::NotFound);
    }
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM friend_requests \
                        WHERE sender_id = $1 AND recipient_id = $2 AND status = 'pending')",
    )
    .bind(sender)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if !pending {
        return Err(ApiError::NotFound);
    }
    let dm = befriend(&mut tx, user.id, sender).await?;
    tx.commit().await?;
    info!(user_id = %user.id, with = %sender, "friend request accepted");
    announce(&state, user.id, sender, dm).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/friends/requests/{id}/decline`: starts the sender's cooldown.
/// The request leaves both lists.
async fn decline(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(sender): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    // As `accept` does, so the two cannot cross.
    lock_pair(&mut tx, user.id, sender).await?;
    let declined = sqlx::query(
        "UPDATE friend_requests SET status = 'declined', closed_at = now() \
         WHERE sender_id = $1 AND recipient_id = $2 AND status = 'pending'",
    )
    .bind(sender)
    .bind(user.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if declined == 0 {
        return Err(ApiError::NotFound);
    }
    tx.commit().await?;
    info!(user_id = %user.id, from = %sender, "friend request declined");
    announce(&state, user.id, sender, false).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/friends/requests/{id}`: the caller withdraws their request.
/// The row stays for the cooldown, so asking again then pushes nothing.
async fn withdraw(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(recipient): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    // As `accept` does: a withdrawal it already read cannot end in a
    // friendship.
    lock_pair(&mut tx, user.id, recipient).await?;
    let gone = sqlx::query(
        "UPDATE friend_requests SET status = 'withdrawn', closed_at = now() \
         WHERE sender_id = $1 AND recipient_id = $2 AND status = 'pending'",
    )
    .bind(user.id)
    .bind(recipient)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if gone == 0 {
        return Err(ApiError::NotFound);
    }
    tx.commit().await?;
    announce(&state, user.id, recipient, false).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/friends/{id}`: ends the friendship. A DM between the two
/// stays usable; it was accepted.
async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(friend): Id,
) -> Result<StatusCode, ApiError> {
    let gone = sqlx::query(
        "DELETE FROM friendships WHERE user_low = LEAST($1, $2) AND user_high = GREATEST($1, $2)",
    )
    .bind(user.id)
    .bind(friend)
    .execute(&state.db)
    .await?
    .rows_affected();
    if gone == 0 {
        return Err(ApiError::NotFound);
    }
    info!(user_id = %user.id, with = %friend, "friend removed");
    announce(&state, user.id, friend, false).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Both requests between the two go, the friendship comes, and a pending
/// message request between them is accepted. Returns whether one was.
async fn befriend(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    a: Uuid,
    b: Uuid,
) -> Result<bool, ApiError> {
    sqlx::query(
        "DELETE FROM friend_requests WHERE (sender_id = $1 AND recipient_id = $2) \
                                       OR (sender_id = $2 AND recipient_id = $1)",
    )
    .bind(a)
    .bind(b)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO friendships (user_low, user_high) VALUES (LEAST($1, $2), GREATEST($1, $2)) \
         ON CONFLICT DO NOTHING",
    )
    .bind(a)
    .bind(b)
    .execute(&mut **tx)
    .await?;
    dms::accept_between(tx, a, b).await
}

pub async fn are_friends(db: &mut PgConnection, a: Uuid, b: Uuid) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM friendships \
                        WHERE user_low = LEAST($1, $2) AND user_high = GREATEST($1, $2))",
    )
    .bind(a)
    .bind(b)
    .fetch_one(db)
    .await?)
}

async fn lock_pair(db: &mut PgConnection, a: Uuid, b: Uuid) -> Result<(), ApiError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 563))")
        .bind(dms::pair_key(a, b))
        .execute(db)
        .await?;
    Ok(())
}

/// After a commit: both lists reload (`fr`); an accepted message request
/// also moves in the recipient's DM list.
async fn announce(state: &AppState, a: Uuid, b: Uuid, dm: bool) {
    for user in [a, b] {
        dms::publish(state, user, UserFrame::Friends).await;
        if dm {
            dms::publish(state, user, UserFrame::MessageRequests).await;
        }
    }
    if dm {
        crate::gateway::delivery::wake(state);
    }
}

#[derive(Debug, Serialize)]
pub struct CodeBody {
    pub code: String,
}

/// `GET /api/me/friend-code`: the caller's code, made on first use.
async fn get_code(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<CodeBody>, ApiError> {
    let current: Option<String> = sqlx::query_scalar("SELECT friend_code FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
        .flatten();
    if let Some(code) = current {
        return Ok(Json(CodeBody { code }));
    }
    Ok(Json(CodeBody {
        code: store_code(&state.db, &user, true).await?,
    }))
}

/// `POST /api/me/friend-code`: a new code; the old one stops working.
async fn regenerate_code(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<CodeBody>, ApiError> {
    let code = store_code(&state.db, &user, false).await?;
    info!(user_id = %user.id, "friend code regenerated");
    Ok(Json(CodeBody { code }))
}

/// Stores a fresh code, again on the rare clash with someone else's.
/// `only_missing`: keep a code another request stored meanwhile.
async fn store_code(db: &PgPool, user: &User, only_missing: bool) -> Result<String, ApiError> {
    for _ in 0..8 {
        let fresh = code::generate();
        let sql = if only_missing {
            "UPDATE users SET friend_code = COALESCE(friend_code, $2) WHERE id = $1 \
             AND deleted_at IS NULL RETURNING friend_code"
        } else {
            "UPDATE users SET friend_code = $2 WHERE id = $1 AND deleted_at IS NULL \
             RETURNING friend_code"
        };
        match sqlx::query_scalar::<_, String>(sql)
            .bind(user.id)
            .bind(&fresh)
            .fetch_optional(db)
            .await
        {
            Ok(Some(code)) => return Ok(code),
            Ok(None) => return Err(ApiError::Unauthenticated),
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Err(ApiError::Internal("no free friend code".into()))
}
