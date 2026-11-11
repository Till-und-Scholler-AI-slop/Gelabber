//! `/api/auth/sessions`: the signed-in devices of the caller's account and
//! signing out one of them (B2). Clients address a session by its public
//! `id`; the token hash never leaves the server.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::info;
use uuid::Uuid;

use super::device::{DeviceClient, DeviceOs};
use super::session::{self, CurrentSession};
use crate::error::ApiError;
use crate::path::Id;
use crate::state::AppState;

/// More than anyone signs in from; a longer list is cut, oldest use last.
const LIST_LIMIT: i64 = 50;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DeviceSession {
    pub id: Uuid,
    #[sqlx(rename = "device_client")]
    pub client: DeviceClient,
    #[sqlx(rename = "device_os")]
    pub os: DeviceOs,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    /// Whether this is the session of the request.
    pub current: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionList {
    pub sessions: Vec<DeviceSession>,
}

/// `GET /api/auth/sessions`: the live sessions of the account, this one
/// first, then by last use.
pub(super) async fn list(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<Json<SessionList>, ApiError> {
    let sessions = sqlx::query_as::<_, DeviceSession>(
        "SELECT id, device_client, device_os, created_at, last_used_at, \
                token_hash = $2 AS current \
         FROM sessions WHERE user_id = $1 AND expires_at > now() \
         ORDER BY token_hash = $2 DESC, last_used_at DESC, id LIMIT $3",
    )
    .bind(current.user.id)
    .bind(&current.hash)
    .bind(LIST_LIMIT)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(SessionList { sessions }))
}

/// `DELETE /api/auth/sessions/{id}`: signs out one device of the account.
/// Another one answers `204` and loses its sockets and media tickets; this
/// one is a logout (`200` with the cleared cookie). An unknown or expired
/// id, or one of another account, is `404`.
pub(super) async fn revoke(
    State(state): State<AppState>,
    current: CurrentSession,
    Id(id): Id,
) -> Result<Response, ApiError> {
    let hash: Vec<u8> = sqlx::query_scalar(
        "DELETE FROM sessions WHERE id = $1 AND user_id = $2 AND expires_at > now() \
         RETURNING token_hash",
    )
    .bind(id)
    .bind(current.user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    session::disconnect(&state, std::slice::from_ref(&hash)).await;
    info!(user_id = %current.user.id, session_id = %id, "device signed out");
    if hash == current.hash {
        return Ok(super::signed_out(&state));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
