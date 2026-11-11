//! Signed-in devices (#519): `GET /api/auth/sessions` lists the account's
//! live sessions with their device, `DELETE /api/auth/sessions/{id}` signs
//! one out. The device comes from `User-Agent` at sign-in; `last_used_at`
//! moves with the sliding expiry.

mod common;

use std::time::Duration;

use axum::http::request::Builder;
use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use common::{Client, SESSION};
use futures_util::StreamExt;
use gelabber_api::{AppState, Config};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const FIREFOX_WINDOWS: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0";
const DESKTOP_LINUX: &str = "Gelabber-Desktop/0.10.0 (linux)";
const SAFARI_IOS: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) \
     AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1";

fn agent(ua: &'static str) -> impl FnOnce(&mut Builder) {
    move |req| {
        let taken = std::mem::take(req);
        *req = taken.header("user-agent", ua);
    }
}

/// A browser that sends `ua` on every request.
struct Device {
    client: Client,
    ua: &'static str,
}

impl Device {
    async fn send(&mut self, method: Method, path: &str, body: Option<Value>) -> common::Response {
        self.client
            .send_with(method, path, body, agent(self.ua))
            .await
    }

    async fn sessions(&mut self) -> Vec<Value> {
        let res = self.send(Method::GET, "/api/auth/sessions", None).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        res.body["sessions"].as_array().expect("sessions").clone()
    }
}

