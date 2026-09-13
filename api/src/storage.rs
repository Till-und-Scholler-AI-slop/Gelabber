//! Object store for chat attachments (issue #7).
//!
//! Production talks to MinIO with path-style S3 (presign PUT for the
//! browser, Head/Delete from the API). Tests and `cargo test` without
//! MinIO env use an in-memory map so the HTTP suite does not need a
//! running bucket.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};
use url::Url;

use crate::config::MinioConfig;

const REGION: &str = "us-east-1";
pub const PUT_TTL: Duration = Duration::from_secs(600);
pub const GET_TTL: Duration = Duration::from_secs(60);

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
    pub fn from_minio(config: Option<&MinioConfig>) -> Result<Self, StoreError> {
        match config {
            Some(cfg) => Ok(Self::Minio(MinioStore::new(cfg)?)),
            None => Ok(Self::Memory(MemoryStore::default())),
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

    pub async fn get(&self, key: &str) -> Result<ObjectBody, StoreError> {
        match self {
            Self::Memory(store) => store.get(key),
            Self::Minio(store) => store.get(key),
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
        let response = self
            .inner
            .http
            .put(url)
            .send()
            .await
            .map_err(|err| StoreError::Other(format!("create bucket: {err}")))?;
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
            .map_err(|err| StoreError::Other(format!("head object: {err}")))?;
        if response.status().as_u16() == 404 {
            return Err(StoreError::NotFound);
        }
        if !response.status().is_success() {
            return Err(StoreError::Other(format!(
                "head object returned {}",
                response.status()
            )));
        }
        let size = response.content_length().unwrap_or(0) as i64;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        Ok(ObjectMeta { content_type, size })
    }

    fn get(&self, key: &str) -> Result<ObjectBody, StoreError> {
        let action = self.inner.public.get_object(Some(&self.inner.creds), key);
        let url = action.sign(GET_TTL);
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
            .map_err(|err| StoreError::Other(format!("put object: {err}")))?;
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
        let response = self
            .inner
            .http
            .delete(url)
            .send()
            .await
            .map_err(|err| StoreError::Other(format!("delete object: {err}")))?;
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
        let store = ObjectStore::from_minio(None).expect("memory store");
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
}
