//! REST over the web origin, with the session cookie and the CSRF token the
//! web client uses (backend/api/src/csrf.rs: `X-CSRF-Token` on every
//! non-safe request).

use std::sync::Arc;

use parking_lot::RwLock;
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error, Clone)]
pub enum ApiError {
    #[error("Keine Verbindung zum Server ({0})")]
    Transport(String),
    /// The API's error envelope `{error, message, fields}`.
    #[error("{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
        fields: serde_json::Value,
    },
    #[error("Unerwartete Antwort ({0})")]
    Decode(String),
}

impl ApiError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Http {
    base: url::Url,
    client: reqwest::Client,
    csrf: Arc<RwLock<Option<String>>>,
}

impl Http {
    pub fn new(server: &str) -> Result<Self, ApiError> {
        let base = url::Url::parse(server).map_err(|e| ApiError::Transport(e.to_string()))?;
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .user_agent(concat!("Gelabber-Desktop/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        Ok(Self {
            base,
            client,
            csrf: Arc::new(RwLock::new(None)),
        })
    }

    pub fn base(&self) -> &url::Url {
        &self.base
    }

    /// Absolute URL for a server path such as `/api/avatars/…` or `/images/…`.
    pub fn url(&self, path: &str) -> String {
        self.base
            .join(path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| path.to_owned())
    }

    pub fn reqwest(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn request<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ApiError> {
        let mut req = self
            .client
            .request(method.clone(), self.url(&format!("/api{path}")));
        if method != Method::GET && method != Method::HEAD {
            if self.csrf.read().is_none() {
                self.refresh_csrf().await?;
            }
            if let Some(token) = self.csrf.read().clone() {
                req = req.header("X-CSRF-Token", token);
            }
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        let res = req
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = res.status();
        let bytes = res
            .bytes()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(envelope(status, &bytes));
        }
        let bytes = if bytes.is_empty() {
            &b"null"[..]
        } else {
            &bytes[..]
        };
        serde_json::from_slice(bytes).map_err(|e| ApiError::Decode(format!("{path}: {e}")))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.request::<(), T>(Method::GET, path, None).await
    }

    pub async fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.request(method, path, Some(body)).await
    }

    /// GET /auth/session; keeps its CSRF token for later writes.
    pub async fn session(&self) -> Result<serde_json::Value, ApiError> {
        let res = self
            .client
            .get(self.url("/api/auth/session"))
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = res.status();
        let bytes = res
            .bytes()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(envelope(status, &bytes));
        }
        let session: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::Decode(format!("/auth/session: {e}")))?;
        if let Some(token) = session.get("csrf_token").and_then(|t| t.as_str()) {
            *self.csrf.write() = Some(token.to_owned());
        }
        Ok(session)
    }

    async fn refresh_csrf(&self) -> Result<(), ApiError> {
        self.session().await.map(|_| ())
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<serde_json::Value, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            email: &'a str,
            password: &'a str,
        }
        self.refresh_csrf().await?;
        let _: serde_json::Value = self
            .send(Method::POST, "/auth/login", &Body { email, password })
            .await?;
        // A new session has a new CSRF token.
        self.session().await
    }

    /// Raw bytes of a same-origin resource (avatars, attachments, images).
    pub async fn bytes(&self, path_or_url: &str) -> Result<Vec<u8>, ApiError> {
        let url = if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            path_or_url.to_owned()
        } else {
            self.url(path_or_url)
        };
        let res = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = res.status();
        let bytes = res
            .bytes()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(envelope(status, &bytes));
        }
        Ok(bytes.to_vec())
    }
}

fn envelope(status: StatusCode, bytes: &[u8]) -> ApiError {
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or(serde_json::Value::Null);
    ApiError::Api {
        status: status.as_u16(),
        code: value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        message: value
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Fehler {}", status.as_u16())),
        fields: value
            .get("fields")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }
}
