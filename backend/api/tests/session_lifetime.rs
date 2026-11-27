//! Sliding session lifetime (issue #195): every use pushes the row's expiry
//! to `now + API_SESSION_IDLE_HOURS`, never past `created_at` plus
//! `API_SESSION_TTL_HOURS` when that is set (by default it is not, GEL-83).
//! Requests renew through the session extractors, WebSocket activity
//! through the gateway; heartbeats do not count. A voice seat holds the
//! session past either end until the call is left.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use gelabber_api::{AppState, Config, app};
use serde_json::json;
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;

use common::ws::{self, Ws};
use common::{Client, SESSION};

/// 4 hours absolute, 1 hour idle: renewals are written in 5-minute steps.
fn lifetime_state(pool: PgPool, ttl_hours: &str, idle_hours: &str) -> AppState {
    lifetime_state_with(pool, Some(ttl_hours), idle_hours)
}

/// The default: no absolute lifetime, only the idle window.
fn idle_only_state(pool: PgPool, idle_hours: &str) -> AppState {
    lifetime_state_with(pool, None, idle_hours)
}

fn lifetime_state_with(pool: PgPool, ttl_hours: Option<&str>, idle_hours: &str) -> AppState {
    let redis = common::redis_url();
    let config = Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "API_LINK_PREVIEWS" => Some("false".to_owned()),
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(redis.clone()),
        "API_SESSION_TTL_HOURS" => ttl_hours.map(str::to_owned),
        "API_SESSION_IDLE_HOURS" => Some(idle_hours.to_owned()),
        "API_WS_HEARTBEAT_MS" => Some("40".to_owned()),
        "API_WS_DEAD_MS" => Some(common::WS_DEAD_MS.to_string()),
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

/// 400 days, the longest a browser keeps a cookie: the row decides.
const COOKIE_MAX_AGE: &str = "Max-Age=34560000";

/// The cookie outlives the row, whose end moves with use and waits for a
/// call (GEL-83), also with an absolute lifetime set.
#[sqlx::test]
async fn cookie_outlives_the_row(pool: PgPool) {
    let mut client = Client::with_state(lifetime_state(pool, "4", "1"));
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    let cookie = res.set_cookie(SESSION).expect("session cookie");
    assert!(cookie.contains(COOKIE_MAX_AGE), "{cookie}");
}

/// A request that moves the session's end sends the cookie again, so a
/// session in use never loses it; one within the renewal step does not.
#[sqlx::test]
async fn a_renewal_sends_the_cookie_again(pool: PgPool) {
    let mut client = signed_in(idle_only_state(pool.clone(), "1")).await;
    let token = client.jar[SESSION].clone();
    set_row(&pool, 600, 300).await;

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);
    let cookie = me.set_cookie(SESSION).expect("cookie sent again");
    assert!(
        cookie.starts_with(&format!("{SESSION}={token};")),
        "{cookie}"
    );
    assert!(cookie.contains(COOKIE_MAX_AGE), "{cookie}");

    let again = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.set_cookie(SESSION), None, "not renewed, not sent");
}

/// GEL-83: without `API_SESSION_TTL_HOURS` a session in use is never
/// signed out on a schedule; one past the old 30-day mark still renews.
#[sqlx::test]
async fn an_active_session_passes_the_old_thirty_day_mark(pool: PgPool) {
    let mut client = signed_in(idle_only_state(pool.clone(), "1")).await;
    set_row(&pool, 31 * 24 * 3600, 300).await;

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);
    let (_, expires_at) = row(&pool).await;
    assert_near(expires_at, Utc::now() + chrono::Duration::hours(1));
}

