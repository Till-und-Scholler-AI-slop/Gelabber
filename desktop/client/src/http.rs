//! REST over the web origin, with the session cookie and the CSRF token the
//! web client uses (backend/api/src/csrf.rs: `X-CSRF-Token` on every
//! non-safe request).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use parking_lot::RwLock;
use reqwest::cookie::CookieStore;
use reqwest::{Method, StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;

use gelabber_i18n::{auth, shell as texts};

use crate::models::CsrfResponse;

/// Per REST request, as the web (`REQUEST_TIMEOUT_MS`).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Uploads of attachment bytes (web `messages/api.ts`).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error, Clone)]
pub enum ApiError {
    #[error("{}", texts::error_transport(.0))]
    Transport(String),
    /// The API's error envelope `{error, message, fields}`.
    #[error("{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
        fields: serde_json::Value,
        /// Seconds to wait before trying again (`retry_after` of a 429,
        /// else its `Retry-After` header).
        retry_after: Option<f64>,
    },
    #[error("{}", texts::error_decode(.0))]
    Decode(String),
}

impl ApiError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The envelope's `error`, or the web's client-side codes `network`,
    /// `timeout` and `internal` (unknown or unreadable answers).
    pub fn code(&self) -> &str {
        match self {
            Self::Api { code, .. } if KNOWN_CODES.contains(&code.as_str()) => code,
            Self::Api { .. } | Self::Decode(_) => "internal",
            Self::Transport(detail) if detail.starts_with(TIMEOUT_PREFIX) => "timeout",
            Self::Transport(_) => "network",
        }
    }

    /// How long a rate limit asks to wait, in seconds (`ApiError.retryAfter`).
    pub fn retry_after(&self) -> Option<f64> {
        match self {
            Self::Api { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// The per-field codes, e.g. `{"email": "invalid"}`.
    pub fn field(&self, name: &str) -> Option<&str> {
        match self {
            Self::Api { fields, .. } => fields.get(name).and_then(|v| v.as_str()),
            _ => None,
        }
    }

    /// The text the web shows for the code (`auth/rules.ts`
    /// `errorMessage`), in the interface's language.
    pub fn text(&self) -> &'static str {
        error_message(self.code())
    }

    pub fn is_unauthenticated(&self) -> bool {
        self.code() == "unauthenticated"
    }

    /// The server hid or removed the thing (`not_found`/`forbidden`).
    pub fn is_gone(&self) -> bool {
        matches!(self.code(), "not_found" | "forbidden")
    }

    fn timeout(detail: impl std::fmt::Display) -> Self {
        Self::Transport(format!("{TIMEOUT_PREFIX}{detail}"))
    }

    fn from_reqwest(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::timeout(error)
        } else {
            Self::Transport(error.to_string())
        }
    }
}

// A marker `code()` reads, not text of its own.
const TIMEOUT_PREFIX: &str = "Zeitüberschreitung: "; // i18n-ok

const KNOWN_CODES: [&str; 21] = [
    "validation_failed",
    "bad_request",
    "unauthenticated",
    "invalid_credentials",
    "csrf_invalid",
    "insecure_transport",
    "forbidden",
    "not_found",
    "email_taken",
    "email_unverified",
    "token_invalid",
    "theme_conflict",
    "invite_invalid",
    "banned",
    "rate_limited",
    "quota_exceeded",
    "upstream_unavailable",
    "upstream_quota",
    "internal",
    "network",
    "timeout",
];

/// The copy per error code, as web/src/auth/rules.ts (the auth catalog's
/// `error_<code>`).
pub fn error_message(code: &str) -> &'static str {
    match code {
        "validation_failed" => auth::error_validation_failed(),
        "unauthenticated" => auth::error_unauthenticated(),
        "invalid_credentials" => auth::error_invalid_credentials(),
        "csrf_invalid" => auth::error_csrf_invalid(),
        "insecure_transport" => auth::error_insecure_transport(),
        "forbidden" => auth::error_forbidden(),
        "not_found" => auth::error_not_found(),
        "email_taken" => auth::error_email_taken(),
        "email_unverified" => auth::error_email_unverified(),
        "token_invalid" => auth::error_token_invalid(),
        "theme_conflict" => auth::error_theme_conflict(),
        "invite_invalid" => auth::error_invite_invalid(),
        "banned" => auth::error_banned(),
        "rate_limited" => auth::error_rate_limited(),
        "quota_exceeded" => auth::error_quota_exceeded(),
        "upstream_unavailable" => auth::error_upstream_unavailable(),
        "upstream_quota" => auth::error_upstream_quota(),
        "network" => auth::error_network(),
        "timeout" => auth::error_timeout(),
        _ => auth::error_internal(),
    }
}

