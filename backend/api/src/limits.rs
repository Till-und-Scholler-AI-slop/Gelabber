//! Server-side rate limits and upload quotas (issue #16).
//!
//! Mutations under `/api` go through [`gate`] once CSRF let them through. Auth (sign-in,
//! registration, email verification, password reset) is keyed by client
//! address, IPv6 per /64; everything else by the account of the session,
//! which the gate resolves once for the handler as well, or by the address
//! when the cookie matches no live session.
//! Password guesses are also counted per account and guesser
//! ([`login_attempt`] / [`login_succeeded`]): a stranger's wrong guesses
//! use up their own address (IPv6 per /56), not the owner's way in
//! (GEL-44). A much higher ceiling per account keeps guesses spread over
//! many addresses from multiplying; addresses the account signed in from
//! are exempt from it.
//! Upload *bytes* are a durable daily quota in Postgres so
//! a restart cannot reset them. Request windows live in process — v1 is a
//! single node.
//!
//! Denied requests are `429` with a stable error code (`rate_limited` or
//! `quota_exceeded`) and `Retry-After`. The UI maps those codes; it does not
//! spin.

use std::borrow::Cow;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

use crate::auth::session;
use crate::cookies::{self, SESSION_COOKIE};
use crate::error::ApiError;
use crate::state::AppState;

pub const AUTH_PER_MIN: u32 = 20;
pub const API_PER_MIN: u32 = 180;
pub const MSG_PER_MIN: u32 = 60;
pub const UPLOAD_PER_HOUR: u32 = 60;
pub const UPLOAD_BYTES_PER_DAY: u64 = 1024 * 1024 * 1024; // 1 GiB
/// Failed password checks per account and guesser (client address, or all
/// signed-in sessions together) within [`LOGIN_FAILURE_WINDOW`].
pub const LOGIN_FAILURES_PER_ACCOUNT: u32 = 10;
/// Failed sign-ins per account within [`LOGIN_FAILURE_WINDOW`] from all
/// addresses together, except known ones (see [`KNOWN_ADDRESS_GRACE`]).
pub const LOGIN_FAILURES_ACCOUNT_CEILING: u32 = 100;
/// An address the account signed in from with its password skips the
/// ceiling for the session lifetime plus this long, counted from its latest
/// such sign-in: the owner signs in there again when that session ends.
pub const KNOWN_ADDRESS_GRACE: Duration = Duration::from_secs(30 * 24 * 3600);
/// The session lifetime [`KNOWN_ADDRESS_GRACE`] adds to when
/// `API_SESSION_TTL_HOURS` sets none (GEL-83): the 30 days that were the
/// default before, so an address stays known for 60 days by default.
pub const KNOWN_ADDRESS_LIFETIME: Duration = Duration::from_secs(30 * 24 * 3600);
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
/// Account mails per address, hour and [`MailBudget`]. Past it the request
/// is answered as usual and nothing is sent, so nobody can flood an inbox
/// through the instance.
pub const MAIL_PER_ADDRESS_PER_HOUR: u32 = 5;
/// Directory joins and requests to join per account and hour (handler
/// bucket `dir_join`).
pub const DIRECTORY_JOIN_PER_HOUR: u32 = 10;
/// Directory searches per account and minute (handler bucket `dir_search`,
/// also on GET): far above the debounced search box and its paging.
pub const DIRECTORY_SEARCH_PER_MIN: u32 = 60;
/// Feedback entries per account and hour (handler bucket `feedback`).
pub const FEEDBACK_PER_HOUR: u32 = 5;
/// A person's replies to their feedback per account and hour (handler
/// bucket `feedback_reply`).
pub const FEEDBACK_REPLIES_PER_HOUR: u32 = 30;
/// Feedback notices per address and hour (mail budget `mail_notice`): the
/// operators' mails about new entries and replies, and the sender's "there
/// is an answer". Above [`MAIL_PER_ADDRESS_PER_HOUR`], since an operator
/// hears about every entry of the instance.
pub const FEEDBACK_NOTICES_PER_HOUR: u32 = 30;
/// Posts of the public feedback form per client address and hour (handler
/// bucket `feedback_public`).
pub const FEEDBACK_PUBLIC_PER_HOUR: u32 = 5;
/// Visitor entries waiting for their confirmation; while this many are
/// pending, the public form takes no more.
pub const FEEDBACK_PENDING_MAX: i64 = 200;
/// Posts the public form accepts per hour over all visitors (each one a
/// stored entry and a confirmation mail), counted from `feedback`.
pub const FEEDBACK_FORM_PER_HOUR: i64 = 20;
/// Posts the public form accepts per 24 hours over all visitors, counted
/// the same way: at most this many confirmation mails a day.
pub const FEEDBACK_FORM_PER_DAY: i64 = 100;

