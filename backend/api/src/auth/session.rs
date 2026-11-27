//! Cookie sessions backed by the `sessions` table, plus the request
//! extractors handlers use to find out who is calling.

use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

use super::device::{Device, DeviceClient};
use super::user::{self, User};
use crate::cookies::{self, SESSION_COOKIE};
use crate::error::ApiError;
use crate::state::AppState;
use crate::token;

/// Creates a session row and returns the raw cookie token. Only the hash is
/// stored; the raw value exists exactly once, in the `Set-Cookie` header.
/// `device` is the client that receives the cookie (`auth::device`).
pub async fn create(
    db: &PgPool,
    user_id: Uuid,
    ttl: Duration,
    device: Device,
) -> Result<String, ApiError> {
    let raw = token::generate();
    let expires_at = Utc::now()
        + chrono::Duration::from_std(ttl)
            .map_err(|err| ApiError::Internal(format!("session ttl out of range: {err}")))?;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, expires_at, device_client, device_os) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(token::hash(&raw))
    .bind(user_id)
    .bind(expires_at)
    .bind(device.client)
    .bind(device.os)
    .execute(db)
    .await?;
    Ok(raw)
}

const SESSION_ROW: &str = "SELECT u.id, u.email, u.name, u.avatar_url, u.created_at, u.bio, \
         u.pronouns, u.status_text, u.accent_color, u.banner_color, u.banner_url, \
         u.avatar_decoration, u.locale, s.created_at AS session_created_at, s.expires_at, \
         s.id AS session_id, s.device_client \
     FROM sessions s JOIN users u ON u.id = s.user_id \
     WHERE s.token_hash = $1 AND s.expires_at > now() AND u.disabled_at IS NULL \
         AND u.deleted_at IS NULL";

