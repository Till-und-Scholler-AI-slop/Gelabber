//! GIF search and sending (v0.7). Klipy is the provider, and only this
//! module talks to it: the browser asks `/api/gifs/*`, loads GIF bytes from
//! `/api/gifs/media`, and never sees the API key. With `KLIPY_API_KEY`
//! unset every route here answers `404` (the status route says
//! `enabled: false`) and no request leaves the server.
//!
//! A sent GIF is an ordinary chat message (`messages.gif`, resolved here by
//! slug at send time); nothing touches voice signaling or the SFU.
//!
//! Klipy's quota belongs to the key, so every user of the instance spends
//! it. Search and trending pages are therefore kept in Redis for
//! `KLIPY_CACHE_SECS` and shared by all users, and once Klipy answers 429
//! the API stops asking it until the cooldown is over.

pub mod klipy;

use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::header::{
    ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE,
    IF_NONE_MATCH, LAST_MODIFIED, RETRY_AFTER, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::config::GifConfig;
use crate::error::{ApiError, FieldErrors};
use crate::limits::MINUTE;
use crate::metrics::HttpMetrics;
use crate::state::AppState;

use self::klipy::Size;

pub const PROVIDER: &str = "klipy";
pub const QUERY_MAX: usize = 100;
const PAGE_MAX: u32 = 50;
const PER_PAGE: u32 = 24;
const API_TIMEOUT: Duration = Duration::from_secs(8);
const MEDIA_TIMEOUT: Duration = Duration::from_secs(30);
/// Klipy's largest GIF renditions average about 4 MB; a message uses WebP.
const MEDIA_MAX_BYTES: u64 = 16 * 1024 * 1024;
const MEDIA_TYPES: &[&str] = &["image/webp", "image/gif", "image/jpeg"];
/// Klipy's search and item answers are tens of KiB; anything near this cap
/// is a broken or hostile upstream, not a page of GIFs.
const JSON_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Media streams open at once across all users. Each holds an upstream
/// connection for up to [`MEDIA_TIMEOUT`]; past this the relay answers 502
/// instead of opening more.
const MEDIA_STREAMS_MAX: usize = 128;
/// Share triggers in flight. They are best effort: past this one is dropped.
const SHARES_MAX: usize = 8;
const FORMATS: &str = "webp,gif,jpg";
/// How long the API leaves Klipy alone after a 429 that names no
/// `Retry-After`, and the longest it accepts from one.
const QUOTA_COOLDOWN: Duration = Duration::from_secs(60);
const QUOTA_COOLDOWN_MAX: Duration = Duration::from_secs(3600);
/// The feed cache is a shortcut: a slow or missing Redis falls through to
/// Klipy instead of holding the picker.
const CACHE_TIMEOUT: Duration = Duration::from_millis(500);
const CACHE_METRIC: &str = "gif_feed";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/gifs", get(status))
        .route("/api/gifs/trending", get(trending))
        .route("/api/gifs/search", get(search))
        .route("/api/gifs/media", get(media))
}

/// One GIF as the client renders it. Search results carry the picker
/// rendition; a message carries the full one. URLs are Klipy's own, as
/// returned; the client loads them through `/api/gifs/media`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gif {
    pub provider: String,
    pub slug: String,
    #[serde(default)]
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Animated WebP (GIF where Klipy has no WebP).
    pub url: String,
    /// First frame as JPEG, shown instead of the animation for reduced motion.
    #[serde(default)]
    pub still: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GifPage {
    pub gifs: Vec<Gif>,
    pub next_page: Option<u32>,
}

/// What the composer sends to post a GIF: the slug of a picker result and
/// the search that found it (empty for trending), for Klipy's share count.
#[derive(Debug, Clone, Deserialize)]
pub struct GifPick {
    pub slug: String,
    #[serde(default)]
    pub query: String,
}

#[derive(Clone)]
pub struct Gifs {
    inner: Arc<Inner>,
}

struct Inner {
    config: GifConfig,
    http: reqwest::Client,
    metrics: Arc<HttpMetrics>,
    media_streams: Arc<Semaphore>,
    shares: Arc<Semaphore>,
    /// Set by Klipy's 429: until then no API call leaves the server.
    quota_until: Mutex<Option<Instant>>,
}

enum Feed<'a> {
    Trending,
    Search(&'a str),
}