type Hook = Arc<dyn Fn() + Send + Sync>;

/// What a download is for; each kind has its own byte cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Download {
    /// Avatars, icons, banners, picture attachments, GIFs.
    Image,
    /// A text attachment shown inline (`TEXT_PREVIEW_MAX`).
    Text,
    /// Any attachment saved to disk.
    File,
}

impl Download {
    pub const fn max_bytes(self) -> usize {
        match self {
            // An attachment's limit (attachments/validate.rs `SIZE_MAX`).
            Self::Image | Self::File => 25 * 1024 * 1024,
            Self::Text => 100 * 1024,
        }
    }
}

#[derive(Clone)]
pub struct Http {
    base: url::Url,
    client: reqwest::Client,
    /// Without cookies: presigned uploads go to another origin (MinIO).
    plain: reqwest::Client,
    /// Downloads from other origins: no cookies, no redirects.
    foreign: reqwest::Client,
    jar: Arc<reqwest::cookie::Jar>,
    csrf: Arc<RwLock<Option<String>>>,
    /// Server clock minus local clock in ms, from each `Date` header.
    clock_offset: Arc<AtomicI64>,
    unauthenticated: Arc<RwLock<Option<Hook>>>,
    /// A sign-out the server has not confirmed yet; see [`Http::sign_out`].
    held: Arc<RwLock<Option<HeldSignOut>>>,
}

#[derive(Clone)]
struct HeldSignOut {
    cookie: String,
    everywhere: bool,
}

/// `Accept-Language` for the interface's language right now.
fn accept_language() -> &'static str {
    match gelabber_i18n::get() {
        gelabber_i18n::Locale::De => "de-DE,de;q=0.9",
        gelabber_i18n::Locale::En => "en-GB,en;q=0.9",
    }
}

