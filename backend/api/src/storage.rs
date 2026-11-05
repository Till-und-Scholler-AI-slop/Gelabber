//! Object store for chat attachments (issue #7).
//!
//! Production talks to MinIO with path-style S3 (presign PUT for the
//! browser, Head/Delete from the API). With `ATTACHMENT_DELIVERY=proxy` the
//! browser never sees MinIO: the API streams uploads and downloads over the
//! internal endpoint (#348). Tests or explicitly enabled development select
//! an in-memory map. Missing production configuration is an error.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use futures_util::StreamExt;
use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};
use url::Url;

use crate::config::{AttachmentDelivery, MinioConfig};

const REGION: &str = "us-east-1";
pub const PUT_TTL: Duration = Duration::from_secs(600);
/// Download links are signed at the start of a window, not per request: the
/// same object keeps the same URL, and with it the browser's cached copy, for
/// the whole window (issue #201). A link stays valid `GET_TTL` from the start
/// of its window, so even one handed out at the end of a window has
/// `GET_TTL - GET_WINDOW` left, which is what the object's own cache header
/// promises.
pub const GET_WINDOW: Duration = Duration::from_secs(30 * 60);
/// Access revocation: a download link, and the browser's cached copy of the
/// redirect and of the picture, keeps working for up to `GET_TTL`, 60
/// minutes, after the membership check that handed it out, also after the
/// member left, was removed or logged out. Accepted by Silas on 2026-10-12;
/// see `api/STORAGE_LIFECYCLE.md`.
pub const GET_TTL: Duration = Duration::from_secs(60 * 60);

/// The current signing window: its start (Unix seconds) and the seconds left
/// until the next one, at least one.
pub fn get_window(now: SystemTime) -> (u64, u64) {
    let now = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let window = GET_WINDOW.as_secs();
    let start = now - now % window;
    (start, (start + window - now).max(1))
}

#[derive(Debug)]
pub enum StoreError {
    NotFound,
    Unconfigured,
    /// A `Range` outside the object (HTTP 416).
    RangeNotSatisfiable,
    /// An upload body longer or shorter than the size it was signed for.
    SizeMismatch,
    Other(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "object not found"),
            Self::Unconfigured => write!(f, "object store is not configured"),
            Self::RangeNotSatisfiable => write!(f, "range not satisfiable"),
            Self::SizeMismatch => write!(f, "upload size does not match"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone)]
pub struct ObjectMeta {
    pub content_type: String,
    pub size: i64,
}

#[derive(Debug, Clone)]
pub struct PresignedPut {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug)]
pub enum ObjectBody {
    /// Browser follows this after the API has checked membership.
    Redirect(String),
    /// Proxy delivery and the in-memory store: the bytes go through the API.
    Stream(StoredObject),
}

/// An object's bytes on their way through the API (#348).
#[derive(Debug)]
pub struct StoredObject {
    /// `Content-Range` of a partial answer (206); `None` for the whole object.
    pub content_range: Option<String>,
    pub content_length: Option<u64>,
    /// The store answered `Range` itself, so the API may say `Accept-Ranges`.
    pub ranges: bool,
    pub body: Body,
}

/// One `bytes=` range as browsers send it for media (`bytes=0-`,
/// `bytes=100-199`, `bytes=-500`); anything else is dropped and the whole
/// object is served, which RFC 9110 allows.
pub fn single_byte_range(raw: &str) -> Option<&str> {
    let spec = raw.trim().strip_prefix("bytes=")?;
    let (first, last) = spec.split_once('-')?;
    let digits = |part: &str| part.len() <= 19 && part.bytes().all(|b| b.is_ascii_digit());
    (digits(first) && digits(last) && !(first.is_empty() && last.is_empty())).then_some(raw.trim())
}

#[derive(Clone)]
pub enum ObjectStore {
    Memory(MemoryStore),
    Minio(MinioStore),
}

impl ObjectStore {
    /// Explicit test/development store; never selected by a missing S3 config.
    pub fn memory() -> Self {
        Self::Memory(MemoryStore::default())
    }

    pub fn from_minio(config: Option<&MinioConfig>) -> Result<Self, StoreError> {
        match config {
            Some(cfg) => Ok(Self::Minio(MinioStore::new(cfg)?)),
            None => Err(StoreError::Unconfigured),
        }
    }