/// Frames one WebSocket may send per minute, and how many at once (GEL-55).
/// Every frame costs a session and access check in the gateway pool. The
/// burst covers a reconnect, which subscribes to every text channel of the
/// open server and every DM at once.
pub const WS_FRAMES: FrameRate = FrameRate {
    per_min: 1200,
    burst: 300,
};
/// Typing frames per socket. Each one reaches every subscriber of the
/// channel, and each of them checks its access to receive it. The clients
/// send one every 3 s while someone types, and a stop.
pub const WS_TYPING: FrameRate = FrameRate {
    per_min: 30,
    burst: 10,
};
/// Voice signal frames (`sig`) per socket: join, leave, publish, mute,
/// deafen and watch, each sent on to the server's members.
pub const WS_SIG: FrameRate = FrameRate {
    per_min: 120,
    burst: 20,
};
/// Presence frames (`p`) per socket. One that flips the user between idle
/// and online reaches every member of every server the user is in. The web
/// client sends one when it goes idle or comes back, the desktop app at
/// most one every 10 s while there is input.
pub const WS_PRESENCE: FrameRate = FrameRate {
    per_min: 30,
    burst: 10,
};

pub(crate) const MINUTE: Duration = Duration::from_secs(60);
pub(crate) const HOUR: Duration = Duration::from_secs(3600);

/// Above this many keys the limiter drops expired windows, at most once per
/// [`SWEEP_EVERY`].
const SOFT_KEYS: usize = 4096;
/// Most keys the limiter holds: about 10 MB at roughly 150 B per entry,
/// `HashMap` overhead included.
const MAX_KEYS: usize = 65_536;
const SWEEP_EVERY: Duration = Duration::from_secs(10);
/// How long the gate keys a session cookie by the account its last lookup
/// found, without a new one. Only the bucket key relies on it; the handler
/// still gets a fresh lookup.
const SESSION_MEMO_TTL: Duration = MINUTE;
/// Most cookies the gate remembers; at this many it starts over.
const SESSION_MEMO_MAX: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub auth_per_min: u32,
    pub api_per_min: u32,
    pub msg_per_min: u32,
    pub upload_per_hour: u32,
    pub upload_bytes_per_day: u64,
    /// Failed password checks per account and guesser within
    /// [`LOGIN_FAILURE_WINDOW`]; `0` turns this count off.
    pub login_failures_per_account: u32,
    /// Failed sign-ins per account from all unknown addresses together
    /// within [`LOGIN_FAILURE_WINDOW`]; `0` turns the ceiling off.
    pub login_failures_ceiling: u32,
    /// GIF search/trending/send requests per user per minute (`0` = off).
    pub gif_per_min: u32,
    /// GIF media loads through the relay per user per minute (`0` = off).
    pub gif_media_per_min: u32,
    /// Messages per user per minute whose links get previews (`0` = off);
    /// a server allows [`PREVIEW_SERVER_FACTOR`] times as many.
    pub preview_per_min: u32,
    /// Inbound frame budget of one WebSocket ([`WS_FRAMES`]).
    pub ws_frames: FrameRate,
    /// Typing frames of one WebSocket ([`WS_TYPING`]).
    pub ws_typing: FrameRate,
    /// Voice signal frames of one WebSocket ([`WS_SIG`]).
    pub ws_sig: FrameRate,
    /// Presence frames of one WebSocket ([`WS_PRESENCE`]).
    pub ws_presence: FrameRate,
}

/// A token bucket's refill per minute and size. `per_min == 0` turns it off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate {
    pub per_min: u32,
    pub burst: u32,
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
            login_failures_ceiling: LOGIN_FAILURES_ACCOUNT_CEILING,
            gif_per_min: GIF_PER_MIN,
            gif_media_per_min: GIF_MEDIA_PER_MIN,
            preview_per_min: PREVIEW_PER_MIN,
            ws_frames: WS_FRAMES,
            ws_typing: WS_TYPING,
            ws_sig: WS_SIG,
            ws_presence: WS_PRESENCE,
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
    /// windows end soonest, the per-account counts (login failures, known
    /// addresses, account mail) last. Anyone holding an IPv6 prefix can mint address keys,
    /// hour-long upload windows among them, one per /64, and such a flood
    /// must not end a lockout early. Per-account counts only
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

/// The account a session cookie had at a lookup, and until when the gate
/// goes by it.
type SessionMemo = (Option<Uuid>, Instant);