impl Http {
    pub fn new(server: &str) -> Result<Self, ApiError> {
        let base = url::Url::parse(server).map_err(|e| ApiError::Transport(e.to_string()))?;
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        let user_agent = concat!("Gelabber-Desktop/", env!("CARGO_PKG_VERSION"));
        let client = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .default_headers(headers)
            .user_agent(user_agent)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let plain = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(UPLOAD_TIMEOUT)
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let foreign = reqwest::Client::builder()
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(UPLOAD_TIMEOUT)
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        Ok(Self {
            base,
            client,
            plain,
            foreign,
            jar,
            csrf: Arc::new(RwLock::new(None)),
            clock_offset: Arc::new(AtomicI64::new(0)),
            unauthenticated: Arc::new(RwLock::new(None)),
            held: Arc::new(RwLock::new(None)),
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

    /// The gateway URL: `ws(s)://<host>/ws` on the same origin.
    pub fn ws_url(&self, path: &str) -> String {
        let mut url = self.base.join(path).unwrap_or_else(|_| self.base.clone());
        let scheme = if self.base.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        let _ = url.set_scheme(scheme);
        url.to_string()
    }

    /// The `Cookie` header the jar would send to the origin, for the
    /// gateway upgrade.
    pub fn cookie_header(&self) -> Option<String> {
        self.jar
            .cookies(&self.base)
            .and_then(|v| v.to_str().ok().map(str::to_owned))
    }

    /// The session cookie's value, so the desktop app can keep the sign-in
    /// across restarts.
    pub fn session_cookie(&self) -> Option<String> {
        self.cookie_header()?
            .split("; ")
            .find_map(|kv| kv.strip_prefix("gelabber_session="))
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    }

    /// Puts a remembered session cookie back before the first request.
    pub fn restore_session_cookie(&self, value: &str) {
        let token = |c: char| c.is_ascii_graphic() && !matches!(c, ';' | ',' | '"' | '\\');
        if value.is_empty() || !value.chars().all(token) {
            return;
        }
        let secure = if self.base.scheme() == "https" {
            "; Secure"
        } else {
            ""
        };
        self.jar.add_cookie_str(
            &format!("gelabber_session={value}; Path=/; HttpOnly{secure}"), // i18n-ok
            &self.base,
        );
    }

    /// Whether the jar holds a session cookie (not whether it is valid).
    pub fn has_session_cookie(&self) -> bool {
        self.cookie_header()
            .is_some_and(|c| c.split("; ").any(|kv| kv.starts_with("gelabber_session=")))
    }

    pub fn csrf_token(&self) -> Option<String> {
        self.csrf.read().clone()
    }

    /// The server's clock, from the last `Date` header (second precision).
    pub fn server_now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
            + chrono::Duration::milliseconds(self.clock_offset.load(Ordering::Relaxed))
    }

    /// Called when a request answers `401 unauthenticated`.
    pub fn set_unauthenticated_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.unauthenticated.write() = Some(hook);
    }

    /// Runs the hook of [`Self::set_unauthenticated_hook`] (a 401 that did
    /// not come over REST, e.g. on the gateway upgrade).
    pub(crate) fn report_unauthenticated(&self) {
        let hook = self.unauthenticated.read().clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    pub async fn request<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ApiError> {
        let body = match body {
            Some(body) => Some(Body::Json(
                serde_json::to_vec(body).map_err(|e| ApiError::Decode(e.to_string()))?,
            )),
            None => None,
        };
        let bytes = self.execute(method, path, body).await?;
        decode(path, &bytes)
    }

    /// Sends one request under `/api`, adds the CSRF header to writes and
    /// retries once after `403 csrf_invalid` with a fresh token, as the web.
    async fn execute(
        &self,
        method: Method,
        path: &str,
        body: Option<Body>,
    ) -> Result<Vec<u8>, ApiError> {
        self.execute_as(method, path, body, None).await
    }

    /// [`Self::execute`] with a session cookie that is no longer in the jar.
    async fn execute_as(
        &self,
        method: Method,
        path: &str,
        body: Option<Body>,
        session: Option<&str>,
    ) -> Result<Vec<u8>, ApiError> {
        let write = method != Method::GET && method != Method::HEAD;
        if write && self.csrf.read().is_none() {
            self.refresh_csrf().await?;
        }
        let mut retried = false;
        loop {
            let mut req = self
                .client
                .request(method.clone(), self.url(&format!("/api{path}")));
            if write && let Some(token) = self.csrf.read().clone() {
                req = req.header("X-CSRF-Token", token);
            }
            // The interface's language, as the browser sends its own: Klipy
            // picks the GIF locale from it.
            req = req.header(header::ACCEPT_LANGUAGE, accept_language());
            if let Some(session) = session {
                // A Cookie header of our own replaces the jar's: send both.
                let jar = self.cookie_header().map(|c| format!("{c}; "));
                let value = format!("{}gelabber_session={session}", jar.unwrap_or_default());
                req = req.header(header::COOKIE, value);
            }
            match &body {
                Some(Body::Json(json)) => {
                    req = req
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(json.clone());
                }
                Some(Body::Raw { content_type, data }) => {
                    req = req
                        .header(header::CONTENT_TYPE, content_type.as_str())
                        .body(data.clone());
                }
                None => {}
            }
            let res = req.send().await.map_err(ApiError::from_reqwest)?;
            self.track_clock(&res);
            let status = res.status();
            let wait = retry_after_header(res.headers());
            let bytes = res.bytes().await.map_err(ApiError::from_reqwest)?;
            if status.is_success() {
                self.remember_csrf(&bytes);
                return Ok(bytes.to_vec());
            }
            let error = envelope(status, &bytes, wait);
            if write && !retried && error.code() == "csrf_invalid" {
                retried = true;
                self.refresh_csrf().await?;
                continue;
            }
            if error.is_unauthenticated() {
                let hook = self.unauthenticated.read().clone();
                if let Some(hook) = hook {
                    hook();
                }
            }
            return Err(error);
        }
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

    /// A write without a body (leave, join, logout, deletes, …).
    pub async fn send_empty<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<T, ApiError> {
        self.request::<(), T>(method, path, None).await
    }

    /// Raw bytes as the body (avatar, banner, server icon).
    pub async fn send_bytes<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<T, ApiError> {
        let body = Body::Raw {
            content_type: content_type.to_owned(),
            data,
        };
        let bytes = self.execute(method, path, Some(body)).await?;
        decode(path, &bytes)
    }

    /// PUT of attachment bytes to a presigned URL with exactly the headers
    /// the presign answer listed. Absolute URLs (MinIO) go without cookies;
    /// the relative proxy URL goes with the session.
    pub async fn put_upload(
        &self,
        upload_url: &str,
        headers: &std::collections::BTreeMap<String, String>,
        data: Vec<u8>,
    ) -> Result<(), ApiError> {
        let same_origin = upload_url.starts_with('/');
        let client = if same_origin {
            &self.client
        } else {
            &self.plain
        };
        let target = if same_origin {
            self.url(upload_url)
        } else {
            upload_url.to_owned()
        };
        let mut req = client.put(target).timeout(UPLOAD_TIMEOUT);
        for (name, value) in headers {
            req = req.header(name.as_str(), value.as_str());
        }
        let res = req
            .body(data)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        if res.status().is_success() {
            Ok(())
        } else {
            Err(ApiError::Api {
                status: res.status().as_u16(),
                code: "upload_failed".into(),
                message: texts::error_upload_failed().into(),
                fields: serde_json::Value::Null,
                retry_after: None,
            })
        }
    }

    /// GET /auth/session; keeps its CSRF token for later writes.
    pub async fn session(&self) -> Result<serde_json::Value, ApiError> {
        let res = self
            .client
            .get(self.url("/api/auth/session"))
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        self.track_clock(&res);
        let status = res.status();
        let bytes = res.bytes().await.map_err(ApiError::from_reqwest)?;
        if !status.is_success() {
            return Err(envelope(status, &bytes, None));
        }
        let session: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::Decode(format!("/auth/session: {e}")))?;
        if let Some(token) = session.get("csrf_token").and_then(|t| t.as_str()) {
            *self.csrf.write() = Some(token.to_owned());
        }
        Ok(session)
    }

    /// Signs out on the server. The session cookie leaves the jar at once
    /// (so it is not kept across a restart either) and stays in memory only
    /// until the server confirms; until then a retry, also a plain one,
    /// repeats "everywhere" if that was asked for.
    pub async fn sign_out(&self, everywhere: bool) -> Result<CsrfResponse, ApiError> {
        let held = {
            let mut held = self.held.write();
            let everywhere = everywhere || held.as_ref().is_some_and(|h| h.everywhere);
            match self.session_cookie() {
                Some(cookie) => {
                    self.jar
                        .add_cookie_str("gelabber_session=; Path=/; Max-Age=0", &self.base); // i18n-ok
                    *held = Some(HeldSignOut { cookie, everywhere });
                }
                None => {
                    if let Some(h) = held.as_mut() {
                        h.everywhere = everywhere;
                    }
                }
            }
            held.clone()
        };
        let everywhere = everywhere || held.as_ref().is_some_and(|h| h.everywhere);
        let path = if everywhere {
            "/auth/logout-all"
        } else {
            "/auth/logout"
        };
        let cookie = held.as_ref().map(|h| h.cookie.as_str());
        let bytes = self.execute_as(Method::POST, path, None, cookie).await?;
        let mut current = self.held.write();
        if current.as_ref().map(|h| &h.cookie) == held.as_ref().map(|h| &h.cookie) {
            *current = None;
        }
        drop(current);
        decode(path, &bytes)
    }

    /// Whether the sign-out the server has not confirmed yet ends every
    /// session of the account (a retry repeats that variant).
    pub fn unconfirmed_sign_out_everywhere(&self) -> bool {
        self.held.read().as_ref().is_some_and(|h| h.everywhere)
    }

    /// A new sign-in: an old unconfirmed sign-out is not retried with it.
    pub fn forget_sign_out(&self) {
        *self.held.write() = None;
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

    /// Raw bytes of a resource (avatars, attachments, images), at most
    /// `kind`'s cap. Another origin (a member's avatar URL, a GIF CDN) gets
    /// neither the session cookie nor a redirect followed.
    pub async fn bytes(&self, path_or_url: &str, kind: Download) -> Result<Vec<u8>, ApiError> {
        let absolute = path_or_url.starts_with("http://") || path_or_url.starts_with("https://");
        let url = if absolute {
            path_or_url.to_owned()
        } else {
            self.url(path_or_url)
        };
        let same_origin = url::Url::parse(&url).is_ok_and(|u| u.origin() == self.base.origin());
        let client = if same_origin {
            &self.client
        } else {
            &self.foreign
        };
        // Attachments can be large: the upload timeout, not the REST one.
        let mut res = client
            .get(url)
            .timeout(UPLOAD_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = res.status();
        let max = kind.max_bytes();
        let too_large = || ApiError::Decode(texts::error_too_large(max));
        if res.content_length().is_some_and(|n| n > max as u64) {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(ApiError::from_reqwest)? {
            if bytes.len() + chunk.len() > max {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(envelope(status, &bytes, None));
        }
        Ok(bytes)
    }

    /// Any successful body with a `csrf_token` string replaces the token
    /// (web `rememberCsrf`): sign-in and sign-out rotate it.
    fn remember_csrf(&self, bytes: &[u8]) {
        if !bytes.windows(10).any(|w| w == b"csrf_token") {
            return;
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
            && let Some(token) = value.get("csrf_token").and_then(|t| t.as_str())
            && !token.is_empty()
        {
            *self.csrf.write() = Some(token.to_owned());
        }
    }

    fn track_clock(&self, res: &reqwest::Response) {
        let Some(date) = res
            .headers()
            .get(header::DATE)
            .and_then(|v| v.to_str().ok())
        else {
            return;
        };
        if let Ok(server) = chrono::DateTime::parse_from_rfc2822(date) {
            let offset = server.timestamp_millis() - chrono::Utc::now().timestamp_millis();
            self.clock_offset.store(offset, Ordering::Relaxed);
        }
    }
}

enum Body {
    Json(Vec<u8>),
    Raw { content_type: String, data: Vec<u8> },
}

fn decode<T: DeserializeOwned>(path: &str, bytes: &[u8]) -> Result<T, ApiError> {
    let bytes = if bytes.is_empty() {
        &b"null"[..]
    } else {
        bytes
    };
    serde_json::from_slice(bytes).map_err(|e| ApiError::Decode(format!("{path}: {e}")))
}

/// `Retry-After` in seconds; an HTTP date counts from now.
fn retry_after_header(headers: &header::HeaderMap) -> Option<f64> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<f64>() {
        return (seconds.is_finite() && seconds >= 0.0).then_some(seconds);
    }
    let at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let wait = (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_milliseconds();
    Some((wait.max(0) as f64) / 1000.0)
}

fn envelope(status: StatusCode, bytes: &[u8], header_wait: Option<f64>) -> ApiError {
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or(serde_json::Value::Null);
    // The body's number first, as the web reads it (api/client.ts).
    let retry_after = value
        .get("retry_after")
        .and_then(serde_json::Value::as_f64)
        .or(header_wait);
    ApiError::Api {
        retry_after,
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
            .unwrap_or_else(|| texts::error_status(status.as_u16())),
        fields: value
            .get("fields")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }
}

/// TLS for `/ws` and `/media/ws`, verified like REST (reqwest's
/// `rustls-platform-verifier`): a private CA in the OS store works for both.
pub fn ws_connector() -> Result<tokio_tungstenite::Connector, String> {
    static CONFIG: std::sync::OnceLock<Result<Arc<rustls::ClientConfig>, String>> =
        std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            use rustls_platform_verifier::BuilderVerifierExt;
            let provider = rustls::crypto::CryptoProvider::get_default()
                .cloned()
                .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| e.to_string())?
                .with_platform_verifier()
                .map(|b| Arc::new(b.with_no_client_auth()))
                .map_err(|e| e.to_string())
        })
        .clone()
        .map(tokio_tungstenite::Connector::Rustls)
}

/// `encodeURIComponent` for path segments (emoji, invite codes).
pub fn encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() * 3);
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A query string from pairs, encoded like `URLSearchParams` (`|`, `:` and
/// `+` in cursors are escaped).
pub fn query(pairs: &[(&str, Option<String>)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut any = false;
    for (key, value) in pairs {
        if let Some(value) = value {
            serializer.append_pair(key, value);
            any = true;
        }
    }
    if any {
        format!("?{}", serializer.finish())
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(encode_component("👍"), "%F0%9F%91%8D");
        assert_eq!(encode_component("abc-2"), "abc-2");
        assert_eq!(
            query(&[
                ("before", Some("2026-11-05T15:24:53.9+00:00|x".into())),
                ("limit", Some("50".into())),
                ("after", None),
            ]),
            "?before=2026-11-05T15%3A24%3A53.9%2B00%3A00%7Cx&limit=50"
        );
        assert_eq!(query(&[("a", None)]), "");
    }

    #[test]
    fn error_codes_map_like_the_web() {
        let api = |code: &str| ApiError::Api {
            status: 400,
            code: code.into(),
            message: String::new(),
            fields: serde_json::json!({"email": "invalid"}),
            retry_after: None,
        };
        assert_eq!(api("not_found").code(), "not_found");
        assert_eq!(api("weird").code(), "internal");
        assert_eq!(api("x").field("email"), Some("invalid"));
        assert_eq!(ApiError::Transport("x".into()).code(), "network");
        assert_eq!(ApiError::timeout("x").code(), "timeout");
        assert_eq!(api("banned").text(), "Du bist auf diesem Server gesperrt.");
    }

    #[test]
    fn rate_limits_say_how_long_to_wait() {
        let mut headers = header::HeaderMap::new();
        assert_eq!(retry_after_header(&headers), None);
        headers.insert(header::RETRY_AFTER, header::HeaderValue::from_static("8"));
        assert_eq!(retry_after_header(&headers), Some(8.0));
        let later = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc2822();
        headers.insert(header::RETRY_AFTER, later.parse().unwrap());
        let wait = retry_after_header(&headers).unwrap();
        assert!((28.0..=30.0).contains(&wait), "{wait}");
        headers.insert(
            header::RETRY_AFTER,
            header::HeaderValue::from_static("soon"),
        );
        assert_eq!(retry_after_header(&headers), None);
        let body = br#"{"error":"rate_limited","message":"Too many","retry_after":12}"#;
        let limited = envelope(StatusCode::TOO_MANY_REQUESTS, body, Some(3.0));
        assert_eq!(limited.code(), "rate_limited");
        assert_eq!(limited.retry_after(), Some(12.0));
        let header_only = envelope(
            StatusCode::TOO_MANY_REQUESTS,
            br#"{"error":"rate_limited"}"#,
            Some(3.0),
        );
        assert_eq!(header_only.retry_after(), Some(3.0));
        assert_eq!(ApiError::Transport("x".into()).retry_after(), None);
    }

    /// Serves each connection with `answer(request head)`; what came in is
    /// kept in the returned list.
    fn serve(
        answer: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static,
    ) -> (String, Arc<parking_lot::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let log = seen.clone();
        let answer = Arc::new(answer);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let (log, answer) = (log.clone(), answer.clone());
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    log.lock().push(head.clone());
                    let _ = stream.write_all(&answer(&head));
                });
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn ok_body(len: usize) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\nContent-Length: {len}\r\n\r\n"
        )
        .into_bytes();
        out.resize(out.len() + len, b'x');
        out
    }

    #[test]
    fn downloads_stop_at_their_cap() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cap = Download::Text.max_bytes();
        let (base, _) = serve(move |head| {
            if head.starts_with("GET /small") {
                ok_body(cap)
            } else {
                // Declares nothing: the cap must hold while reading.
                let mut out = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
                out.resize(out.len() + cap + 1, b'x');
                out
            }
        });
        let http = Http::new(&base).unwrap();
        let small = runtime.block_on(http.bytes("/small", Download::Text));
        assert_eq!(small.unwrap().len(), cap);
        let large = runtime.block_on(http.bytes("/large", Download::Text));
        assert!(large.is_err(), "{large:?}");
        // The same body is fine for a picture.
        let image = runtime.block_on(http.bytes("/large", Download::Image));
        assert_eq!(image.unwrap().len(), cap + 1);
    }

    #[test]
    fn a_foreign_download_carries_no_cookie_and_follows_no_redirect() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (api, api_seen) = serve(|_| ok_body(1));
        let target = format!("{api}/api/auth/logout");
        let (foreign, foreign_seen) = serve(move |_| {
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        });
        let http = Http::new(&api).unwrap();
        http.restore_session_cookie("secret");
        let answer =
            runtime.block_on(http.bytes(&format!("{foreign}/avatar.png"), Download::Image));
        assert!(answer.is_err(), "{answer:?}");
        let heads = foreign_seen.lock().clone();
        assert_eq!(heads.len(), 1);
        assert!(
            !heads[0].to_ascii_lowercase().contains("cookie"),
            "{}",
            heads[0]
        );
        assert!(api_seen.lock().is_empty());
        // The own origin still gets the session.
        runtime
            .block_on(http.bytes(&format!("{api}/api/avatars/x"), Download::Image))
            .unwrap();
        assert!(api_seen.lock()[0].contains("gelabber_session=secret"));
    }