/// Registers ada from `ua`.
async fn register(state: &AppState, ua: &'static str) -> Device {
    let mut device = Device {
        client: Client::with_state(state.clone()),
        ua,
    };
    device.send(Method::GET, "/api/auth/session", None).await;
    let res = device
        .send(
            Method::POST,
            "/api/auth/register",
            Some(json!({ "email": "ada@example.com", "password": "password123", "name": "Ada" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    device
}

/// Signs `email` in from `ua`.
async fn login(state: &AppState, email: &str, ua: &'static str) -> Device {
    let mut device = Device {
        client: Client::with_state(state.clone()),
        ua,
    };
    device.send(Method::GET, "/api/auth/session", None).await;
    let res = device
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": "password123" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    device
}

fn session_hash(device: &Device) -> Vec<u8> {
    gelabber_api::token::hash(&device.client.jar[SESSION])
}

async fn set_last_used(pool: &PgPool, device: &Device, ago_secs: i64) {
    sqlx::query("UPDATE sessions SET last_used_at = now() - make_interval(secs => $2) WHERE token_hash = $1")
        .bind(session_hash(device))
        .bind(ago_secs as f64)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn the_list_names_each_device_this_one_first(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut firefox = register(&state, FIREFOX_WINDOWS).await;
    let mut desktop = login(&state, "ada@example.com", DESKTOP_LINUX).await;
    let safari = login(&state, "ada@example.com", SAFARI_IOS).await;
    // Safari was used most recently, the desktop app a day ago, Firefox
    // (the caller) a week ago: the caller still comes first.
    set_last_used(&pool, &firefox, 7 * 86_400).await;
    set_last_used(&pool, &desktop, 86_400).await;
    set_last_used(&pool, &safari, 60).await;
    // Another account's session is not listed.
    let mut other = Client::new(pool.clone());
    other.bootstrap().await;
    other
        .register("bob@example.com", "password123", "Bob")
        .await;

    let sessions = firefox.sessions().await;
    let devices: Vec<(&str, &str, bool)> = sessions
        .iter()
        .map(|s| {
            (
                s["client"].as_str().unwrap(),
                s["os"].as_str().unwrap(),
                s["current"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        devices,
        [
            ("firefox", "windows", true),
            ("safari", "ios", false),
            ("desktop", "linux", false),
        ]
    );
    for session in &sessions {
        let mut keys: Vec<&str> = session
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "client",
                "created_at",
                "current",
                "id",
                "last_used_at",
                "os"
            ],
            "{session}"
        );
        Uuid::parse_str(session["id"].as_str().unwrap()).unwrap();
    }

    // Each device sees itself as the current one.
    let sessions = desktop.sessions().await;
    assert_eq!(sessions[0]["client"], "desktop");
    assert_eq!(sessions[0]["current"], true);
}

#[sqlx::test]
async fn an_expired_session_is_not_listed(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut firefox = register(&state, FIREFOX_WINDOWS).await;
    let desktop = login(&state, "ada@example.com", DESKTOP_LINUX).await;
    sqlx::query(
        "UPDATE sessions SET expires_at = now() - interval '1 second' WHERE token_hash = $1",
    )
    .bind(session_hash(&desktop))
    .execute(&pool)
    .await
    .unwrap();

    let sessions = firefox.sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["current"], true);

    // Nor can it be signed out: it is gone already.
    let id: Uuid = sqlx::query_scalar("SELECT id FROM sessions WHERE token_hash = $1")
        .bind(session_hash(&desktop))
        .fetch_one(&pool)
        .await
        .unwrap();
    let res = firefox
        .send(Method::DELETE, &format!("/api/auth/sessions/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
}

#[sqlx::test]
async fn the_list_needs_a_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let res = client.send(Method::GET, "/api/auth/sessions", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "unauthenticated");
}

async fn connect(addr: std::net::SocketAddr, device: &Device) -> Ws {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    req.headers_mut().insert(
        "cookie",
        format!("{SESSION}={}", device.client.jar[SESSION])
            .parse()
            .unwrap(),
    );
    tokio_tungstenite::connect_async(req)
        .await
        .expect("connect")
        .0
}

async fn closed(ws: &mut Ws) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(frame) = ws.next().await {
            if matches!(frame, Ok(Message::Close(_)) | Err(_)) {
                return;
            }
        }
    })
    .await
    .expect("socket closed promptly");
}

#[sqlx::test]
async fn signing_out_another_device_ends_it_and_closes_its_socket(pool: PgPool) {
    let (addr, _socket_state) = common::serve_ws(pool.clone()).await;
    let state = common::ws_state(pool.clone());
    let mut firefox = register(&state, FIREFOX_WINDOWS).await;
    let mut desktop = login(&state, "ada@example.com", DESKTOP_LINUX).await;
    let mut socket = connect(addr, &desktop).await;

    let sessions = firefox.sessions().await;
    let id = sessions[1]["id"].as_str().unwrap().to_owned();
    assert_eq!(sessions[1]["client"], "desktop");
    let res = firefox
        .send(Method::DELETE, &format!("/api/auth/sessions/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert!(res.set_cookie(SESSION).is_none());

    closed(&mut socket).await;
    let res = desktop.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    // This device stays signed in.
    assert_eq!(firefox.sessions().await.len(), 1);
    let res = firefox.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[sqlx::test]
async fn signing_out_this_device_is_a_logout(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut firefox = register(&state, FIREFOX_WINDOWS).await;
    let mut desktop = login(&state, "ada@example.com", DESKTOP_LINUX).await;
    let id = firefox.sessions().await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let old_csrf = firefox.client.csrf.clone();

    let res = firefox
        .send(Method::DELETE, &format!("/api/auth/sessions/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let cleared = res.set_cookie(SESSION).expect("session cookie cleared");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(res.set_cookie(common::CSRF).is_some());
    let token = res.body["csrf_token"].as_str().expect("csrf_token");
    assert_ne!(Some(token.to_owned()), old_csrf);

    let res = firefox.send(Method::GET, "/api/auth/session", None).await;
    assert_eq!(res.body["user"], Value::Null);
    // The other device is untouched.
    assert_eq!(desktop.sessions().await.len(), 1);
}

#[sqlx::test]
async fn another_accounts_session_is_not_found(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = register(&state, FIREFOX_WINDOWS).await;
    let mut bob = Client::new(pool.clone());
    bob.bootstrap().await;
    bob.register("bob@example.com", "password123", "Bob").await;
    let bobs: Uuid = sqlx::query_scalar(
        "SELECT s.id FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE u.email = 'bob@example.com'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    for path in [
        format!("/api/auth/sessions/{bobs}"),
        format!("/api/auth/sessions/{}", Uuid::new_v4()),
        "/api/auth/sessions/not-a-uuid".to_owned(),
    ] {
        let res = ada.send(Method::DELETE, &path, None).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}: {}", res.body);
        assert_eq!(res.body["error"], "not_found");
    }
    let res = bob.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[sqlx::test]
async fn signing_out_a_device_needs_the_csrf_token(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut firefox = register(&state, FIREFOX_WINDOWS).await;
    let desktop = login(&state, "ada@example.com", DESKTOP_LINUX).await;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM sessions WHERE token_hash = $1")
        .bind(session_hash(&desktop))
        .fetch_one(&pool)
        .await
        .unwrap();

    firefox.client.csrf = None;
    let res = firefox
        .send(Method::DELETE, &format!("/api/auth/sessions/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "csrf_invalid");
    let live: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sessions WHERE id = $1)")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(live);
}

/// 4 hours absolute, 1 hour idle: renewals are written in 5-minute steps.
fn lifetime_state(pool: PgPool) -> AppState {
    let config = Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "API_LINK_PREVIEWS" => Some("false".to_owned()),
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(common::redis_url()),
        "API_SESSION_TTL_HOURS" => Some("4".to_owned()),
        "API_SESSION_IDLE_HOURS" => Some("1".to_owned()),
        _ => None,
    })
    .expect("lifetime test config");
    AppState::with_pool(&config, pool).expect("state")
}

async fn row(pool: &PgPool) -> (DateTime<Utc>, String, String) {
    sqlx::query_as("SELECT last_used_at, device_client, device_os FROM sessions")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Moves the session back: used `used_ago` seconds ago, expiring in
/// `expires_in` seconds.
async fn age(pool: &PgPool, used_ago: i64, expires_in: i64) {
    sqlx::query(
        "UPDATE sessions SET created_at = now() - interval '10 minutes', \
             last_used_at = now() - make_interval(secs => $1), \
             expires_at = now() + make_interval(secs => $2)",
    )
    .bind(used_ago as f64)
    .bind(expires_in as f64)
    .execute(pool)
    .await
    .unwrap();
}

fn assert_near(actual: DateTime<Utc>, expected: DateTime<Utc>) {
    let off = (actual - expected).num_seconds().abs();
    assert!(off <= 5, "expected about {expected}, got {actual}");
}

#[sqlx::test]
async fn last_use_moves_only_with_renewal(pool: PgPool) {
    let mut firefox = register(&lifetime_state(pool.clone()), FIREFOX_WINDOWS).await;

    // Within one renewal step of the full idle window: nothing is written.
    age(&pool, 120, 3600 - 60).await;
    let (before, _, _) = row(&pool).await;
    let res = firefox.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(row(&pool).await.0, before);

    // Due for renewal: the same write moves the last use to now.
    age(&pool, 1200, 300).await;
    let res = firefox.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let (after, client, os) = row(&pool).await;
    assert_near(after, Utc::now());
    assert_eq!((client.as_str(), os.as_str()), ("firefox", "windows"));
}

#[sqlx::test]
async fn an_unknown_device_is_filled_on_the_next_renewal(pool: PgPool) {
    let mut firefox = register(&lifetime_state(pool.clone()), FIREFOX_WINDOWS).await;
    // A session from before devices were recorded.
    sqlx::query("UPDATE sessions SET device_client = 'unknown', device_os = 'other'")
        .execute(&pool)
        .await
        .unwrap();

    // Not due: the device stays unknown.
    age(&pool, 120, 3600 - 60).await;
    firefox.send(Method::GET, "/api/me", None).await;
    let (_, client, os) = row(&pool).await;
    assert_eq!((client.as_str(), os.as_str()), ("unknown", "other"));

    // Due: the renewal stores this request's device.
    age(&pool, 1200, 300).await;
    firefox.send(Method::GET, "/api/me", None).await;
    let (_, client, os) = row(&pool).await;
    assert_eq!((client.as_str(), os.as_str()), ("firefox", "windows"));

    // A known device is never overwritten by a later request's.
    firefox.ua = SAFARI_IOS;
    age(&pool, 1200, 300).await;
    firefox.send(Method::GET, "/api/me", None).await;
    let (_, client, os) = row(&pool).await;
    assert_eq!((client.as_str(), os.as_str()), ("firefox", "windows"));
}
