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

/// Per REST request, as the web (`REQUEST_TIMEOUT_MS`).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Uploads of attachment bytes (web `messages/api.ts`).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);

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

    /// The per-field codes, e.g. `{"email": "invalid"}`.
    pub fn field(&self, name: &str) -> Option<&str> {
        match self {
            Self::Api { fields, .. } => fields.get(name).and_then(|v| v.as_str()),
            _ => None,
        }
    }

    /// The German text the web shows for the code (`auth/rules.ts`
    /// `errorMessage`).
    pub fn german(&self) -> &'static str {
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

const TIMEOUT_PREFIX: &str = "Zeitüberschreitung: ";

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

/// German copy per error code, verbatim from web/src/auth/rules.ts.
pub fn error_message(code: &str) -> &'static str {
    match code {
        "validation_failed" => "Bitte die markierten Felder prüfen.",
        "unauthenticated" => "Bitte melde dich erneut an.",
        "invalid_credentials" => "E-Mail oder Passwort ist falsch.",
        "csrf_invalid" => "Sicherheits-Token abgelaufen. Bitte noch einmal versuchen.",
        "insecure_transport" => {
            "Anmelden geht nur über eine verschlüsselte Verbindung. Öffne Gelabber mit https:// (oder auf diesem Rechner über http://localhost)."
        }
        "forbidden" => "Dafür fehlt dir die Berechtigung.",
        "not_found" => "Das gibt es nicht (mehr).",
        "email_taken" => "Diese E-Mail-Adresse ist schon registriert.",
        "email_unverified" => {
            "Bitte bestätige zuerst deine E-Mail-Adresse. Wir haben dir gerade einen neuen Link geschickt."
        }
        "token_invalid" => "Dieser Link ist abgelaufen oder wurde schon benutzt.",
        "theme_conflict" => "Auf einem anderen Gerät geändert. Bitte erneut versuchen.",
        "invite_invalid" => "Diese Einladung ist abgelaufen oder aufgebraucht.",
        "banned" => "Du bist auf diesem Server gesperrt.",
        "rate_limited" => "Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen.",
        "quota_exceeded" => "Tageslimit für Uploads erreicht.",
        "upstream_unavailable" => {
            "Der GIF-Dienst antwortet gerade nicht. Bitte gleich noch einmal versuchen."
        }
        "upstream_quota" => {
            "Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte später noch einmal versuchen."
        }
        "network" => "Keine Verbindung zum Server.",
        "timeout" => "Der Server antwortet nicht. Bitte noch einmal versuchen.",
        _ => "Da ist etwas schiefgelaufen. Bitte noch einmal versuchen.",
    }
}

type Hook = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct Http {
    base: url::Url,
    client: reqwest::Client,
    /// Without cookies: presigned uploads go to another origin (MinIO).
    plain: reqwest::Client,
    jar: Arc<reqwest::cookie::Jar>,
    csrf: Arc<RwLock<Option<String>>>,
    /// Server clock minus local clock in ms, from each `Date` header.
    clock_offset: Arc<AtomicI64>,
    unauthenticated: Arc<RwLock<Option<Hook>>>,
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
        // Klipy picks the GIF locale from it, like the browser's.
        headers.insert(
            header::ACCEPT_LANGUAGE,
            header::HeaderValue::from_static("de-DE,de;q=0.9"),
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
        Ok(Self {
            base,
            client,
            plain,
            jar,
            csrf: Arc::new(RwLock::new(None)),
            clock_offset: Arc::new(AtomicI64::new(0)),
            unauthenticated: Arc::new(RwLock::new(None)),
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
            &format!("gelabber_session={value}; Path=/; HttpOnly{secure}"),
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
            let bytes = res.bytes().await.map_err(ApiError::from_reqwest)?;
            if status.is_success() {
                self.remember_csrf(&bytes);
                return Ok(bytes.to_vec());
            }
            let error = envelope(status, &bytes);
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
                message: "Der Upload ist fehlgeschlagen.".into(),
                fields: serde_json::Value::Null,
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
        // Attachments can be large: the upload timeout, not the REST one.
        let res = self
            .client
            .get(url)
            .timeout(UPLOAD_TIMEOUT)
            .send()
            .await
            .map_err(ApiError::from_reqwest)?;
        let status = res.status();
        let bytes = res.bytes().await.map_err(ApiError::from_reqwest)?;
        if !status.is_success() {
            return Err(envelope(status, &bytes));
        }
        Ok(bytes.to_vec())
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
        };
        assert_eq!(api("not_found").code(), "not_found");
        assert_eq!(api("weird").code(), "internal");
        assert_eq!(api("x").field("email"), Some("invalid"));
        assert_eq!(ApiError::Transport("x".into()).code(), "network");
        assert_eq!(ApiError::timeout("x").code(), "timeout");
        assert_eq!(
            api("banned").german(),
            "Du bist auf diesem Server gesperrt."
        );
    }

    #[test]
    fn ws_url_follows_the_scheme() {
        let http = Http::new("http://127.0.0.1:5180").unwrap();
        assert_eq!(http.ws_url("/ws"), "ws://127.0.0.1:5180/ws");
        let https = Http::new("https://gelabber.example").unwrap();
        assert_eq!(https.ws_url("/media/ws"), "wss://gelabber.example/media/ws");
    }
}