/// Fixed-window counters, keyed by `bucket:id`. Cheap and enough for one
/// process. Past [`SOFT_KEYS`] expired windows are dropped every
/// [`SWEEP_EVERY`], not on every hit; at [`MAX_KEYS`] a new key first makes
/// room (see [`Inner::make_room`]), so a flood of keys can neither grow the
/// map without bound nor make every hit scan it.
pub struct Limiter {
    inner: Mutex<Inner>,
    soft: usize,
    hard: usize,
    /// Account of each recently looked-up session cookie, by the first half
    /// of its hash, `None` for a cookie with no live session; see
    /// [`Self::session_owner`].
    sessions: Mutex<HashMap<[u8; 16], SessionMemo>>,
    sessions_max: usize,
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
            sessions: Mutex::new(HashMap::new()),
            sessions_max: SESSION_MEMO_MAX.min(hard),
        }
    }

    /// The account the gate's last lookup of session cookie `raw` found
    /// (`Some(None)`: no live session), if that lookup is less than
    /// [`SESSION_MEMO_TTL`] old. A request the gate refuses then costs no
    /// query, also when it carries the same made-up cookie each time.
    fn session_owner(&self, raw: &str) -> Option<Option<Uuid>> {
        let now = Instant::now();
        let sessions = self.sessions.lock().unwrap_or_else(|err| err.into_inner());
        sessions
            .get(&memo_key(raw))
            .filter(|(_, until)| now < *until)
            .map(|(owner, _)| *owner)
    }

    fn remember_session(&self, raw: &str, owner: Option<Uuid>) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|err| err.into_inner());
        // Starting over costs each forgotten cookie one lookup, and keeps
        // a flood of made-up cookies from making every insert scan the map.
        if sessions.len() >= self.sessions_max {
            sessions.clear();
        }
        sessions.insert(memo_key(raw), (owner, Instant::now() + SESSION_MEMO_TTL));
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
        let mut inner = self.with_room_for(&key, now);
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

    /// The map, locked, with room for `key`.
    fn with_room_for(&self, key: &str, now: Instant) -> std::sync::MutexGuard<'_, Inner> {
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if inner.windows.len() > self.soft && now >= inner.next_sweep {
            inner.sweep(now);
        }
        // Only a new key pays for room; a known one never scans the map.
        if inner.windows.len() >= self.hard && !inner.windows.contains_key(key) {
            inner.make_room(now, self.hard);
        }
        inner
    }

    /// Sets a mark: a window of one hit that ends `window` from now, also
    /// when one was still running. [`Self::blocked`] with a max of 1 sees it.
    pub fn mark(&self, bucket: &str, id: &str, window: Duration) {
        self.mark_at(bucket, id, window, Instant::now());
    }

    fn mark_at(&self, bucket: &str, id: &str, window: Duration, now: Instant) {
        let key = format!("{bucket}:{id}");
        let mut inner = self.with_room_for(&key, now);
        inner.windows.insert(
            key,
            Window {
                count: 1,
                reset_at: now + window,
            },
        );
    }

    /// Like [`Self::hit`] once the window is full, without recording a hit.
    pub fn blocked(&self, bucket: &str, id: &str, max: u32) -> Option<u64> {
        self.blocked_at(bucket, id, max, Instant::now())
    }

    fn blocked_at(&self, bucket: &str, id: &str, max: u32, now: Instant) -> Option<u64> {
        if max == 0 {
            return None;
        }
        let key = format!("{bucket}:{id}");
        let inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let slot = inner.windows.get(&key).filter(|slot| now < slot.reset_at)?;
        (slot.count >= max).then(|| {
            slot.reset_at
                .saturating_duration_since(now)
                .as_secs()
                .max(1)
        })
    }

    /// Takes back one hit [`Self::hit`] recorded in a window that is still
    /// running.
    pub fn refund(&self, bucket: &str, id: &str) {
        let key = format!("{bucket}:{id}");
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = inner.windows.get_mut(&key)
            && now < slot.reset_at
        {
            slot.count = slot.count.saturating_sub(1);
        }
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

/// Middleware after the CSRF check: count mutating `/api` requests.
/// GET/HEAD/OPTIONS pass. Health, ready, metrics and `/ws` are not on this router.
///
/// Outside the sign-in routes the session cookie is resolved here, once:
/// its account is the key of the session buckets, so a user's sessions
/// share one allowance, and the handler's extractors take the result from
/// the request ([`session::Resolved`]) instead of querying again. The key
/// comes from a lookup at most [`SESSION_MEMO_TTL`] old, so a refused
/// request with a cookie seen recently costs no query; one let through
/// always gets a current lookup.
pub async fn gate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if is_safe(request.method()) {
        return Ok(next.run(request).await);
    }

    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    let ip = client_ip(request.headers(), peer);
    let path = request.uri().path();

    let outcome = if is_auth_path(path) {
        check(
            &state.limiter,
            "auth",
            &ip,
            state.limits.auth_per_min,
            MINUTE,
        )
    } else {
        let upload = is_upload(request.method(), path);
        let message = request.method() == Method::POST && path.ends_with("/messages");
        let raw = cookies::get(request.headers(), SESSION_COOKIE)
            .filter(|raw| crate::token::is_valid(raw));
        let mut resolved = None;
        let owner = match raw.as_deref() {
            None => None,
            Some(raw) => match state.limiter.session_owner(raw) {
                Some(owner) => owner,
                None => {
                    let session = lookup(&state, raw).await?;
                    let owner = session.as_ref().map(|session| session.user.id);
                    resolved = Some(session);
                    owner
                }
            },
        };
        let outcome = session_buckets(&state, &actor_key(owner, &ip), upload, message);
        if outcome.is_ok() {
            // Let through: the handler gets the session as it is now.
            let session = match (resolved, raw.as_deref()) {
                (Some(session), _) => session,
                (None, Some(raw)) => lookup(&state, raw).await?,
                (None, None) => None,
            };
            request.extensions_mut().insert(session::Resolved(session));
        }
        outcome
    };

    if let Err((bucket, retry_after)) = outcome {
        state.metrics.rate_limited(bucket);
        return Err(ApiError::RateLimited { retry_after });
    }

    Ok(next.run(request).await)
}

