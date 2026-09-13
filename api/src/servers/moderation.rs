//! Kick, ban, unban (issue #15). Admin is `manage_server`. A ban row
//! survives membership, so an invite cannot bring the user back.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::{EventKind, publish_server};
use crate::json::Body;
use crate::path::{Id, Ids};
use crate::servers::membership::{self, Membership};
use crate::servers::permissions::Permission;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/servers/{id}/kick", post(kick_member))
        .route("/api/servers/{id}/ban", post(ban_member))
        .route("/api/servers/{id}/bans", get(list_bans))
        .route(
            "/api/servers/{id}/bans/{user_id}",
            axum::routing::delete(unban_member),
        )
}

#[derive(Debug, Deserialize)]
pub struct TargetBody {
    #[serde(default)]
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Ban {
    pub user_id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub banned_by: Uuid,
    pub banned_at: DateTime<Utc>,
}

fn parse_target(raw: &str) -> Result<Uuid, ApiError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "user_id", "required",
        )])));
    }
    value.parse().map_err(|_| ApiError::NotFound)
}

async fn kick_member(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<TargetBody>,
) -> Result<StatusCode, ApiError> {
    let actor = membership::load(&state.db, server_id, user.id).await?;
    actor.require(Permission::ManageServer)?;
    let target_id = parse_target(&body.user_id)?;
    guard_target(&actor, user.id, target_id)?;

    let removed = delete_membership(&state.db, server_id, target_id).await?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    info!(server_id = %server_id, user_id = %target_id, "member kicked");
    after_removal(&state, server_id, target_id, "kicked").await;
    Ok(StatusCode::NO_CONTENT)
}

async fn ban_member(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<TargetBody>,
) -> Result<StatusCode, ApiError> {
    let actor = membership::load(&state.db, server_id, user.id).await?;
    actor.require(Permission::ManageServer)?;
    let target_id = parse_target(&body.user_id)?;
    guard_target(&actor, user.id, target_id)?;
    ensure_user_exists(&state.db, target_id).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM server_members WHERE server_id = $1 AND user_id = $2")
        .bind(server_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO server_bans (server_id, user_id, banned_by) VALUES ($1, $2, $3) \
         ON CONFLICT (server_id, user_id) DO NOTHING",
    )
    .bind(server_id)
    .bind(target_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    info!(server_id = %server_id, user_id = %target_id, "member banned");
    after_removal(&state, server_id, target_id, "banned").await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_bans(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<Vec<Ban>>, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ManageServer)?;
    let rows = sqlx::query_as::<_, Ban>(
        "SELECT b.user_id, u.name, u.avatar_url, b.banned_by, b.banned_at \
         FROM server_bans b JOIN users u ON u.id = b.user_id \
         WHERE b.server_id = $1 \
         ORDER BY b.banned_at, b.user_id",
    )
    .bind(server_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

async fn unban_member(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Ids(server_id, target_id): Ids,
) -> Result<StatusCode, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ManageServer)?;

    let deleted = sqlx::query("DELETE FROM server_bans WHERE server_id = $1 AND user_id = $2")
        .bind(server_id)
        .bind(target_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::NotFound);
    }
    info!(server_id = %server_id, user_id = %target_id, "member unbanned");
    Ok(StatusCode::NO_CONTENT)
}

fn guard_target(actor: &Membership, actor_id: Uuid, target_id: Uuid) -> Result<(), ApiError> {
    if target_id == actor_id {
        return Err(ApiError::Forbidden("You cannot kick or ban yourself."));
    }
    if target_id == actor.server.owner_id {
        return Err(ApiError::Forbidden(
            "The server owner cannot be kicked or banned.",
        ));
    }
    Ok(())
}

async fn ensure_user_exists(db: &PgPool, user_id: Uuid) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(db)
        .await?;
    if exists {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

async fn delete_membership(db: &PgPool, server_id: Uuid, user_id: Uuid) -> Result<bool, ApiError> {
    let deleted = sqlx::query("DELETE FROM server_members WHERE server_id = $1 AND user_id = $2")
        .bind(server_id)
        .bind(user_id)
        .execute(db)
        .await?
        .rows_affected();
    Ok(deleted > 0)
}

pub async fn is_banned(db: &PgPool, server_id: Uuid, user_id: Uuid) -> Result<bool, ApiError> {
    let banned: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM server_bans WHERE server_id = $1 AND user_id = $2)",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(banned)
}

async fn after_removal(state: &AppState, server_id: Uuid, user_id: Uuid, reason: &'static str) {
    let channel_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM channels WHERE server_id = $1")
        .bind(server_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    state
        .gateway
        .revoke_server(user_id, server_id, &channel_ids, reason)
        .await;
    if let Err(err) = state
        .gateway
        .forget_server_presence(user_id, server_id)
        .await
    {
        warn!(
            error = err.code(),
            %server_id,
            %user_id,
            reason,
            "presence cleanup after removal failed"
        );
    }
    if let Err(err) = publish_server(
        state,
        server_id,
        EventKind::D,
        Some(user_id),
        Some(serde_json::json!({ "k": if reason == "banned" { "b" } else { "k" } })),
    )
    .await
    {
        warn!(
            error = err.code(),
            %server_id,
            reason,
            "gateway publish after removal failed"
        );
    }
}
