//! Link previews (#254). After a message with links is sent or edited, a
//! background worker fetches up to three of its links through the SSRF
//! guard in [`net`], reads title, description, site name and image
//! ([`html`], oEmbed for YouTube), copies the image into the object store
//! and attaches the result to the message with an edit event. Sending never
//! waits for it. Results are cached per URL for a day (an hour after a
//! failure). Browsers load preview images from `/api/previews/{id}/{file}`,
//! never from the linked site; a YouTube video loads from
//! `youtube-nocookie.com` only once the viewer presses play.
//!
//! `API_LINK_PREVIEWS=false` turns all of it off; links stay links.

pub mod html;
pub mod net;
pub mod urls;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::HeaderValue;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection, PgPool};
use tokio::sync::{Semaphore, mpsc};
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::config::PreviewConfig;
use crate::error::ApiError;
use crate::limits::{MINUTE, PREVIEW_SERVER_FACTOR};
use crate::state::AppState;
use crate::storage::StoreError;

use self::net::{FetchError, Kind, Policy};

/// Previews older than this are fetched again when a new message links them.
const FRESH_HOURS: i32 = 24;
/// A failed fetch is retried after this long.
const RETRY_HOURS: i32 = 1;
/// Unreferenced previews are dropped after this long.
const KEEP_UNUSED_DAYS: i32 = 7;
/// One link: page, image and redirects together.
const UNFURL_TIMEOUT: Duration = Duration::from_secs(15);
const WORKERS: usize = 4;
const QUEUE: usize = 256;
/// Largest preview image in pixels; the bytes are capped by `net`.
const IMAGE_MAX_EDGE: u32 = 4096;
const SWEEP_EVERY: Duration = Duration::from_secs(3600);
/// How long a stored image may wait for its row before cleanup removes it.
const PENDING_SECS: f64 = 600.0;
const YOUTUBE_OEMBED: &str = "https://www.youtube.com/oembed";
const YOUTUBE_THUMBNAILS: &str = "https://i.ytimg.com";

/// One preview as messages carry it. Text is plain text from the page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkPreview {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    /// `/api/previews/{id}/{file}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<PreviewImage>,
    /// A YouTube video: the client offers a click-to-play embed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub youtube_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewImage {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct LinkPreviews {
    inner: Arc<Inner>,
}

struct Inner {
    policy: Policy,
    /// oEmbed endpoint and thumbnail origin; tests use stand-ins.
    youtube: (String, String),
    queue: mpsc::Sender<Job>,
    receiver: Mutex<Option<mpsc::Receiver<Job>>>,
}

#[derive(Debug)]
pub struct Job {
    pub message_id: Uuid,
    pub channel_id: Uuid,
    /// Event scope: the server, or the DM channel.
    pub server_id: Uuid,
    pub urls: Vec<Url>,
}

impl LinkPreviews {
    pub fn new(config: &PreviewConfig) -> Self {
        Self::with_policy(Policy {
            deny_hosts: config.deny_hosts.clone(),
            ..Policy::default()
        })
    }

    /// Tests point the fetcher at local stand-ins through the policy's
    /// fixed DNS answers and trusted addresses.
    pub fn with_policy(policy: Policy) -> Self {
        let (queue, receiver) = mpsc::channel(QUEUE);
        Self {
            inner: Arc::new(Inner {
                policy,
                youtube: (YOUTUBE_OEMBED.to_owned(), YOUTUBE_THUMBNAILS.to_owned()),
                queue,
                receiver: Mutex::new(Some(receiver)),
            }),
        }
    }

