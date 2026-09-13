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
