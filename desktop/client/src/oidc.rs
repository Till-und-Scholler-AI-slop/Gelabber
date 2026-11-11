//! Sign-in through a provider from the desktop app (#518). The app signs in
//! in the system browser and gets the result back on a loopback port:
//!
//! 1. [`Loopback::bind`] takes a free port on `127.0.0.1`.
//! 2. `POST /api/auth/oidc/{provider}/start` with `desktop {port, state,
//!    challenge}` answers the provider's sign-in page.
//! 3. The system browser opens it; the provider sends the person to the
//!    API, which sends the browser on to
//!    `http://127.0.0.1:{port}/callback?state=…&code=…` (or `&error=…`).
//! 4. [`Loopback::wait`] takes that one request and answers a page that
//!    says the window can be closed.
//! 5. `POST /api/auth/oidc/desktop/exchange` trades the code, proven by the
//!    PKCE verifier only this app knows.
//!
//! Dropping the flow's future (the waiting card's "Abbrechen") drops the
//! listener and closes the port.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use gelabber_i18n::auth;

use crate::http::{ApiError, Http};
use crate::models::{OidcDesktop, OidcExchange, OidcStart};

/// How long the app waits for the browser to come back (the server keeps a
/// flow for as long).
pub const LOOPBACK_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// A connection that sends no request head within this is dropped (a
/// browser's speculative preconnect, a port scan).
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
/// The most of a request head the receiver reads.
const HEAD_MAX: usize = 8 * 1024;
/// The receiver's only path.
const CALLBACK_PATH: &str = "/callback";

/// A PKCE pair (RFC 7636, `S256`): the verifier stays in the app, the
/// challenge goes to the server with `start`.
#[derive(Clone, PartialEq, Eq)]
pub struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    /// A fresh pair: 32 random bytes as base64url without padding (43
    /// characters) and its challenge.
    pub fn new() -> Self {
        let verifier = random_token();
        let challenge = challenge(&verifier);
        Self {
            verifier,
            challenge,
        }
    }

    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkce")
            .field("verifier", &"<redacted>")
            .field("challenge", &self.challenge)
            .finish()
    }
}

/// The `S256` challenge of `verifier`: base64url(SHA-256(verifier)).
pub fn challenge(verifier: &str) -> String {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, verifier.as_bytes());
    B64.encode(digest.as_ref())
}

/// The app's own `state` for the loopback callback: another 32 random
/// bytes as base64url, 43 characters the server accepts.
pub fn app_state() -> String {
    random_token()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

/// What the browser brought back to the loopback receiver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callback {
    /// The result code for the exchange.
    Code(String),
    /// The flow failed before a result existed: `cancelled` or `failed`.
    Error(&'static str),
}

/// Why a provider flow ended without an answer from the exchange.
#[derive(Clone, Debug, thiserror::Error)]
pub enum OidcError {
    /// `start` or the exchange answered an error.
    #[error("{0}")]
    Api(#[from] ApiError),
    /// An `oidc_error` code: `cancelled` or `failed` from the callback, or
    /// `expired` when the browser did not come back in time.
    #[error("{}", flow_error_message(.0))]
    Flow(&'static str),
    /// The loopback port or the system browser failed; the detail is for
    /// the log.
    #[error("{}", auth::oidc_error_failed())]
    Local(String),
}

impl OidcError {
    /// The API's error code, or the `oidc_error` code of the flow.
    pub fn code(&self) -> &str {
        match self {
            Self::Api(error) => error.code(),
            Self::Flow(code) => code,
            Self::Local(_) => "failed",
        }
    }
}

/// The copy of an `oidc_error` the flow itself ends with (web
/// `oidcErrorMessage`).
fn flow_error_message(code: &str) -> &'static str {
    match code {
        "cancelled" => auth::oidc_error_cancelled(),
        "expired" => auth::oidc_error_expired(),
        _ => auth::oidc_error_failed(),
    }
}

/// A finished desktop flow: the exchange's answer, plus the code and
/// verifier that `complete` needs after a `needs_name`.
#[derive(Clone)]
pub struct OidcOutcome {
    pub exchange: OidcExchange,
    pub code: String,
    pub verifier: String,
}

impl std::fmt::Debug for OidcOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcOutcome")
            .field("exchange", &self.exchange)
            .field("code", &"<redacted>")
            .field("verifier", &"<redacted>")
            .finish()
    }
}

