//! End-to-end auth flow against a real Postgres (`#[sqlx::test]` creates one
//! throw-away database per test from `DATABASE_URL` and applies
//! `api/migrations`). Requests are driven in-process; a tiny cookie jar
//! replays `Set-Cookie` like a browser would.

use std::collections::BTreeMap;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::{CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use gelabber_api::{AppState, Config, app};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const SESSION: &str = "gelabber_session";
const CSRF: &str = "gelabber_csrf";

fn state(pool: PgPool) -> AppState {
    let config = Config::from_source(|key| match key {
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
        _ => None,
    })
    .expect("test config");
    AppState::with_pool(&config, pool).expect("state")
}

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Response {
    fn set_cookie(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{name}=")))
            .map(str::to_owned)
    }
}

/// Browser stand-in: one router, a cookie jar, and the CSRF token from the
/// last JSON body that carried one.
struct Client {
    app: Router,
    jar: BTreeMap<String, String>,
    csrf: Option<String>,
}

impl Client {
    fn new(pool: PgPool) -> Self {
        Self {
            app: app(state(pool)),
            jar: BTreeMap::new(),
            csrf: None,
        }
    }

    async fn send(&mut self, method: Method, path: &str, body: Option<Value>) -> Response {
        self.send_with(method, path, body, |_| {}).await
    }

    async fn send_with(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
        tweak: impl FnOnce(&mut axum::http::request::Builder),
    ) -> Response {
        let mut builder = Request::builder().method(method.clone()).uri(path);
        if !self.jar.is_empty() {
            let cookie = self
                .jar
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header(COOKIE, cookie);
        }
        if method != Method::GET
            && let Some(token) = &self.csrf
        {
            builder = builder.header("x-csrf-token", token);
        }
        tweak(&mut builder);
        let request = match body {
            Some(json) => builder
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };

        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let body: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| panic!("non-JSON body: {}", String::from_utf8_lossy(&bytes)))
        };

        for raw in headers.get_all(SET_COOKIE).iter() {
            let raw = raw.to_str().unwrap();
            let (pair, attrs) = raw.split_once(';').unwrap_or((raw, ""));
            let (name, value) = pair.split_once('=').unwrap();
            if attrs.contains("Max-Age=0") || value.is_empty() {
                self.jar.remove(name);
            } else {
                self.jar.insert(name.to_owned(), value.to_owned());
            }
        }
        if let Some(token) = body.get("csrf_token").and_then(Value::as_str) {
            self.csrf = Some(token.to_owned());
        }

        Response {
            status,
            headers,
            body,
        }
    }

    /// `GET /api/auth/session`: what the web client does on first paint.
    async fn bootstrap(&mut self) -> Response {
        self.send(Method::GET, "/api/auth/session", None).await
    }

    async fn register(&mut self, email: &str, password: &str, name: &str) -> Response {
        self.send(
            Method::POST,
            "/api/auth/register",
            Some(json!({ "email": email, "password": password, "name": name })),
        )
        .await
    }

    async fn login(&mut self, email: &str, password: &str) -> Response {
        self.send(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": password })),
        )
        .await
    }
}

fn assert_session_cookie_attributes(raw: &str) {
    for attr in ["HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(raw.contains(attr), "{attr} missing in {raw}");
    }
    assert!(raw.contains("Max-Age=7200"), "ttl from config: {raw}");
    assert!(!raw.contains("Secure"), "Secure is opt-in via env: {raw}");
}

#[sqlx::test]
async fn session_bootstrap_issues_csrf_cookie_without_user(pool: PgPool) {
    let mut client = Client::new(pool);

    let first = client.bootstrap().await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.body["user"], Value::Null);
    let token = first.body["csrf_token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64);
    let cookie = first.set_cookie(CSRF).expect("csrf cookie issued");
    assert!(cookie.contains(&token));
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));

    // Second call reuses the cookie instead of rotating it.
    let second = client.bootstrap().await;
    assert_eq!(second.body["csrf_token"], token);
    assert!(second.set_cookie(CSRF).is_none());
}

#[sqlx::test]
async fn register_signs_in_and_sets_httponly_lax_cookies(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;

    let res = client
        .register("  Ada@Example.com ", "correct horse battery", " Ada ")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["user"]["email"], "ada@example.com");
    assert_eq!(res.body["user"]["name"], "Ada");
    assert_eq!(res.body["user"]["avatar_url"], Value::Null);
    assert!(res.body["user"]["id"].is_string());
    assert!(res.body["user"].get("password_hash").is_none());
    assert!(res.body["user"].get("password").is_none());

    let session_cookie = res.set_cookie(SESSION).expect("session cookie");
    assert_session_cookie_attributes(&session_cookie);
    let csrf_cookie = res.set_cookie(CSRF).expect("csrf rotated on register");
    assert!(csrf_cookie.contains(res.body["csrf_token"].as_str().unwrap()));

    // The browser is now "in": session resolves to the user.
    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["email"], "ada@example.com");

    let session = client.bootstrap().await;
    assert_eq!(session.body["user"]["email"], "ada@example.com");

    // Argon2id PHC string in the database, nothing resembling the password.
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(stored.starts_with("$argon2id$v=19$"), "{stored}");
    assert!(!stored.contains("correct horse"));

    // Only the hash of the session token is stored.
    let hashes: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(hashes.len(), 1);
    let raw_token = client.jar.get(SESSION).unwrap();
    assert_ne!(hashes[0], raw_token.as_bytes());
    assert_eq!(hashes[0].len(), 32);
}

