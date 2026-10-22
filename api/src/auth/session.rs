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
/// absent; the next sign-in sweeps them (`revoke`).
pub async fn resolve<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    raw: &str,
) -> Result<Option<User>, ApiError> {
    if !token::is_valid(raw) {
        return Ok(None);
    }
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.email, u.name, u.avatar_url, u.created_at, u.bio, u.pronouns, \
             u.status_text, u.accent_color, u.banner_color, u.banner_url, u.avatar_decoration \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(token::hash(raw))
    .fetch_optional(db)
    .await?)
}

/// Removes the session behind `raw` (if given) and, in the same statement,
/// every expired row. Its caller is `signed_in` (login, register without
/// account mail, change-password, verify-email, password-reset confirm),
/// which sweeps the expired rows of every browser this way; logout and the
/// browser's own previous session go through [`revoke_current`] instead.
/// The sweep is backed by `sessions_expires_at_idx`, so it costs the rows
/// that expired since the last sign-in; `tests/auth.rs` pins it.
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
        Ok(Self(
            session_of(parts, state).await?.map(|session| session.user),
        ))
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

/// The exact session that authorized a long-lived connection or media ticket.
/// The cookie itself never leaves the extractor.
#[derive(Debug, Clone)]
pub struct CurrentSession {
    pub user: User,
    pub hash: Vec<u8>,
    pub expires_at: chrono::DateTime<Utc>,
}

impl CurrentSession {
    pub fn key(&self) -> String {
        fingerprint(&self.hash)
    }

    pub async fn lock_live<'a>(
        &self,
        db: &'a PgPool,
    ) -> Result<Option<sqlx::Transaction<'a, sqlx::Postgres>>, ApiError> {
        let mut tx = db.begin().await?;
        let exists: Option<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions WHERE token_hash = $1 AND user_id = $2 AND expires_at > now() FOR SHARE")
            .bind(&self.hash).bind(self.user.id).fetch_optional(&mut *tx).await?;
        Ok(exists.map(|_| tx))
    }
}

impl FromRequestParts<AppState> for CurrentSession {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        session_of(parts, state)
            .await?
            .ok_or(ApiError::Unauthenticated)
    }
}

/// The live session behind a raw cookie token, or `None`. Expired rows are
/// treated as absent, like in [`resolve`]; a cookie no token can have costs
/// no query.
pub async fn lookup(db: &PgPool, raw: &str) -> Result<Option<CurrentSession>, ApiError> {
    if !token::is_valid(raw) {
        return Ok(None);
    }
    let hash = token::hash(raw);
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        user: User,
        expires_at: chrono::DateTime<Utc>,
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT u.id, u.email, u.name, u.avatar_url, u.created_at, u.bio, u.pronouns, u.status_text, u.accent_color, u.banner_color, u.banner_url, u.avatar_decoration, s.expires_at FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = $1 AND s.expires_at > now()",
    ).bind(&hash).fetch_optional(db).await?;
    Ok(row.map(|Row { user, expires_at }| CurrentSession {
        user,
        hash,
        expires_at,
    }))
}

/// The session `limits::gate` looked up for this request, `None` when the
/// cookie matches no live one. Mutations under `/api` carry it, so their
/// extractors do not query the session a second time.
#[derive(Debug, Clone)]
pub struct Resolved(pub Option<CurrentSession>);

/// The request's session: the one the gate resolved, else a lookup of the
/// cookie (GETs, `/ws`, and the sign-in routes, which the gate keys by
/// address alone).
async fn session_of(parts: &Parts, state: &AppState) -> Result<Option<CurrentSession>, ApiError> {
    if let Some(Resolved(session)) = parts.extensions.get::<Resolved>() {
        return Ok(session.clone());
    }
    match cookies::get(&parts.headers, SESSION_COOKIE) {
        Some(raw) => lookup(&state.db, &raw).await,
        None => Ok(None),
    }
}

/// Stable Redis identifier for the stored hash, never the raw cookie token.
pub fn fingerprint(hash: &[u8]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Every session of `user_id`: "sign out everywhere". Same order as
/// [`revoke_current`]: rows first, then the sockets and tickets they back.
pub async fn revoke_all(state: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    let revoked: Vec<Vec<u8>> =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING token_hash")
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
    disconnect(state, &revoked).await;
    Ok(())
}

/// Closes the sockets and media tickets of sessions already deleted.
pub async fn disconnect(state: &AppState, hashes: &[Vec<u8>]) {
    for hash in hashes {
        let key = fingerprint(hash);
        state.gateway.revoke_session(&key).await;
        crate::media::cleanup_session(state.redis.clone(), key).await;
    }
}

/// Delete first, then notify only sockets/tickets bound to that session hash.
/// Database revalidation also covers other API instances and a lost notification.
pub async fn revoke_current(state: &AppState, raw: Option<&str>) -> Result<(), ApiError> {
    let Some(hash) = raw.filter(|raw| token::is_valid(raw)).map(token::hash) else {
        return Ok(());
    };
    // The targeted DB revoke is durable even if Redis is unavailable. No other
    // session's expired-row cleanup delays this logout.
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(&hash)
        .execute(&state.db)
        .await?;
    let key = fingerprint(&hash);
    state.gateway.revoke_session(&key).await;
    crate::media::cleanup_session(state.redis.clone(), key).await;
    Ok(())
}
