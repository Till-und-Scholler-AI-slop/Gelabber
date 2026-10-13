//! Object store for chat attachments (issue #7).
//!
//! Production talks to MinIO with path-style S3 (presign PUT for the
//! browser, Head/Delete from the API). Tests or explicitly enabled development
//! select an in-memory map. Missing production configuration is an error.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};
use url::Url;

use crate::config::MinioConfig;

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
    Other(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "object not found"),
            Self::Unconfigured => write!(f, "object store is not configured"),
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

#[derive(Debug, Clone)]
pub enum ObjectBody {
    /// Browser follows this after the API has checked membership.
    Redirect(String),
    /// In-memory store (tests): stream through the API.
    Bytes {
        content_type: String,
        bytes: Vec<u8>,
    },
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

    /// The object for display (`<img src>`): one stable link per window.
    pub async fn get(&self, key: &str) -> Result<ObjectBody, StoreError> {
        match self {
            Self::Memory(store) => store.get(key),
            Self::Minio(store) => store.get_at(key, None, SystemTime::now()),
        }
    }

    /// The object as a file to save. MinIO answers with `disposition` as its
    /// `Content-Disposition`: a browser ignores `<a download>` once the link
    /// redirects to another origin, so the header has to come from there.
    /// The memory store streams through the API, which sets it itself.
    pub async fn download(&self, key: &str, disposition: &str) -> Result<ObjectBody, StoreError> {
        match self {
            Self::Memory(store) => store.get(key),
            Self::Minio(store) => store.get_at(key, Some(disposition), SystemTime::now()),
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
        let (content_type, bytes) = map.get(key).ok_or(StoreError::NotFound)?;
        Ok(ObjectBody::Bytes {
            content_type: content_type.clone(),
            bytes: bytes.clone(),
        })
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
        Ok(Self {
            inner: Arc::new(MinioInner {
                internal,
                public,
                creds,
                http,
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
        now: SystemTime,
    ) -> Result<ObjectBody, StoreError> {
        let mut action = self.inner.public.get_object(Some(&self.inner.creds), key);
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

    async fn put(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> Result<(), StoreError> {
        let signed = self.presign_put(key, content_type, bytes.len() as i64)?;
        let mut request = self.inner.http.put(&signed.url);
        for (name, value) in &signed.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = request
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
        })
        .expect("minio store")
    }

    fn link(body: Result<ObjectBody, StoreError>) -> String {
        match body.expect("get") {
            ObjectBody::Redirect(url) => url,
            ObjectBody::Bytes { .. } => panic!("minio redirects"),
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
        let at = |secs| link(store.get_at("att/x", None, start + Duration::from_secs(secs)));
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
    fn minio_download_variant_signs_its_disposition() {
        let store = minio();
        let now = UNIX_EPOCH + Duration::from_secs(1_791_799_200);
        let disposition = "attachment; filename=\"B_r.png\"; filename*=UTF-8''B%C3%A4r.png";
        let url = link(store.get_at("att/x", Some(disposition), now));
        assert_eq!(
            query(&url, "response-content-disposition").as_deref(),
            Some(disposition)
        );
        assert!(query(&url, "X-Amz-Signature").is_some(), "{url}");
        // The display link of the same window stays as it was.
        assert_ne!(url, link(store.get_at("att/x", None, now)));
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