    /// Tests: YouTube's oEmbed endpoint and thumbnail origin elsewhere.
    pub fn with_youtube(self, oembed: &str, thumbnails: &str) -> Self {
        let inner = Arc::into_inner(self.inner).expect("not shared yet");
        Self {
            inner: Arc::new(Inner {
                youtube: (oembed.to_owned(), thumbnails.to_owned()),
                ..inner
            }),
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.inner.policy
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/previews/{id}/{file}", get(image))
}

/// Starts the workers and the hourly sweep once per state.
pub fn start(state: AppState) {
    let Some(previews) = &state.previews else {
        return;
    };
    let Some(mut receiver) = previews.inner.receiver.lock().expect("lock").take() else {
        return;
    };
    let workers = Arc::new(Semaphore::new(WORKERS));
    let jobs = state.clone();
    tokio::spawn(async move {
        while let Some(job) = receiver.recv().await {
            let Ok(permit) = workers.clone().acquire_owned().await else {
                break;
            };
            let state = jobs.clone();
            tokio::spawn(async move {
                process(&state, job).await;
                drop(permit);
            });
        }
    });
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            if state.db.is_closed() {
                break;
            }
            if let Err(err) = sweep(&state.db).await {
                warn!(error = %err, "link preview sweep failed");
            }
        }
    });
}

/// Queues previews for a message just written, if it links anything and
/// its author and server are within their rate. Never blocks the request.
pub fn enqueue(state: &AppState, author_id: Uuid, job: Job) {
    let Some(previews) = &state.previews else {
        return;
    };
    if job.urls.is_empty() {
        return;
    }
    let per_min = state.limits.preview_per_min;
    if per_min > 0 {
        let user = state
            .limiter
            .hit("preview", &author_id.to_string(), per_min, MINUTE);
        let server = state.limiter.hit(
            "preview-server",
            &job.server_id.to_string(),
            per_min.saturating_mul(PREVIEW_SERVER_FACTOR),
            MINUTE,
        );
        if user.is_err() || server.is_err() {
            debug!(message_id = %job.message_id, "link previews skipped: rate");
            return;
        }
    }
    if let Err(err) = previews.inner.queue.try_send(job) {
        warn!(error = %err, "link preview queue full; previews skipped");
    }
}

async fn process(state: &AppState, job: Job) {
    let mut ids = Vec::new();
    for url in &job.urls {
        match tokio::time::timeout(UNFURL_TIMEOUT, preview_for(state, url)).await {
            Ok(Ok(Some(id))) => ids.push(id),
            Ok(Ok(None)) => {}
            Ok(Err(err)) => warn!(error = err.code(), "link preview failed"),
            Err(_) => debug!(url = %url, "link preview timed out"),
        }
    }
    if let Err(err) = crate::messages::attach_previews(state, &job, &ids).await {
        warn!(error = err.code(), message_id = %job.message_id, "link previews not attached");
    }
}

fn url_hash(url: &Url) -> Vec<u8> {
    Sha256::digest(url.as_str().as_bytes()).to_vec()
}

#[derive(FromRow)]
struct Cached {
    id: Uuid,
    ok: bool,
    fresh: bool,
}

/// The preview row for `url`, fetched now unless a fresh one exists;
/// `None` when the page gave nothing to show.
async fn preview_for(state: &AppState, url: &Url) -> Result<Option<Uuid>, ApiError> {
    let hash = url_hash(url);
    let cached = sqlx::query_as::<_, Cached>(
        "SELECT id, status = 'ok' AS ok, \
         fetched_at > now() - make_interval(hours => CASE WHEN status = 'ok' THEN $2 ELSE $3 END) AS fresh \
         FROM link_previews WHERE url_hash = $1",
    )
    .bind(&hash)
    .bind(FRESH_HOURS)
    .bind(RETRY_HOURS)
    .fetch_optional(&state.db)
    .await?;
    if let Some(cached) = &cached
        && cached.fresh
    {
        return Ok(cached.ok.then_some(cached.id));
    }
    let previews = state.previews.as_ref().expect("previews on");
    let unfurled = match unfurl(&previews.inner, url).await {
        Ok(unfurled) => Some(unfurled),
        Err(err) => {
            debug!(url = %url, error = ?err, "link preview fetch failed");
            None
        }
    };
    let id = cached.as_ref().map_or_else(Uuid::new_v4, |c| c.id);
    let image = match unfurled.as_ref().and_then(|u| u.image.as_ref()) {
        Some(image) => store_image(state, id, image).await.ok(),
        None => None,
    };
    let saved = save(&state.db, id, &hash, url, unfurled.as_ref(), image.as_ref()).await;
    if saved.is_err()
        && let Some((key, _, _)) = &image
    {
        discard(state, key).await;
    }
    saved
}