    /// Whether attachment bytes go through the API (`ATTACHMENT_DELIVERY=proxy`,
    /// or the in-memory store, which has no URL of its own) instead of
    /// between the browser and MinIO.
    pub fn proxies(&self) -> bool {
        match self {
            Self::Memory(_) => true,
            Self::Minio(store) => store.inner.delivery == AttachmentDelivery::Proxy,
        }
    }

    pub async fn ensure_ready(&self) -> Result<(), StoreError> {
        match self {
            Self::Memory(_) => Ok(()),
            Self::Minio(store) => store.ensure_ready().await,
        }
    }

    pub fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        size: i64,
    ) -> Result<PresignedPut, StoreError> {
        match self {
            Self::Memory(store) => store.presign_put(key, content_type, size),
            Self::Minio(store) => store.presign_put(key, content_type, size),
        }
    }

    pub async fn head(&self, key: &str) -> Result<ObjectMeta, StoreError> {
        match self {
            Self::Memory(store) => store.head(key),
            Self::Minio(store) => store.head(key).await,
        }
    }

    /// The object for the browser. Redirect delivery: one stable link per
    /// window, the window of `now`, and MinIO answers with `content_type`,
    /// the type the API vouches for, not the one stored with the upload,
    /// and with `disposition` (`None` for display) as its
    /// `Content-Disposition`: a browser ignores `<a download>` once the link
    /// redirects to another origin, so the header has to come from there.
    /// Proxy delivery and the memory store stream the bytes (one `range` of
    /// them, where the store can), and the API sets those headers itself.
    pub async fn serve(
        &self,
        key: &str,
        content_type: &str,
        disposition: Option<&str>,
        range: Option<&str>,
        now: SystemTime,
    ) -> Result<ObjectBody, StoreError> {
        match self {
            Self::Memory(store) => store.get(key),
            Self::Minio(store) if store.inner.delivery == AttachmentDelivery::Proxy => {
                store.stream(key, range).await.map(ObjectBody::Stream)
            }
            Self::Minio(store) => store.get_at(key, disposition, Some(content_type), now),
        }
    }

    /// Proxy delivery: the upload body, exactly `size` bytes, into the
    /// object over the internal endpoint.
    pub async fn upload(
        &self,
        key: &str,
        content_type: &str,
        size: u64,
        body: Body,
    ) -> Result<(), StoreError> {
        match self {
            Self::Memory(store) => {
                let limit = usize::try_from(size).map_err(|_| StoreError::SizeMismatch)?;
                let bytes = axum::body::to_bytes(body, limit)
                    .await
                    .map_err(|_| StoreError::SizeMismatch)?;
                if bytes.len() as u64 != size {
                    return Err(StoreError::SizeMismatch);
                }
                store.put(key, content_type, bytes.to_vec());
                Ok(())
            }
            Self::Minio(store) => store.upload(key, content_type, size, body).await,
        }
    }

    /// The first `len` bytes (fewer for a smaller object), to check content.
    pub async fn prefix(&self, key: &str, len: usize) -> Result<Vec<u8>, StoreError> {
        match self {
            Self::Memory(store) => store
                .fetch(key)
                .map(|(_, bytes)| bytes.into_iter().take(len).collect()),
            Self::Minio(store) => store.prefix(key, len).await,
        }
    }

    /// The object's bytes through the API (small objects only: avatars).
    pub async fn fetch(&self, key: &str) -> Result<(String, Vec<u8>), StoreError> {
        match self {
            Self::Memory(store) => store.fetch(key),
            Self::Minio(store) => store.fetch(key).await,
        }
    }

    pub async fn put(
        &self,
        key: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<(), StoreError> {
        match self {
            Self::Memory(store) => {
                store.put(key, content_type, bytes);
                Ok(())
            }
            Self::Minio(store) => store.put(key, content_type, bytes).await,
        }
    }

    pub async fn delete(&self, key: &str) -> Result<(), StoreError> {
        match self {
            Self::Memory(store) => {
                store.delete(key);
                Ok(())
            }
            Self::Minio(store) => store.delete(key).await,
        }
    }
}

type MemoryObjects = HashMap<String, (String, Vec<u8>)>;

#[derive(Clone, Default)]
pub struct MemoryStore {
    inner: Arc<Mutex<MemoryObjects>>,
}

