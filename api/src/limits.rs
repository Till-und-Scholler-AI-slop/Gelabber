//! Server-side rate limits and upload quotas (issue #16).
//!
//! Mutations under `/api` go through [`gate`] before CSRF. Auth (login /
//! register) is keyed by client IP; everything else by session cookie (or
//! IP if there is none). Upload *bytes* are a durable daily quota in
//! Postgres so a restart cannot reset them. Request windows live in
//! process — v1 is a single node.
//!
//! Denied requests are `429` with a stable error code (`rate_limited` or
//! `quota_exceeded`) and `Retry-After`. The UI maps those codes; it does not
//! spin.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
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

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub auth_per_min: u32,
    pub api_per_min: u32,
    pub msg_per_min: u32,
    pub upload_per_hour: u32,
    pub upload_bytes_per_day: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            auth_per_min: AUTH_PER_MIN,
            api_per_min: API_PER_MIN,
            msg_per_min: MSG_PER_MIN,
            upload_per_hour: UPLOAD_PER_HOUR,
            upload_bytes_per_day: UPLOAD_BYTES_PER_DAY,
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
    let ip = client_ip(request.headers());
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
    } else if request.method() == Method::POST && path.ends_with("/attachments") {
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

fn client_ip(headers: &HeaderMap) -> String {
    forwarded_ip(headers).unwrap_or_else(|| "unknown".to_owned())
}

fn forwarded_ip(headers: &HeaderMap) -> Option<String> {
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.split(',').next())
        .map(str::trim)
        .filter(|ip| !ip.is_empty());
    if let Some(ip) = forwarded {
        return Some(ip.to_owned());
    }
    headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
        .map(str::to_owned)
}

/// Daily byte quota, counted from attachment metadata (pending + bound).
pub async fn check_upload_quota(
    state: &AppState,
    uploader_id: Uuid,
    size: i64,
) -> Result<(), ApiError> {
    let cap = state.limits.upload_bytes_per_day;
    if cap == 0 {
        return Ok(());
    }
    let used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM attachments \
         WHERE uploader_id = $1 AND created_at >= date_trunc('day', now())",
    )
    .bind(uploader_id)
    .fetch_one(&state.db)
    .await?;
    let used = u64::try_from(used).unwrap_or(0);
    let add = u64::try_from(size).unwrap_or(0);
    if used.saturating_add(add) > cap {
        state.metrics.rate_limited("quota");
        return Err(ApiError::QuotaExceeded);
    }
    Ok(())
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
    fn forwarded_for_takes_the_leftmost_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.0.0.9, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&headers), "10.0.0.9");
    }
}
