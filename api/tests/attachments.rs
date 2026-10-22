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

/// Smallest bytes that pass the content check for `image/png`.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

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
    let png = PNG;

    let signed = presign_ok(
        &mut owner,
        &channel_id,
        "cat.png",
        "image/png",
        png.len() as i64,
    )
    .await;
    // The memory store has no URL of its own: uploads go through the API.
    assert_eq!(
        signed["upload_url"],
        format!("/api/attachments/{}/upload", signed["id"].as_str().unwrap())
    );
    assert_eq!(signed["headers"]["Content-Type"], "image/png");
    assert_eq!(signed["headers"]["Content-Length"], png.len().to_string());
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

    put_object(&owner, &attachment_id, "image/png", png).await;

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
    let png = PNG;
    let signed = presign_ok(
        &mut owner,
        &channel_id,
        "a.png",
        "image/png",
        PNG.len() as i64,
    )
    .await;
    let attachment_id = signed["id"].as_str().unwrap();
    put_object(&owner, attachment_id, "image/png", png).await;

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
async fn jpeg_png_and_webp_bind_for_the_other_member(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let images: &[(&str, &str, &[u8])] = &[
        ("cat.jpg", "image/jpeg", &[0xff, 0xd8, 0xff, 0xd9]),
        ("cat.png", "image/png", PNG),
        ("cat.webp", "image/webp", b"RIFF\x04\0\0\0WEBP"),
    ];
    for (filename, content_type, bytes) in images {
        let signed = presign_ok(
            &mut owner,
            &channel_id,
            filename,
            content_type,
            bytes.len() as i64,
        )
        .await;
        let attachment_id = signed["id"].as_str().unwrap();
        put_object(&owner, attachment_id, content_type, bytes).await;
        let posted = owner
            .send(
                Method::POST,
                &format!("/api/channels/{channel_id}/messages"),
                Some(json!({
                    "content": filename,
                    "attachment_ids": [attachment_id],
                })),
            )
            .await;
        assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
        assert_eq!(posted.body["attachments"][0]["content_type"], *content_type);
        assert_eq!(posted.body["attachments"][0]["size"], bytes.len());

        let (status, headers, body) = member
            .send_raw(
                Method::GET,
                &format!("/api/attachments/{attachment_id}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], *content_type);
        assert_eq!(body, *bytes);
    }
}

#[sqlx::test]
async fn mismatched_upload_is_dropped_and_not_referenced(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let signed = presign_ok(&mut owner, &channel_id, "cat.jpg", "image/jpeg", 8).await;
    let attachment_id = signed["id"].as_str().unwrap();
    // Declared 8 bytes, stored 3: truncated upload must not become a message.
    put_object(&owner, attachment_id, "image/jpeg", &[1, 2, 3]).await;

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
    assert_eq!(posted.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(posted.body["fields"]["size"], "invalid");

    let page = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body["messages"].as_array().unwrap().is_empty());

    let (owner_status, _, _) = owner
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{attachment_id}"),
            None,
        )
        .await;
    assert_eq!(owner_status, StatusCode::NOT_FOUND);
    let (member_status, _, _) = member
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{attachment_id}"),
            None,
        )
        .await;
    assert_eq!(member_status, StatusCode::NOT_FOUND);
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
    let png = PNG;
    let signed = presign_ok(
        &mut owner,
        &channel_id,
        "pic.png",
        "image/png",
        PNG.len() as i64,
    )
    .await;
    let attachment_id = signed["id"].as_str().unwrap();
    put_object(&owner, attachment_id, "image/png", png).await;
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

/// `?download=1` saves the file instead of showing it (#201 review): the
/// picture link stays `inline`, the variant says `attachment`, with the exact
/// name as `filename*` and a quote-free ASCII fallback.
#[sqlx::test]
async fn download_variant_asks_to_save_the_file(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let png = PNG;
    let name = "Bär \"1\".png";
    let signed = presign_ok(&mut owner, &channel_id, name, "image/png", PNG.len() as i64).await;
    let attachment_id = signed["id"].as_str().unwrap();
    put_object(&owner, attachment_id, "image/png", png).await;
    let posted = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "", "attachment_ids": [attachment_id] })),
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
    assert_eq!(
        headers["content-disposition"],
        "inline; filename=\"B_r _1_.png\"; filename*=UTF-8''B%C3%A4r%20%221%22.png"
    );
    assert_eq!(headers["vary"], "Cookie");
    assert_eq!(bytes, png);

    let (status, headers, bytes) = member
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{attachment_id}?download=1"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"B_r _1_.png\"; filename*=UTF-8''B%C3%A4r%20%221%22.png"
    );
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(bytes, png);
}