#[derive(sqlx::FromRow)]
struct SessionRow {
    #[sqlx(flatten)]
    user: User,
    session_created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    session_id: Uuid,
    device_client: DeviceClient,
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
/// absent; the next sign-in sweeps them (`revoke`). So is any session of a
/// disabled account (#517), whose rows disabling removes. Read-only: the
/// request extractors renew the session on top ([`touch`]).
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
/// its absolute end (`created_at + ttl`) when there is one.
pub fn renewed_expiry(
    created_at: DateTime<Utc>,
    now: DateTime<Utc>,
    idle: Duration,
    ttl: Option<Duration>,
) -> DateTime<Utc> {
    let after = |from: DateTime<Utc>, d: Duration| {
        chrono::Duration::from_std(d)
            .ok()
            .and_then(|d| from.checked_add_signed(d))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    };
    let idle_end = after(now, idle);
    match ttl {
        Some(ttl) => idle_end.min(after(created_at, ttl)),
        None => idle_end,
    }
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
    ttl: Option<Duration>,
) -> bool {
    renewed_expiry(created_at, Utc::now(), idle, ttl) - expires_at >= renew_step(idle)
}

/// The session cookie's `Max-Age`: 400 days, the longest a browser keeps a
/// cookie. The row alone decides whether the session is alive, and its end
/// moves with use (GEL-83), so the cookie must outlive it; a request that
/// renews the row sends the cookie again ([`refresh_cookie`]), so a session
/// in use never loses it either.
pub const COOKIE_MAX_AGE: Duration = Duration::from_secs(400 * 24 * 3600);

/// The session cookie for `raw`, as sign-in and renewal send it.
pub fn cookie(state: &AppState, raw: impl Into<String>) -> cookies::SetCookie {
    cookies::SetCookie::new(SESSION_COOKIE, raw, COOKIE_MAX_AGE.as_secs())
        .secure(state.cookie_secure)
}

/// How long a voice seat keeps its session alive past the session's end:
/// the socket that holds the seat moves the end this far ahead whenever less
/// than half of it is left ([`hold_for_seat`]). So the end waits until the
/// call is left, and comes at most this long after.
pub const SEAT_HOLD: Duration = Duration::from_secs(120);

/// A socket that holds a voice seat keeps its session from ending
/// mid-call (GEL-83): once less than half of [`SEAT_HOLD`] is left, it
/// moves the row's end to `now + SEAT_HOLD`, past the idle window and the
/// absolute end alike. Like [`touch`] it never moves an end back nor
/// revives a row that already expired or was revoked. Returns the row's
/// expiry afterwards, `None` when it is gone.
pub async fn hold_for_seat(db: &PgPool, hash: &[u8]) -> Result<Option<DateTime<Utc>>, ApiError> {
    Ok(sqlx::query_scalar(
        "UPDATE sessions SET expires_at = GREATEST(expires_at, now() + make_interval(secs => $2)) \
         WHERE token_hash = $1 AND expires_at > now() RETURNING expires_at",
    )
    .bind(hash)
    .bind(SEAT_HOLD.as_secs_f64())
    .fetch_optional(db)
    .await?)
}

/// Sliding lifetime (#195): a use of the session pushes its expiry to
/// [`renewed_expiry`]. Throttled by [`needs_renewal`]; only ever moves the
/// expiry forward, and never revives a row that has already expired or was
/// revoked meanwhile. Returns the expiry the row has afterwards, or `None`
/// when it is gone. A request whose renewal moved the row sends the cookie
/// again ([`refresh_cookie`]); the row alone decides whether the session is
/// still alive.
///
/// The same write moves `last_used_at`, so "last active" is accurate to one
/// renewal step. A row still on the `unknown` device (issued before devices
/// were recorded) takes `device`, the one of the request that renews it.
pub async fn touch(
    state: &AppState,
    hash: &[u8],
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    device: Option<Device>,
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
    // Both CASEs read the row as it was, so the client guards the OS too.
    Ok(sqlx::query_scalar(
        "UPDATE sessions SET expires_at = GREATEST(expires_at, $2), last_used_at = now(), \
             device_client = CASE WHEN device_client = 'unknown' AND $3::text IS NOT NULL \
                 THEN $3 ELSE device_client END, \
             device_os = CASE WHEN device_client = 'unknown' AND $3::text IS NOT NULL \
                 THEN $4 ELSE device_os END \
         WHERE token_hash = $1 AND expires_at > now() RETURNING expires_at",
    )
    .bind(hash)
    .bind(target)
    .bind(device.map(|device| device.client))
    .bind(device.map(|device| device.os))
    .fetch_optional(&state.db)
    .await?)
}

/// [`touch`] for the request extractors: a failed renewal must not fail a
/// request the session already authorized. The session then simply keeps
/// its current expiry until the next use. `device` is the request's; only a
/// row whose device is still unknown takes it.
async fn renew(state: &AppState, session: &CurrentSession, device: Device) -> DateTime<Utc> {
    let fill = (session.device_client == DeviceClient::Unknown).then_some(device);
    match touch(
        state,
        &session.hash,
        session.created_at,
        session.expires_at,
        fill,
    )
    .await
    {
        Ok(Some(expires_at)) => expires_at,
        Ok(None) => session.expires_at,
        Err(err) => {
            warn!(error = err.code(), "session not renewed");
            session.expires_at
        }
    }
}

/// Brings every live session within the current settings: no later than
/// `idle` from now nor `ttl` after it began (when set). Run at startup, so
/// lowering `API_SESSION_IDLE_HOURS` or setting `API_SESSION_TTL_HOURS`
/// also shortens the sessions that exist; renewal alone never moves an
/// expiry back. A row keeps up to [`SEAT_HOLD`] from now, so a call held
/// past the end ([`hold_for_seat`]) survives the restart that ran this.
/// Returns how many rows it shortened.
pub async fn cap_to_settings(
    db: &PgPool,
    idle: Duration,
    ttl: Option<Duration>,
) -> Result<u64, ApiError> {
    // `LEAST` skips NULL: without a TTL only the idle window caps.
    let result = sqlx::query(
        "WITH capped AS ( \
             SELECT token_hash, GREATEST( \
                 LEAST(now() + make_interval(secs => $1), created_at + make_interval(secs => $2)), \
                 LEAST(expires_at, now() + make_interval(secs => $3))) AS until \
             FROM sessions WHERE expires_at > now()) \
         UPDATE sessions s SET expires_at = capped.until FROM capped \
         WHERE s.token_hash = capped.token_hash AND s.expires_at > capped.until",
    )
    .bind(idle.as_secs_f64())
    .bind(ttl.map(|ttl| ttl.as_secs_f64()))
    .bind(SEAT_HOLD.as_secs_f64())
    .execute(db)
    .await?;
    Ok(result.rows_affected())
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
    /// The public id clients address the session by (`/api/auth/sessions`).
    pub id: Uuid,
    /// As stored; `unknown` for a row issued before devices were recorded.
    pub device_client: DeviceClient,
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
        let exists = self.lock_row(&mut tx).await?;
        Ok(exists.map(|_| tx))
    }