/// [`session::lookup`], remembered for the next request's bucket key.
async fn lookup(state: &AppState, raw: &str) -> Result<Option<session::CurrentSession>, ApiError> {
    let session = session::lookup(&state.db, raw).await?;
    state
        .limiter
        .remember_session(raw, session.as_ref().map(|session| session.user.id));
    Ok(session)
}

fn memo_key(raw: &str) -> [u8; 16] {
    let hash = crate::token::hash(raw);
    let mut key = [0; 16];
    key.copy_from_slice(&hash[..16]);
    key
}

/// `up` or `msg` where they apply, then `api`, all for `actor`.
fn session_buckets(
    state: &AppState,
    actor: &str,
    upload: bool,
    message: bool,
) -> Result<(), (&'static str, u64)> {
    if upload {
        check(
            &state.limiter,
            "up",
            actor,
            state.limits.upload_per_hour,
            HOUR,
        )?;
    } else if message {
        check(
            &state.limiter,
            "msg",
            actor,
            state.limits.msg_per_min,
            MINUTE,
        )?;
    }
    check(
        &state.limiter,
        "api",
        actor,
        state.limits.api_per_min,
        MINUTE,
    )
}

/// Requests that store a file and count in the `up` bucket.
fn is_upload(method: &Method, path: &str) -> bool {
    (*method == Method::POST && path.ends_with("/attachments"))
        || (*method == Method::PUT
            && (matches!(path, "/api/me/avatar" | "/api/me/banner")
                || (path.starts_with("/api/servers/") && path.ends_with("/icon"))))
        || (*method == Method::POST && path == "/api/feedback/images")
}

/// Routes an anonymous caller uses to sign in or to make the API send mail;
/// they share the per-IP auth window. Every mutation of sign-in through a
/// provider (`/api/auth/oidc/…`, #518) counts there too.
fn is_auth_path(path: &str) -> bool {
    matches!(
        path,
        "/api/auth/login"
            | "/api/auth/register"
            | "/api/auth/verify-email"
            | "/api/auth/verify-email/resend"
            | "/api/auth/password-reset"
            | "/api/auth/password-reset/confirm"
    ) || path.starts_with("/api/auth/oidc/")
}