impl Gifs {
    pub fn new(config: &GifConfig, metrics: Arc<HttpMetrics>) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // The media allowlist is checked once; a redirect would leave it.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("gelabber/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                config: config.clone(),
                http,
                metrics,
                media_streams: Arc::new(Semaphore::new(MEDIA_STREAMS_MAX)),
                shares: Arc::new(Semaphore::new(SHARES_MAX)),
                quota_until: Mutex::new(None),
            }),
        })
    }

    /// Klipy's per-user id for personalised trending and recents: stable per
    /// user and key, but not the account id.
    fn customer_id(&self, user_id: Uuid) -> String {
        let mut hash = Sha256::new();
        hash.update(b"gelabber-gif\0");
        hash.update(self.inner.config.api_key.as_bytes());
        hash.update(b"\0");
        hash.update(user_id.as_bytes());
        hex(&hash.finalize()[..16])
    }

    /// The Redis key of one search or trending page, `None` with the cache
    /// off. Not per user: every user of the instance spends the same quota,
    /// so Klipy's personalisation of trending is given up on a hit. The
    /// key, base and filter are part of it, so a config change starts over.
    fn cache_key(&self, feed: &Feed<'_>, page: u32, locale: Option<&str>) -> Option<String> {
        let config = &self.inner.config;
        if config.cache_ttl.is_zero() {
            return None;
        }
        let (kind, q) = match feed {
            Feed::Trending => ("trending", ""),
            Feed::Search(q) => ("search", *q),
        };
        let mut hash = Sha256::new();
        for part in [
            config.api_base.as_str(),
            config.api_key.as_str(),
            config.content_filter.as_str(),
            kind,
            q,
            &page.to_string(),
            locale.unwrap_or_default(),
        ] {
            hash.update(part.as_bytes());
            hash.update(b"\0");
        }
        Some(format!("gifs:feed:v1:{}", hex(&hash.finalize())))
    }

    /// While Klipy's last 429 holds, answers at once instead of asking again.
    fn quota_check(&self) -> Result<(), ApiError> {
        let mut until = self
            .inner
            .quota_until
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        match *until {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    *until = None;
                    return Ok(());
                }
                Err(ApiError::UpstreamQuota {
                    retry_after: left.as_secs_f64().ceil() as u64,
                })
            }
            None => Ok(()),
        }
    }

    fn quota_exhausted(&self, retry_after: Option<&reqwest::header::HeaderValue>) -> ApiError {
        let cooldown = retry_after
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(QUOTA_COOLDOWN)
            .clamp(Duration::from_secs(1), QUOTA_COOLDOWN_MAX);
        *self
            .inner
            .quota_until
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(Instant::now() + cooldown);
        // The key's quota is shared by every user of the instance, so this
        // is not the caller's own limit: an upstream failure the operator
        // has to see (log line and `rate_limited{bucket="klipy"}`). One per
        // cooldown, since no call goes out until it ends.
        self.inner.metrics.rate_limited("klipy");
        warn!(
            cooldown_secs = cooldown.as_secs(),
            "klipy returned 429: API key quota exhausted, pausing GIF requests"
        );
        ApiError::UpstreamQuota {
            retry_after: cooldown.as_secs(),
        }
    }

    /// `{base}/api/v1/{key}/gifs/{path}`. The key is a path segment, so no
    /// URL built here may reach a log line or an error body.
    fn endpoint(&self, path: &str) -> Result<Url, ApiError> {
        let config = &self.inner.config;
        Url::parse(&format!(
            "{}/api/v1/{}/gifs/{path}",
            config.api_base.trim_end_matches('/'),
            config.api_key
        ))
        .map_err(|_| ApiError::Internal("gif endpoint url".to_owned()))
    }

    /// One Klipy API call, timed into
    /// `gelabber_upstream_request_duration_seconds{upstream="klipy"}`.
    async fn get_json(&self, operation: &'static str, url: Url) -> Result<Value, ApiError> {
        self.quota_check()?;
        let started = Instant::now();
        let result = self.fetch_json(url).await;
        self.inner
            .metrics
            .upstream(PROVIDER, operation, started.elapsed());
        result
    }

    async fn fetch_json(&self, url: Url) -> Result<Value, ApiError> {
        let response = self
            .inner
            .http
            .get(url)
            .timeout(API_TIMEOUT)
            .send()
            .await
            .map_err(|err| ApiError::Upstream(format!("klipy: {}", err.without_url())))?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(self.quota_exhausted(response.headers().get(RETRY_AFTER)));
        }
        if !status.is_success() {
            return Err(ApiError::Upstream(format!("klipy returned {status}")));
        }
        let bytes = read_capped(response, JSON_MAX_BYTES).await?;
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|err| ApiError::Upstream(format!("klipy json: {err}")))?;
        if !klipy::ok(&body) {
            return Err(ApiError::Upstream("klipy result false".to_owned()));
        }
        Ok(body)
    }

    async fn feed(
        &self,
        feed: Feed<'_>,
        user_id: Uuid,
        page: u32,
        locale: Option<String>,
    ) -> Result<GifPage, ApiError> {
        let operation = match feed {
            Feed::Trending => "trending",
            Feed::Search(_) => "search",
        };
        let mut url = self.endpoint(operation)?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("page", &page.to_string())
                .append_pair("per_page", &PER_PAGE.to_string())
                .append_pair("customer_id", &self.customer_id(user_id))
                .append_pair("content_filter", &self.inner.config.content_filter)
                .append_pair("format_filter", FORMATS);
            if let Feed::Search(q) = feed {
                query.append_pair("q", q);
            }
            if let Some(locale) = locale {
                query.append_pair("locale", &locale);
            }
        }
        let body = self.get_json(operation, url).await?;
        Ok(klipy::page(
            &body,
            page,
            Size::Preview,
            &self.inner.config.media_origins,
        ))
    }

    /// The message rendition of one slug, `None` if Klipy does not know it.
    pub async fn resolve(&self, slug: &str) -> Result<Option<Gif>, ApiError> {
        let mut url = self.endpoint("items")?;
        url.query_pairs_mut()
            .append_pair("slugs", slug)
            .append_pair("format_filter", FORMATS);
        let body = self.get_json("items", url).await?;
        Ok(
            klipy::items(&body, Size::Message, &self.inner.config.media_origins)
                .into_iter()
                .find(|gif| gif.slug == slug),
        )
    }

    /// Klipy's share trigger after a GIF was posted. Best effort, off the
    /// request path: the message is already stored. At most [`SHARES_MAX`]
    /// run at once; with Klipy hanging, further ones are dropped rather than
    /// piling up. Shutdown does not wait for them.
    pub fn shared(&self, slug: &str, query: &str, user_id: Uuid) {
        let Ok(permit) = self.inner.shares.clone().try_acquire_owned() else {
            debug!("gif share trigger dropped: too many in flight");
            return;
        };
        let Ok(url) = self.endpoint(&format!("share/{slug}")) else {
            return;
        };
        let body = serde_json::json!({
            "customer_id": self.customer_id(user_id),
            "q": query,
        })
        .to_string();
        let http = self.inner.http.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = http
                .post(url)
                .timeout(API_TIMEOUT)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await;
            match result {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => warn!(status = %response.status(), "gif share trigger refused"),
                Err(err) => warn!(error = %err.without_url(), "gif share trigger failed"),
            }
        });
    }

    fn media_allowed(&self, url: &str) -> bool {
        klipy::media_allowed(url, &self.inner.config.media_origins)
    }

    fn media_slot(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        self.inner
            .media_streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                self.inner.metrics.rate_limited("gif_media_streams");
                ApiError::Upstream("gif media: all relay streams busy".to_owned())
            })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A search or trending page from the shared cache, else from Klipy (and
/// then cached). Redis trouble only costs the shortcut.
async fn cached_feed(
    state: &AppState,
    gifs: &Gifs,
    feed: Feed<'_>,
    user_id: Uuid,
    page: u32,
    locale: Option<String>,
) -> Result<GifPage, ApiError> {
    let key = gifs.cache_key(&feed, page, locale.as_deref());
    if let Some(key) = &key {
        let hit = cache_get(state, key).await;
        gifs.inner.metrics.cache(CACHE_METRIC, hit.is_some());
        if let Some(hit) = hit {
            return Ok(hit);
        }
    }
    let result = gifs.feed(feed, user_id, page, locale).await?;
    if let Some(key) = key {
        cache_put(state, key, &result, gifs.inner.config.cache_ttl).await;
    }
    Ok(result)
}

async fn cache_get(state: &AppState, key: &str) -> Option<GifPage> {
    let key = key.to_owned();
    let lookup = state.gateway.with_redis(move |mut conn| {
        let key = key.clone();
        async move {
            redis::cmd("GET")
                .arg(key)
                .query_async::<Option<String>>(&mut conn)
                .await
        }
    });
    match tokio::time::timeout(CACHE_TIMEOUT, lookup).await {
        Ok(Ok(raw)) => raw.and_then(|raw| serde_json::from_str(&raw).ok()),
        Ok(Err(err)) => {
            debug!(error = %err, "gif cache read failed");
            None
        }
        Err(_) => {
            debug!("gif cache read timed out");
            None
        }
    }
}

async fn cache_put(state: &AppState, key: String, page: &GifPage, ttl: Duration) {
    let Ok(raw) = serde_json::to_string(page) else {
        return;
    };
    let ttl_ms = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX);
    let store = state.gateway.with_redis(move |mut conn| {
        let (key, raw) = (key.clone(), raw.clone());
        async move {
            redis::cmd("SET")
                .arg(key)
                .arg(raw)
                .arg("PX")
                .arg(ttl_ms)
                .query_async::<()>(&mut conn)
                .await
        }
    });
    match tokio::time::timeout(CACHE_TIMEOUT, store).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => debug!(error = %err, "gif cache write failed"),
        Err(_) => debug!("gif cache write timed out"),
    }
}

