//! Server-side rate limits and upload quotas (issue #16).
//!
//! Mutations under `/api` go through [`gate`] before CSRF. Auth (login /
//! register) is keyed by client IP; everything else by session cookie (or
//! IP if there is none). Failed logins are also counted per account
//! ([`login_blocked`] / [`login_failed`]), so spreading guesses over many
//! addresses does not multiply them. Upload *bytes* are a durable daily quota in
//! Postgres so a restart cannot reset them. Request windows live in
//! process — v1 is a single node.
//!
//! Denied requests are `429` with a stable error code (`rate_limited` or
//! `quota_exceeded`) and `Retry-After`. The UI maps those codes; it does not
//! spin.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

use crate::cookies::{self, SESSION_COOKIE};
use crate::error::ApiError;
use crate::state::AppState;

pub const AUTH_PER_MIN: u32 = 20;
pub const API_PER_MIN: u32 = 180;
pub const MSG_PER_MIN: u32 = 60;
pub const UPLOAD_PER_HOUR: u32 = 60;
pub const UPLOAD_BYTES_PER_DAY: u64 = 1024 * 1024 * 1024; // 1 GiB
pub const LOGIN_FAILURES_PER_ACCOUNT: u32 = 10;
/// GIF search, trending and sends per user. Each one is a request against
/// the operator's Klipy key, whose testing tier allows 100 per hour in total.
pub const GIF_PER_MIN: u32 = 30;
/// GIF image loads through `/api/gifs/media` per user. Far above
/// [`GIF_PER_MIN`]: one picker page is 24 tiles and a chat history can hold
/// dozens of GIFs, each with a still and an animated rendition.
pub const GIF_MEDIA_PER_MIN: u32 = 240;
/// Messages with links per user per minute that get previews; past it the
/// links stay plain links. Each can mean three page and three image fetches.
pub const PREVIEW_PER_MIN: u32 = 10;
pub const PREVIEW_SERVER_FACTOR: u32 = 6;
/// Window of the per-account failure count, and so the longest wait.
pub const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

pub(crate) const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub auth_per_min: u32,
    pub api_per_min: u32,
    pub msg_per_min: u32,
    pub upload_per_hour: u32,
    pub upload_bytes_per_day: u64,
    /// Failed logins per account within [`LOGIN_FAILURE_WINDOW`]; `0` turns
    /// the per-account count off.
    pub login_failures_per_account: u32,
    /// GIF search/trending/send requests per user per minute (`0` = off).
    pub gif_per_min: u32,
    /// GIF media loads through the relay per user per minute (`0` = off).
    pub gif_media_per_min: u32,
    /// Messages per user per minute whose links get previews (`0` = off);
    /// a server allows [`PREVIEW_SERVER_FACTOR`] times as many.
    pub preview_per_min: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            auth_per_min: AUTH_PER_MIN,
            api_per_min: API_PER_MIN,
            msg_per_min: MSG_PER_MIN,
            upload_per_hour: UPLOAD_PER_HOUR,
            upload_bytes_per_day: UPLOAD_BYTES_PER_DAY,
            login_failures_per_account: LOGIN_FAILURES_PER_ACCOUNT,
            gif_per_min: GIF_PER_MIN,
            gif_media_per_min: GIF_MEDIA_PER_MIN,
            preview_per_min: PREVIEW_PER_MIN,
        }
    }
}

struct Window {
    count: u32,
    reset_at: Instant,
}

/// Fixed-window counters, keyed by `bucket:id`. Cheap and enough for one
/// process. Expired rows are dropped when the map grows.
pub struct Limiter {
    windows: Mutex<HashMap<String, Window>>,
}

impl Limiter {
    pub fn new() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Records one hit. `Ok(retry_after)` means the caller is over the
    /// limit; `Err` is never used — the `Result` shape matches handlers.
    pub fn hit(&self, bucket: &str, id: &str, max: u32, window: Duration) -> Result<(), u64> {
        if max == 0 {
            return Ok(());
        }
        let key = format!("{bucket}:{id}");
        let now = Instant::now();
        let mut map = self.windows.lock().unwrap_or_else(|err| err.into_inner());
        if map.len() > 4096 {
            map.retain(|_, slot| slot.reset_at > now);
        }
        let slot = map.entry(key).or_insert(Window {
            count: 0,
            reset_at: now + window,
        });
        if now >= slot.reset_at {
            slot.count = 0;
            slot.reset_at = now + window;
        }
        if slot.count >= max {
            let wait = slot.reset_at.saturating_duration_since(now);
            return Err(wait.as_secs().max(1));
        }
        slot.count += 1;
        Ok(())
    }