/// Counts one anonymous request that [`gate`] does not see (a GET, such as
/// a provider's callback) against the per-IP `auth` window.
pub fn anonymous_hit(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<(), ApiError> {
    let ip = client_ip(headers, peer);
    check(
        &state.limiter,
        "auth",
        &ip,
        state.limits.auth_per_min,
        MINUTE,
    )
    .map_err(|(bucket, retry_after)| {
        state.metrics.rate_limited(bucket);
        ApiError::RateLimited { retry_after }
    })
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

/// Who checks a password, which decides the counts a wrong one lands in.
#[derive(Debug, Clone, Copy)]
pub enum Guesser<'a> {
    /// Anyone, from this client address ([`client_ip`]): sign-in and the
    /// confirmation link.
    Address(&'a str),
    /// A signed-in session of the account: password change, account
    /// deletion, linking a provider. All of them share one count, apart from
    /// sign-in: a session opened through a provider or a reset link brings no
    /// fresh allowance, and their guesses cannot keep the owner from signing
    /// in.
    Session,
}

/// A guess [`login_attempt`] counted. Hand it to [`login_succeeded`] when
/// the password was right.
#[must_use]
pub struct Guess {
    account: String,
    bucket: &'static str,
    /// The key of the guesser's own count in `bucket`; for an address also
    /// the key of its known mark.
    own: String,
    from_address: bool,
    /// Whether the guess also counted against the account's ceiling.
    ceiling: bool,
}

/// Counts one password guess against `email`, known account or not, before
/// the hash runs: guesses that arrive together cannot all pass a check made
/// before the first failure is recorded. Refuses one past the limit like
/// every other limit (`429 rate_limited`, `Retry-After`), at no Argon2 run.
///
/// Each guesser has its own count per account, so a stranger's wrong guesses
/// never use up the owner's. Guesses from an address also count against the
/// account's ceiling, unless the account signed in from that address
/// recently (see [`KNOWN_ADDRESS_GRACE`]): there its owner gets in even
/// while many addresses guess at once.
pub fn login_attempt(
    state: &AppState,
    email: &str,
    guesser: Guesser<'_>,
) -> Result<Guess, ApiError> {
    count_guess(&state.limiter, &state.limits, email, guesser).map_err(|retry_after| {
        state.metrics.rate_limited("login");
        ApiError::RateLimited { retry_after }
    })
}

fn count_guess(
    limiter: &Limiter,
    limits: &Limits,
    email: &str,
    guesser: Guesser<'_>,
) -> Result<Guess, u64> {
    let account = account_key(email);
    let (bucket, own, from_address) = match guesser {
        Guesser::Address(ip) => ("login", format!("{account}|{}", guesser_key(ip)), true),
        Guesser::Session => ("login_session", account.clone(), false),
    };
    limiter.hit(
        bucket,
        &own,
        limits.login_failures_per_account,
        LOGIN_FAILURE_WINDOW,
    )?;
    let ceiling = from_address && limiter.blocked("known", &own, 1).is_none();
    if ceiling
        && let Err(retry_after) = limiter.hit(
            "login_all",
            &account,
            limits.login_failures_ceiling,
            LOGIN_FAILURE_WINDOW,
        )
    {
        // Refused before any hash: no guess this address made.
        limiter.refund(bucket, &own);
        return Err(retry_after);
    }
    Ok(Guess {
        account,
        bucket,
        own,
        from_address,
        ceiling,
    })
}

/// The password [`login_attempt`] counted was right: it is no failure, and
/// an address it came from is known to the account from now on.
pub fn login_succeeded(state: &AppState, guess: Guess) {
    guess_was_right(
        &state.limiter,
        guess,
        state.session_ttl.unwrap_or(KNOWN_ADDRESS_LIFETIME) + KNOWN_ADDRESS_GRACE,
    );
}

fn guess_was_right(limiter: &Limiter, guess: Guess, known_for: Duration) {
    limiter.refund(guess.bucket, &guess.own);
    if guess.ceiling {
        limiter.refund("login_all", &guess.account);
    }
    if guess.from_address {
        // From the latest sign-in, so the mark outlives the newest session.
        limiter.mark("known", &guess.own, known_for);
    }
}

/// The guesser a [`client_ip`] key stands for in the login counts: IPv6 per
/// /56 rather than /64. A home connection is usually delegated a /56, so its
/// holder needs ten networks, not ten of its own /64s, to fill an account's
/// ceiling.
fn guesser_key(ip: &str) -> Cow<'_, str> {
    let Some(v6) = ip
        .strip_suffix("/64")
        .and_then(|prefix| prefix.parse::<Ipv6Addr>().ok())
    else {
        return Cow::Borrowed(ip);
    };
    let s = v6.segments();
    let prefix = Ipv6Addr::new(s[0], s[1], s[2], s[3] & 0xff00, 0, 0, 0, 0);
    Cow::Owned(format!("{prefix}/56"))
}

/// Which hourly allowance an account mail draws on. Anyone can register an
/// address or ask for its reset, so either budget can be used up by a
/// stranger; separate budgets keep registrations from silencing resets.
/// Reset mails, however triggered, only ever reach the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailBudget {
    /// Confirmation links and the "already registered" notice.
    Registration,
    /// Password reset links.
    Reset,
    /// Beta invitations an operator mails (`operator::invitations`), kept
    /// apart so they never use up the invitee's confirmation mails.
    Invitation,
    /// Confirmation links of the public feedback form (#542), kept apart
    /// so the form cannot use up a stranger's registration mails.
    Feedback,
    /// Feedback notices to operators and senders (#542).
    Notice,
}

impl MailBudget {
    fn bucket(self) -> &'static str {
        match self {
            Self::Registration => "mail",
            Self::Reset => "mail_reset",
            Self::Invitation => "mail_invite",
            Self::Feedback => "mail_feedback",
            Self::Notice => "mail_notice",
        }
    }

    /// Mails per address and hour this budget allows.
    pub fn per_hour(self) -> u32 {
        match self {
            Self::Registration | Self::Reset | Self::Invitation | Self::Feedback => {
                MAIL_PER_ADDRESS_PER_HOUR
            }
            Self::Notice => FEEDBACK_NOTICES_PER_HOUR,
        }
    }
}

/// Reserves one account mail to `email`; `false` once the hourly allowance
/// of that address is used up and the mail should be dropped. A reserved
/// mail that is not sent after all goes back with [`mail_unsent`].
pub fn mail_allowed(state: &AppState, budget: MailBudget, email: &str) -> bool {
    mail_reserved(state, budget, email).is_ok()
}

/// [`mail_allowed`] for a caller that may say why nothing was sent (an
/// operator): past the budget, the seconds until the next mail fits.
pub fn mail_reserved(state: &AppState, budget: MailBudget, email: &str) -> Result<(), u64> {
    let bucket = budget.bucket();
    let reserved = state
        .limiter
        .hit(bucket, &account_key(email), budget.per_hour(), HOUR);
    if reserved.is_err() {
        state.metrics.rate_limited(bucket);
    }
    reserved
}

