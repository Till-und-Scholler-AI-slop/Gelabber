//! Server-side rate limits and upload quotas (issue #16).

mod common;

use std::collections::HashMap;

use axum::http::header::RETRY_AFTER;
use axum::http::{Method, StatusCode};
use gelabber_api::{AppState, Config};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

fn state_with(pool: PgPool, extra: &[(&str, &str)]) -> AppState {
    let extra: HashMap<String, String> = extra
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let config = Config::from_source(|key| {
        if let Some(value) = extra.get(key) {
            return Some(value.clone());
        }
        match key {
            "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
            "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
            "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
            _ => None,
        }
    })
    .expect("test config");
    AppState::with_pool(&config, pool).expect("state")
}

async fn create_server(client: &mut Client) -> Value {
    let res = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

fn text_channel_id(server: &Value) -> String {
    server["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .expect("text channel")["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[sqlx::test]
async fn auth_rate_limit_returns_429_with_code_and_retry_after(pool: PgPool) {
    let mut client = Client::from_state(state_with(pool, &[("API_RATE_AUTH_PER_MIN", "2")]));
    client.bootstrap().await;

    for i in 0..2 {
        let res = client
            .register(
                &format!("user{i}@example.com"),
                "password123",
                &format!("User{i}"),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    }

    let denied = client
        .register("user2@example.com", "password123", "User2")
        .await;
    assert_eq!(
        denied.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        denied.body
    );
    assert_eq!(denied.body["error"], "rate_limited");
    assert!(denied.body["retry_after"].as_u64().unwrap() >= 1);
    let retry = denied
        .headers
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .unwrap();
    assert!(retry.parse::<u64>().unwrap() >= 1);
}

#[sqlx::test]
async fn message_rate_limit_clears_without_a_hang(pool: PgPool) {
    let mut client = Client::from_state(state_with(pool, &[("API_RATE_MSG_PER_MIN", "1")]));
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = create_server(&mut client).await;
    let channel = text_channel_id(&server);

    let ok = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": "one" })),
        )
        .await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.body);

    let denied = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": "two" })),
        )
        .await;
    assert_eq!(
        denied.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        denied.body
    );
    assert_eq!(denied.body["error"], "rate_limited");
}

#[sqlx::test]
async fn daily_upload_quota_is_server_side(pool: PgPool) {
    let mut client = Client::from_state(state_with(
        pool,
        &[("API_UPLOAD_QUOTA_BYTES_PER_DAY", "100")],
    ));
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = create_server(&mut client).await;
    let channel = text_channel_id(&server);

    let ok = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({
                "filename": "a.txt",
                "content_type": "text/plain",
                "size": 80,
            })),
        )
        .await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.body);

    let denied = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({
                "filename": "b.txt",
                "content_type": "text/plain",
                "size": 40,
            })),
        )
        .await;
    assert_eq!(
        denied.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        denied.body
    );
    assert_eq!(denied.body["error"], "quota_exceeded");
}

#[sqlx::test]
async fn avatar_uploads_count_against_the_upload_rate(pool: PgPool) {
    let mut client = Client::from_state(state_with(pool, &[("API_RATE_UPLOAD_PER_HOUR", "2")]));
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let png = gelabber_api::image_size::fixtures::png(64, 64);
    for _ in 0..2 {
        let (status, body) = client
            .send_bytes(Method::PUT, "/api/me/avatar", "image/png", png.clone())
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = client
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", png)
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "rate_limited");
}

