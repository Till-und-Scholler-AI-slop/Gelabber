//! Server-side rate limits and upload quotas (issue #16).
//!
//! Mutations under `/api` go through [`gate`] before CSRF. Auth (sign-in,
//! registration, email verification, password reset) is keyed by client
//! address, IPv6 per /64; everything else by a hash of the session cookie,
//! or by the address when there is no cookie shaped like a session token.
//! Failed logins are also counted per account ([`login_blocked`] /
//! [`login_failed`]), so spreading guesses over many addresses does not
//! multiply them. Upload *bytes* are a durable daily quota in Postgres so
//! a restart cannot reset them. Request windows live in process — v1 is a
//! single node.
//!
//! Denied requests are `429` with a stable error code (`rate_limited` or
//! `quota_exceeded`) and `Retry-After`. The UI maps those codes; it does not
//! spin.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
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
/// Account mails (confirmation, reset, "already registered") per address
/// and hour. Past it the request is answered as usual and nothing is sent,
/// so nobody can flood an inbox through the instance.
pub const MAIL_PER_ADDRESS_PER_HOUR: u32 = 5;

pub(crate) const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

/// Above this many keys the limiter drops expired windows, at most once per
/// [`SWEEP_EVERY`].
const SOFT_KEYS: usize = 4096;
/// Most keys the limiter holds: about 10 MB at roughly 150 B per entry,
/// `HashMap` overhead included.
const MAX_KEYS: usize = 65_536;
const SWEEP_EVERY: Duration = Duration::from_secs(10);

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

struct Inner {
    windows: HashMap<String, Window>,
    next_sweep: Instant,
}

impl Inner {
    fn sweep(&mut self, now: Instant) {
        self.windows.retain(|_, slot| slot.reset_at > now);
        self.next_sweep = now + SWEEP_EVERY;
    }

    /// Makes room for a new key once the map holds `hard` of them: expired
    /// windows go first; if that is not enough, the eighth of the keys whose
    /// windows end soonest, the per-account counts (login failures, account
    /// mail) last. Anyone can mint session-bucket keys, hour-long upload
    /// windows among them, with a made-up session-shaped cookie per request,
    /// and such a flood must not end a lockout early. Per-account counts only
    /// grow through the per-address auth window, so filling the map with them
    /// takes many addresses; then the ones closest to their end go first.
    fn make_room(&mut self, now: Instant, hard: usize) {
        self.sweep(now);
        if self.windows.len() < hard {
            return;
        }
        let mut by_expiry: Vec<(bool, Instant, String)> = self
            .windows
            .iter()
            .map(|(key, slot)| (is_per_account(key), slot.reset_at, key.clone()))
            .collect();
        let evicted = (hard / 8).max(1).min(by_expiry.len());
        if evicted < by_expiry.len() {
            by_expiry.select_nth_unstable_by_key(evicted, |entry| (entry.0, entry.1));
        }
        for (_, _, key) in &by_expiry[..evicted] {
            self.windows.remove(key);
        }
        tracing::warn!(
            evicted,
            "rate limiter at capacity; evicted keys closest to expiry"
        );
    }
}

/// Fixed-window counters, keyed by `bucket:id`. Cheap and enough for one
/// process. Past [`SOFT_KEYS`] expired windows are dropped every
/// [`SWEEP_EVERY`], not on every hit; at [`MAX_KEYS`] a new key first makes
/// room (see [`Inner::make_room`]), so a flood of keys can neither grow the
/// map without bound nor make every hit scan it.
pub struct Limiter {
    inner: Mutex<Inner>,
    soft: usize,
    hard: usize,
}

impl Limiter {
    pub fn new() -> Self {
        Self::with_caps(SOFT_KEYS, MAX_KEYS)
    }