async fn upload(
    client: &mut Client,
    channel_id: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> String {
    let signed = presign_ok(
        client,
        channel_id,
        filename,
        content_type,
        bytes.len() as i64,
    )
    .await;
    let id = signed["id"].as_str().unwrap().to_owned();
    put_object(client, &id, content_type, bytes).await;
    id
}

async fn post_files(client: &mut Client, channel_id: &str, ids: &[String]) -> (StatusCode, Value) {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "", "attachment_ids": ids })),
        )
        .await;
    (res.status, res.body)
}

/// Issue #255: several files in one message, in the order they were sent.
#[sqlx::test]
async fn three_images_and_a_pdf_in_one_message(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let mut ids = Vec::new();
    for name in ["a.png", "b.png", "c.png"] {
        ids.push(upload(&mut owner, &channel_id, name, "image/png", PNG).await);
    }
    ids.push(
        upload(
            &mut owner,
            &channel_id,
            "doc.pdf",
            "application/pdf",
            b"%PDF-1.7\n",
        )
        .await,
    );
    let (status, body) = post_files(&mut owner, &channel_id, &ids).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let names: Vec<&str> = body["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["filename"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a.png", "b.png", "c.png", "doc.pdf"]);

    // A PDF is only ever a download, also from the display link.
    let (status, headers, _) = member
        .send_raw(Method::GET, &format!("/api/attachments/{}", ids[3]), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/pdf");
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"doc.pdf\""
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
}

#[sqlx::test]
async fn eleventh_file_and_oversized_message_are_rejected(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);
    let eleven: Vec<String> = (0..11).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let (status, body) = post_files(&mut owner, &channel_id, &eleven).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["fields"]["attachment_ids"], "too_long");

    // Five files of 21 MiB: each is allowed, together over 100 MiB.
    let mut big = PNG.to_vec();
    big.resize(21 * 1024 * 1024, 0);
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            upload(
                &mut owner,
                &channel_id,
                &format!("{i}.png"),
                "image/png",
                &big,
            )
            .await,
        );
    }
    let (status, body) = post_files(&mut owner, &channel_id, &ids).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["fields"]["attachment_ids"], "too_large");
    let (status, body) = post_files(&mut owner, &channel_id, &ids[..4]).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[sqlx::test]
async fn declared_type_must_match_the_bytes(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);
    let good = upload(&mut owner, &channel_id, "ok.png", "image/png", PNG).await;
    let bad = upload(
        &mut owner,
        &channel_id,
        "cat.png",
        "image/png",
        b"MZ\x90\0not a png",
    )
    .await;
    let (status, body) = post_files(&mut owner, &channel_id, &[good, bad]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    // The field names the position of the rejected file.
    assert_eq!(body["fields"]["attachment_ids.1"], "mismatch", "{body}");
    let page = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert!(page.body["messages"].as_array().unwrap().is_empty());
}