struct Unfurled {
    meta: html::Meta,
    youtube_id: Option<String>,
    image: Option<Image>,
}

struct Image {
    bytes: Vec<u8>,
    content_type: &'static str,
    width: u32,
    height: u32,
}

#[derive(Debug)]
#[allow(dead_code)] // Read through `Debug` in the logs.
enum UnfurlError {
    Fetch(FetchError),
    Empty,
}

impl From<FetchError> for UnfurlError {
    fn from(err: FetchError) -> Self {
        Self::Fetch(err)
    }
}

async fn unfurl(previews: &Inner, url: &Url) -> Result<Unfurled, UnfurlError> {
    let policy = &previews.policy;
    let (meta, youtube_id) = match youtube_id(url) {
        Some(id) => (youtube(previews, &id).await, Some(id)),
        None => {
            let page = policy.get(url, Kind::Html).await?;
            let text = html::decode(&page.body, page.charset.as_deref());
            (html::parse(&text, &page.url), None)
        }
    };
    if meta.is_empty() {
        return Err(UnfurlError::Empty);
    }
    let image = match &meta.image {
        Some(image_url) => fetch_image(policy, image_url).await,
        None => None,
    };
    Ok(Unfurled {
        meta,
        youtube_id,
        image,
    })
}

/// A raster image whose header proves a sane size (no SVG, no pixel bombs).
async fn fetch_image(policy: &Policy, url: &Url) -> Option<Image> {
    let fetched = policy.get(url, Kind::Image).await.ok()?;
    let content_type = crate::avatars::sniff(&fetched.body)?;
    let (width, height) = crate::image_size::dimensions(&fetched.body)?;
    if width > IMAGE_MAX_EDGE || height > IMAGE_MAX_EDGE {
        return None;
    }
    Some(Image {
        bytes: fetched.body,
        content_type,
        width,
        height,
    })
}

/// The video id of a YouTube watch, short, live, embed or `youtu.be` link.
pub fn youtube_id(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let mut path = url.path_segments()?;
    let id = match host {
        "youtu.be" => path.next()?.to_owned(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            match path.next()? {
                "watch" => url
                    .query_pairs()
                    .find(|(key, _)| key == "v")
                    .map(|(_, v)| v.into_owned())?,
                "shorts" | "live" | "embed" => path.next()?.to_owned(),
                _ => return None,
            }
        }
        _ => return None,
    };
    (id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(id)
}

#[derive(Deserialize)]
struct OEmbed {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author_name: Option<String>,
}

/// YouTube's oEmbed answer, which every region gets without a consent page,
/// and its standard thumbnail. Without oEmbed the thumbnail alone.
async fn youtube(previews: &Inner, id: &str) -> html::Meta {
    let (oembed, thumbnails) = &previews.youtube;
    let Ok(mut oembed) = Url::parse(oembed) else {
        return html::Meta::default();
    };
    oembed
        .query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("url", &format!("https://www.youtube.com/watch?v={id}"));
    let data = match previews.policy.get(&oembed, Kind::Json).await {
        Ok(fetched) => serde_json::from_slice::<OEmbed>(&fetched.body).ok(),
        Err(err) => {
            debug!(error = ?err, "youtube oembed failed");
            None
        }
    };
    let (title, author) = data.map_or((None, None), |d| (d.title, d.author_name));
    html::Meta {
        title: html::clean(title, html::TITLE_MAX),
        description: html::clean(author, html::SITE_NAME_MAX),
        site_name: Some("YouTube".into()),
        image: Url::parse(&format!("{thumbnails}/vi/{id}/hqdefault.jpg")).ok(),
    }
}

fn object_key(id: Uuid, file: Uuid) -> String {
    format!("previews/{id}/{file}")
}

/// Stores the image with a pending cleanup row that [`save`] withdraws.
async fn store_image(
    state: &AppState,
    id: Uuid,
    image: &Image,
) -> Result<(String, u32, u32), ApiError> {
    let key = object_key(id, Uuid::new_v4());
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, next_attempt, retain_until) \
         VALUES ($1, now() + $2 * interval '1 second', now() + $2 * interval '1 second')",
    )
    .bind(&key)
    .bind(PENDING_SECS)
    .execute(&state.db)
    .await?;
    state
        .store
        .put(&key, image.content_type, image.bytes.clone())
        .await
        .map_err(|err| ApiError::Internal(format!("object store: {err}")))?;
    Ok((key, image.width, image.height))
}