/// Klipy's search ignores case and extra spaces; one spelling per search
/// lets the cache serve "Cat " and "cat" alike.
fn normalize_query(q: &str) -> String {
    q.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn enabled(state: &AppState) -> Result<&Gifs, ApiError> {
    state.gifs.as_ref().ok_or(ApiError::NotFound)
}

/// One Klipy API request (search, trending, or resolving a sent GIF) for
/// this user.
pub(crate) fn rate_limit(state: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    hit(state, "gif", user_id, state.limits.gif_per_min)
}

/// One image load through `/api/gifs/media` for this user.
fn rate_limit_media(state: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    hit(state, "gif_media", user_id, state.limits.gif_media_per_min)
}

fn hit(state: &AppState, bucket: &'static str, user_id: Uuid, max: u32) -> Result<(), ApiError> {
    state
        .limiter
        .hit(bucket, &user_id.to_string(), max, MINUTE)
        .map_err(|retry_after| {
            state.metrics.rate_limited(bucket);
            ApiError::RateLimited { retry_after }
        })
}

/// The whole body, or an upstream error once it passes `max` bytes (by its
/// `Content-Length` or while reading).
async fn read_capped(mut response: reqwest::Response, max: u64) -> Result<Vec<u8>, ApiError> {
    let too_large = || ApiError::Upstream("klipy body too large".to_owned());
    if response.content_length().is_some_and(|length| length > max) {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| ApiError::Upstream(format!("klipy body: {}", err.without_url())))?
    {
        if (bytes.len() + chunk.len()) as u64 > max {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn page(raw: Option<u32>) -> Result<u32, ApiError> {
    match raw {
        None => Ok(1),
        Some(n) if (1..=PAGE_MAX).contains(&n) => Ok(n),
        Some(_) => Err(ApiError::Validation(FieldErrors::from([(
            "page", "invalid",
        )]))),
    }
}

/// Klipy's `locale` is a two-letter country code. The browser's first
/// language tag gives it: its region (`de-AT` → `at`), else the language.
fn locale(headers: &HeaderMap) -> Option<String> {
    let first = headers
        .get(ACCEPT_LANGUAGE)?
        .to_str()
        .ok()?
        .split(',')
        .next()?
        .split(';')
        .next()?
        .trim();
    let mut parts = first.split(['-', '_']);
    let language = parts.next()?;
    let region = parts.find(|part| part.len() == 2);
    let code = region.unwrap_or(language);
    (code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()))
        .then(|| code.to_ascii_lowercase())
}

#[derive(Serialize)]
struct Status {
    enabled: bool,
    provider: Option<&'static str>,
}

async fn status(State(state): State<AppState>, CurrentUser(_user): CurrentUser) -> Json<Status> {
    let enabled = state.gifs.is_some();
    Json(Status {
        enabled,
        provider: enabled.then_some(PROVIDER),
    })
}

#[derive(Debug, Default, Deserialize)]
struct FeedParams {
    q: Option<String>,
    page: Option<u32>,
}

async fn trending(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Query(params): Query<FeedParams>,
) -> Result<Json<GifPage>, ApiError> {
    let gifs = enabled(&state)?;
    let page = page(params.page)?;
    rate_limit(&state, user.id)?;
    Ok(Json(
        cached_feed(
            &state,
            gifs,
            Feed::Trending,
            user.id,
            page,
            locale(&headers),
        )
        .await?,
    ))
}

async fn search(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Query(params): Query<FeedParams>,
) -> Result<Json<GifPage>, ApiError> {
    let gifs = enabled(&state)?;
    let q = params.q.as_deref().unwrap_or_default().trim();
    let mut fields = FieldErrors::new();
    if q.is_empty() {
        fields.insert("q", "required");
    } else if q.chars().count() > QUERY_MAX {
        fields.insert("q", "too_long");
    } else if q.chars().any(char::is_control) {
        fields.insert("q", "invalid");
    }
    if !fields.is_empty() {
        return Err(ApiError::Validation(fields));
    }
    let page = page(params.page)?;
    rate_limit(&state, user.id)?;
    let q = normalize_query(q);
    Ok(Json(
        cached_feed(
            &state,
            gifs,
            Feed::Search(&q),
            user.id,
            page,
            locale(&headers),
        )
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct MediaParams {
    url: String,
}

/// Streams one GIF rendition from Klipy's media hosts. Signed-in users
/// only, allowlisted origins only, no redirects, bounded size; nothing is
/// stored here. The browser keeps it in its private cache. Loads are
/// counted per user (`gif_media`, GETs pass the global gate) and at most
/// [`MEDIA_STREAMS_MAX`] run at once, so the relay cannot be turned into a
/// bandwidth amplifier against Klipy or our own egress.
async fn media(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Query(params): Query<MediaParams>,
) -> Result<Response, ApiError> {
    let gifs = enabled(&state)?;
    if !gifs.media_allowed(&params.url) {
        return Err(ApiError::NotFound);
    }
    rate_limit_media(&state, user.id)?;
    let permit = gifs.media_slot()?;
    let mut request = gifs
        .inner
        .http
        .get(params.url.as_str())
        .timeout(MEDIA_TIMEOUT);
    for name in [IF_NONE_MATCH, IF_MODIFIED_SINCE] {
        if let Some(value) = headers.get(&name) {
            request = request.header(name.as_str(), value.as_bytes());
        }
    }
    let upstream = request
        .send()
        .await
        .map_err(|err| ApiError::Upstream(format!("gif media: {}", err.without_url())))?;
    let status = upstream.status();
    let mut response = Response::builder();
    let forward =
        |builder: axum::http::response::Builder, name: axum::http::HeaderName| match upstream
            .headers()
            .get(name.as_str())
            .and_then(|value| HeaderValue::from_bytes(value.as_bytes()).ok())
        {
            Some(value) => builder.header(name, value),
            None => builder,
        };
    response = forward(response, ETAG);
    response = forward(response, LAST_MODIFIED);
    response = response
        .header(CACHE_CONTROL, "private, max-age=3600")
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header("cross-origin-resource-policy", "same-origin");
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return response
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .map_err(|err| ApiError::Internal(err.to_string()));
    }
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
        return Err(ApiError::NotFound);
    }
    if !status.is_success() {
        return Err(ApiError::Upstream(format!("gif media returned {status}")));
    }
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|value| MEDIA_TYPES.contains(&value.as_str()))
        .ok_or_else(|| ApiError::Upstream("gif media type".to_owned()))?;
    let length = upstream.content_length();
    if length.is_some_and(|length| length > MEDIA_MAX_BYTES) {
        return Err(ApiError::Upstream("gif media too large".to_owned()));
    }
    response = response.header(CONTENT_TYPE, content_type);
    if let Some(length) = length {
        response = response.header(CONTENT_LENGTH, length);
    }
    // The slot stays taken until the stream ends or the client goes away.
    let stream = futures_util::stream::try_unfold(
        (upstream, 0u64, permit),
        |(mut upstream, seen, permit)| async move {
            match upstream.chunk().await {
                Ok(Some(chunk)) => {
                    let seen = seen + chunk.len() as u64;
                    if seen > MEDIA_MAX_BYTES {
                        return Err(io::Error::other("gif media too large"));
                    }
                    Ok(Some((chunk, (upstream, seen, permit))))
                }
                Ok(None) => Ok(None),
                Err(err) => Err(io::Error::other(err.without_url().to_string())),
            }
        },
    );
    response
        .status(StatusCode::OK)
        .body(Body::from_stream(stream))
        .map_err(|err| ApiError::Internal(err.to_string()))
        .map(IntoResponse::into_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn locale_prefers_the_region_of_the_first_language() {
        assert_eq!(locale(&headers("de-AT,de;q=0.9")).as_deref(), Some("at"));
        assert_eq!(locale(&headers("de")).as_deref(), Some("de"));
        assert_eq!(locale(&headers("zh-Hant-TW")).as_deref(), Some("tw"));
        assert_eq!(locale(&headers("*")), None);
        assert_eq!(locale(&HeaderMap::new()), None);
    }

    #[test]
    fn customer_id_is_stable_per_user_and_hides_the_account_id() {
        let gifs = Gifs::new(
            &GifConfig {
                api_key: "key".into(),
                api_base: "https://api.klipy.com".into(),
                media_origins: vec![],
                content_filter: "medium".into(),
                cache_ttl: Duration::from_secs(600),
            },
            Arc::new(HttpMetrics::new()),
        )
        .unwrap();
        let user = Uuid::from_u128(7);
        assert_eq!(gifs.customer_id(user), gifs.customer_id(user));
        assert_ne!(gifs.customer_id(user), gifs.customer_id(Uuid::from_u128(8)));
        assert_eq!(gifs.customer_id(user).len(), 32);
        assert!(!gifs.customer_id(user).contains(&user.simple().to_string()));
    }

    fn relay(cache_secs: u64) -> Gifs {
        Gifs::new(
            &GifConfig {
                api_key: "key".into(),
                api_base: "https://api.klipy.com".into(),
                media_origins: vec![],
                content_filter: "medium".into(),
                cache_ttl: Duration::from_secs(cache_secs),
            },
            Arc::new(HttpMetrics::new()),
        )
        .unwrap()
    }

    #[test]
    fn cache_key_separates_what_klipy_answers_differently() {
        let gifs = relay(600);
        let key = |feed: Feed<'_>, page: u32, locale: Option<&str>| {
            gifs.cache_key(&feed, page, locale).unwrap()
        };
        let base = key(Feed::Search("cat"), 1, Some("de"));
        assert!(base.starts_with("gifs:feed:v1:"));
        assert!(!base.contains("cat") && !base.contains("key"));
        assert_eq!(base, key(Feed::Search("cat"), 1, Some("de")));
        for other in [
            key(Feed::Search("cats"), 1, Some("de")),
            key(Feed::Search("cat"), 2, Some("de")),
            key(Feed::Search("cat"), 1, Some("at")),
            key(Feed::Search("cat"), 1, None),
            key(Feed::Trending, 1, Some("de")),
        ] {
            assert_ne!(base, other);
        }
        assert_eq!(relay(0).cache_key(&Feed::Trending, 1, None), None);
    }

    #[test]
    fn queries_are_folded_for_the_cache() {
        assert_eq!(normalize_query("  Hallo   Welt "), "hallo welt");
        assert_eq!(normalize_query("ÄRGER"), "ärger");
    }

    #[test]
    fn a_429_pauses_klipy_calls_for_its_retry_after() {
        let gifs = relay(600);
        assert!(gifs.quota_check().is_ok());
        let err = gifs.quota_exhausted(Some(&HeaderValue::from_static("120")));
        assert!(matches!(err, ApiError::UpstreamQuota { retry_after: 120 }));
        assert!(matches!(
            gifs.quota_check(),
            Err(ApiError::UpstreamQuota { retry_after }) if (119..=120).contains(&retry_after)
        ));
        let err = gifs.quota_exhausted(Some(&HeaderValue::from_static("soon")));
        assert!(matches!(err, ApiError::UpstreamQuota { retry_after: 60 }));
        let err = gifs.quota_exhausted(Some(&HeaderValue::from_static("999999")));
        assert!(matches!(err, ApiError::UpstreamQuota { retry_after: 3600 }));
        *gifs.inner.quota_until.lock().unwrap() = Some(Instant::now());
        assert!(gifs.quota_check().is_ok(), "an expired pause lifts itself");
        assert!(gifs.inner.quota_until.lock().unwrap().is_none());
    }

    #[test]
    fn page_is_bounded() {
        assert_eq!(page(None).unwrap(), 1);
        assert_eq!(page(Some(PAGE_MAX)).unwrap(), PAGE_MAX);
        assert!(page(Some(0)).is_err());
        assert!(page(Some(PAGE_MAX + 1)).is_err());
    }
}