/// Gives back a mail [`mail_allowed`] reserved: only sent mail counts.
pub fn mail_unsent(state: &AppState, budget: MailBudget, email: &str) {
    state.limiter.refund(budget.bucket(), &account_key(email));
}

/// The address is normalized by validation; hashed so the limiter's keys
/// hold no addresses.
fn account_key(email: &str) -> String {
    crate::auth::session::fingerprint(&crate::token::hash(email))
}

/// Limiter keys of the per-account counts above, which eviction spares.
fn is_per_account(key: &str) -> bool {
    key.starts_with("login:")
        || key.starts_with("login_session:")
        || key.starts_with("login_all:")
        || key.starts_with("known:")
        || key.starts_with("mail:")
        || key.starts_with("mail_reset:")
        || key.starts_with("mail_invite:")
        || key.starts_with("mail_feedback:")
        || key.starts_with("mail_notice:")
}

/// The client address the rate limits are keyed by, with IPv6 grouped per
/// /64: the rightmost `X-Forwarded-For` entry, which the Caddy next to the
/// API wrote, or the TCP peer without one (`gelabber_shared::client_addr`).
pub(crate) fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
    gelabber_shared::client_addr::client_key(
        headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok()),
        peer,
    )
}

/// Whom the session buckets (`api`, `msg`, `up`) count: the account of a
/// live session, otherwise the client address. A cookie that matches no
/// session, made up or expired, therefore shares the address's windows
/// instead of opening its own, and signing in again does not reset them.
fn actor_key(user: Option<Uuid>, ip: &str) -> String {
    match user {
        Some(id) => format!("u:{id}"),
        None => ip.to_owned(),
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

/// Charge `size` bytes to today's `consumed` at once, for uploads with no
/// attachment row to reserve against: avatars and banners. They are small
/// and replace the previous image, so this bounds write volume, not storage.
/// One autocommit statement that holds no other lock; never refunded, also
/// not when the PUT or the profile save fails afterwards.
pub async fn consume_upload_quota(
    state: &AppState,
    db: &mut sqlx::PgConnection,
    uploader_id: Uuid,
    size: i64,
) -> Result<(), ApiError> {
    let cap = state.limits.upload_bytes_per_day;
    let cap = if cap == 0 {
        i64::MAX
    } else {
        cap.min(i64::MAX as u64) as i64
    };
    let day: Option<chrono::NaiveDate> = sqlx::query_scalar(
        "INSERT INTO upload_daily_usage (uploader_id, day, consumed) \
         SELECT $1, (now() AT TIME ZONE 'UTC')::date, $2 WHERE $2 <= $3 \
         ON CONFLICT (uploader_id, day) DO UPDATE \
         SET consumed=upload_daily_usage.consumed + $2 \
         WHERE upload_daily_usage.reserved <= $3 - $2 \
           AND upload_daily_usage.consumed <= $3 - $2 - upload_daily_usage.reserved \
         RETURNING day",
    )
    .bind(uploader_id)
    .bind(size)
    .bind(cap)
    .fetch_optional(db)
    .await?;
    if day.is_none() {
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
    fn sign_in_through_a_provider_counts_as_auth() {
        for path in [
            "/api/auth/login",
            "/api/auth/verify-email/resend",
            "/api/auth/oidc/google/start",
            "/api/auth/oidc/pending",
            "/api/auth/oidc/complete",
            "/api/auth/oidc/desktop/exchange",
        ] {
            assert!(is_auth_path(path), "{path}");
        }
        for path in ["/api/auth/logout", "/api/auth/oidcx", "/api/servers"] {
            assert!(!is_auth_path(path), "{path}");
        }
    }

    #[test]
    fn uploads_count_in_the_up_bucket() {
        for (method, path) in [
            (Method::POST, "/api/channels/x/attachments"),
            (Method::PUT, "/api/me/avatar"),
            (Method::PUT, "/api/me/banner"),
            (Method::PUT, "/api/servers/x/icon"),
            (Method::POST, "/api/feedback/images"),
        ] {
            assert!(is_upload(&method, path), "{method} {path}");
        }
        for (method, path) in [
            (Method::POST, "/api/feedback"),
            (Method::PUT, "/api/feedback/images"),
            (Method::POST, "/api/feedback/x/messages"),
            (Method::POST, "/api/me/avatar"),
            (Method::PUT, "/api/servers/x/read"),
        ] {
            assert!(!is_upload(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn a_signed_in_caller_is_keyed_by_account() {
        let id = Uuid::new_v4();
        let key = actor_key(Some(id), "192.0.2.7");
        assert_eq!(key, format!("u:{id}"));
        // The same account from anywhere, and never an address.
        assert_eq!(actor_key(Some(id), "198.51.100.1"), key);
        assert_ne!(actor_key(Some(Uuid::new_v4()), "192.0.2.7"), key);
        assert_eq!(actor_key(None, "192.0.2.7"), "192.0.2.7");
    }

    #[test]
    fn the_gate_remembers_whose_cookie_it_looked_up() {
        let limiter = Limiter::with_caps(8, 4);
        let ada = crate::token::generate();
        let made_up = crate::token::generate();
        assert_eq!(limiter.session_owner(&ada), None);
        let id = Uuid::new_v4();
        limiter.remember_session(&ada, Some(id));
        limiter.remember_session(&made_up, None);
        assert_eq!(limiter.session_owner(&ada), Some(Some(id)));
        assert_eq!(limiter.session_owner(&made_up), Some(None));
        // At the cap it starts over instead of growing.
        for _ in 0..3 {
            limiter.remember_session(&crate::token::generate(), None);
        }
        assert_eq!(limiter.session_owner(&ada), None);
        assert!(limiter.sessions.lock().unwrap().len() <= 4);
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
        // One /64 per upload request mints one of these each; they end
        // later than a lockout that is well under way.
        let limiter = Limiter::with_caps(8, 64);
        for _ in 0..2 {
            limiter
                .hit("login", "victim", 2, LOGIN_FAILURE_WINDOW)
                .unwrap();
        }
        limiter.hit("mail", "victim", 1, HOUR).unwrap();
        limiter.hit("mail_reset", "victim", 1, HOUR).unwrap();
        for i in 0..640 {
            limiter.hit("up", &i.to_string(), 5, HOUR).unwrap();
        }
        assert!(limiter.len() <= 64);
        assert!(limiter.blocked("login", "victim", 2).is_some());
        assert!(limiter.hit("mail", "victim", 1, HOUR).is_err());
        assert!(limiter.hit("mail_reset", "victim", 1, HOUR).is_err());
    }

    #[test]
    fn feedback_mail_budgets_are_spared_by_eviction() {
        for bucket in [MailBudget::Feedback.bucket(), MailBudget::Notice.bucket()] {
            assert!(is_per_account(&format!("{bucket}:victim")), "{bucket}");
        }
        let limiter = Limiter::with_caps(8, 64);
        limiter.hit("mail_feedback", "victim", 1, HOUR).unwrap();
        limiter.hit("mail_notice", "victim", 1, HOUR).unwrap();
        for i in 0..640 {
            limiter.hit("up", &i.to_string(), 5, HOUR).unwrap();
        }
        assert!(limiter.hit("mail_feedback", "victim", 1, HOUR).is_err());
        assert!(limiter.hit("mail_notice", "victim", 1, HOUR).is_err());
    }

    /// A flood of address keys must neither lift the account's ceiling nor
    /// make the owner's address unknown again.
    #[test]
    fn the_login_ceiling_and_known_addresses_are_spared_by_eviction() {
        let limiter = Limiter::with_caps(8, 64);
        limiter
            .hit("login_all", "victim", 1, LOGIN_FAILURE_WINDOW)
            .unwrap();
        limiter.mark("known", "victim|home", KNOWN_ADDRESS_GRACE);
        for i in 0..640 {
            limiter.hit("up", &i.to_string(), 5, HOUR).unwrap();
        }
        assert!(limiter.blocked("login_all", "victim", 1).is_some());
        assert!(limiter.blocked("known", "victim|home", 1).is_some());
    }

    fn login_limits(per_account: u32, ceiling: u32) -> Limits {
        Limits {
            login_failures_per_account: per_account,
            login_failures_ceiling: ceiling,
            ..Limits::default()
        }
    }

    /// A guess the ceiling refuses is no guess of that address: once the
    /// ceiling is out of the way it still has its whole allowance.
    #[test]
    fn a_guess_refused_by_the_ceiling_leaves_the_address_count_alone() {
        let limiter = Limiter::new();
        let full = login_limits(2, 1);
        let stranger = Guesser::Address("203.0.113.1");
        let other = Guesser::Address("198.51.100.2");
        let _ = count_guess(&limiter, &full, "ada@example.com", stranger).unwrap();
        for _ in 0..3 {
            assert!(count_guess(&limiter, &full, "ada@example.com", other).is_err());
        }
        let no_ceiling = login_limits(2, 0);
        for _ in 0..2 {
            let _ = count_guess(&limiter, &no_ceiling, "ada@example.com", other).unwrap();
        }
        assert!(count_guess(&limiter, &no_ceiling, "ada@example.com", other).is_err());
    }

    /// An address that used up its wrong passwords gets in again after 15
    /// minutes, not earlier and not never (`deploy/README.md`, "Sign-in
    /// protection").
    #[test]
    fn a_guesser_out_of_guesses_is_refused_for_fifteen_minutes() {
        let limiter = Limiter::new();
        let limits = login_limits(2, 0);
        let max = limits.login_failures_per_account;
        let guesser = Guesser::Address("203.0.113.9");
        let before = Instant::now();
        let guess = count_guess(&limiter, &limits, "ada@example.com", guesser).unwrap();
        let _ = count_guess(&limiter, &limits, "ada@example.com", guesser).unwrap();
        let after = Instant::now();
        let refused = |at| {
            limiter
                .blocked_at(guess.bucket, &guess.own, max, at)
                .is_some()
        };
        assert!(
            refused(before + 15 * MINUTE - Duration::from_secs(1)),
            "free again before 15 minutes"
        );
        assert!(
            !refused(after + 15 * MINUTE),
            "still refused after 15 minutes"
        );
    }

    /// One delegated /56 is one guesser; another /56 is not.
    #[test]
    fn login_guessers_group_ipv6_per_56() {
        assert_eq!(guesser_key("2001:db8:0:1::/64"), "2001:db8::/56");
        assert_eq!(guesser_key("2001:db8:0:ff::/64"), "2001:db8::/56");
        assert_eq!(guesser_key("2001:db8:0:100::/64"), "2001:db8:0:100::/56");
        assert_eq!(guesser_key("203.0.113.7"), "203.0.113.7");
        assert_eq!(guesser_key("unknown"), "unknown");
        let limiter = Limiter::new();
        let limits = login_limits(2, 0);
        for i in 1..=2 {
            let ip = format!("2001:db8:0:{i}::/64");
            let _ =
                count_guess(&limiter, &limits, "ada@example.com", Guesser::Address(&ip)).unwrap();
        }
        let same = Guesser::Address("2001:db8:0:3::/64");
        assert!(count_guess(&limiter, &limits, "ada@example.com", same).is_err());
        let next = Guesser::Address("2001:db8:0:100::/64");
        assert!(count_guess(&limiter, &limits, "ada@example.com", next).is_ok());
    }

    /// Every session of an account draws on one count, which sign-in does
    /// not share.
    #[test]
    fn signed_in_guesses_share_one_count_apart_from_sign_in() {
        let limiter = Limiter::new();
        let limits = login_limits(2, 1);
        for _ in 0..2 {
            let _ = count_guess(&limiter, &limits, "ada@example.com", Guesser::Session).unwrap();
        }
        assert!(count_guess(&limiter, &limits, "ada@example.com", Guesser::Session).is_err());
        let home = Guesser::Address("192.0.2.10");
        assert!(count_guess(&limiter, &limits, "ada@example.com", home).is_ok());
    }

    /// A right password from an address makes it known for `known_for`
    /// from that sign-in, also when a mark from an earlier one still runs.
    #[test]
    fn each_sign_in_renews_the_known_mark() {
        let limiter = Limiter::new();
        let limits = login_limits(2, 1);
        let home = Guesser::Address("192.0.2.10");
        let guess = count_guess(&limiter, &limits, "ada@example.com", home).unwrap();
        let own = guess.own.clone();
        // An earlier sign-in's mark with one minute left.
        limiter.mark("known", &own, MINUTE);
        guess_was_right(&limiter, guess, HOUR);
        let after_old_mark = Instant::now() + 2 * MINUTE;
        assert!(
            limiter
                .blocked_at("known", &own, 1, after_old_mark)
                .is_some()
        );
        let guess = count_guess(&limiter, &limits, "ada@example.com", home).unwrap();
        assert!(!guess.ceiling, "a known address skips the ceiling");
        let after_new_mark = Instant::now() + HOUR + MINUTE;
        assert!(
            limiter
                .blocked_at("known", &own, 1, after_new_mark)
                .is_none()
        );
    }

    #[test]
    fn mail_budgets_allow_their_hourly_count() {
        for budget in [
            MailBudget::Registration,
            MailBudget::Reset,
            MailBudget::Invitation,
            MailBudget::Feedback,
        ] {
            assert_eq!(budget.per_hour(), MAIL_PER_ADDRESS_PER_HOUR, "{budget:?}");
        }
        assert_eq!(MailBudget::Notice.per_hour(), 30);
        assert_eq!(MailBudget::Feedback.bucket(), "mail_feedback");
        assert_eq!(MailBudget::Notice.bucket(), "mail_notice");
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
    fn refund_takes_back_one_hit() {
        let limiter = Limiter::new();
        limiter.hit("login", "a", 2, MINUTE).unwrap();
        limiter.hit("login", "a", 2, MINUTE).unwrap();
        limiter.refund("login", "a");
        assert!(limiter.hit("login", "a", 2, MINUTE).is_ok());
        assert!(limiter.hit("login", "a", 2, MINUTE).is_err());
        // Nothing to take back from an unknown key or below zero.
        limiter.refund("login", "b");
        limiter.refund("login", "b");
        assert!(limiter.hit("login", "b", 1, MINUTE).is_ok());
        assert!(limiter.hit("login", "b", 1, MINUTE).is_err());
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