    /// [`Self::lock_live`] for a socket's checks. Only the wait for the row
    /// lock is bounded by `lock_wait` (a logout or deletion holding the row);
    /// the wait for a pool connection is bounded by the pool's own acquire
    /// timeout. A busy pool then delays the socket's next batch, which keeps
    /// queueing, instead of ending the socket (#612).
    pub async fn lock_live_within<'a>(
        &self,
        db: &'a PgPool,
        lock_wait: Duration,
    ) -> Result<Option<sqlx::Transaction<'a, sqlx::Postgres>>, ApiError> {
        let mut tx = db.begin().await?;
        let exists = tokio::time::timeout(lock_wait, self.lock_row(&mut tx))
            .await
            .map_err(|_| ApiError::Internal("session lock timed out".into()))??;
        Ok(exists.map(|_| tx))
    }

    /// The account is checked too, like in [`resolve`]: a disabled one ends
    /// its sockets and voice leases even with session rows left (#694). Only
    /// the session row is locked: disabling and deletion lock the user row
    /// first and then delete the sessions. A socket that locks the user row
    /// later in the same transaction (`presence::restore`) can still meet
    /// them in a deadlock; disabling tries again then.
    async fn lock_row(&self, db: &mut sqlx::PgConnection) -> Result<Option<Vec<u8>>, ApiError> {
        Ok(sqlx::query_scalar(
            "SELECT s.token_hash FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = $1 AND s.user_id = $2 AND s.expires_at > now() \
                 AND u.disabled_at IS NULL AND u.deleted_at IS NULL \
             FOR SHARE OF s",
        )
        .bind(&self.hash)
        .bind(self.user.id)
        .fetch_optional(db)
        .await?)
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
        id: row.session_id,
        device_client: row.device_client,
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
    let renewed = renew(state, &session, Device::from_headers(&parts.headers)).await;
    if renewed > session.expires_at
        && let Some(CookieRefresh(flag)) = parts.extensions.get::<CookieRefresh>()
    {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    session.expires_at = renewed;
    if from_gate {
        parts.extensions.insert(Resolved(Some(session.clone())));
    }
    Ok(Some(session))
}

/// Stable Redis identifier for the stored hash, never the raw cookie token.
pub fn fingerprint(hash: &[u8]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Set by [`session_of`] when this request moved the session's end, so
/// [`refresh_cookie`] sends the cookie again.
#[derive(Clone)]
struct CookieRefresh(std::sync::Arc<std::sync::atomic::AtomicBool>);

/// Sends the session cookie again, with a fresh `Max-Age`, on a response
/// to a request that renewed its session (GEL-83). Renewals are throttled
/// to one per step ([`needs_renewal`]), so most responses carry none. A
/// response that sets or clears the cookie itself (sign-in, logout) keeps
/// its own.
pub async fn refresh_cookie(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let raw = cookies::get(request.headers(), SESSION_COOKIE).filter(|raw| token::is_valid(raw));
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if raw.is_some() {
        request.extensions_mut().insert(CookieRefresh(flag.clone()));
    }
    let mut response = next.run(request).await;
    let Some(raw) = raw else {
        return response;
    };
    let prefix = format!("{SESSION_COOKIE}=");
    let sets_own = response
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .any(|value| value.as_bytes().starts_with(prefix.as_bytes()));
    if flag.load(std::sync::atomic::Ordering::Relaxed) && !sets_own {
        let (name, value) = cookie(&state, raw).header();
        response.headers_mut().append(name, value);
    }
    response
}

/// Why a session ended, as the gateway's close frame tells the client
/// (GEL-83, `docs/native/protocol-gateway.md` §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// Its time ran out (idle window or absolute end), or no other reason
    /// is known.
    Expired,
    /// Another device (or this one) signed it out: a logout, "sign out
    /// everywhere", the signed-in devices list.
    SignedOut,
    /// The account's password changed or was reset.
    PasswordChanged,
    /// The operator disabled the account.
    Disabled,
    /// The account was deleted.
    Deleted,
}

impl EndReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::SignedOut => "signed_out",
            Self::PasswordChanged => "password_changed",
            Self::Disabled => "disabled",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [
            Self::Expired,
            Self::SignedOut,
            Self::PasswordChanged,
            Self::Disabled,
            Self::Deleted,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == raw)
    }
}