#[sqlx::test]
async fn register_validates_inline(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    let res = client.register("nope", "short", "   ").await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["error"], "validation_failed");
    assert_eq!(res.body["fields"]["email"], "invalid");
    assert_eq!(res.body["fields"]["password"], "too_short");
    assert_eq!(res.body["fields"]["name"], "required");
    assert!(res.set_cookie(SESSION).is_none());

    let res = client
        .send(
            Method::POST,
            "/api/auth/register",
            Some(json!({ "email": "a@b.co" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["password"], "required");
    assert_eq!(res.body["fields"]["name"], "required");
}

#[sqlx::test]
async fn register_rejects_duplicate_email_case_insensitively(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let first = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(first.status, StatusCode::CREATED);

    let dup = client
        .register("ADA@example.com", "otherpassword", "Ada 2")
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT, "{}", dup.body);
    assert_eq!(dup.body["error"], "email_taken");
    assert_eq!(dup.body["fields"]["email"], "taken");
}

#[sqlx::test]
async fn login_logout_round_trip(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    client
        .register("ada@example.com", "password123", "Ada")
        .await;
    client.send(Method::POST, "/api/auth/logout", None).await;
    assert!(
        !client.jar.contains_key(SESSION),
        "logout clears the cookie"
    );

    let wrong = client.login("ada@example.com", "password124").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.body["error"], "invalid_credentials");

    let unknown = client.login("nobody@example.com", "password123").await;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        unknown.body, wrong.body,
        "unknown user is indistinguishable"
    );

    let ok = client.login("Ada@Example.com", "password123").await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert_eq!(ok.body["user"]["email"], "ada@example.com");
    assert_session_cookie_attributes(&ok.set_cookie(SESSION).unwrap());
    let session_token = client.jar.get(SESSION).unwrap().clone();

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);

    let out = client.send(Method::POST, "/api/auth/logout", None).await;
    assert_eq!(out.status, StatusCode::OK, "{}", out.body);
    let cleared = out.set_cookie(SESSION).unwrap();
    assert!(cleared.starts_with("gelabber_session=;"), "{cleared}");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(out.body["csrf_token"].is_string(), "csrf rotates on logout");
    assert!(!client.jar.contains_key(SESSION));

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    assert_eq!(me.body["error"], "unauthenticated");

    // The old token is dead server-side too, not just forgotten by the browser.
    client.jar.insert(SESSION.to_owned(), session_token);
    let replay = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn mutations_require_matching_csrf_header(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let cookie_token = client.jar.get(CSRF).unwrap().clone();

    // No header at all.
    client.csrf = None;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "csrf_invalid");

    // Header does not match the cookie.
    client.csrf = Some("0".repeat(64));
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body["error"], "csrf_invalid");

    // Header matches, but no cookie (token stolen without the cookie).
    client.csrf = Some(cookie_token.clone());
    client.jar.remove(CSRF);
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // Browser-flagged cross-site request with an otherwise valid pair.
    client.jar.insert(CSRF.to_owned(), cookie_token.clone());
    let res = client
        .send_with(
            Method::POST,
            "/api/auth/register",
            Some(json!({ "email": "ada@example.com", "password": "password123", "name": "Ada" })),
            |b| {
                let headers = b.headers_mut().unwrap();
                headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
            },
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body["error"], "csrf_invalid");

    // Matching pair: goes through.
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    // Authenticated mutations are covered as well.
    client.csrf = None;
    let res = client
        .send(Method::PATCH, "/api/me", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = client.send(Method::POST, "/api/auth/logout", None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // GET never needs the header.
    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[sqlx::test]
async fn profile_name_and_avatar_are_readable_and_editable(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    client
        .register("ada@example.com", "password123", "Ada")
        .await;

    let res = client
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "name": " Ada Lovelace ", "avatar_url": "https://cdn.example/ada.png" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "Ada Lovelace");
    assert_eq!(res.body["avatar_url"], "https://cdn.example/ada.png");

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.body["name"], "Ada Lovelace");
    assert_eq!(me.body["avatar_url"], "https://cdn.example/ada.png");

    // Partial patch leaves the other field alone; empty avatar clears it.
    let res = client
        .send(Method::PATCH, "/api/me", Some(json!({ "avatar_url": "" })))
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["name"], "Ada Lovelace");
    assert_eq!(res.body["avatar_url"], Value::Null);

    let res = client
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "name": "", "avatar_url": "javascript:alert(1)" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["name"], "required");
    assert_eq!(res.body["fields"]["avatar_url"], "invalid");

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(
        me.body["name"], "Ada Lovelace",
        "rejected patch changed nothing"
    );

    // Session shows the same profile the client would render after login.
    let session = client.bootstrap().await;
    assert_eq!(session.body["user"]["name"], "Ada Lovelace");
}

#[sqlx::test]
async fn profile_requires_a_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "unauthenticated");

    let res = client
        .send(Method::PATCH, "/api/me", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    // A forged cookie value is just "no session", never a 500.
    client
        .jar
        .insert(SESSION.to_owned(), "not-a-real-token".to_owned());
    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    client.jar.insert(SESSION.to_owned(), "f".repeat(64));
    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn malformed_json_is_a_json_error(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    let res = client
        .send_with(Method::POST, "/api/auth/login", None, |b| {
            b.headers_mut()
                .unwrap()
                .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        })
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.body);
    assert_eq!(res.body["error"], "bad_request");

    let res = client
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!("not an object")),
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.body["error"], "bad_request");
}