    /// "Sign out everywhere" that the server did not confirm: the cookie is
    /// gone from the jar at once, and the retry (the strip's plain one) is
    /// again "everywhere", with the old session.
    #[test]
    fn an_unconfirmed_sign_out_everywhere_is_retried_as_one() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let failing = fail.clone();
        let (base, seen) = serve(move |head| {
            let json = |status: &str, body: &str| {
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .into_bytes()
            };
            if head.starts_with("GET ") {
                json("200 OK", r#"{"user":null,"csrf_token":"c"}"#)
            } else if failing.load(Ordering::SeqCst) {
                json("503 Service Unavailable", r#"{"error":"internal"}"#)
            } else {
                json("200 OK", r#"{"csrf_token":"d"}"#)
            }
        });
        let posts = || -> Vec<String> {
            seen.lock()
                .iter()
                .filter(|h| h.starts_with("POST"))
                .cloned()
                .collect()
        };
        let http = Http::new(&base).unwrap();
        http.restore_session_cookie("tok");
        assert!(runtime.block_on(http.sign_out(true)).is_err());
        assert!(http.unconfirmed_sign_out_everywhere());
        assert!(!http.has_session_cookie());
        assert_eq!(http.session_cookie(), None);
        fail.store(false, Ordering::SeqCst);
        runtime.block_on(http.sign_out(false)).unwrap();
        let sent = posts();
        assert_eq!(sent.len(), 2);
        for head in &sent {
            assert!(head.starts_with("POST /api/auth/logout-all "), "{head}");
            assert!(head.contains("gelabber_session=tok"), "{head}");
        }
        // Confirmed: nothing is held any more.
        assert!(!http.unconfirmed_sign_out_everywhere());
        runtime.block_on(http.sign_out(false)).unwrap();
        let last = posts().pop().unwrap();
        assert!(last.starts_with("POST /api/auth/logout "), "{last}");
        assert!(!last.contains("gelabber_session"), "{last}");
    }

    #[test]
    fn ws_url_follows_the_scheme() {
        let http = Http::new("http://127.0.0.1:5180").unwrap();
        assert_eq!(http.ws_url("/ws"), "ws://127.0.0.1:5180/ws");
        let https = Http::new("https://gelabber.example").unwrap();
        assert_eq!(https.ws_url("/media/ws"), "wss://gelabber.example/media/ws");
    }
}