/// The one-shot receiver of the provider flow's result.
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl Loopback {
    /// A free port on `127.0.0.1`: never `localhost` (which may resolve to
    /// `::1`, where the server's redirect does not go) and never every
    /// interface.
    pub async fn bind() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    /// Waits for `GET /callback?state={state}&…`. Any other request gets a
    /// `404` and the receiver keeps waiting; the matching one gets a page
    /// saying the window can be closed (in the app's language), and then
    /// the listener closes. After `timeout`, `Flow("expired")`.
    pub async fn wait(self, state: &str, timeout: Duration) -> Result<Callback, OidcError> {
        let page = closing_page();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => return Err(OidcError::Flow("expired")),
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        connections.spawn(serve(stream, state.to_owned(), page.clone()));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "oidc loopback: accept failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                Some(served) = connections.join_next() => {
                    if let Ok(Some(callback)) = served {
                        return Ok(callback);
                    }
                }
            }
        }
    }
}

/// Answers one connection: the callback if it is the one, else a `404`.
async fn serve(mut stream: TcpStream, state: String, page: String) -> Option<Callback> {
    let head = tokio::time::timeout(CONNECTION_TIMEOUT, read_head(&mut stream))
        .await
        .ok()??;
    let callback = parse_callback(&head, &state);
    let response = match callback {
        Some(_) => response("200 OK", "text/html; charset=utf-8", &page), // i18n-ok
        None => response("404 Not Found", "text/plain; charset=utf-8", ""), // i18n-ok
    };
    let written = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    if written.is_err() {
        // The browser left; the result still counts, it is in the request.
        tracing::debug!("oidc loopback: the browser closed before the page");
    }
    callback
}

/// The request head up to its blank line, at most [`HEAD_MAX`] bytes. All
/// of it is read before answering, so closing does not reset the
/// connection under the browser.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::with_capacity(1024);
    let mut buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        head.extend_from_slice(&buf[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            return String::from_utf8(head).ok();
        }
        if head.len() > HEAD_MAX {
            return None;
        }
    }
}

/// The callback in a request head: `GET /callback?…` whose `state` is the
/// app's. A `code` is the result; else the `error` is `cancelled`, or
/// `failed` for anything else.
fn parse_callback(head: &str, expected_state: &str) -> Option<Callback> {
    let line = head.lines().next()?;
    let mut parts = line.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    if method != "GET" || !version.starts_with("HTTP/") || parts.next().is_some() {
        return None;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return None;
    }
    let first = |name: &str| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let state = first("state")?;
    if !same(&state, expected_state) {
        return None;
    }
    if let Some(code) = first("code").filter(|code| !code.is_empty()) {
        return Some(Callback::Code(code));
    }
    Some(Callback::Error(match first("error").as_deref() {
        Some("cancelled") => "cancelled",
        _ => "failed",
    }))
}

/// Compares in time that does not depend on where the strings differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}", // i18n-ok
        body.len()
    )
}

