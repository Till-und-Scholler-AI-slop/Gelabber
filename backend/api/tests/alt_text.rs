//! Alt text per picture (#575): set with the send, changed or cleared by
//! the author through an edit, checked like message text, pictures only.

mod common;

use axum::http::{Method, StatusCode};
use image::{ImageEncoder, Rgba, RgbaImage};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

async fn owner_and_member(pool: PgPool) -> (Client, Client, String) {
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

    let server = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await
        .body;
    let id = server["id"].as_str().unwrap();
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let joined = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    let channel = server["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (owner, member, channel)
}

fn png() -> Vec<u8> {
    let picture = RgbaImage::from_pixel(8, 6, Rgba([10, 20, 30, 255]));
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&picture, 8, 6, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

async fn upload(
    client: &mut Client,
    channel: &str,
    name: &str,
    kind: &str,
    bytes: &[u8],
) -> String {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({ "filename": name, "content_type": kind, "size": bytes.len() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id = res.body["id"].as_str().unwrap().to_owned();
    client
        .store
        .put(&format!("att/{id}"), kind, bytes.to_vec())
        .await
        .unwrap();
    id
}

async fn send(client: &mut Client, channel: &str, body: Value) -> common::Response {
    client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(body),
        )
        .await
}

async fn edit(client: &mut Client, message: &str, body: Value) -> common::Response {
    client
        .send(
            Method::PATCH,
            &format!("/api/messages/{message}"),
            Some(body),
        )
        .await
}

#[sqlx::test]
async fn a_picture_is_sent_with_and_without_alt_text(pool: PgPool) {
    let (mut owner, mut member, channel) = owner_and_member(pool).await;
    let a = upload(&mut owner, &channel, "IMG_1.png", "image/png", &png()).await;
    let b = upload(&mut owner, &channel, "IMG_2.png", "image/png", &png()).await;
    let res = send(
        &mut owner,
        &channel,
        json!({
            "attachment_ids": [a, b],
            "attachment_descriptions": { a.clone(): "  Fehlerdialog:\r\n\"Upload fehlgeschlagen (413)\" " },
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let files = &res.body["attachments"];
    assert_eq!(
        files[0]["description"],
        "Fehlerdialog:\n\"Upload fehlgeschlagen (413)\""
    );
    assert!(files[1].get("description").is_none(), "{}", files[1]);
    // Others read it with the history.
    let page = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    assert_eq!(
        page.body["messages"][0]["attachments"][0]["description"],
        files[0]["description"]
    );
}

#[sqlx::test]
async fn alt_text_is_checked_like_message_text(pool: PgPool) {
    let (mut owner, _member, channel) = owner_and_member(pool).await;
    let pic = upload(&mut owner, &channel, "a.png", "image/png", &png()).await;
    let doc = upload(
        &mut owner,
        &channel,
        "a.pdf",
        "application/pdf",
        b"%PDF-1.4\n%%EOF",
    )
    .await;
    for (text, code) in [
        ("x".repeat(1001), "too_long"),
        ("Klingel\u{7}".to_owned(), "invalid"),
    ] {
        let res = send(
            &mut owner,
            &channel,
            json!({ "attachment_ids": [pic], "attachment_descriptions": { pic.clone(): text } }),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(
            res.body["fields"]["attachment_descriptions"], code,
            "{}",
            res.body
        );
    }
    // Only for pictures of this message.
    let other = uuid::Uuid::new_v4().to_string();
    for body in [
        json!({ "attachment_ids": [pic], "attachment_descriptions": { other: "x" } }),
        json!({ "attachment_ids": [doc], "attachment_descriptions": { doc.clone(): "x" } }),
    ] {
        let res = send(&mut owner, &channel, body).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["fields"]["attachment_descriptions"], "invalid");
    }
    // Exactly 1000 characters fit, and the uploads are still there to send.
    let res = send(
        &mut owner,
        &channel,
        json!({ "attachment_ids": [pic], "attachment_descriptions": { pic.clone(): "ä".repeat(1000) } }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

#[sqlx::test]
async fn the_author_changes_and_clears_it_by_editing(pool: PgPool) {
    let (mut owner, mut member, channel) = owner_and_member(pool).await;
    let pic = upload(&mut owner, &channel, "a.png", "image/png", &png()).await;
    let sent = send(
        &mut owner,
        &channel,
        json!({ "content": "Schaut mal", "attachment_ids": [pic] }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    let id = sent.body["id"].as_str().unwrap().to_owned();
    let revision = sent.body["revision"].as_i64().unwrap();

    // Alt text alone: the text stays, the message is not marked edited,
    // and a new revision goes out to every client.
    let res = edit(
        &mut owner,
        &id,
        json!({ "attachment_descriptions": { pic.clone(): "Ein Hund am Strand" } }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["content"], "Schaut mal");
    assert!(res.body["edited_at"].is_null(), "{}", res.body);
    assert_eq!(
        res.body["attachments"][0]["description"],
        "Ein Hund am Strand"
    );
    let changed = res.body["revision"].as_i64().unwrap();
    assert!(changed > revision);

    // The same again changes nothing.
    let res = edit(
        &mut owner,
        &id,
        json!({ "attachment_descriptions": { pic.clone(): "Ein Hund am Strand" } }),
    )
    .await;
    assert_eq!(res.body["revision"].as_i64().unwrap(), changed);

    // Others may not.
    let res = edit(
        &mut member,
        &id,
        json!({ "attachment_descriptions": { pic.clone(): "Eine Katze" } }),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);

    // Text and alt text together; empty clears it.
    let res = edit(
        &mut owner,
        &id,
        json!({ "content": "Schaut mal!", "attachment_descriptions": { pic.clone(): "" } }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["content"], "Schaut mal!");
    assert!(!res.body["edited_at"].is_null());
    assert!(
        res.body["attachments"][0].get("description").is_none(),
        "{}",
        res.body
    );
    let page = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    assert!(
        page.body["messages"][0]["attachments"][0]
            .get("description")
            .is_none()
    );
}
