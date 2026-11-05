//! Server icons: uploaded by `manage_server`, served by the API, replaced,
//! removed and deleted with their server.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;
use gelabber_api::AppState;

/// A 64×64 PNG header (signature + IHDR): enough for the API's size check.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x40\0\0\0\x40\x08\x06\0\0\0\0\0\0\0";
/// A 64×64 lossy WebP header (`VP8 ` key frame).
const WEBP: &[u8] = b"RIFF\x16\0\0\0WEBPVP8 \x0a\0\0\0\x10\x02\x00\x9d\x01\x2a\x40\x00\x40\x00";
/// A 512×512 PNG header: twice the accepted edge.
const BIG_PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x02\0\0\0\x02\0\x08\x06\0\0\0\0\0\0\0";

async fn signed_in(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, "password123", name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn create_server(client: &mut Client) -> String {
    let res = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["icon_url"], Value::Null);
    res.body["id"].as_str().unwrap().to_owned()
}

async fn put_icon(client: &mut Client, server: &str, bytes: &[u8]) -> (StatusCode, Value) {
    client
        .send_bytes(
            Method::PUT,
            &format!("/api/servers/{server}/icon"),
            "image/png",
            bytes.to_vec(),
        )
        .await
}

async fn join(owner: &mut Client, member: &mut Client, server: &str) {
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let code = res.body["code"].as_str().unwrap().to_owned();
    let res = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

fn key_of(url: &str) -> String {
    format!(
        "server-icons/{}",
        url.strip_prefix("/api/server-icons/").unwrap()
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_serves_replaces_and_removes(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let server = create_server(&mut ada).await;

    let (status, first) = put_icon(&mut ada, &server, PNG).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_url = first["icon_url"].as_str().unwrap().to_owned();
    assert!(first_url.starts_with(&format!("/api/server-icons/{server}/")));
    assert_eq!(first["role"], "owner");

    let (status, headers, bytes) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["x-content-type-options"], "nosniff");

    // The list and the detail carry it.
    let res = ada.send(Method::GET, "/api/servers", None).await;
    assert_eq!(res.body[0]["icon_url"], first_url.as_str());
    let res = ada
        .send(Method::GET, &format!("/api/servers/{server}"), None)
        .await;
    assert_eq!(res.body["icon_url"], first_url.as_str());

    let (status, second) = put_icon(&mut ada, &server, WEBP).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let second_url = second["icon_url"].as_str().unwrap().to_owned();
    assert_ne!(second_url, first_url);
    let (status, _, _) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "replaced icon is deleted");

    let res = ada
        .send(Method::DELETE, &format!("/api/servers/{server}/icon"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["icon_url"], Value::Null);
    let (status, _, _) = ada.send_raw(Method::GET, &second_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "removed icon is deleted");
    assert!(state.store.head(&key_of(&second_url)).await.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn needs_manage_server_and_a_small_raster_image(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_in(&state, "bob@example.com", "Bob").await;
    let server = create_server(&mut ada).await;

    let (status, _) = put_icon(&mut bob, &server, PNG).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a stranger sees no server");

    join(&mut ada, &mut bob, &server).await;
    let (status, _) = put_icon(&mut bob, &server, PNG).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let res = bob
        .send(Method::DELETE, &format!("/api/servers/{server}/icon"), None)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    let (status, _) = put_icon(&mut ada, &server, b"<svg/>").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = put_icon(&mut ada, &server, BIG_PNG).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = put_icon(&mut ada, &server, &[PNG, &vec![0; 512 * 1024]].concat()).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // Granted the flag, a member may set it; every signed-in account,
    // member or not, may load it (invite previews).
    let res = ada
        .send(
            Method::PATCH,
            &format!("/api/servers/{server}"),
            Some(json!({ "member_permissions": ["manage_server", "send_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let (status, body) = put_icon(&mut bob, &server, PNG).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let url = body["icon_url"].as_str().unwrap().to_owned();
    let mut cleo = signed_in(&state, "cleo@example.com", "Cleo").await;
    let (status, _, _) = cleo.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::OK);
    let mut anonymous = Client::from_state(state);
    let (status, _, _) = anonymous.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn deleting_the_server_deletes_its_icon(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let server = create_server(&mut ada).await;
    let (_, body) = put_icon(&mut ada, &server, PNG).await;
    let key = key_of(body["icon_url"].as_str().unwrap());
    assert!(state.store.head(&key).await.is_ok());

    let res = ada
        .send(Method::DELETE, &format!("/api/servers/{server}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert!(state.store.head(&key).await.is_err());
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM storage_cleanup")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "nothing left for the worker");
}