#[sqlx::test]
async fn renamed_html_is_served_as_plain_text_download(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let html = b"<!doctype html><script>alert(1)</script>";
    let id = upload(&mut owner, &channel_id, "notes.txt", "text/plain", html).await;
    let (status, body) = post_files(&mut owner, &channel_id, std::slice::from_ref(&id)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, headers, bytes) = member
        .send_raw(Method::GET, &format!("/api/attachments/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/plain");
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"notes.txt\""
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(bytes, html);

    // text/html itself never gets a presign.
    let res = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({ "filename": "x.html", "content_type": "text/html", "size": 4 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test]
async fn video_and_audio_play_inline(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let webm = [0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81];
    let ogg = b"OggS\0\x02\0\0";
    let video = upload(&mut owner, &channel_id, "clip.webm", "video/webm", &webm).await;
    let audio = upload(&mut owner, &channel_id, "voice.ogg", "audio/ogg", ogg).await;
    let (status, body) = post_files(&mut owner, &channel_id, &[video.clone(), audio.clone()]).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    for (id, kind, name) in [
        (&video, "video/webm", "clip.webm"),
        (&audio, "audio/ogg", "voice.ogg"),
    ] {
        let (status, headers, _) = member
            .send_raw(Method::GET, &format!("/api/attachments/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], kind);
        assert_eq!(
            headers["content-disposition"],
            format!("inline; filename=\"{name}\"")
        );
    }
}

/// Review of #255: Excel CSVs on German systems are Windows-1252, its
/// "Unicode text" is UTF-16; both are text.
#[sqlx::test]
async fn text_in_other_encodings_is_accepted(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);
    let latin = upload(
        &mut owner,
        &channel_id,
        "preise.csv",
        "text/csv",
        b"Gr\xF6\xDFe;Preis\n",
    )
    .await;
    let utf16 = upload(
        &mut owner,
        &channel_id,
        "liste.txt",
        "text/plain",
        b"\xFF\xFEa\0;\0b\0",
    )
    .await;
    let (status, body) = post_files(&mut owner, &channel_id, &[latin, utf16]).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// Review of #255: up to ten files go up one after another; the first must
/// still bind after a slow set, not only within its own upload link.
#[sqlx::test]
async fn pending_upload_outlives_its_upload_link(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);
    let id = upload(&mut owner, &channel_id, "a.png", "image/png", PNG).await;
    let left: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - now())::float8 FROM attachments WHERE id = $1",
    )
    .bind(uuid::Uuid::parse_str(&id).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(left > 3000.0 && left <= 3600.0, "{left}");
}

/// #348: with uploads through the API, the presign answer names the API's
/// own link and every header the client must send, CSRF included, so a
/// client that only replays those headers (as `putPresigned` always did)
/// uploads, binds and downloads without ever seeing MinIO.
#[sqlx::test]
async fn proxied_upload_and_download_stay_on_the_app_origin(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let signed = presign_ok(
        &mut owner,
        &channel_id,
        "Bär.png",
        "image/png",
        PNG.len() as i64,
    )
    .await;
    let id = signed["id"].as_str().unwrap().to_owned();
    assert_eq!(
        signed["upload_url"],
        format!("/api/attachments/{id}/upload")
    );
    assert_eq!(
        signed["headers"]["X-CSRF-Token"].as_str(),
        owner.csrf.as_deref()
    );

    let (status, body) = owner.put_upload(&signed, PNG.to_vec(), |_| {}).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = post_files(&mut owner, &channel_id, std::slice::from_ref(&id)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, headers, bytes) = member
        .send_raw(Method::GET, &format!("/api/attachments/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["content-length"], PNG.len().to_string());
    assert_eq!(headers["cache-control"], "private, max-age=3600");
    assert_eq!(headers["vary"], "Cookie");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["content-security-policy"], "sandbox");
    assert_eq!(
        headers["content-disposition"],
        "inline; filename=\"B_r.png\"; filename*=UTF-8''B%C3%A4r.png"
    );

    let (status, headers, bytes) = member
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{id}?download=1"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["cache-control"], "private, no-store");
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment; ")
    );

    // Bound now: the link is spent, as MinIO's would be for the bind check.
    let (status, _) = owner.put_upload(&signed, PNG.to_vec(), |_| {}).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// #348: the API's upload link keeps the rules of MinIO's signed one: only
/// the uploader, with the CSRF header, the signed type and the signed size.
#[sqlx::test]
async fn proxied_upload_keeps_the_rules_of_a_signed_link(pool: PgPool) {
    use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let signed = presign_ok(
        &mut owner,
        &channel_id,
        "a.png",
        "image/png",
        PNG.len() as i64,
    )
    .await;
    let id = signed["id"].as_str().unwrap().to_owned();
    let stored = |client: &Client| {
        let store = client.store.clone();
        let key = format!("att/{id}");
        async move { store.head(&key).await.is_ok() }
    };

    // Another member, with their own valid CSRF pair: not their upload.
    let mut foreign = signed.clone();
    foreign["headers"]["X-CSRF-Token"] = json!(member.csrf.clone().unwrap());
    let (status, _) = member.put_upload(&foreign, PNG.to_vec(), |_| {}).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = owner
        .put_upload(&signed, PNG.to_vec(), |headers| {
            headers.remove("x-csrf-token");
        })
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = owner
        .put_upload(&signed, PNG.to_vec(), |headers| {
            headers.insert(CONTENT_TYPE, "text/html".parse().unwrap());
        })
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["content_type"], "invalid");

    let mut longer = PNG.to_vec();
    longer.push(0);
    let (status, body) = owner.put_upload(&signed, longer, |_| {}).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["size"], "invalid");

    // A Content-Length that lies about the body is caught by the store.
    let (status, body) = owner
        .put_upload(&signed, PNG[..4].to_vec(), |headers| {
            headers.insert(CONTENT_LENGTH, PNG.len().into());
        })
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["fields"]["size"], "invalid");
    assert!(!stored(&owner).await, "nothing was stored yet");

    let (status, _) = owner.put_upload(&signed, PNG.to_vec(), |_| {}).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(stored(&owner).await);
}

/// #348: past its ten minutes the API's upload link is gone like MinIO's.
#[sqlx::test]
async fn proxied_upload_link_expires_with_put_ttl(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);
    let signed = presign_ok(
        &mut owner,
        &channel_id,
        "a.png",
        "image/png",
        PNG.len() as i64,
    )
    .await;
    sqlx::query("UPDATE attachments SET created_at = now() - interval '11 minutes' WHERE id = $1")
        .bind(uuid::Uuid::parse_str(signed["id"].as_str().unwrap()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = owner.put_upload(&signed, PNG.to_vec(), |_| {}).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
