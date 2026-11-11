//! `/api/servers/{id}/join-requests`: the queue of requests to join. The
//! owner and members holding `moderate_members` read it, accept and decline.
//! It keeps working while the directory is off and whatever the listing
//! says, so pending requests can still be answered.
//!
//! After every change to a server's pending requests, everyone who answers
//! them hears the new count (`jq`); an accepted person hears `jn`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::{debug, info};
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::ApiError;
use crate::gateway::protocol::UserFrame;
use crate::path::{Id, Ids};
use crate::servers;
use crate::servers::access;
use crate::servers::membership;
use crate::servers::permissions::Permission;
use crate::state::AppState;

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/servers/{id}/join-requests", get(list))
        .route(
            "/api/servers/{id}/join-requests/{user_id}/accept",
            post(accept),
        )
        .route(
            "/api/servers/{id}/join-requests/{user_id}/decline",
            post(decline),
        )
}

/// One pending request, as the queue shows it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct JoinRequest {
    pub user_id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

/// `GET /api/servers/{id}/join-requests`: the pending requests, oldest
/// first. At most [`super::PENDING_PER_SERVER`], so no paging.
async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<Vec<JoinRequest>>, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ModerateMembers)?;
    let rows = sqlx::query_as::<_, JoinRequest>(
        "SELECT r.user_id, u.name, u.avatar_url, r.message, r.created_at \
         FROM server_join_requests r JOIN users u ON u.id = r.user_id \
         WHERE r.server_id = $1 AND r.status = 'pending' \
         ORDER BY r.created_at, r.user_id",
    )
    .bind(server_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

/// `POST /api/servers/{id}/join-requests/{user_id}/accept`: the requester
/// becomes a member. A banned requester's request goes and answers `404`;
/// one who is a member already only loses the request. An account under
/// deletion answers `404` like a request that is gone.
async fn accept(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Ids(server_id, requester): Ids,
) -> Result<StatusCode, ApiError> {
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user.id).await?;
    member.require(Permission::ModerateMembers)?;
    // An account under deletion has no requests left; one that is being
    // deleted right now is waited for (`servers::account_alive`). Before the
    // request's row lock, in the order the deletion takes them.
    if !servers::account_alive(&mut tx, requester).await? {
        return Err(ApiError::NotFound);
    }

    let pending: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM server_join_requests \
         WHERE server_id = $1 AND user_id = $2 AND status = 'pending' FOR UPDATE",
    )
    .bind(server_id)
    .bind(requester)
    .fetch_optional(&mut *tx)
    .await?;
    if pending.is_none() {
        return Err(ApiError::NotFound);
    }
    sqlx::query("DELETE FROM server_join_requests WHERE server_id = $1 AND user_id = $2")
        .bind(server_id)
        .bind(requester)
        .execute(&mut *tx)
        .await?;

    let (banned, already): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM server_bans WHERE server_id = $1 AND user_id = $2), \
                EXISTS (SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
    )
    .bind(server_id)
    .bind(requester)
    .fetch_one(&mut *tx)
    .await?;
    // A ban deletes the request itself; this is the race with one.
    let joined = !banned && !already;
    if joined {
        sqlx::query("INSERT INTO server_members (server_id, user_id) VALUES ($1, $2)")
            .bind(server_id)
            .bind(requester)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    if joined {
        info!(server_id = %server_id, user_id = %requester, by = %user.id, "join request accepted");
        if let Err(error) = state
            .gateway
            .publish_user(requester, &UserFrame::Joined { s: server_id })
            .await
        {
            debug!(error = error.code(), "join not announced");
        }
    }
    publish_queue(&state, server_id).await;
    if banned {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/servers/{id}/join-requests/{user_id}/decline`: the request
/// stays as declined, which starts the requester's cooldown.
async fn decline(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Ids(server_id, requester): Ids,
) -> Result<StatusCode, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ModerateMembers)?;
    let declined = sqlx::query(
        "UPDATE server_join_requests \
         SET status = 'declined', decided_at = now(), decided_by = $3 \
         WHERE server_id = $1 AND user_id = $2 AND status = 'pending'",
    )
    .bind(server_id)
    .bind(requester)
    .bind(user.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if declined == 0 {
        return Err(ApiError::NotFound);
    }
    info!(server_id = %server_id, user_id = %requester, by = %user.id, "join request declined");
    publish_queue(&state, server_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// After the commit: the server's pending count to everyone who answers
/// its requests (`jq`). Best effort; a client catches up on its next read.
pub(crate) async fn publish_queue(state: &AppState, server_id: Uuid) {
    let found = async {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM server_join_requests \
             WHERE server_id = $1 AND status = 'pending'",
        )
        .bind(server_id)
        .fetch_one(&state.db)
        .await?;
        let holders = access::holders(&state.db, server_id, Permission::ModerateMembers).await?;
        Ok::<_, ApiError>((n, holders))
    }
    .await;
    let (n, holders) = match found {
        Ok(found) => found,
        Err(error) => {
            debug!(error = error.code(), %server_id, "join queue count not announced");
            return;
        }
    };
    let frame = UserFrame::JoinRequests { s: server_id, n };
    for holder in holders {
        if let Err(error) = state.gateway.publish_user(holder, &frame).await {
            debug!(error = error.code(), %server_id, "join queue count not announced");
        }
    }
}
