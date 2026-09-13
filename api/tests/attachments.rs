//! Presign, type/size limits, bind-to-message, and auth-gated download
//! (issue #7). Bytes go through the in-memory store — no live MinIO.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

async fn two_users(pool: PgPool) -> (Client, Client) {
    let shared = common::state(pool);
    let mut owner = Client::from_state(shared.clone());
    owner.bootstrap().await;
    let res = owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let mut member = Client::from_state(shared);
    member.bootstrap().await;
    let res = member
        .register("member@example.com", "password123", "Bob")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    (owner, member)
}

async fn create_server(client: &mut Client, name: &str) -> Value {
    let res = client
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

async fn server_with_member(owner: &mut Client, member: &mut Client) -> Value {
    let server = create_server(owner, "Team").await;
    let id = server["id"].as_str().unwrap();
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let res = member
        .send(
            Method::POST,
            &format!(
                "/api/invites/{}/join",
                invite.body["code"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    server
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

async fn presign_ok(
    client: &mut Client,
    channel_id: &str,
    filename: &str,
    content_type: &str,
    size: i64,
) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({
                "filename": filename,
                "content_type": content_type,
                "size": size,
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

async fn put_object(client: &Client, attachment_id: &str, content_type: &str, bytes: &[u8]) {
    let key = format!("att/{attachment_id}");
    client
        .store
        .put(&key, content_type, bytes.to_vec())
        .await
        .expect("put");
}

#[sqlx::test]
async fn presign_requires_session_and_membership(pool: PgPool) {
    let mut anon = Client::new(pool.clone());
    anon.bootstrap().await;
    let id = "00000000-0000-0000-0000-000000000001";
    let res = anon
        .send(
            Method::POST,
            &format!("/api/channels/{id}/attachments"),
            Some(json!({
                "filename": "a.png",
                "content_type": "image/png",
                "size": 12,
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let (mut owner, mut stranger) = two_users(pool).await;
    let server = create_server(&mut owner, "Secret").await;
    let channel_id = text_channel_id(&server);
    let denied = stranger
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({
                "filename": "a.png",
                "content_type": "image/png",
                "size": 12,
            })),
        )
        .await;
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn type_and_size_are_enforced(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);

    let exe = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({
                "filename": "virus.exe",
                "content_type": "application/x-msdownload",
                "size": 12,
            })),
        )
        .await;
    assert_eq!(exe.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(exe.body["fields"]["content_type"], "invalid");

    let huge = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({
                "filename": "big.png",
                "content_type": "image/png",
                "size": 25 * 1024 * 1024 + 1,
            })),
        )
        .await;
    assert_eq!(huge.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(huge.body["fields"]["size"], "too_long");

    let empty = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({
                "filename": "",
                "content_type": "image/png",
                "size": 0,
            })),
        )
        .await;
    assert_eq!(empty.body["fields"]["filename"], "required");
    assert_eq!(empty.body["fields"]["size"], "required");
}

#[sqlx::test]
async fn send_files_is_required_to_presign_and_attach(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap();
    let channel_id = text_channel_id(&server);

    let stripped = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({ "member_permissions": ["send_messages", "join_voice"] })),
        )
        .await;
    assert_eq!(stripped.status, StatusCode::OK, "{}", stripped.body);

    let denied = member
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({
                "filename": "a.png",
                "content_type": "image/png",
                "size": 4,
            })),
        )
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.body["error"], "forbidden");
}

#[sqlx::test]
async fn message_carries_attachment_after_upload(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);
    let png = [0x89, b'P', b'N', b'G'];

    let signed = presign_ok(
        &mut owner,
        &channel_id,
        "cat.png",
        "image/png",
        png.len() as i64,
    )
    .await;
    assert!(signed["upload_url"].as_str().unwrap().contains("gelabber/"));
    assert_eq!(signed["headers"]["Content-Type"], "image/png");
    assert_eq!(signed["attachment"]["filename"], "cat.png");
    let attachment_id = signed["id"].as_str().unwrap().to_owned();

    // Claiming the id without putting the object is rejected.
    let missing = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({
                "content": "look",
                "attachment_ids": [attachment_id],
            })),
        )
        .await;
    assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing.body["fields"]["attachment_ids"], "invalid");

    put_object(&owner, &attachment_id, "image/png", &png).await;

    let posted = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({
                "content": "look",
                "attachment_ids": [attachment_id],
            })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    assert_eq!(posted.body["content"], "look");
    assert_eq!(posted.body["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(posted.body["attachments"][0]["id"], attachment_id);
    assert_eq!(posted.body["attachments"][0]["filename"], "cat.png");
    assert_eq!(posted.body["attachments"][0]["size"], png.len());

    let page = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(
        page.body["messages"][0]["attachments"][0]["id"],
        attachment_id
    );

    let (status, headers, bytes) = owner
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{attachment_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(bytes, png);

    // Re-binding the same upload is invalid.
    let reuse = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({
                "content": "again",
                "attachment_ids": [attachment_id],
            })),
        )
        .await;
    assert_eq!(reuse.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test]
async fn attachment_only_message_and_file_only_edit(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);
    let png = [1, 2, 3, 4];
    let signed = presign_ok(&mut owner, &channel_id, "a.png", "image/png", 4).await;
    let attachment_id = signed["id"].as_str().unwrap();
    put_object(&owner, attachment_id, "image/png", &png).await;

    let posted = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({
                "content": "   ",
                "attachment_ids": [attachment_id],
            })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    assert_eq!(posted.body["content"], "");
    assert_eq!(posted.body["attachments"].as_array().unwrap().len(), 1);

    let edited = owner
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", posted.body["id"].as_str().unwrap()),
            Some(json!({ "content": "" })),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(edited.body["attachments"][0]["filename"], "a.png");
}

#[sqlx::test]
async fn cannot_steal_someone_elses_upload(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let signed = presign_ok(&mut owner, &channel_id, "secret.png", "image/png", 3).await;
    let attachment_id = signed["id"].as_str().unwrap();
    put_object(&owner, attachment_id, "image/png", &[9, 8, 7]).await;

    let stolen = member
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({
                "content": "mine",
                "attachment_ids": [attachment_id],
            })),
        )
        .await;
    assert_eq!(stolen.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(stolen.body["fields"]["attachment_ids"], "invalid");

    let (status, _, _) = member
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{attachment_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn member_can_download_after_message_is_posted(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let png = [0x89, 1, 2, 3];
    let signed = presign_ok(&mut owner, &channel_id, "pic.png", "image/png", 4).await;
    let attachment_id = signed["id"].as_str().unwrap();
    put_object(&owner, attachment_id, "image/png", &png).await;
    let posted = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({
                "content": "pic",
                "attachment_ids": [attachment_id],
            })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);

    let (status, headers, bytes) = member
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{attachment_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(bytes, png);
}
