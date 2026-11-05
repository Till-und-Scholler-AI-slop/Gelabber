//! 1:1 DMs as a channel kind: pair uniqueness, same message paths (issue #9).

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

async fn two_users(pool: PgPool) -> (Client, Client) {
    let mut ada = Client::new(pool.clone());
    ada.bootstrap().await;
    let res = ada.register("ada@example.com", "password123", "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let mut bob = Client::new(pool);
    bob.bootstrap().await;
    let res = bob.register("bob@example.com", "password123", "Bob").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    (ada, bob)
}

async fn third_user(pool: PgPool) -> Client {
    let mut cara = Client::new(pool);
    cara.bootstrap().await;
    let res = cara
        .register("cara@example.com", "password123", "Cara")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    cara
}

async fn open_dm(client: &mut Client, user_id: &str) -> (StatusCode, Value) {
    let res = client
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": user_id })),
        )
        .await;
    (res.status, res.body)
}

#[sqlx::test]
async fn dm_routes_require_a_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let id = "00000000-0000-0000-0000-000000000001";

    let res = client.send(Method::GET, "/api/dms", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let res = client
        .send(Method::POST, "/api/dms", Some(json!({ "user_id": id })))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let res = client
        .send(Method::GET, &format!("/api/dms/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn open_is_idempotent_and_order_independent(pool: PgPool) {
    let (mut ada, mut bob) = two_users(pool).await;

    let (status, first) = open_dm(&mut ada, bob.user_id()).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["kind"], "dm");
    assert_eq!(first["peer"]["id"], bob.user_id());
    assert_eq!(first["peer"]["name"], "Bob");
    assert!(first.get("server_id").is_none());

    let (status, again) = open_dm(&mut ada, bob.user_id()).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["id"], first["id"]);

    let (status, from_bob) = open_dm(&mut bob, ada.user_id()).await;
    assert_eq!(status, StatusCode::OK, "{from_bob}");
    assert_eq!(from_bob["id"], first["id"]);
    assert_eq!(from_bob["peer"]["id"], ada.user_id());
    assert_eq!(from_bob["peer"]["name"], "Ada");
}

#[sqlx::test]
async fn exactly_two_participants_and_self_rejected(pool: PgPool) {
    let (mut ada, bob) = two_users(pool.clone()).await;

    let self_dm = ada
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": ada.user_id() })),
        )
        .await;
    assert_eq!(self_dm.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(self_dm.body["fields"]["user_id"], "invalid");

    let missing = ada
        .send(Method::POST, "/api/dms", Some(json!({ "user_id": "" })))
        .await;
    assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing.body["fields"]["user_id"], "required");

    let unknown = ada
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": "00000000-0000-0000-0000-000000000099" })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    let (_, dm) = open_dm(&mut ada, bob.user_id()).await;
    let channel_id: uuid::Uuid = dm["id"].as_str().unwrap().parse().unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_members WHERE channel_id = $1")
            .bind(channel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
}

#[sqlx::test]
async fn list_and_get_are_scoped_to_participants(pool: PgPool) {
    let (mut ada, mut bob) = two_users(pool.clone()).await;
    let mut cara = third_user(pool).await;
    let (_, dm) = open_dm(&mut ada, bob.user_id()).await;
    let id = dm["id"].as_str().unwrap();

    let list = ada.send(Method::GET, "/api/dms", None).await;
    assert_eq!(list.status, StatusCode::OK);
    let rows = list.body.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], id);
    assert_eq!(rows[0]["peer"]["name"], "Bob");

    let bob_list = bob.send(Method::GET, "/api/dms", None).await;
    assert_eq!(bob_list.body.as_array().unwrap().len(), 1);
    assert_eq!(bob_list.body[0]["peer"]["name"], "Ada");

    let cara_list = cara.send(Method::GET, "/api/dms", None).await;
    assert_eq!(cara_list.body, json!([]));

    let got = ada.send(Method::GET, &format!("/api/dms/{id}"), None).await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.body["peer"]["id"], bob.user_id());

    let hidden = cara
        .send(Method::GET, &format!("/api/dms/{id}"), None)
        .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn messages_use_the_same_paths(pool: PgPool) {
    let (mut ada, mut bob) = two_users(pool.clone()).await;
    let mut cara = third_user(pool).await;
    let (_, dm) = open_dm(&mut ada, bob.user_id()).await;
    let channel_id = dm["id"].as_str().unwrap();

    let posted = ada
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "  hallo  " })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    assert_eq!(posted.body["content"], "hallo");
    assert_eq!(posted.body["channel_id"], channel_id);
    assert_eq!(posted.body["author"]["name"], "Ada");

    let page = bob
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page.body["messages"][0]["id"], posted.body["id"]);

    let edited = ada
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", posted.body["id"].as_str().unwrap()),
            Some(json!({ "content": "hallo, edit" })),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.body["content"], "hallo, edit");

    let stranger = cara
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(stranger.status, StatusCode::NOT_FOUND);

    let stranger_post = cara
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "nein" })),
        )
        .await;
    assert_eq!(stranger_post.status, StatusCode::NOT_FOUND);

    let deleted = ada
        .send(
            Method::DELETE,
            &format!("/api/messages/{}", posted.body["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
}

#[sqlx::test]
async fn dm_is_not_a_server_channel_kind(pool: PgPool) {
    let (mut ada, bob) = two_users(pool).await;
    let server = ada
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED);
    let server_id = server.body["id"].as_str().unwrap();

    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/channels"),
            Some(json!({ "name": "secret", "kind": "dm" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["kind"], "invalid");

    let (_, dm) = open_dm(&mut ada, bob.user_id()).await;
    let id = dm["id"].as_str().unwrap();

    let patch = ada
        .send(
            Method::PATCH,
            &format!("/api/channels/{id}"),
            Some(json!({ "name": "nope" })),
        )
        .await;
    assert_eq!(patch.status, StatusCode::NOT_FOUND);

    let delete = ada
        .send(Method::DELETE, &format!("/api/channels/{id}"), None)
        .await;
    assert_eq!(delete.status, StatusCode::NOT_FOUND);

    let detail = ada
        .send(Method::GET, &format!("/api/servers/{server_id}"), None)
        .await;
    let kinds: Vec<&str> = detail.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ch| ch["kind"].as_str().unwrap())
        .collect();
    assert!(!kinds.contains(&"dm"));
}