/// Without an absolute lifetime the idle window still ends a session.
#[sqlx::test]
async fn an_idle_session_ends_without_an_absolute_lifetime(pool: PgPool) {
    let mut client = signed_in(idle_only_state(pool.clone(), "1")).await;
    set_row(&pool, 7200, -1).await;

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
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

/// Mutations renew too, not only reads (they may resolve the session in the
/// rate-limit gate before the handler's extractor runs).
#[sqlx::test]
async fn a_mutation_slides_the_expiry(pool: PgPool) {
    let mut client = signed_in(lifetime_state(pool.clone(), "4", "1")).await;
    set_row(&pool, 600, 300).await;

    let res = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Renew" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
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

/// Lower settings apply to sessions that already exist: the API caps every
/// row at its start, since renewal only ever moves an expiry forward.
#[sqlx::test]
async fn shorter_settings_cap_existing_sessions(pool: PgPool) {
    let long = lifetime_state(pool.clone(), "48", "24");
    signed_in(long).await;
    let short = lifetime_state(pool.clone(), "4", "1");

    // Idle cap: in use for 2 hours, would run 20 more.
    set_row(&pool, 2 * 3600, 20 * 3600).await;
    let capped = gelabber_api::auth::session::cap_to_settings(
        &short.db,
        short.session_idle,
        short.session_ttl,
    )
    .await
    .unwrap();
    assert_eq!(capped, 1);
    let (_, expires_at) = row(&pool).await;
    assert_near(expires_at, Utc::now() + chrono::Duration::hours(1));

    // Absolute cap: 3.5 hours old, so half an hour left of 4.
    set_row(&pool, 7 * 1800, 50 * 60).await;
    gelabber_api::auth::session::cap_to_settings(&short.db, short.session_idle, short.session_ttl)
        .await
        .unwrap();
    let (_, expires_at) = row(&pool).await;
    assert_near(expires_at, Utc::now() + chrono::Duration::minutes(30));

    // Already within both: untouched.
    set_row(&pool, 600, 1800).await;
    let capped = gelabber_api::auth::session::cap_to_settings(
        &short.db,
        short.session_idle,
        short.session_ttl,
    )
    .await
    .unwrap();
    assert_eq!(capped, 0);
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
    ws::connect(addr, client).await
}

/// Answers server heartbeats for `span`; fails if the socket closes.
async fn stay_connected(ws: &mut Ws, span: Duration) {
    assert_eq!(ws::within(ws, span, |_| false).await, None);
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
    ws.close(None).await.ok();

    // 2 s short of one renewal step (5 minutes): the upgrade leaves the row
    // alone, and the socket's copy of the expiry is due within 2 s. A renewal
    // from the heartbeats after that would move the row.
    set_row(&pool, 600, 3600 - 300 + 2).await;
    let mut ws = connect(addr, &client).await;
    let (_, due) = row(&pool).await;
    stay_connected(&mut ws, Duration::from_secs(3)).await;
    assert_eq!(row(&pool).await.1, due, "heartbeats are not a use");

    // The same socket renews on activity, so the check above could see one.
    ws::send(&mut ws, json!({ "op": "p" })).await;
    for _ in 0..20 {
        stay_connected(&mut ws, Duration::from_millis(100)).await;
        if row(&pool).await.1 != due {
            break;
        }
    }
    assert_near(row(&pool).await.1, Utc::now() + chrono::Duration::hours(1));
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
                    ws::send(&mut ws, json!({ "op": "h" })).await;
                }
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "socket must close once the session expired");
}

/// The close frame's code when the session ended (GEL-83).
const SESSION_ENDED: u16 = 4401;

#[sqlx::test]
async fn the_close_frame_says_the_session_expired(pool: PgPool) {
    let state = lifetime_state(pool.clone(), "1", "1");
    let client = signed_in(state.clone()).await;
    let addr = serve(state).await;
    set_row(&pool, 3600 - 1, 1).await;
    let mut ws = connect(addr, &client).await;

    assert_eq!(
        ws::close_frame(&mut ws).await,
        Some((SESSION_ENDED, "expired".to_owned()))
    );
}

async fn create_voice(client: &mut Client) -> (String, String) {
    let res = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Call" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = res.body["id"].as_str().unwrap().to_owned();
    let res = client
        .send(
            Method::POST,
            &format!("/api/servers/{server}/channels"),
            Some(json!({ "name": "Lounge", "kind": "voice" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    (server, res.body["id"].as_str().unwrap().to_owned())
}

/// GEL-83: a session never ends while its socket holds a voice seat, not
/// even past the absolute end; the end waits until the call is left.
#[sqlx::test]
async fn a_voice_seat_holds_the_session_past_its_end(pool: PgPool) {
    let state = lifetime_state(pool.clone(), "1", "1");
    let mut client = signed_in(state.clone()).await;
    let (server, voice) = create_voice(&mut client).await;
    let addr = serve(state).await;
    // Past the absolute end of one hour, 3 seconds left: no use renews it.
    set_row(&pool, 3600 + 60, 3).await;
    let (_, end) = row(&pool).await;
    let mut ws = connect(addr, &client).await;
    ws::send(
        &mut ws,
        json!({ "op": "sig", "t": "j", "s": server, "c": voice }),
    )
    .await;
    ws::until(&mut ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    // The seat moves the end ahead, and the call goes on past the old one.
    let deadline = tokio::time::Instant::now() + ws::WAIT;
    loop {
        stay_connected(&mut ws, Duration::from_millis(100)).await;
        let (_, held) = row(&pool).await;
        if Utc::now() > end && held > Utc::now() + chrono::Duration::seconds(60) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the seat did not hold the session: it ends at {held}"
        );
    }
    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);

    // Left the call: nothing holds it any more, and the end comes.
    // The leave sends nothing back; the error answer to the frame after it
    // shows it was handled.
    ws::send(
        &mut ws,
        json!({ "op": "sig", "t": "l", "s": server, "c": voice }),
    )
    .await;
    ws::send(&mut ws, json!({ "op": "nope" })).await;
    ws::until(&mut ws, |f| f["op"] == "err").await;
    set_row(&pool, 3600 + 60, 1).await;
    assert_eq!(
        ws::close_frame(&mut ws).await,
        Some((SESSION_ENDED, "expired".to_owned()))
    );
}