#[derive(FromRow)]
struct Saved {
    id: Uuid,
    old_image: Option<String>,
}

/// Inserts or refreshes the row; the image it replaces is queued for
/// deletion and the new one's pending cleanup withdrawn, in one transaction.
async fn save(
    db: &PgPool,
    id: Uuid,
    hash: &[u8],
    url: &Url,
    unfurled: Option<&Unfurled>,
    image: Option<&(String, u32, u32)>,
) -> Result<Option<Uuid>, ApiError> {
    let meta = unfurled.map(|u| &u.meta);
    let mut tx = db.begin().await?;
    let saved = sqlx::query_as::<_, Saved>(
        "WITH old AS (SELECT image_key FROM link_previews WHERE url_hash = $2 FOR UPDATE) \
         INSERT INTO link_previews AS lp \
           (id, url_hash, url, status, title, description, site_name, youtube_id, \
            image_key, image_width, image_height, fetched_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now()) \
         ON CONFLICT (url_hash) DO UPDATE SET status = EXCLUDED.status, \
           title = EXCLUDED.title, description = EXCLUDED.description, \
           site_name = EXCLUDED.site_name, youtube_id = EXCLUDED.youtube_id, \
           image_key = EXCLUDED.image_key, image_width = EXCLUDED.image_width, \
           image_height = EXCLUDED.image_height, fetched_at = now() \
         RETURNING lp.id, (SELECT image_key FROM old) AS old_image",
    )
    .bind(id)
    .bind(hash)
    .bind(url.as_str())
    .bind(if unfurled.is_some() { "ok" } else { "failed" })
    .bind(meta.and_then(|m| m.title.clone()))
    .bind(meta.and_then(|m| m.description.clone()))
    .bind(meta.and_then(|m| m.site_name.clone()))
    .bind(unfurled.and_then(|u| u.youtube_id.clone()))
    .bind(image.map(|i| i.0.clone()))
    .bind(image.map(|i| i.1 as i32))
    .bind(image.map(|i| i.2 as i32))
    .fetch_one(&mut *tx)
    .await?;
    if let Some((key, _, _)) = image {
        sqlx::query("DELETE FROM storage_cleanup WHERE object_key = $1 AND attempts = 0")
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(old) = &saved.old_image
        && image.is_none_or(|(key, _, _)| key != old)
    {
        queue_delete(&mut tx, old).await?;
    }
    tx.commit().await?;
    Ok(unfurled.is_some().then_some(saved.id))
}

async fn queue_delete(db: &mut PgConnection, key: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, retain_until) VALUES ($1, now()) \
         ON CONFLICT (object_key) DO UPDATE SET next_attempt = now()",
    )
    .bind(key)
    .execute(db)
    .await
    .map(|_| ())
}

/// An image whose row never landed: its pending cleanup row removes it
/// once due; deleting now just saves the wait.
async fn discard(state: &AppState, key: &str) {
    if let Err(err) = state.store.delete(key).await {
        debug!(error = %err, key, "preview image left to cleanup");
    }
}

/// Drops previews no message has used for [`KEEP_UNUSED_DAYS`].
pub async fn sweep(db: &PgPool) -> Result<u64, sqlx::Error> {
    let mut tx = db.begin().await?;
    let keys: Vec<Option<String>> = sqlx::query_scalar(
        "DELETE FROM link_previews lp WHERE fetched_at < now() - make_interval(days => $1) \
         AND NOT EXISTS (SELECT 1 FROM message_previews mp WHERE mp.preview_id = lp.id) \
         RETURNING image_key",
    )
    .bind(KEEP_UNUSED_DAYS)
    .fetch_all(&mut *tx)
    .await?;
    for key in keys.iter().flatten() {
        queue_delete(&mut tx, key).await?;
    }
    tx.commit().await?;
    Ok(keys.len() as u64)
}

