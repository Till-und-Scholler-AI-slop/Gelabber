//! Uploaded avatars (#208): stored and served by the API, replaced objects
//! deleted, foreign upload paths refused on `PATCH /api/me`.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;

use common::Client;
use gelabber_api::AppState;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really pixels";
const WEBP: &[u8] = b"RIFF\x10\0\0\0WEBPVP8 pixels";

async fn signed_in(state: &AppState, email: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, "password123", "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_serves_bytes_and_replaces_the_old_object(pool: PgPool) {
    let mut ada = signed_in(&common::state(pool), "ada@example.com").await;

    let (status, first) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_url = first["avatar_url"].as_str().unwrap().to_owned();
    assert!(first_url.starts_with(&format!("/api/avatars/{}/", ada.user_id())));

    let (status, headers, bytes) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["content-type"], "image/png");
    assert!(
        headers["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");

    // Saving the profile form sends the uploaded path back unchanged.
    let res = ada
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "name": "Ada L", "avatar_url": first_url })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["avatar_url"], first_url.as_str());

    let (status, second) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/webp", WEBP.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let second_url = second["avatar_url"].as_str().unwrap().to_owned();
    assert_ne!(second_url, first_url);
    let (status, _, _) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "replaced avatar is deleted");

    // Switching to an external URL deletes the upload as well.
    let res = ada
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "avatar_url": "https://cdn.example/a.png" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let (status, _, _) = ada.send_raw(Method::GET, &second_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn refuses_non_images_strangers_and_foreign_paths(pool: PgPool) {
    // One state: the in-memory store is per state.
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com").await;
    let (status, _) = ada
        .send_bytes(
            Method::PUT,
            "/api/me/avatar",
            "image/png",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = ada
        .send_bytes(
            Method::PUT,
            "/api/me/avatar",
            "image/png",
            [PNG, &vec![0; 512 * 1024]].concat(),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    let (_, uploaded) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    let url = uploaded["avatar_url"].as_str().unwrap().to_owned();

    let mut bob = signed_in(&state, "bob@example.com").await;
    let res = bob
        .send(Method::PATCH, "/api/me", Some(json!({ "avatar_url": url })))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    // Other signed-in users see it; signed-out requests do not.
    let (status, _, _) = bob.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::OK);
    let mut anonymous = Client::from_state(state);
    let (status, _, _) = anonymous.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = bob
        .send_raw(Method::GET, "/api/avatars/not-a-uuid/x", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
