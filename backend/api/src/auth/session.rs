//! Cookie sessions backed by the `sessions` table, plus the request
//! extractors handlers use to find out who is calling.

use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::warn;
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

const SESSION_ROW: &str = "SELECT u.id, u.email, u.name, u.avatar_url, u.created_at, u.bio, \
         u.pronouns, u.status_text, u.accent_color, u.banner_color, u.banner_url, \
         u.avatar_decoration, s.created_at AS session_created_at, s.expires_at \
     FROM sessions s JOIN users u ON u.id = s.user_id \
     WHERE s.token_hash = $1 AND s.expires_at > now()";

#[derive(sqlx::FromRow)]
struct SessionRow {
    #[sqlx(flatten)]
    user: User,
    session_created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

async fn row_of<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    hash: &[u8],
) -> Result<Option<SessionRow>, ApiError> {
    Ok(sqlx::query_as::<_, SessionRow>(SESSION_ROW)
        .bind(hash)
        .fetch_optional(db)
        .await?)
}

/// Resolves a raw cookie token to its user. Expired rows are treated as
/// absent; the next sign-in sweeps them (`revoke`). Read-only: the request
/// extractors renew the session on top ([`touch`]).
pub async fn resolve<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    raw: &str,
) -> Result<Option<User>, ApiError> {
    if !token::is_valid(raw) {
        return Ok(None);
    }
    Ok(row_of(db, &token::hash(raw)).await?.map(|row| row.user))
}

/// Where a session used now should expire: `idle` from now, but never past
/// its absolute end (`created_at + ttl`).
pub fn renewed_expiry(
    created_at: DateTime<Utc>,
    now: DateTime<Utc>,
    idle: Duration,
    ttl: Duration,
) -> DateTime<Utc> {
    let after = |from: DateTime<Utc>, d: Duration| {
        chrono::Duration::from_std(d)
            .ok()
            .and_then(|d| from.checked_add_signed(d))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    };
    after(now, idle).min(after(created_at, ttl))
}

/// How far the expiry must move before [`touch`] writes it: a twelfth of the
/// idle window, at most an hour. A busy session costs one `UPDATE` per step,
/// not one per request, and may end up to one step before `idle` is over.
fn renew_step(idle: Duration) -> chrono::Duration {
    chrono::Duration::from_std((idle / 12).min(Duration::from_secs(3600)))
        .unwrap_or(chrono::Duration::zero())
}

/// Whether a session expiring at `expires_at` is due for renewal.
pub fn needs_renewal(
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    idle: Duration,
    ttl: Duration,
) -> bool {
    renewed_expiry(created_at, Utc::now(), idle, ttl) - expires_at >= renew_step(idle)
}

/// Sliding lifetime (#195): a use of the session pushes its expiry to
/// [`renewed_expiry`]. Throttled by [`needs_renewal`]; only ever moves the
/// expiry forward, and never revives a row that has already expired or was
/// revoked meanwhile. Returns the expiry the row has afterwards, or `None`
/// when it is gone. The cookie keeps its `Max-Age` of the absolute TTL; the
/// row alone decides whether the session is still alive.
pub async fn touch(
    state: &AppState,
    hash: &[u8],
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    if !needs_renewal(
        created_at,
        expires_at,
        state.session_idle,
        state.session_ttl,
    ) {
        return Ok(Some(expires_at));
    }
    let target = renewed_expiry(
        created_at,
        Utc::now(),
        state.session_idle,
        state.session_ttl,
    );
    Ok(sqlx::query_scalar(
        "UPDATE sessions SET expires_at = GREATEST(expires_at, $2) \
         WHERE token_hash = $1 AND expires_at > now() RETURNING expires_at",
    )
    .bind(hash)
    .bind(target)
    .fetch_optional(&state.db)
    .await?)
}

/// [`touch`] for the request extractors: a failed renewal must not fail a
/// request the session already authorized. The session then simply keeps
/// its current expiry until the next use.
async fn renew(state: &AppState, session: &CurrentSession) -> DateTime<Utc> {
    match touch(state, &session.hash, session.created_at, session.expires_at).await {
        Ok(Some(expires_at)) => expires_at,
        Ok(None) => session.expires_at,
        Err(err) => {
            warn!(error = err.code(), "session not renewed");
            session.expires_at
        }
    }
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
    pub created_at: DateTime<Utc>,
    /// The row's expiry after this request renewed it.
    pub expires_at: DateTime<Utc>,
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
    Ok(row_of(db, &hash).await?.map(|row| CurrentSession {
        user: row.user,
        hash,
        created_at: row.session_created_at,
        expires_at: row.expires_at,
    }))
}

/// The session `limits::gate` looked up for this request, `None` when the
/// cookie matches no live one. Mutations under `/api` carry it, so their
/// extractors do not query the session a second time.
#[derive(Debug, Clone)]
pub struct Resolved(pub Option<CurrentSession>);

/// The request's session: the one the gate resolved, else a lookup of the
/// cookie (GETs, `/ws`, and the sign-in routes, which the gate keys by
/// address alone). Either way this use renews it ([`renew`], #195); the
/// renewed expiry goes back into [`Resolved`], so a second extractor of
/// the same request does not write again.
async fn session_of(
    parts: &mut Parts,
    state: &AppState,
) -> Result<Option<CurrentSession>, ApiError> {
    let resolved = parts.extensions.get::<Resolved>().cloned();
    let from_gate = resolved.is_some();
    let session = match resolved {
        Some(Resolved(session)) => session,
        None => match cookies::get(&parts.headers, SESSION_COOKIE) {
            Some(raw) => lookup(&state.db, &raw).await?,
            None => None,
        },
    };
    let Some(mut session) = session else {
        return Ok(None);
    };
    session.expires_at = renew(state, &session).await;
    if from_gate {
        parts.extensions.insert(Resolved(Some(session.clone())));
    }
    Ok(Some(session))
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