#[derive(FromRow)]
struct PreviewRow {
    message_id: Uuid,
    url: String,
    title: Option<String>,
    description: Option<String>,
    site_name: Option<String>,
    youtube_id: Option<String>,
    image_key: Option<String>,
    image_width: Option<i32>,
    image_height: Option<i32>,
}

impl From<PreviewRow> for LinkPreview {
    fn from(row: PreviewRow) -> Self {
        let image = match (row.image_key, row.image_width, row.image_height) {
            (Some(key), Some(width), Some(height)) => {
                key.strip_prefix("previews/").map(|path| PreviewImage {
                    url: format!("/api/previews/{path}"),
                    width: width as u32,
                    height: height as u32,
                })
            }
            _ => None,
        };
        Self {
            url: row.url,
            title: row.title,
            description: row.description,
            site_name: row.site_name,
            image,
            youtube_id: row.youtube_id,
        }
    }
}

const PREVIEWS_SQL: &str = "SELECT mp.message_id, lp.url, lp.title, lp.description, lp.site_name, \
     lp.youtube_id, lp.image_key, lp.image_width, lp.image_height \
     FROM message_previews mp JOIN link_previews lp ON lp.id = mp.preview_id \
     WHERE mp.message_id = ANY($1) AND lp.status = 'ok' \
     ORDER BY mp.message_id, mp.position";

/// Previews of a page of messages, one query.
pub async fn for_messages(
    db: impl sqlx::PgExecutor<'_>,
    messages: &mut [crate::messages::Message],
) -> Result<(), ApiError> {
    let ids: Vec<Uuid> = messages.iter().map(|m| m.id).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, PreviewRow>(PREVIEWS_SQL)
        .bind(&ids)
        .fetch_all(db)
        .await?;
    let mut by_message: HashMap<Uuid, Vec<LinkPreview>> = HashMap::new();
    for row in rows {
        by_message
            .entry(row.message_id)
            .or_default()
            .push(row.into());
    }
    for message in messages {
        message.previews = by_message.remove(&message.id).unwrap_or_default();
    }
    Ok(())
}

/// `/api/previews/{id}/{file}`: the stored image of a current preview, to
/// any signed-in account (previews show public pages).
async fn image(
    State(state): State<AppState>,
    CurrentUser(_viewer): CurrentUser,
    Path((id, file)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (Ok(id), Ok(file)) = (Uuid::parse_str(&id), Uuid::parse_str(&file)) else {
        return Err(ApiError::NotFound);
    };
    let key = object_key(id, file);
    let current: Option<String> =
        sqlx::query_scalar("SELECT image_key FROM link_previews WHERE id = $1 AND image_key = $2")
            .bind(id)
            .bind(&key)
            .fetch_optional(&state.db)
            .await?;
    if current.is_none() {
        return Err(ApiError::NotFound);
    }
    let (_, bytes) = match state.store.fetch(&key).await {
        Ok(object) => object,
        Err(StoreError::NotFound) => return Err(ApiError::NotFound),
        Err(err) => return Err(ApiError::Internal(format!("object store: {err}"))),
    };
    // Only what the bytes prove, never the stored header, and no sniffing.
    let content_type = crate::avatars::sniff(&bytes).ok_or(ApiError::NotFound)?;
    if !crate::image_size::fits(&bytes, IMAGE_MAX_EDGE, IMAGE_MAX_EDGE) {
        return Err(ApiError::NotFound);
    }
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static(content_type)),
            // Each stored image has its own file name.
            (
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            ),
            (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::youtube_id;
    use url::Url;

    #[test]
    fn finds_youtube_video_ids() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(
                youtube_id(&Url::parse(url).unwrap()).as_deref(),
                Some("dQw4w9WgXcQ"),
                "{url}"
            );
        }
        for url in [
            "https://www.youtube.com/",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/channel/UCabcdefghijk",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ/../x\"",
        ] {
            assert_eq!(youtube_id(&Url::parse(url).unwrap()), None, "{url}");
        }
    }
}