impl MemoryStore {
    fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        size: i64,
    ) -> Result<PresignedPut, StoreError> {
        Ok(PresignedPut {
            url: format!("http://127.0.0.1:1/gelabber/{key}"),
            headers: vec![
                ("Content-Type".to_owned(), content_type.to_owned()),
                ("Content-Length".to_owned(), size.to_string()),
            ],
        })
    }

    fn head(&self, key: &str) -> Result<ObjectMeta, StoreError> {
        let map = self.inner.lock().expect("memory store");
        let (content_type, bytes) = map.get(key).ok_or(StoreError::NotFound)?;
        Ok(ObjectMeta {
            content_type: content_type.clone(),
            size: bytes.len() as i64,
        })
    }

    fn get(&self, key: &str) -> Result<ObjectBody, StoreError> {
        let map = self.inner.lock().expect("memory store");
        let (_, bytes) = map.get(key).ok_or(StoreError::NotFound)?;
        Ok(ObjectBody::Stream(StoredObject {
            content_range: None,
            content_length: Some(bytes.len() as u64),
            ranges: false,
            body: Body::from(bytes.clone()),
        }))
    }

    fn fetch(&self, key: &str) -> Result<(String, Vec<u8>), StoreError> {
        let map = self.inner.lock().expect("memory store");
        map.get(key).cloned().ok_or(StoreError::NotFound)
    }

    fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) {
        self.inner
            .lock()
            .expect("memory store")
            .insert(key.to_owned(), (content_type.to_owned(), bytes));
    }

    fn delete(&self, key: &str) {
        self.inner.lock().expect("memory store").remove(key);
    }
}

#[derive(Clone)]
pub struct MinioStore {
    inner: Arc<MinioInner>,
}

struct MinioInner {
    internal: Bucket,
    public: Bucket,
    creds: Credentials,
    http: reqwest::Client,
    /// Proxy delivery: no deadline for the whole transfer, which may take
    /// minutes on a slow line, only for connecting and between reads.
    stream_http: reqwest::Client,
    delivery: AttachmentDelivery,
}

