//! Shared in-process HTTP client for the integration tests: one router, a
//! cookie jar that replays `Set-Cookie` like a browser would, and the CSRF
//! token from the last JSON body that carried one.

// Each integration-test binary compiles this module on its own and uses a
// different subset of it.
#![allow(dead_code)]

use std::collections::BTreeMap;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::{CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use gelabber_api::storage::ObjectStore;
use gelabber_api::{AppState, Config, app};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

pub const SESSION: &str = "gelabber_session";
pub const CSRF: &str = "gelabber_csrf";

pub fn state(pool: PgPool) -> AppState {
    let config = Config::from_source(|key| match key {
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
        _ => None,
    })
    .expect("test config");
    AppState::with_pool(&config, pool).expect("state")
}

pub fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

/// Real Redis + short heartbeat so gateway tests finish quickly.
pub fn ws_state(pool: PgPool) -> AppState {
    let redis = redis_url();
    let config = Config::from_source(|key| match key {
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(redis.clone()),
        "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
        "API_WS_HEARTBEAT_MS" => Some("40".to_owned()),
        "API_WS_DEAD_MS" => Some("180".to_owned()),
        "API_WS_REPLAY" => Some("8".to_owned()),
        "API_WS_IDLE_MS" => Some("30000".to_owned()),
        "API_WS_PRESENCE_TTL_MS" => Some("2000".to_owned()),
        "API_WS_TYPING_TTL_MS" => Some("400".to_owned()),
        _ => None,
    })
    .expect("ws test config");
    AppState::with_pool(&config, pool).expect("state")
}

pub async fn serve_ws(pool: PgPool) -> (std::net::SocketAddr, AppState) {
    let state = ws_state(pool);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("local addr");
    let router = app(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    state
        .gateway
        .wait_ready(std::time::Duration::from_secs(2))
        .await
        .expect("redis pub/sub");
    (addr, state)
}

/// Like [`ws_state`] but with a short per-client idle so presence tests finish quickly.
pub fn ws_state_idle(pool: PgPool, idle_ms: u64) -> AppState {
    let redis = redis_url();
    let config = Config::from_source(|key| match key {
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(redis.clone()),
        "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
        "API_WS_HEARTBEAT_MS" => Some("40".to_owned()),
        "API_WS_DEAD_MS" => Some("2000".to_owned()),
        "API_WS_REPLAY" => Some("8".to_owned()),
        "API_WS_IDLE_MS" => Some(idle_ms.to_string()),
        "API_WS_PRESENCE_TTL_MS" => Some("2000".to_owned()),
        "API_WS_TYPING_TTL_MS" => Some("400".to_owned()),
        _ => None,
    })
    .expect("idle ws test config");
    AppState::with_pool(&config, pool).expect("state")
}

pub async fn serve_ws_idle(pool: PgPool, idle_ms: u64) -> (std::net::SocketAddr, AppState) {
    let state = ws_state_idle(pool, idle_ms);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("local addr");
    let router = app(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    state
        .gateway
        .wait_ready(std::time::Duration::from_secs(2))
        .await
        .expect("redis pub/sub");
    (addr, state)
}

pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Response {
    pub fn set_cookie(&self, name: &str) -> Option<String> {
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
pub struct Client {
    pub app: Router,
    pub store: ObjectStore,
    pub jar: BTreeMap<String, String>,
    pub csrf: Option<String>,
    pub user_id: Option<String>,
}

impl Client {
    pub fn new(pool: PgPool) -> Self {
        Self::with_state(state(pool))
    }

    pub fn with_state(state: AppState) -> Self {
        Self {
            app: app(state.clone()),
            store: state.store.clone(),
            jar: BTreeMap::new(),
            csrf: None,
            user_id: None,
        }
    }

    /// Same as [`Self::new`] but with a live Redis (ticket mint / gateway).
    pub fn with_redis(pool: PgPool) -> Self {
        Self::with_state(ws_state(pool))
    }

    pub async fn send(&mut self, method: Method, path: &str, body: Option<Value>) -> Response {
        self.send_with(method, path, body, |_| {}).await
    }

    /// Like [`send`] but keeps the raw body (redirects, file bytes).
    pub async fn send_raw(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
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
        let bytes = to_bytes(response.into_body(), 8 << 20)
            .await
            .unwrap()
            .to_vec();
        (status, headers, bytes)
    }

    pub async fn send_with(
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
        if let Some(id) = body
            .get("user")
            .and_then(|user| user.get("id"))
            .and_then(Value::as_str)
        {
            self.user_id = Some(id.to_owned());
        }

        Response {
            status,
            headers,
            body,
        }
    }

    /// `GET /api/auth/session`: what the web client does on first paint.
    pub async fn bootstrap(&mut self) -> Response {
        self.send(Method::GET, "/api/auth/session", None).await
    }

    pub async fn register(&mut self, email: &str, password: &str, name: &str) -> Response {
        self.send(
            Method::POST,
            "/api/auth/register",
            Some(json!({ "email": email, "password": password, "name": name })),
        )
        .await
    }

    pub fn user_id(&self) -> &str {
        self.user_id
            .as_deref()
            .expect("user id after register/login")
    }

    pub async fn login(&mut self, email: &str, password: &str) -> Response {
        self.send(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": password })),
        )
        .await
    }
}
