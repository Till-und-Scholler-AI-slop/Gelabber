//! Sliding session lifetime (issue #195): every use pushes the row's expiry
//! to `now + API_SESSION_IDLE_HOURS`, never past `created_at` plus
//! `API_SESSION_TTL_HOURS`. Requests renew through the session extractors,
//! WebSocket activity through the gateway; heartbeats do not count.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::{AppState, Config, app};
use serde_json::json;
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::COOKIE;

use common::{Client, SESSION};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 4 hours absolute, 1 hour idle: renewals are written in 5-minute steps.
fn lifetime_state(pool: PgPool, ttl_hours: &str, idle_hours: &str) -> AppState {
    let redis = common::redis_url();
    let config = Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "API_LINK_PREVIEWS" => Some("false".to_owned()),
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(redis.clone()),
        "API_SESSION_TTL_HOURS" => Some(ttl_hours.to_owned()),
        "API_SESSION_IDLE_HOURS" => Some(idle_hours.to_owned()),
        "API_WS_HEARTBEAT_MS" => Some("40".to_owned()),
        "API_WS_DEAD_MS" => Some("5000".to_owned()),
        _ => None,
    })
    .expect("lifetime test config");
    AppState::with_pool(&config, pool).expect("state")
}

async fn signed_in(state: AppState) -> Client {
    let mut client = Client::with_state(state);
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn row(pool: &PgPool) -> (DateTime<Utc>, DateTime<Utc>) {
    sqlx::query_as("SELECT created_at, expires_at FROM sessions")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn set_row(pool: &PgPool, created_ago_secs: i64, expires_in_secs: i64) {
    sqlx::query(
        "UPDATE sessions SET created_at = now() - make_interval(secs => $1), \
                             expires_at = now() + make_interval(secs => $2)",
    )
    .bind(created_ago_secs as f64)
    .bind(expires_in_secs as f64)
    .execute(pool)
    .await
    .unwrap();
}

fn assert_near(actual: DateTime<Utc>, expected: DateTime<Utc>) {
    let off = (actual - expected).num_seconds().abs();
    assert!(off <= 5, "expected about {expected}, got {actual}");
}

#[sqlx::test]
async fn cookie_keeps_the_absolute_lifetime(pool: PgPool) {
    let mut client = Client::with_state(lifetime_state(pool, "4", "1"));
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    let cookie = res.set_cookie(SESSION).expect("session cookie");
    assert!(cookie.contains("Max-Age=14400"), "{cookie}");
}

#[sqlx::test]
async fn sign_in_starts_with_the_idle_window(pool: PgPool) {
    signed_in(lifetime_state(pool.clone(), "4", "1")).await;
    let (created_at, expires_at) = row(&pool).await;
    assert_near(expires_at, created_at + chrono::Duration::hours(1));
}

#[sqlx::test]
async fn a_request_slides_the_expiry(pool: PgPool) {
    let mut client = signed_in(lifetime_state(pool.clone(), "4", "1")).await;
    set_row(&pool, 600, 300).await;

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);
    let (_, expires_at) = row(&pool).await;
    assert_near(expires_at, Utc::now() + chrono::Duration::hours(1));
}

#[sqlx::test]
async fn renewal_never_passes_the_absolute_end(pool: PgPool) {
    let mut client = signed_in(lifetime_state(pool.clone(), "4", "1")).await;
    // Signed in 3 h 50 min ago: 10 minutes of absolute lifetime are left,
    // less than the idle window.
    set_row(&pool, 3 * 3600 + 50 * 60, 60).await;

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);
    let (created_at, expires_at) = row(&pool).await;
    assert_eq!(expires_at, created_at + chrono::Duration::hours(4));
}

