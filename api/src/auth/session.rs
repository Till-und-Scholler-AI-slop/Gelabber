//! Cookie sessions backed by the `sessions` table, plus the request
//! extractors handlers use to find out who is calling.

use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::user::{self, User};
use crate::cookies::{self, SESSION_COOKIE};
use crate::error::ApiError;
use crate::state::AppState;
use crate::token;

/// Creates a session row and returns the raw cookie token. Only the hash is
/// stored; the raw value exists exactly once, in the `Set-Cookie` header.
pub async fn create(db: &PgPool, user_id: Uuid, ttl: Duration) -> Result<String, ApiError> {
    let raw = token::generate();
    let expires_at = Utc::now()
        + chrono::Duration::from_std(ttl)
            .map_err(|err| ApiError::Internal(format!("session ttl out of range: {err}")))?;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(token::hash(&raw))
        .bind(user_id)
        .bind(expires_at)
        .execute(db)
        .await?;
    Ok(raw)
}

/// Resolves a raw cookie token to its user. Expired rows are treated as
/// absent (and cleaned up opportunistically by `revoke`).
pub async fn resolve(db: &PgPool, raw: &str) -> Result<Option<User>, ApiError> {
    if !token::is_valid(raw) {
        return Ok(None);
    }
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.email, u.name, u.avatar_url, u.created_at \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(token::hash(raw))
    .fetch_optional(db)
    .await?)
}

/// Removes the session behind `raw` (if any) and, in the same statement,
/// every expired row. Called on logout and before a new login/register so a
/// browser never accumulates more than one live row per sign-in.
pub async fn revoke(db: &PgPool, raw: Option<&str>) -> Result<(), ApiError> {
    let hash = raw.filter(|raw| token::is_valid(raw)).map(token::hash);
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1 OR expires_at <= now()")
        .bind(hash)
        .execute(db)
        .await?;
    Ok(())
}

/// The signed-in user, or `None`. Never fails on a missing cookie.
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<User>);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let Some(raw) = cookies::get(&parts.headers, SESSION_COOKIE) else {
            return Ok(Self(None));
        };
        Ok(Self(resolve(&state.db, &raw).await?))
    }
}

/// The signed-in user; rejects with `401 unauthenticated` otherwise.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match MaybeUser::from_request_parts(parts, state).await?.0 {
            Some(user) => Ok(Self(user)),
            None => Err(ApiError::Unauthenticated),
        }
    }
}

/// Re-reads the user row (used after a profile update so the response
/// reflects the committed state even if a concurrent request changed it).
pub async fn reload(db: &PgPool, id: Uuid) -> Result<User, ApiError> {
    user::by_id(db, id)
        .await?
        .ok_or_else(|| ApiError::Internal(format!("user {id} vanished mid-request")))
}