impl MinioStore {
    fn new(config: &MinioConfig) -> Result<Self, StoreError> {
        let internal_endpoint = Url::parse(&config.endpoint)
            .map_err(|err| StoreError::Other(format!("MINIO_ENDPOINT: {err}")))?;
        let public_endpoint = Url::parse(&config.public_endpoint)
            .map_err(|err| StoreError::Other(format!("MINIO_PUBLIC_ENDPOINT: {err}")))?;
        let internal = Bucket::new(
            internal_endpoint,
            UrlStyle::Path,
            config.bucket.clone(),
            REGION,
        )
        .map_err(|err| StoreError::Other(format!("minio bucket: {err}")))?;
        let public = Bucket::new(
            public_endpoint,
            UrlStyle::Path,
            config.bucket.clone(),
            REGION,
        )
        .map_err(|err| StoreError::Other(format!("minio public bucket: {err}")))?;
        let creds = Credentials::new(config.access_key.clone(), config.secret_key.clone());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|err| StoreError::Other(format!("http client: {err}")))?;
        let stream_http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|err| StoreError::Other(format!("http client: {err}")))?;
        Ok(Self {
            inner: Arc::new(MinioInner {
                internal,
                public,
                creds,
                http,
                stream_http,
                delivery: config.delivery,
            }),
        })
    }

    async fn ensure_ready(&self) -> Result<(), StoreError> {
        let action = self.inner.internal.create_bucket(&self.inner.creds);
        let url = action.sign(Duration::from_secs(60));
        let response =
            self.inner.http.put(url).send().await.map_err(|err| {
                StoreError::Other(format!("create bucket: {}", err.without_url()))
            })?;
        let status = response.status().as_u16();
        // 200/201 created, 409 already exists.
        if status == 200 || status == 201 || status == 409 {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        Err(StoreError::Other(format!(
            "create bucket returned {status}: {body}"
        )))
    }

    fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        size: i64,
    ) -> Result<PresignedPut, StoreError> {
        let mut action = self.inner.public.put_object(Some(&self.inner.creds), key);
        action
            .headers_mut()
            .insert("content-type", content_type.to_owned());
        // Signed so MinIO rejects a PUT whose Content-Length is not this size.
        action
            .headers_mut()
            .insert("content-length", size.to_string());
        let url = action.sign(PUT_TTL);
        Ok(PresignedPut {
            url: url.to_string(),
            headers: vec![
                ("Content-Type".to_owned(), content_type.to_owned()),
                ("Content-Length".to_owned(), size.to_string()),
            ],
        })
    }

    async fn head(&self, key: &str) -> Result<ObjectMeta, StoreError> {
        let action = self
            .inner
            .internal
            .head_object(Some(&self.inner.creds), key);
        let url = action.sign(Duration::from_secs(30));
        let response = self
            .inner
            .http
            .head(url)
            .send()
            .await
            .map_err(|err| StoreError::Other(format!("head object: {}", err.without_url())))?;
        if response.status().as_u16() == 404 {
            return Err(StoreError::NotFound);
        }
        if !response.status().is_success() {
            return Err(StoreError::Other(format!(
                "head object returned {}",
                response.status()
            )));
        }
        object_meta_from_head(response.headers())
    }

    /// Signed as of the start of `now`'s window (see `GET_WINDOW`).
    fn get_at(
        &self,
        key: &str,
        disposition: Option<&str>,
        content_type: Option<&str>,
        now: SystemTime,
    ) -> Result<ObjectBody, StoreError> {
        let mut action = self.inner.public.get_object(Some(&self.inner.creds), key);
        if let Some(content_type) = content_type {
            action
                .query_mut()
                .insert("response-content-type", content_type.to_owned());
        }
        // Signed into the URL, so MinIO answers with it: the object never
        // changes, and the link outlives this cache time (see GET_WINDOW).
        action.query_mut().insert(
            "response-cache-control",
            format!("private, max-age={}", (GET_TTL - GET_WINDOW).as_secs()),
        );
        if let Some(disposition) = disposition {
            action
                .query_mut()
                .insert("response-content-disposition", disposition.to_owned());
        }
        let (start, _) = get_window(now);
        let time = jiff::Timestamp::from_second(start as i64)
            .map_err(|err| StoreError::Other(format!("signing time: {err}")))?;
        let url = action.sign_with_time(GET_TTL, &time);
        Ok(ObjectBody::Redirect(url.to_string()))
    }

    /// Proxy delivery: the object, or one `range` of it, from the internal
    /// endpoint, passed on as it arrives.
    async fn stream(&self, key: &str, range: Option<&str>) -> Result<StoredObject, StoreError> {
        let action = self.inner.internal.get_object(Some(&self.inner.creds), key);
        let url = action.sign(Duration::from_secs(60));
        let mut request = self.inner.stream_http.get(url);
        if let Some(range) = range.and_then(single_byte_range) {
            request = request.header(reqwest::header::RANGE, range);
        }
        let response = request
            .send()
            .await
            .map_err(|err| StoreError::Other(format!("get object: {}", err.without_url())))?;
        match response.status().as_u16() {
            200 | 206 => {}
            404 => return Err(StoreError::NotFound),
            416 => return Err(StoreError::RangeNotSatisfiable),
            status => return Err(StoreError::Other(format!("get object returned {status}"))),
        }
        let content_range = (response.status().as_u16() == 206)
            .then(|| response.headers().get(reqwest::header::CONTENT_RANGE))
            .flatten()
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_length = response.content_length();
        let body = Body::from_stream(
            response
                .bytes_stream()
                .map(|chunk| chunk.map_err(|err| std::io::Error::other(err.without_url()))),
        );
        Ok(StoredObject {
            content_range,
            content_length,
            ranges: true,
            body,
        })
    }

    /// Proxy delivery: the browser's upload body into the object, signed
    /// for exactly `size` bytes like the presigned PUT. A body that runs
    /// past `size` stops the transfer; one that ends early fails it.
    async fn upload(
        &self,
        key: &str,
        content_type: &str,
        size: u64,
        body: Body,
    ) -> Result<(), StoreError> {
        let mut action = self.inner.internal.put_object(Some(&self.inner.creds), key);
        action
            .headers_mut()
            .insert("content-type", content_type.to_owned());
        action
            .headers_mut()
            .insert("content-length", size.to_string());
        let url = action.sign(Duration::from_secs(60));
        // Behind a real server hyper stops the body at its Content-Length,
        // which the API checked against `size`; a body that runs past it or
        // breaks off is the client's mistake, not the store's.
        let seen = Arc::new(AtomicU64::new(0));
        let client_failed = Arc::new(AtomicBool::new(false));
        let counted = {
            let (seen, client_failed) = (seen.clone(), client_failed.clone());
            body.into_data_stream().map(move |chunk| {
                let chunk = chunk.map_err(|err| {
                    client_failed.store(true, Ordering::Relaxed);
                    std::io::Error::other(err)
                })?;
                let len = chunk.len() as u64;
                if seen.fetch_add(len, Ordering::Relaxed) + len > size {
                    client_failed.store(true, Ordering::Relaxed);
                    return Err(std::io::Error::other("upload longer than signed"));
                }
                Ok(chunk)
            })
        };
        let result = self
            .inner
            .stream_http
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(reqwest::Body::wrap_stream(counted))
            .send()
            .await;
        if client_failed.load(Ordering::Relaxed) {
            return Err(StoreError::SizeMismatch);
        }
        let response = result
            .map_err(|err| StoreError::Other(format!("put object: {}", err.without_url())))?;
        if !response.status().is_success() {
            return Err(StoreError::Other(format!(
                "put object returned {}",
                response.status()
            )));
        }
        if seen.load(Ordering::Relaxed) != size {
            return Err(StoreError::SizeMismatch);
        }
        Ok(())
    }

    async fn prefix(&self, key: &str, len: usize) -> Result<Vec<u8>, StoreError> {
        let action = self.inner.internal.get_object(Some(&self.inner.creds), key);
        let url = action.sign(Duration::from_secs(30));
        let response = self
            .inner
            .http
            .get(url)
            .header(
                reqwest::header::RANGE,
                format!("bytes=0-{}", len.max(1) - 1),
            )
            .send()
            .await
            .map_err(|err| StoreError::Other(format!("get object: {}", err.without_url())))?;
        if response.status().as_u16() == 404 {
            return Err(StoreError::NotFound);
        }
        if !response.status().is_success() {
            return Err(StoreError::Other(format!(
                "get object returned {}",
                response.status()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|err| StoreError::Other(format!("get object: {}", err.without_url())))?;
        // A server that ignores Range sends everything; keep the prefix.
        Ok(bytes.into_iter().take(len).collect())
    }

    async fn fetch(&self, key: &str) -> Result<(String, Vec<u8>), StoreError> {
        let action = self.inner.internal.get_object(Some(&self.inner.creds), key);
        let url = action.sign(Duration::from_secs(30));
        let response = self
            .inner
            .http
            .get(url)
            .send()
            .await
            .map_err(|err| StoreError::Other(format!("get object: {}", err.without_url())))?;
        if response.status().as_u16() == 404 {
            return Err(StoreError::NotFound);
        }
        if !response.status().is_success() {
            return Err(StoreError::Other(format!(
                "get object returned {}",
                response.status()
            )));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let bytes = response
            .bytes()
            .await
            .map_err(|err| StoreError::Other(format!("get object: {}", err.without_url())))?;
        Ok((content_type, bytes.to_vec()))
    }

    /// Server-side upload over the internal endpoint; the public one may
    /// not be reachable from inside the container network.
    async fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> Result<(), StoreError> {
        let mut action = self.inner.internal.put_object(Some(&self.inner.creds), key);
        action
            .headers_mut()
            .insert("content-type", content_type.to_owned());
        let url = action.sign(Duration::from_secs(30));
        let response = self
            .inner
            .http
            .put(url)
            .header("content-type", content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|err| StoreError::Other(format!("put object: {}", err.without_url())))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(StoreError::Other(format!(
                "put object returned {}",
                response.status()
            )))
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        let action = self
            .inner
            .internal
            .delete_object(Some(&self.inner.creds), key);
        let url = action.sign(Duration::from_secs(30));
        let response =
            self.inner.http.delete(url).send().await.map_err(|err| {
                StoreError::Other(format!("delete object: {}", err.without_url()))
            })?;
        if response.status().is_success() || response.status().as_u16() == 404 {
            Ok(())
        } else {
            Err(StoreError::Other(format!(
                "delete object returned {}",
                response.status()
            )))
        }
    }
}

/// Size and type from a MinIO HEAD.
///
/// `reqwest::Response::content_length` is the **body** size hint. A HEAD
/// response has no body, so that hint is 0 (or unknown) even when MinIO
/// sends `Content-Length` set to the object size. Using the hint made every
/// non-empty upload fail the bind check (`size` invalid) after a successful
/// PUT, so the message was never created and the other participant never
/// saw the image.
fn object_meta_from_head(headers: &reqwest::header::HeaderMap) -> Result<ObjectMeta, StoreError> {
    let raw = headers
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| StoreError::Other("head object missing content-length".into()))?;
    let size = raw.parse::<i64>().map_err(|_| {
        StoreError::Other(format!(
            "head object content-length is not an integer: {raw}"
        ))
    })?;
    if size < 0 {
        return Err(StoreError::Other(
            "head object content-length is negative".into(),
        ));
    }
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream");
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_owned();
    Ok(ObjectMeta { content_type, size })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MinioConfig;

    fn header<'a>(signed: &'a PresignedPut, name: &str) -> Option<&'a str> {
        signed
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn memory_presign_lists_content_length() {
        let store = ObjectStore::memory();
        let signed = store
            .presign_put("att/x", "image/png", 12)
            .expect("presign");
        assert_eq!(header(&signed, "content-type"), Some("image/png"));
        assert_eq!(header(&signed, "content-length"), Some("12"));
    }

    #[test]
    fn minio_presign_signs_content_length() {
        let store = ObjectStore::from_minio(Some(&MinioConfig {
            endpoint: "http://minio:9000".into(),
            public_endpoint: "http://localhost:9000".into(),
            access_key: "gelabber".into(),
            secret_key: "gelabbergelabber".into(),
            bucket: "gelabber".into(),
            root_credentials: false,
            delivery: crate::config::AttachmentDelivery::Redirect,
        }))
        .expect("minio store");
        let signed = store
            .presign_put("att/x", "image/png", 12)
            .expect("presign");
        assert_eq!(header(&signed, "content-length"), Some("12"));
        let url = signed.url.to_ascii_lowercase();
        assert!(
            url.contains("content-length"),
            "signed headers must include content-length: {url}"
        );
    }

    #[test]
    fn get_window_is_aligned_and_never_empty() {
        let at = |secs| UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(get_window(at(3_600)), (3_600, 1_800));
        assert_eq!(get_window(at(3_601)), (3_600, 1_799));
        assert_eq!(get_window(at(5_399)), (3_600, 1));
        assert_eq!(get_window(at(5_400)), (5_400, 1_800));
    }

    fn minio() -> MinioStore {
        MinioStore::new(&MinioConfig {
            endpoint: "http://minio:9000".into(),
            public_endpoint: "http://localhost:9000".into(),
            access_key: "gelabber".into(),
            secret_key: "gelabbergelabber".into(),
            bucket: "gelabber".into(),
            root_credentials: false,
            delivery: crate::config::AttachmentDelivery::Redirect,
        })
        .expect("minio store")
    }

    fn link(body: Result<ObjectBody, StoreError>) -> String {
        match body.expect("get") {
            ObjectBody::Redirect(url) => url,
            ObjectBody::Stream(_) => panic!("minio redirects"),
        }
    }

    fn query(url: &str, name: &str) -> Option<String> {
        url::Url::parse(url)
            .expect("url")
            .query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    }

    #[test]
    fn minio_download_link_is_stable_within_its_window() {
        let store = minio();
        // 2026-10-12T10:00:00Z, the start of a window.
        let start = UNIX_EPOCH + Duration::from_secs(1_791_799_200);
        assert_eq!(get_window(start).1, 1_800);
        let at = |secs| link(store.get_at("att/x", None, None, start + Duration::from_secs(secs)));
        let first = at(0);
        assert_eq!(first, at(1));
        assert_eq!(first, at(1_799));
        assert_ne!(first, at(1_800));
        // Signed as of the window start, not of the request.
        assert_eq!(
            query(&at(1_799), "X-Amz-Date").as_deref(),
            Some("20261012T100000Z")
        );
        assert_eq!(
            query(&at(1_800), "X-Amz-Date").as_deref(),
            Some("20261012T103000Z")
        );
        assert_eq!(query(&first, "X-Amz-Expires").as_deref(), Some("3600"));
        assert_eq!(
            query(&first, "response-cache-control").as_deref(),
            Some("private, max-age=1800")
        );
        assert_eq!(query(&first, "response-content-disposition"), None);
    }

    #[test]
    fn minio_link_signs_the_vouched_content_type() {
        let store = minio();
        let now = UNIX_EPOCH + Duration::from_secs(1_791_799_200);
        let url = link(store.get_at("att/x", None, Some("text/plain; charset=utf-8"), now));
        assert_eq!(
            query(&url, "response-content-type").as_deref(),
            Some("text/plain; charset=utf-8")
        );
    }

    #[test]
    fn minio_download_variant_signs_its_disposition() {
        let store = minio();
        let now = UNIX_EPOCH + Duration::from_secs(1_791_799_200);
        let disposition = "attachment; filename=\"B_r.png\"; filename*=UTF-8''B%C3%A4r.png";
        let url = link(store.get_at("att/x", Some(disposition), None, now));
        assert_eq!(
            query(&url, "response-content-disposition").as_deref(),
            Some(disposition)
        );
        assert!(query(&url, "X-Amz-Signature").is_some(), "{url}");
        // The display link of the same window stays as it was.
        assert_ne!(url, link(store.get_at("att/x", None, None, now)));
    }

    #[test]
    fn only_a_single_byte_range_is_passed_on() {
        assert_eq!(single_byte_range("bytes=0-"), Some("bytes=0-"));
        assert_eq!(single_byte_range(" bytes=100-199 "), Some("bytes=100-199"));
        assert_eq!(single_byte_range("bytes=-500"), Some("bytes=-500"));
        for other in [
            "bytes=-",
            "bytes=0-1,5-6",
            "items=0-1",
            "bytes=a-b",
            "bytes=0-1\r\nX: y",
            "bytes=99999999999999999999-",
        ] {
            assert_eq!(single_byte_range(other), None, "{other}");
        }
    }

    /// What a stand-in MinIO saw of one request.
    #[derive(Debug, Default, Clone)]
    struct Seen {
        method: String,
        path: String,
        range: Option<String>,
        content_length: Option<String>,
        chunked: bool,
        body: Vec<u8>,
    }

    /// A stand-in for MinIO on the internal endpoint: GET answers the
    /// object (8 bytes) or one range of it, `/gelabber/att/gone` is 404,
    /// `bytes=100-` is 416, and PUT takes any body.
    async fn fake_minio() -> (MinioStore, Arc<Mutex<Vec<Seen>>>) {
        use axum::http::{Request, StatusCode};
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let router = axum::Router::new().fallback(move |request: Request<Body>| {
            let log = log.clone();
            async move {
                let (parts, body) = request.into_parts();
                let header = |name: &str| {
                    parts
                        .headers
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned)
                };
                let entry = Seen {
                    method: parts.method.to_string(),
                    path: parts.uri.path().to_owned(),
                    range: header("range"),
                    content_length: header("content-length"),
                    chunked: header("transfer-encoding").is_some(),
                    body: axum::body::to_bytes(body, 1 << 20).await.unwrap().to_vec(),
                };
                log.lock().unwrap().push(entry.clone());
                let object = b"01234567";
                let response = axum::http::Response::builder();
                if entry.method == "PUT" {
                    return response.status(200).body(Body::empty()).unwrap();
                }
                if entry.path.ends_with("/gone") {
                    return response.status(404).body(Body::empty()).unwrap();
                }
                match entry.range.as_deref() {
                    Some("bytes=2-4") => response
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header("content-range", "bytes 2-4/8")
                        .body(Body::from(&object[2..5]))
                        .unwrap(),
                    Some("bytes=100-") => response.status(416).body(Body::empty()).unwrap(),
                    _ => response.status(200).body(Body::from(&object[..])).unwrap(),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let store = MinioStore::new(&MinioConfig {
            endpoint: format!("http://{addr}"),
            public_endpoint: "https://unreachable.invalid:9000".into(),
            access_key: "gelabber".into(),
            secret_key: "gelabbergelabber".into(),
            bucket: "gelabber".into(),
            root_credentials: false,
            delivery: AttachmentDelivery::Proxy,
        })
        .expect("minio store");
        (store, seen)
    }

    async fn body_bytes(object: StoredObject) -> Vec<u8> {
        axum::body::to_bytes(object.body, 1 << 20)
            .await
            .unwrap()
            .to_vec()
    }

    /// #348: proxy delivery reads from the internal endpoint, never the
    /// public one, and passes a media player's range on.
    #[tokio::test]
    async fn proxy_delivery_streams_the_object_and_its_ranges() {
        let (minio, seen) = fake_minio().await;
        let store = ObjectStore::Minio(minio);
        assert!(store.proxies());
        let now = SystemTime::now();
        let stream = |body: Result<ObjectBody, StoreError>| match body.expect("serve") {
            ObjectBody::Stream(object) => object,
            ObjectBody::Redirect(url) => panic!("proxy delivery redirected to {url}"),
        };

        let whole = stream(store.serve("att/x", "image/png", None, None, now).await);
        assert_eq!(whole.content_range, None);
        assert_eq!(whole.content_length, Some(8));
        assert!(whole.ranges);
        assert_eq!(body_bytes(whole).await, b"01234567");

        let part = stream(
            store
                .serve("att/x", "video/mp4", None, Some("bytes=2-4"), now)
                .await,
        );
        assert_eq!(part.content_range.as_deref(), Some("bytes 2-4/8"));
        assert_eq!(part.content_length, Some(3));
        assert_eq!(body_bytes(part).await, b"234");

        // Several ranges are not passed on: the whole object answers.
        let many = stream(
            store
                .serve("att/x", "video/mp4", None, Some("bytes=0-1,4-5"), now)
                .await,
        );
        assert_eq!(many.content_range, None);

        assert!(matches!(
            store
                .serve("att/x", "video/mp4", None, Some("bytes=100-"), now)
                .await,
            Err(StoreError::RangeNotSatisfiable)
        ));
        assert!(matches!(
            store.serve("att/gone", "image/png", None, None, now).await,
            Err(StoreError::NotFound)
        ));

        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen[0].path, "/gelabber/att/x");
        assert_eq!(seen[0].range, None);
        assert_eq!(seen[1].range.as_deref(), Some("bytes=2-4"));
        assert_eq!(seen[2].range, None);
    }

    /// #348: the upload goes to MinIO with the signed length, not chunked
    /// (MinIO refuses a PUT without a length), and a body longer than
    /// signed never completes.
    #[tokio::test]
    async fn proxy_upload_sends_exactly_the_signed_length() {
        let (minio, seen) = fake_minio().await;
        let store = ObjectStore::Minio(minio);
        store
            .upload("att/x", "image/png", 4, Body::from(&b"abcd"[..]))
            .await
            .expect("upload");
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen[0].method, "PUT");
            assert_eq!(seen[0].path, "/gelabber/att/x");
            assert_eq!(seen[0].content_length.as_deref(), Some("4"));
            assert!(!seen[0].chunked);
            assert_eq!(seen[0].body, b"abcd");
        }
        assert!(matches!(
            store
                .upload("att/y", "image/png", 4, Body::from(&b"abcdef"[..]))
                .await,
            Err(StoreError::SizeMismatch)
        ));
    }

    #[test]
    fn redirect_delivery_keeps_the_browser_on_minio() {
        let store = ObjectStore::Minio(minio());
        assert!(!store.proxies());
        assert!(ObjectStore::memory().proxies());
    }

    /// MinIO HEAD carries the object size in `Content-Length` and an empty
    /// body. reqwest's `content_length()` reports that body, not the header.
    #[tokio::test]
    async fn head_object_size_uses_content_length_header_not_body() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 2048];
            let _ = std::io::Read::read(&mut sock, &mut buf);
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 128\r\nContent-Type: image/webp\r\nConnection: close\r\n\r\n";
            let _ = std::io::Write::write_all(&mut sock, resp.as_bytes());
        });

        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client")
            .head(format!("http://{addr}/gelabber/att/x"))
            .send()
            .await
            .expect("head");
        assert!(response.status().is_success());
        assert_ne!(
            response.content_length(),
            Some(128),
            "body size hint must not be treated as the object size"
        );
        let meta = object_meta_from_head(response.headers()).expect("meta");
        assert_eq!(meta.size, 128);
        assert_eq!(meta.content_type, "image/webp");
    }

    #[test]
    fn head_object_meta_rejects_a_missing_length() {
        let headers = reqwest::header::HeaderMap::new();
        let err = object_meta_from_head(&headers).expect_err("missing length");
        let message = err.to_string();
        assert!(
            message.contains("content-length"),
            "missing length must not become size 0: {message}"
        );
    }
}