/// Where a revocation leaves its reason for sockets on other API processes,
/// which learn of it only from the database ([`end_reason`]).
fn end_key(fingerprint: &str) -> String {
    format!("gb:session:end:{fingerprint}")
}

/// How long a revocation's reason stays readable: well past the sockets'
/// own backstop check (10 s) and a lost notice.
const END_REASON_TTL: Duration = Duration::from_secs(600);

/// Why the session behind `hash` is gone, for the close frame of a socket
/// that found it so. The reason its revocation left ([`disconnect`]); else
/// a disabled or deleted account; else it expired. Best effort: without
/// Redis or the database the answer is `Expired`.
pub async fn end_reason(state: &AppState, hash: &[u8], user_id: Uuid) -> EndReason {
    let key = end_key(&fingerprint(hash));
    let stored = tokio::time::timeout(
        Duration::from_millis(500),
        state.gateway.with_redis(|mut conn| {
            let key = key.clone();
            async move {
                redis::cmd("GET")
                    .arg(key)
                    .query_async::<Option<String>>(&mut conn)
                    .await
            }
        }),
    )
    .await;
    if let Ok(Ok(Some(raw))) = stored
        && let Some(reason) = EndReason::parse(&raw)
    {
        return reason;
    }
    let account = tokio::time::timeout(
        Duration::from_millis(500),
        sqlx::query_as::<_, (bool, bool)>(
            "SELECT disabled_at IS NOT NULL, deleted_at IS NOT NULL FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&state.db),
    )
    .await;
    match account {
        Ok(Ok(Some((_, true)))) => EndReason::Deleted,
        Ok(Ok(Some((true, _)))) => EndReason::Disabled,
        _ => EndReason::Expired,
    }
}

/// Every session of `user_id`: "sign out everywhere". Same order as
/// [`revoke_current`]: rows first, then the sockets and tickets they back.
pub async fn revoke_all(
    state: &AppState,
    user_id: Uuid,
    reason: EndReason,
) -> Result<(), ApiError> {
    let revoked: Vec<Vec<u8>> =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING token_hash")
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
    disconnect(state, &revoked, reason).await;
    Ok(())
}

/// Closes the sockets and media tickets of sessions already deleted, and
/// tells them `reason`. The reason goes to Redis first, so sockets on other
/// API processes read it once their check finds the row gone.
pub async fn disconnect(state: &AppState, hashes: &[Vec<u8>], reason: EndReason) {
    if hashes.is_empty() {
        return;
    }
    let keys: Vec<String> = hashes.iter().map(|hash| fingerprint(hash)).collect();
    let stored = tokio::time::timeout(
        Duration::from_millis(500),
        state.gateway.with_redis(|mut conn| {
            let mut pipe = redis::pipe();
            for key in &keys {
                pipe.cmd("SET")
                    .arg(end_key(key))
                    .arg(reason.as_str())
                    .arg("EX")
                    .arg(END_REASON_TTL.as_secs())
                    .ignore();
            }
            async move { pipe.query_async::<()>(&mut conn).await }
        }),
    )
    .await;
    if !matches!(stored, Ok(Ok(()))) {
        warn!("session end reason not stored; other processes say expired");
    }
    for key in keys {
        state.gateway.revoke_session(&key, reason).await;
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
    disconnect(state, std::slice::from_ref(&hash), EndReason::SignedOut).await;
    Ok(())
}
