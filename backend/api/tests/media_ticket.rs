//! Short SFU join tickets (issue 11). Session + join_voice + voice channel.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;

use common::{Client, redis_url};

async fn owner_with_voice_redis(pool: PgPool) -> (Client, String, String, String) {
    let mut owner = Client::with_redis(pool);
    owner.bootstrap().await;
    let res = owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED, "{}", server.body);
    let server_id = server.body["id"].as_str().unwrap().to_owned();
    let voice = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/channels"),
            Some(json!({ "name": "Lounge", "kind": "voice" })),
        )
        .await;
    assert_eq!(voice.status, StatusCode::CREATED, "{}", voice.body);
    let text_id = server.body["channels"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (
        owner,
        server_id,
        voice.body["id"].as_str().unwrap().to_owned(),
        text_id,
    )
}

async fn owner_with_voice(pool: PgPool) -> (Client, String, String, String) {
    let mut owner = Client::new(pool);
    owner.bootstrap().await;
    let res = owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED, "{}", server.body);
    let server_id = server.body["id"].as_str().unwrap().to_owned();
    let voice = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/channels"),
            Some(json!({ "name": "Lounge", "kind": "voice" })),
        )
        .await;
    assert_eq!(voice.status, StatusCode::CREATED, "{}", voice.body);
    let text_id = server.body["channels"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (
        owner,
        server_id,
        voice.body["id"].as_str().unwrap().to_owned(),
        text_id,
    )
}

#[sqlx::test]
async fn ticket_requires_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let res = client
        .send(
            Method::POST,
            "/api/channels/00000000-0000-0000-0000-000000000001/media-ticket",
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn ticket_rejects_text_channel(pool: PgPool) {
    let (mut owner, _, _, text_id) = owner_with_voice(pool).await;
    let res = owner
        .send(
            Method::POST,
            &format!("/api/channels/{text_id}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.body);
    assert_eq!(res.body["error"], "bad_request");
}

#[sqlx::test]
async fn ticket_hides_foreign_channel(pool: PgPool) {
    let (owner, _, voice_id, _) = owner_with_voice(pool.clone()).await;
    let mut stranger = Client::new(pool);
    stranger.bootstrap().await;
    stranger
        .register("other@example.com", "password123", "Eve")
        .await;
    let res = stranger
        .send(
            Method::POST,
            &format!("/api/channels/{voice_id}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let _ = owner;
}

#[sqlx::test]
async fn ticket_is_short_and_stored_in_redis(pool: PgPool) {
    let redis = match redis::Client::open(redis_url()) {
        Ok(client) => client,
        Err(_) => return,
    };
    if redis.get_multiplexed_async_connection().await.is_err() {
        return;
    }

    let (mut owner, _, voice_id, _) = owner_with_voice_redis(pool).await;
    let res = owner
        .send(
            Method::POST,
            &format!("/api/channels/{voice_id}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let ticket = res.body["ticket"].as_str().expect("ticket");
    assert_eq!(ticket.len(), 12);
    assert!(
        ticket
            .bytes()
            .all(|b| b"abcdefghjkmnpqrstuvwxyz23456789".contains(&b))
    );
    assert_eq!(res.body["media_path"], "/media/ws");
    assert_eq!(res.body["expires_in"], 30);
    assert!(!res.body.to_string().contains("livekit"));
    assert!(!res.body.to_string().contains("identity"));

    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let raw: Option<String> = redis::cmd("GET")
        .arg(format!("gb:mt:{ticket}"))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(raw.is_some(), "ticket should be in Redis");
}