#[sqlx::test]
async fn avatar_and_banner_bytes_count_against_the_daily_quota(pool: PgPool) {
    let png = gelabber_api::image_size::fixtures::png(64, 64);
    let size = png.len() as i64;
    // Room for both images and ten bytes more.
    let quota = (2 * size + 10).to_string();
    let mut client = Client::from_state(state_with(
        pool.clone(),
        &[("API_UPLOAD_QUOTA_BYTES_PER_DAY", quota.as_str())],
    ));
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let uid: uuid::Uuid = client.user_id().parse().unwrap();

    for path in ["/api/me/avatar", "/api/me/banner"] {
        let (status, body) = client
            .send_bytes(Method::PUT, path, "image/png", png.clone())
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = client
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", png)
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "quota_exceeded");
    // Denied before its pending cleanup row or the PUT.
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM storage_cleanup")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pending, 0);

    // Attachments draw on the same day's bytes.
    let server = create_server(&mut client).await;
    let channel = text_channel_id(&server);
    let denied = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({
                "filename": "a.txt",
                "content_type": "text/plain",
                "size": 11,
            })),
        )
        .await;
    assert_eq!(
        denied.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        denied.body
    );
    assert_eq!(denied.body["error"], "quota_exceeded");

    let (reserved, consumed): (i64, i64) =
        sqlx::query_as("SELECT reserved, consumed FROM upload_daily_usage WHERE uploader_id = $1")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((reserved, consumed), (0, 2 * size));
}

/// A cookie shaped like a session token that no session has.
fn made_up_session() -> String {
    gelabber_api::token::generate()
}

#[sqlx::test]
async fn an_account_shares_its_windows_across_sessions(pool: PgPool) {
    let state = state_with(pool, &[("API_RATE_MSG_PER_MIN", "1")]);
    let mut first = Client::from_state(state.clone());
    first.bootstrap().await;
    let res = first
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let mut second = Client::from_state(state);
    second.bootstrap().await;
    let res = second.login("ada@example.com", "password123").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_ne!(
        first.jar.get(common::SESSION),
        second.jar.get(common::SESSION)
    );

    let server = create_server(&mut first).await;
    let channel = text_channel_id(&server);
    let path = format!("/api/channels/{channel}/messages");
    let ok = first
        .send(Method::POST, &path, Some(json!({ "content": "one" })))
        .await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.body);

    // Another session of the same account is no fresh allowance.
    let denied = second
        .send(Method::POST, &path, Some(json!({ "content": "two" })))
        .await;
    assert_eq!(
        denied.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        denied.body
    );
    assert_eq!(denied.body["error"], "rate_limited");
}

#[sqlx::test]
async fn made_up_session_cookies_share_the_address_window(pool: PgPool) {
    let state = state_with(pool, &[("API_RATE_API_PER_MIN", "2")]);
    let mut anon = Client::from_state(state.clone());
    anon.bootstrap().await;
    for _ in 0..2 {
        anon.jar
            .insert(common::SESSION.to_owned(), made_up_session());
        let res = anon
            .send(Method::POST, "/api/servers", Some(json!({ "name": "X" })))
            .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    }
    // A new cookie per request no longer opens a new window.
    anon.jar
        .insert(common::SESSION.to_owned(), made_up_session());
    let denied = anon
        .send(Method::POST, "/api/servers", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(
        denied.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        denied.body
    );

    // A signed-in user behind the same address keeps their own window.
    let mut ada = Client::from_state(state);
    ada.bootstrap().await;
    let res = ada.register("ada@example.com", "password123", "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    create_server(&mut ada).await;
}

#[sqlx::test]
async fn a_revoked_session_counts_against_the_address(pool: PgPool) {
    let state = state_with(pool, &[("API_RATE_API_PER_MIN", "1")]);
    let mut ada = Client::from_state(state.clone());
    ada.bootstrap().await;
    let res = ada.register("ada@example.com", "password123", "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let stale = ada.jar.get(common::SESSION).cloned().unwrap();
    let out = ada.send(Method::POST, "/api/auth/logout", None).await;
    assert!(out.status.is_success(), "{}", out.body);

    // Logout used the account's window; the dead cookie now uses the
    // address's, which is still open, and then is refused like any other.
    ada.jar.insert(common::SESSION.to_owned(), stale);
    let res = ada
        .send(Method::POST, "/api/servers", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    let denied = ada
        .send(Method::POST, "/api/servers", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(
        denied.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        denied.body
    );
}