    /// [`Self::new`] with other key counts, so tests can reach them.
    fn with_caps(soft: usize, hard: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                windows: HashMap::new(),
                next_sweep: Instant::now(),
            }),
            soft,
            hard,
        }
    }

    /// Records one hit. `Err(retry_after)`, in seconds, means the caller is
    /// over the limit; the `Result` shape matches handlers.
    pub fn hit(&self, bucket: &str, id: &str, max: u32, window: Duration) -> Result<(), u64> {
        self.hit_at(bucket, id, max, window, Instant::now())
    }

    fn hit_at(
        &self,
        bucket: &str,
        id: &str,
        max: u32,
        window: Duration,
        now: Instant,
    ) -> Result<(), u64> {
        if max == 0 {
            return Ok(());
        }
        let key = format!("{bucket}:{id}");
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if inner.windows.len() > self.soft && now >= inner.next_sweep {
            inner.sweep(now);
        }
        // Only a new key pays for room; a known one never scans the map.
        if inner.windows.len() >= self.hard && !inner.windows.contains_key(&key) {
            inner.make_room(now, self.hard);
        }
        let slot = inner.windows.entry(key).or_insert(Window {
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
        let inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let slot = inner.windows.get(&key).filter(|slot| now < slot.reset_at)?;
        (slot.count >= max).then(|| {
            slot.reset_at
                .saturating_duration_since(now)
                .as_secs()
                .max(1)
        })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .windows
            .len()
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
    let actor = actor_key(request.headers(), &ip);

    let outcome = if is_auth_path(path) {
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
            &actor,
            state.limits.upload_per_hour,
            HOUR,
        )
        .and_then(|()| {
            check(
                &state.limiter,
                "api",
                &actor,
                state.limits.api_per_min,
                MINUTE,
            )
        })
    } else if request.method() == Method::POST && path.ends_with("/messages") {
        check(
            &state.limiter,
            "msg",
            &actor,
            state.limits.msg_per_min,
            MINUTE,
        )
        .and_then(|()| {
            check(
                &state.limiter,
                "api",
                &actor,
                state.limits.api_per_min,
                MINUTE,
            )
        })
    } else {
        check(
            &state.limiter,
            "api",
            &actor,
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

/// Routes an anonymous caller uses to sign in or to make the API send mail;
/// they share the per-IP auth window.
fn is_auth_path(path: &str) -> bool {
    matches!(
        path,
        "/api/auth/login"
            | "/api/auth/register"
            | "/api/auth/verify-email"
            | "/api/auth/password-reset"
            | "/api/auth/password-reset/confirm"
    )
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

/// Counts one account mail to `email`; `false` once the hourly allowance of
/// that address is used up and the mail should be dropped.
pub fn mail_allowed(state: &AppState, email: &str) -> bool {
    let allowed = state
        .limiter
        .hit("mail", &account_key(email), MAIL_PER_ADDRESS_PER_HOUR, HOUR)
        .is_ok();
    if !allowed {
        state.metrics.rate_limited("mail");
    }
    allowed
}

/// The address is normalized by validation; hashed so the limiter's keys
/// hold no addresses.
fn account_key(email: &str) -> String {
    crate::auth::session::fingerprint(&crate::token::hash(email))
}

/// Limiter keys of the per-account counts above, which eviction spares.
fn is_per_account(key: &str) -> bool {
    key.starts_with("login:") || key.starts_with("mail:")
}

/// The client address the rate limits are keyed by, parsed, with IPv6
/// grouped per /64 (one subscriber's prefix, like one IPv4 address behind
/// NAT). Caddy, in front of the API in every supported setup, sends
/// `X-Forwarded-For` as the one client address it resolved: its peer, or,
/// when that peer is a proxy it trusts, the rightmost entry that is no
/// trusted proxy. A proxy that appends instead puts its own peer last. So the
/// rightmost entry is the one written by the proxy next to the API; entries
/// to its left come from the client and are not believed, and a rightmost
/// entry that is no address falls back to the TCP peer, never to one further
/// left. Without the header (the API reached directly) the TCP peer is the
/// client.
fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
    forwarded_ip(headers)
        .or(peer)
        .map(rate_key)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .rfind(|ip| !ip.is_empty())?
        .parse()
        .ok()
}

/// IPv4 as is (also when it arrives IPv4-mapped); IPv6 as its /64, since a
/// single client usually holds a whole prefix and picks addresses from it.
fn rate_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{}/64", Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
    }
}

/// Whom the session buckets (`api`, `msg`, `up`) count: the session cookie
/// when it is shaped like a token, as part of its hash so the limiter never
/// holds a live token, otherwise the client address. A cookie no session can
/// have therefore shares the address's windows instead of opening its own.
fn actor_key(headers: &HeaderMap, ip: &str) -> String {
    match cookies::get(headers, SESSION_COOKIE) {
        Some(raw) if crate::token::is_valid(&raw) => {
            // 128 of the 256 bits are plenty to tell sessions apart.
            let hash = crate::auth::session::fingerprint(&crate::token::hash(&raw));
            format!("s:{}", &hash[..32])
        }
        _ => ip.to_owned(),
    }
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

    fn forwarded(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", value.parse().unwrap());
        headers
    }

    #[test]
    fn ipv6_clients_are_grouped_per_64() {
        let one = client_ip(&forwarded("2001:db8:1:2::1"), None);
        assert_eq!(one, "2001:db8:1:2::/64");
        assert_eq!(client_ip(&forwarded("2001:db8:1:2:ffff::9"), None), one);
        assert_eq!(
            client_ip(&forwarded("2001:db8:1:3::1"), None),
            "2001:db8:1:3::/64"
        );
        let peer = Some("2001:db8:1:2:abcd::5".parse().unwrap());
        assert_eq!(client_ip(&HeaderMap::new(), peer), one);
    }

    #[test]
    fn ipv4_mapped_addresses_count_as_ipv4() {
        assert_eq!(client_ip(&forwarded("::ffff:1.2.3.4"), None), "1.2.3.4");
        let peer = Some("::ffff:1.2.3.4".parse().unwrap());
        assert_eq!(client_ip(&HeaderMap::new(), peer), "1.2.3.4");
    }

    #[test]
    fn an_unparsable_rightmost_entry_falls_back_to_the_peer() {
        let headers = forwarded("10.0.0.9, not-an-ip");
        let peer = Some(IpAddr::from([192, 0, 2, 7]));
        // Never the entry further left: the client wrote that one.
        assert_eq!(client_ip(&headers, peer), "192.0.2.7");
        assert_eq!(client_ip(&headers, None), "unknown");
    }

    fn with_session(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            format!("{SESSION_COOKIE}={cookie}").parse().unwrap(),
        );
        headers
    }

    #[test]
    fn a_token_shaped_session_cookie_is_keyed_by_its_hash() {
        let token = crate::token::generate();
        let key = actor_key(&with_session(&token), "192.0.2.7");
        assert!(key.starts_with("s:"), "{key}");
        assert_eq!(key.len(), 2 + 32);
        assert!(!key.contains(&token));
        assert!(!key.contains(&token[..32]));
        assert_eq!(actor_key(&with_session(&token), "198.51.100.1"), key);
        let other = actor_key(&with_session(&crate::token::generate()), "192.0.2.7");
        assert_ne!(other, key);
    }

    #[test]
    fn other_session_cookies_count_against_the_address() {
        let not_hex = "g".repeat(64);
        let short = "f".repeat(63);
        for cookie in ["not-a-token", "abc123", not_hex.as_str(), short.as_str()] {
            assert_eq!(actor_key(&with_session(cookie), "192.0.2.7"), "192.0.2.7");
        }
        assert_eq!(actor_key(&HeaderMap::new(), "192.0.2.7"), "192.0.2.7");
    }

    #[test]
    fn distinct_keys_never_exceed_the_hard_cap() {
        let limiter = Limiter::with_caps(8, 64);
        for i in 0..640 {
            limiter.hit("api", &i.to_string(), 5, MINUTE).unwrap();
            assert!(limiter.len() <= 64, "{} keys after {i}", limiter.len());
        }
    }

    #[test]
    fn a_login_lockout_outlives_a_flood_of_short_windows() {
        let limiter = Limiter::with_caps(8, 64);
        limiter
            .hit("login", "victim", 2, LOGIN_FAILURE_WINDOW)
            .unwrap();
        limiter
            .hit("login", "victim", 2, LOGIN_FAILURE_WINDOW)
            .unwrap();
        // Any longer window outlasts the flood, not only per-account ones.
        limiter.hit("up", "uploader", 1, HOUR).unwrap();
        for i in 0..640 {
            limiter.hit("api", &i.to_string(), 5, MINUTE).unwrap();
        }
        assert!(limiter.len() <= 64);
        assert!(limiter.blocked("login", "victim", 2).is_some());
        assert!(limiter.hit("up", "uploader", 1, HOUR).is_err());
    }

    #[test]
    fn per_account_counts_outlive_a_flood_of_hour_long_windows() {
        // A made-up session-shaped cookie per upload request mints one of
        // these each; they end later than a lockout that is well under way.
        let limiter = Limiter::with_caps(8, 64);
        for _ in 0..2 {
            limiter
                .hit("login", "victim", 2, LOGIN_FAILURE_WINDOW)
                .unwrap();
        }
        limiter.hit("mail", "victim", 1, HOUR).unwrap();
        for i in 0..640 {
            limiter.hit("up", &i.to_string(), 5, HOUR).unwrap();
        }
        assert!(limiter.len() <= 64);
        assert!(limiter.blocked("login", "victim", 2).is_some());
        assert!(limiter.hit("mail", "victim", 1, HOUR).is_err());
    }

    #[test]
    fn per_account_counts_alone_still_keep_the_cap() {
        let limiter = Limiter::with_caps(8, 64);
        for i in 0..640 {
            limiter
                .hit("login", &i.to_string(), 2, LOGIN_FAILURE_WINDOW)
                .unwrap();
            assert!(limiter.len() <= 64, "{} keys after {i}", limiter.len());
        }
    }

    #[test]
    fn expired_windows_are_swept_on_a_timer_not_on_every_hit() {
        let limiter = Limiter::with_caps(8, 64);
        let start = Instant::now();
        let window = Duration::from_secs(1);
        for i in 0..16 {
            limiter
                .hit_at("api", &i.to_string(), 5, window, start)
                .unwrap();
        }
        assert_eq!(limiter.len(), 16);

        // Every window is over, but the last sweep (at `start`) is recent.
        let later = start + Duration::from_secs(2);
        limiter.hit_at("api", "0", 5, window, later).unwrap();
        assert_eq!(limiter.len(), 16);

        limiter
            .hit_at("api", "1", 5, window, start + SWEEP_EVERY)
            .unwrap();
        assert_eq!(limiter.len(), 1);
    }

    #[test]
    fn at_the_cap_expired_windows_go_before_live_ones() {
        let limiter = Limiter::with_caps(8, 64);
        let start = Instant::now();
        limiter
            .hit_at("login", "victim", 2, LOGIN_FAILURE_WINDOW, start)
            .unwrap();
        for i in 0..63 {
            limiter
                .hit_at("api", &i.to_string(), 5, Duration::from_secs(1), start)
                .unwrap();
        }
        assert_eq!(limiter.len(), 64);

        // A known key pays nothing, not even at the cap.
        let later = start + Duration::from_secs(2);
        limiter
            .hit_at("api", "0", 5, Duration::from_secs(1), later)
            .unwrap();
        assert_eq!(limiter.len(), 64);

        // A new one first drops what has expired, and nothing live.
        limiter.hit_at("api", "new", 5, MINUTE, later).unwrap();
        assert_eq!(limiter.len(), 3);
        assert!(
            limiter
                .hit_at("login", "victim", 2, LOGIN_FAILURE_WINDOW, later)
                .is_ok()
        );
        assert!(
            limiter
                .hit_at("login", "victim", 2, LOGIN_FAILURE_WINDOW, later)
                .is_err()
        );
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