#[sqlx::test]
async fn a_fresh_expiry_is_not_written_again(pool: PgPool) {
    let mut client = signed_in(lifetime_state(pool.clone(), "4", "1")).await;
    set_row(&pool, 600, 300).await;
    client.send(Method::GET, "/api/me", None).await;
    let (_, renewed) = row(&pool).await;

    // Within one renewal step (5 minutes here) requests read, never write.
    for _ in 0..3 {
        let me = client.send(Method::GET, "/api/me", None).await;
        assert_eq!(me.status, StatusCode::OK);
    }
    let (_, after) = row(&pool).await;
    assert_eq!(after, renewed);
}

#[sqlx::test]
async fn an_idle_session_ends_and_stays_ended(pool: PgPool) {
    let mut client = signed_in(lifetime_state(pool.clone(), "4", "1")).await;
    set_row(&pool, 7200, -1).await;
    let (_, expired) = row(&pool).await;

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    let res = client
        .send(Method::POST, "/api/auth/logout-all", Some(json!({})))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let (_, after) = row(&pool).await;
    assert_eq!(after, expired, "an expired row is never revived");
}

async fn serve(state: AppState) -> std::net::SocketAddr {
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
        .wait_ready(Duration::from_secs(2))
        .await
        .expect("redis pub/sub");
    addr
}

async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let mut request = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("ws request");
    let cookie = client.jar.get(SESSION).expect("session cookie");
    request.headers_mut().insert(
        COOKIE,
        format!("{SESSION}={cookie}")
            .parse()
            .expect("cookie header"),
    );
    let (ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .unwrap_or_else(|err| panic!("ws connect failed: {err}"));
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    ws
}

async fn send_json(ws: &mut Ws, value: serde_json::Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .expect("ws send");
}

/// Answers server heartbeats for `span`; fails if the socket closes.
async fn stay_connected(ws: &mut Ws, span: Duration) {
    let deadline = tokio::time::Instant::now() + span;
    loop {
        let next = tokio::time::timeout_at(deadline, ws.next()).await;
        match next {
            Err(_) => return,
            Ok(Some(Ok(Message::Text(text)))) => {
                if text.contains(r#""op":"h""#) {
                    send_json(ws, json!({ "op": "h" })).await;
                }
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => {
                panic!("socket closed early")
            }
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// Opening the socket is a use. Activity frames renew through the same
/// `touch` afterwards (unit test in `gateway::conn`); heartbeats do not.
#[sqlx::test]
async fn the_upgrade_renews_and_heartbeats_do_not(pool: PgPool) {
    let state = lifetime_state(pool.clone(), "4", "1");
    let client = signed_in(state.clone()).await;
    let addr = serve(state).await;
    set_row(&pool, 600, 120).await;
    let mut ws = connect(addr, &client).await;
    let (_, renewed) = row(&pool).await;
    assert_near(renewed, Utc::now() + chrono::Duration::hours(1));

    stay_connected(&mut ws, Duration::from_millis(300)).await;
    assert_eq!(row(&pool).await.1, renewed, "heartbeats are not a use");
}

#[sqlx::test]
async fn socket_outlives_an_expiry_that_was_renewed_elsewhere(pool: PgPool) {
    let state = lifetime_state(pool.clone(), "1", "1");
    let client = signed_in(state.clone()).await;
    let addr = serve(state).await;
    // One hour after sign-in minus a second: the upgrade cannot renew past
    // the absolute end, so the socket arms its timer one second ahead.
    set_row(&pool, 3600 - 1, 1).await;
    let mut ws = connect(addr, &client).await;

    // Another request (here: the database directly) extends the row.
    sqlx::query("UPDATE sessions SET expires_at = now() + interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    stay_connected(&mut ws, Duration::from_millis(2500)).await;
}

#[sqlx::test]
async fn socket_closes_when_the_session_runs_out(pool: PgPool) {
    let state = lifetime_state(pool.clone(), "1", "1");
    let client = signed_in(state.clone()).await;
    let addr = serve(state).await;
    set_row(&pool, 3600 - 1, 1).await;
    let mut ws = connect(addr, &client).await;

    let closed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(Message::Text(text))) if text.contains(r#""op":"h""#) => {
                    send_json(&mut ws, json!({ "op": "h" })).await;
                }
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "socket must close once the session expired");
}