    /// Like [`Self::hit`] once the window is full, without recording a hit.
    pub fn blocked(&self, bucket: &str, id: &str, max: u32) -> Option<u64> {
        if max == 0 {
            return None;
        }
        let key = format!("{bucket}:{id}");
        let now = Instant::now();
        let map = self.windows.lock().unwrap_or_else(|err| err.into_inner());
        let slot = map.get(&key).filter(|slot| now < slot.reset_at)?;
        (slot.count >= max).then(|| {
            slot.reset_at
                .saturating_duration_since(now)
                .as_secs()
                .max(1)
        })
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Outer middleware: count mutating `/api` requests. GET/HEAD/OPTIONS
/// pass. Health, ready, metrics and `/ws` are not on this router.
pub async fn gate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if is_safe(request.method()) {
        return Ok(next.run(request).await);
    }

    let path = request.uri().path();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    let ip = client_ip(request.headers(), peer);
    let session = cookies::get(request.headers(), SESSION_COOKIE);
    let actor = session.as_deref().unwrap_or(ip.as_str());

    let outcome = if path == "/api/auth/login" || path == "/api/auth/register" {
        check(
            &state.limiter,
            "auth",
            &ip,
            state.limits.auth_per_min,
            MINUTE,
        )
    } else if (request.method() == Method::POST && path.ends_with("/attachments"))
        || (request.method() == Method::PUT && matches!(path, "/api/me/avatar" | "/api/me/banner"))
    {
        check(
            &state.limiter,
            "up",
            actor,
            state.limits.upload_per_hour,
            HOUR,
        )
        .and_then(|()| {
            check(
                &state.limiter,
                "api",
                actor,
                state.limits.api_per_min,
                MINUTE,
            )
        })
    } else if request.method() == Method::POST && path.ends_with("/messages") {
        check(
            &state.limiter,
            "msg",
            actor,
            state.limits.msg_per_min,
            MINUTE,
        )
        .and_then(|()| {
            check(
                &state.limiter,
                "api",
                actor,
                state.limits.api_per_min,
                MINUTE,
            )
        })
    } else {
        check(
            &state.limiter,
            "api",
            actor,
            state.limits.api_per_min,
            MINUTE,
        )
    };

    if let Err((bucket, retry_after)) = outcome {
        state.metrics.rate_limited(bucket);
        return Err(ApiError::RateLimited { retry_after });
    }

    Ok(next.run(request).await)
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn check(
    limiter: &Limiter,
    bucket: &'static str,
    id: &str,
    max: u32,
    window: Duration,
) -> Result<(), (&'static str, u64)> {
    limiter
        .hit(bucket, id, max, window)
        .map_err(|retry| (bucket, retry))
}

/// Refuses a login for an account with too many recent failures. Checked
/// before the password hash, so a blocked guess costs no Argon2 run, and
/// answered like every other limit (`429 rate_limited`, `Retry-After`).
pub fn login_blocked(state: &AppState, email: &str) -> Result<(), ApiError> {
    let max = state.limits.login_failures_per_account;
    match state.limiter.blocked("login", &account_key(email), max) {
        Some(retry_after) => {
            state.metrics.rate_limited("login");
            Err(ApiError::RateLimited { retry_after })
        }
        None => Ok(()),
    }
}

/// Counts one failed login against `email`, known account or not, so the
/// answer does not tell the two apart.
pub fn login_failed(state: &AppState, email: &str) {
    let max = state.limits.login_failures_per_account;
    // Over the limit is already refused by `login_blocked` next time.
    let _ = state
        .limiter
        .hit("login", &account_key(email), max, LOGIN_FAILURE_WINDOW);
}

/// The address is normalized by validation; hashed so the limiter's keys
/// hold no addresses.
fn account_key(email: &str) -> String {
    crate::auth::session::fingerprint(&crate::token::hash(email))
}

/// The address the rate limits are keyed by. Caddy, in front of the API in
/// every supported setup, overwrites `X-Forwarded-For` with the peer it saw
/// (it trusts no proxy before it), and a proxy that does trust one appends
/// its own peer last. So the rightmost entry is the one written by the
/// proxy next to the API; entries to its left come from the client and are
/// not believed. Without the header (the API reached directly) the TCP
/// peer is the client.
fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
    forwarded_ip(headers)
        .or_else(|| peer.map(|ip| ip.to_string()))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn forwarded_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .rfind(|ip| !ip.is_empty())
        .map(str::to_owned)
}

/// Lock quota in the SAME transaction as attachment insertion. The insert
/// trigger records the reservation; this conditional upsert holds the ledger
/// row lock until that insert/commit and serializes competing seats.
pub async fn reserve_upload_quota(
    state: &AppState,
    db: &mut sqlx::PgConnection,
    uploader_id: Uuid,
    size: i64,
) -> Result<chrono::NaiveDate, ApiError> {
    let cap = state.limits.upload_bytes_per_day;
    let cap = if cap == 0 {
        i64::MAX
    } else {
        cap.min(i64::MAX as u64) as i64
    };
    let day: Option<chrono::NaiveDate> = sqlx::query_scalar(
        "INSERT INTO upload_daily_usage (uploader_id, day, reserved) \
         SELECT $1, (now() AT TIME ZONE 'UTC')::date, 0 WHERE $2 <= $3 \
         ON CONFLICT (uploader_id, day) DO UPDATE \
         SET reserved=upload_daily_usage.reserved \
         WHERE upload_daily_usage.reserved <= $3 - $2 \
           AND upload_daily_usage.consumed <= $3 - $2 - upload_daily_usage.reserved \
         RETURNING day",
    )
    .bind(uploader_id)
    .bind(size)
    .bind(cap)
    .fetch_optional(db)
    .await?;
    match day {
        Some(day) => Ok(day),
        None => {
            state.metrics.rate_limited("quota");
            Err(ApiError::QuotaExceeded)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_max_then_denies() {
        let limiter = Limiter::new();
        assert!(limiter.hit("auth", "1.1.1.1", 2, MINUTE).is_ok());
        assert!(limiter.hit("auth", "1.1.1.1", 2, MINUTE).is_ok());
        let err = limiter.hit("auth", "1.1.1.1", 2, MINUTE).unwrap_err();
        assert!(err >= 1);
        // A different IP is a different bucket.
        assert!(limiter.hit("auth", "2.2.2.2", 2, MINUTE).is_ok());
    }

    #[test]
    fn zero_max_disables_the_bucket() {
        let limiter = Limiter::new();
        for _ in 0..8 {
            assert!(limiter.hit("api", "x", 0, MINUTE).is_ok());
        }
    }

    #[test]
    fn window_reset_allows_again() {
        let limiter = Limiter::new();
        let window = Duration::from_millis(30);
        assert!(limiter.hit("msg", "u", 1, window).is_ok());
        assert!(limiter.hit("msg", "u", 1, window).is_err());
        std::thread::sleep(Duration::from_millis(40));
        assert!(limiter.hit("msg", "u", 1, window).is_ok());
    }

    #[test]
    fn forwarded_for_takes_the_rightmost_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.0.0.9, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&headers, None), "10.0.0.1");
        headers.append("x-forwarded-for", "10.0.0.2".parse().unwrap());
        assert_eq!(client_ip(&headers, None), "10.0.0.2");
    }

    #[test]
    fn without_forwarded_for_the_peer_is_the_client() {
        let mut headers = HeaderMap::new();
        // Nothing in front of the API wrote this; the client did.
        headers.insert("x-real-ip", "10.9.9.9".parse().unwrap());
        let peer = Some(IpAddr::from([192, 0, 2, 7]));
        assert_eq!(client_ip(&headers, peer), "192.0.2.7");
        assert_eq!(client_ip(&headers, None), "unknown");
    }

    #[test]
    fn blocked_reports_a_full_window_without_counting() {
        let limiter = Limiter::new();
        assert_eq!(limiter.blocked("login", "a", 2), None);
        limiter.hit("login", "a", 2, MINUTE).unwrap();
        assert_eq!(limiter.blocked("login", "a", 2), None);
        limiter.hit("login", "a", 2, MINUTE).unwrap();
        assert!(limiter.blocked("login", "a", 2).is_some());
        assert_eq!(limiter.blocked("login", "b", 2), None);
        assert_eq!(limiter.blocked("login", "a", 0), None);
    }
}