/// The page the system browser shows once the app has its result, in the
/// app's language (the account's, when signed in).
fn closing_page() -> String {
    let lang = gelabber_i18n::get().as_str();
    let text = escape(auth::oidc_loopback_done());
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Gelabber</title><style>body{{margin:0 16px;min-height:90vh;display:grid;place-items:center;font:16px/1.5 system-ui,sans-serif;color:#1f2328;background:#fff}}@media (prefers-color-scheme:dark){{body{{color:#e6e6e6;background:#1e1f22}}}}</style></head><body><p>{text}</p></body></html>" // i18n-ok
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Http {
    /// Runs a provider flow through the system browser: binds the loopback
    /// receiver, starts the flow, hands the provider's page to `open` (the
    /// app's `open_url`), waits up to `timeout` (normally
    /// [`LOOPBACK_TIMEOUT`]) for the browser to come back, and exchanges
    /// the code. A `signed_in` answer leaves the session in the jar.
    pub async fn oidc_desktop(
        &self,
        provider: &str,
        start: &OidcStart,
        timeout: Duration,
        open: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<OidcOutcome, OidcError> {
        let loopback = Loopback::bind()
            .await
            .map_err(|error| OidcError::Local(error.to_string()))?;
        let pkce = Pkce::new();
        let state = app_state();
        let desktop = OidcDesktop {
            port: loopback.port(),
            state: state.clone(),
            challenge: pkce.challenge().to_owned(),
        };
        let url = self.oidc_start(provider, start, &desktop).await?;
        open(&url).map_err(OidcError::Local)?;
        let code = match loopback.wait(&state, timeout).await? {
            Callback::Code(code) => code,
            Callback::Error(code) => return Err(OidcError::Flow(code)),
        };
        let exchange = self.oidc_exchange(&code, pkce.verifier()).await?;
        Ok(OidcOutcome {
            exchange,
            code,
            verifier: pkce.verifier,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use parking_lot::Mutex;

    const STATE: &str = "app-state-0123456789-0123456789-0123456789~x";

    #[test]
    fn challenge_matches_rfc_7636_appendix_b() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn pairs_and_states_are_fresh_url_safe_and_43_chars() {
        let url_safe = |s: &str| {
            s.len() == 43
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        };
        let pair = Pkce::new();
        assert!(url_safe(pair.verifier()), "{}", pair.verifier());
        assert!(url_safe(pair.challenge()), "{}", pair.challenge());
        assert_eq!(pair.challenge(), challenge(pair.verifier()));
        assert_ne!(pair.verifier(), Pkce::new().verifier());
        assert!(!format!("{pair:?}").contains(pair.verifier()));
        let outcome = OidcOutcome {
            exchange: OidcExchange::Linked(crate::models::SignInMethods {
                password: true,
                providers: Vec::new(),
            }),
            code: "one-time-code".into(),
            verifier: pair.verifier().to_owned(),
        };
        let shown = format!("{outcome:?}");
        assert!(!shown.contains("one-time-code") && !shown.contains(pair.verifier()));
        let state = app_state();
        assert!(url_safe(&state));
        assert_ne!(state, app_state());
    }

    #[test]
    fn only_the_callback_with_the_apps_state_is_taken() {
        let head = |line: &str| format!("{line}\r\nHost: 127.0.0.1\r\n\r\n");
        let code = format!("GET /callback?state={STATE}&code=abc HTTP/1.1");
        assert_eq!(
            parse_callback(&head(&code), STATE),
            Some(Callback::Code("abc".into()))
        );
        // The server encodes `~` as `%7E`.
        let encoded = "GET /callback?state=app-state-0123456789-0123456789-0123456789%7Ex&error=cancelled HTTP/1.1";
        assert_eq!(
            parse_callback(&head(encoded), STATE),
            Some(Callback::Error("cancelled"))
        );
        let failed = format!("GET /callback?state={STATE}&error=whatever HTTP/1.1");
        assert_eq!(
            parse_callback(&head(&failed), STATE),
            Some(Callback::Error("failed"))
        );
        for refused in [
            "GET /callback?state=other&code=abc HTTP/1.1".to_owned(),
            "GET /callback?code=abc HTTP/1.1".to_owned(),
            format!("POST /callback?state={STATE}&code=abc HTTP/1.1"),
            format!("GET /callback/x?state={STATE}&code=abc HTTP/1.1"),
            format!("GET /favicon.ico?state={STATE}&code=abc HTTP/1.1"),
            format!("GET /callback?state={STATE}x&code=abc HTTP/1.1"),
            "garbage".to_owned(),
        ] {
            assert_eq!(parse_callback(&head(&refused), STATE), None, "{refused}");
        }
    }

    /// Sends `request` to the receiver and returns the whole answer.
    async fn get(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.unwrap();
        answer
    }

    fn request(target: &str) -> String {
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/html\r\n\r\n")
    }

    #[tokio::test]
    async fn binds_only_to_ipv4_loopback() {
        let loopback = Loopback::bind().await.unwrap();
        let addr = loopback.local_addr().unwrap();
        assert_eq!(addr.ip(), std::net::IpAddr::from([127, 0, 0, 1]));
        assert_eq!(addr.port(), loopback.port());
        // The server takes ports from 1024 on.
        assert!(loopback.port() >= 1024);
    }

    #[tokio::test]
    async fn the_matching_callback_is_taken_and_the_rest_ignored() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port();
        let waiting = tokio::spawn(loopback.wait(STATE, Duration::from_secs(10)));

        // A preconnect that never sends anything holds nothing up.
        let _idle = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        for target in [
            "/favicon.ico".to_owned(),
            "/callback?state=wrong&code=stolen".to_owned(),
            format!("/other?state={STATE}&code=abc"),
        ] {
            let answer = get(port, &request(&target)).await;
            assert!(answer.starts_with("HTTP/1.1 404 "), "{target}: {answer}");
        }
        assert!(!waiting.is_finished());

        let answer = get(
            port,
            &request(&format!("/callback?state={STATE}&code=c0de")),
        )
        .await;
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
        assert!(answer.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(answer.contains("Cache-Control: no-store\r\n"));
        let callback = waiting.await.unwrap().unwrap();
        assert_eq!(callback, Callback::Code("c0de".into()));
        // One shot: the port is closed now.
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_page_speaks_the_apps_language() {
        for (locale, lang, text) in [
            (
                gelabber_i18n::Locale::De,
                "lang=\"de\"",
                "Du kannst dieses Fenster schließen und zu Gelabber zurückkehren.",
            ),
            (
                gelabber_i18n::Locale::En,
                "lang=\"en\"",
                "You can close this window and return to Gelabber.",
            ),
        ] {
            let _language = gelabber_i18n::on_this_thread(locale);
            let loopback = Loopback::bind().await.unwrap();
            let port = loopback.port();
            let request = request(&format!("/callback?state={STATE}&error=cancelled"));
            let (callback, answer) = tokio::join!(
                loopback.wait(STATE, Duration::from_secs(10)),
                get(port, &request),
            );
            assert_eq!(callback.unwrap(), Callback::Error("cancelled"));
            let body = answer.split_once("\r\n\r\n").unwrap().1;
            assert!(body.contains(lang), "{body}");
            assert!(body.contains(text), "{body}");
            let length = format!("Content-Length: {}\r\n", body.len());
            assert!(answer.contains(&length), "{answer}");
        }
    }

    #[tokio::test]
    async fn waiting_ends_as_expired() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port();
        let error = loopback
            .wait(STATE, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "expired");
        assert_eq!(
            error.to_string(),
            "Die Anmeldung ist abgelaufen oder wurde in einem anderen Browser gestartet. Bitte noch einmal versuchen."
        );
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }

    /// A stand-in API: `start` sends the "browser" straight back to the
    /// app's loopback port, `exchange` signs in. Keeps every request.
    struct FakeApi {
        base: String,
        seen: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    }

    async fn fake_api(callback: &'static str, exchange: &'static str) -> FakeApi {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    let (head, body) = loop {
                        let n = stream.read(&mut buf).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        data.extend_from_slice(&buf[..n]);
                        let text = String::from_utf8_lossy(&data).into_owned();
                        if let Some((head, body)) = text.split_once("\r\n\r\n") {
                            let length = head
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length: ")
                                        .map(|v| v.trim().parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            if body.len() >= length {
                                break (head.to_owned(), body.to_owned());
                            }
                        }
                    };
                    let line = head.lines().next().unwrap().to_owned();
                    let json: serde_json::Value =
                        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                    log.lock().push((line.clone(), json.clone()));
                    let answer = if line.starts_with("GET /api/auth/session") {
                        r#"{"user":null,"csrf_token":"csrf-anonymous"}"#.to_owned()
                    } else if line.starts_with("POST /api/auth/oidc/google/start") {
                        let desktop = &json["desktop"];
                        let url = format!(
                            "http://127.0.0.1:{}/callback?state={}&{callback}",
                            desktop["port"],
                            desktop["state"].as_str().unwrap()
                        );
                        serde_json::json!({ "url": url }).to_string()
                    } else {
                        exchange.to_owned()
                    };
                    // Only the exchange signs in.
                    let cookie = if line.contains("/exchange") {
                        "Set-Cookie: gelabber_session=fresh; Path=/; HttpOnly\r\n"
                    } else {
                        ""
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{cookie}Connection: close\r\n\r\n{answer}",
                        answer.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        FakeApi { base, seen }
    }

    /// The system browser: follows the provider's page to the loopback.
    fn browser(url: &str) -> Result<(), String> {
        let url = url.to_owned();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let parsed = url::Url::parse(&url).unwrap();
            let mut stream =
                std::net::TcpStream::connect(("127.0.0.1", parsed.port().unwrap())).unwrap();
            let target = format!("{}?{}", parsed.path(), parsed.query().unwrap());
            stream.write_all(request(&target).as_bytes()).unwrap();
            let mut answer = String::new();
            let _ = stream.read_to_string(&mut answer);
        });
        Ok(())
    }

    #[tokio::test]
    async fn a_desktop_sign_in_runs_start_browser_and_exchange() {
        let exchange = r#"{"result":"signed_in","user":null,"csrf_token":"csrf-signed-in"}"#;
        let api = fake_api("code=result-code", exchange).await;
        let http = Http::new(&api.base).unwrap();
        let mut start = OidcStart::new(crate::models::OidcPurpose::SignIn);
        start.redirect = Some("/invite/abc".into());
        let outcome = http
            .oidc_desktop("google", &start, Duration::from_secs(10), browser)
            .await
            .unwrap();
        assert!(matches!(outcome.exchange, OidcExchange::SignedIn(_)));
        assert_eq!(outcome.code, "result-code");
        // The exchange rotated CSRF and left the session in the jar.
        assert_eq!(http.csrf_token().as_deref(), Some("csrf-signed-in"));
        assert_eq!(http.session_cookie().as_deref(), Some("fresh"));

        let seen = api.seen.lock().clone();
        let (line, body) = &seen[1];
        assert!(
            line.starts_with("POST /api/auth/oidc/google/start "),
            "{line}"
        );
        assert_eq!(body["purpose"], "sign_in");
        assert_eq!(body["redirect"], "/invite/abc");
        assert!(body.get("current_password").is_none());
        let desktop = &body["desktop"];
        let port = desktop["port"].as_u64().unwrap();
        assert!(port >= 1024);
        assert_eq!(desktop["state"].as_str().unwrap().len(), 43);
        let (line, exchanged) = &seen[2];
        assert!(
            line.starts_with("POST /api/auth/oidc/desktop/exchange "),
            "{line}"
        );
        assert_eq!(exchanged["code"], "result-code");
        let verifier = exchanged["verifier"].as_str().unwrap();
        assert_eq!(verifier, outcome.verifier);
        assert_eq!(desktop["challenge"].as_str().unwrap(), challenge(verifier));
    }

    #[tokio::test]
    async fn a_cancelled_flow_skips_the_exchange() {
        let api = fake_api("error=cancelled", "{}").await;
        let http = Http::new(&api.base).unwrap();
        let start = OidcStart::new(crate::models::OidcPurpose::Link);
        let error = http
            .oidc_desktop("google", &start, Duration::from_secs(10), browser)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "cancelled");
        assert_eq!(error.to_string(), "Anmeldung abgebrochen.");
        let seen = api.seen.lock().clone();
        assert!(
            seen.iter().all(|(line, _)| !line.contains("/exchange")),
            "{seen:?}"
        );
    }

    #[tokio::test]
    async fn a_browser_that_cannot_open_fails_the_flow() {
        let api = fake_api("code=x", "{}").await;
        let http = Http::new(&api.base).unwrap();
        let start = OidcStart::new(crate::models::OidcPurpose::SignIn);
        let error = http
            .oidc_desktop("google", &start, Duration::from_secs(10), |_| {
                Err("no browser".into())
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "failed");
    }
}
