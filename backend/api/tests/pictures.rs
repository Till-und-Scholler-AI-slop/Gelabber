//! Pictures (#574): privacy metadata stripped before binding, size as
//! shown in the JSON, thumbnails and placeholder from the media worker.
//! Bytes go through the in-memory store.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use image::codecs::jpeg::JpegEncoder;
use image::{ImageEncoder, Rgb, RgbImage};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

async fn owner_and_member(pool: PgPool) -> (Client, Client, String) {
    let mut shared = common::state(pool);
    // The tests wait for the media worker; it looks every 2 s in production.
    shared.workers.pictures = Some(Duration::from_millis(50));
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

/// A 1200×900 photo as a phone writes it: EXIF with orientation 6 (shown
/// rotated, so 900×1200) and a GPS block, XMP, and a "motion photo" video
/// appended after the picture.
fn phone_photo() -> Vec<u8> {
    let picture = RgbImage::from_fn(1200, 900, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    });
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 85)
        .write_image(&picture, 1200, 900, image::ExtendedColorType::Rgb8)
        .unwrap();
    let mut tiff = b"II*\0\x08\0\0\0\x02\0".to_vec();
    tiff.extend_from_slice(b"\x12\x01\x03\0\x01\0\0\0\x06\0\0\0");
    tiff.extend_from_slice(b"\x25\x88\x04\0\x01\0\0\0\x26\0\0\0");
    tiff.extend_from_slice(b"\0\0\0\0");
    tiff.extend_from_slice(b"GPSLatitude 52.5200 N GPSLongitude 13.4050 E");
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend_from_slice(&tiff);
    let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    xmp.extend_from_slice(b"<x:xmpmeta><exif:GPSLatitude>52,31N</exif:GPSLatitude></x:xmpmeta>");
    let mut head = Vec::new();
    for payload in [exif, xmp] {
        head.extend_from_slice(&[0xFF, 0xE1]);
        head.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        head.extend_from_slice(&payload);
    }
    jpeg.splice(2..2, head);
    jpeg.extend_from_slice(b"\0\0\0\x18ftypmp42 video GPSCoordinates");
    jpeg
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
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

async fn post(client: &mut Client, channel: &str, ids: &[&str]) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": "", "attachment_ids": ids })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

/// Until the media worker (started with the app) has handled the picture.
async fn wait_for_worker(pool: &PgPool, id: &str) {
    let id = Uuid::parse_str(id).unwrap();
    for _ in 0..200 {
        let done: i16 = sqlx::query_scalar("SELECT media_version FROM attachments WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        if done >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("media worker did not handle {id}");
}

#[sqlx::test]
async fn phone_photo_loses_its_location_and_gets_thumbnails(pool: PgPool) {
    let (mut owner, mut member, channel) = owner_and_member(pool.clone()).await;
    let photo = phone_photo();
    let id = upload(&mut owner, &channel, "IMG_2031.jpg", "image/jpeg", &photo).await;
    let message = post(&mut owner, &channel, &[&id]).await;
    let attachment = &message["attachments"][0];
    // The size as shown is known with the message, before the worker ran.
    assert_eq!(attachment["width"], 900);
    assert_eq!(attachment["height"], 1200);
    let size = attachment["size"].as_i64().unwrap();
    assert!(size < photo.len() as i64, "{size} >= {}", photo.len());

    // The original as members download it: no location, no XMP, no video,
    // still turned the same way.
    let (status, _, original) = member
        .send_raw(Method::GET, &format!("/api/attachments/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(original.len() as i64, size);
    assert!(!contains(&original, b"GPS"));
    assert!(!contains(&original, b"xmpmeta"));
    assert!(!contains(&original, b"ftyp"));
    let decoded = image::load_from_memory(&original).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (1200, 900));
    assert_eq!(
        gelabber_api::attachments::metadata::orientation("image/jpeg", &original),
        6
    );

    wait_for_worker(&pool, &id).await;
    for (edge, long, short) in [("400", 400, 300), ("800", 800, 600)] {
        let (status, headers, thumb) = member
            .send_raw(
                Method::GET,
                &format!("/api/attachments/{id}?size={edge}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "image/jpeg");
        assert_eq!(headers["cache-control"], "private, max-age=3600");
        let thumb_picture = image::load_from_memory(&thumb).unwrap();
        // Upright: the orientation is applied to the pixels.
        assert_eq!(
            (thumb_picture.width(), thumb_picture.height()),
            (short, long)
        );
        assert!(!contains(&thumb, b"GPS"));
        assert!(!contains(&thumb, b"Exif"));
        assert!(thumb.len() < 200 * 1024, "{edge}: {} bytes", thumb.len());
    }

    let listed = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    let attachment = &listed.body["messages"][0]["attachments"][0];
    assert_eq!(attachment["thumbnail"], true);
    assert_eq!(attachment["placeholder_version"], 1);
    let placeholder = attachment["placeholder"].as_str().unwrap();
    assert!(
        placeholder.starts_with('#') && placeholder.len() == 7,
        "{placeholder}"
    );
    assert_eq!(
        (attachment["width"].as_i64(), attachment["height"].as_i64()),
        (Some(900), Some(1200))
    );
}

#[sqlx::test]
async fn thumbnail_link_serves_the_original_until_it_exists(pool: PgPool) {
    let (mut owner, mut member, channel) = owner_and_member(pool.clone()).await;
    // A size but no pixel data, so not decodable: the worker keeps it
    // without thumbnails.
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x40\0\0\0\x40\x08\x06\0\0\0\0\0\0\0";
    let id = upload(&mut owner, &channel, "pic.png", "image/png", png).await;
    post(&mut owner, &channel, &[&id]).await;
    wait_for_worker(&pool, &id).await;

    let (status, headers, bytes) = member
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{id}?size=400"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, png);
    assert_eq!(headers["cache-control"], "private, no-store");

    let (status, _, _) = member
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{id}?size=123"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test]
async fn thumbnails_need_membership_and_go_with_the_picture(pool: PgPool) {
    let (mut owner, _member, channel) = owner_and_member(pool.clone()).await;
    let id = upload(&mut owner, &channel, "p.jpg", "image/jpeg", &phone_photo()).await;
    let message = post(&mut owner, &channel, &[&id]).await;
    wait_for_worker(&pool, &id).await;

    let mut stranger = Client::new(pool.clone());
    stranger.bootstrap().await;
    stranger
        .register("stranger@example.com", "password123", "Eve")
        .await;
    let (status, _, _) = stranger
        .send_raw(
            Method::GET,
            &format!("/api/attachments/{id}?size=400"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let message_id = message["id"].as_str().unwrap();
    let deleted = owner
        .send(Method::DELETE, &format!("/api/messages/{message_id}"), None)
        .await;
    assert!(deleted.status.is_success(), "{}", deleted.body);
    let mut queued: Vec<String> = sqlx::query_scalar(
        "SELECT object_key FROM storage_cleanup WHERE object_key LIKE $1 ORDER BY object_key",
    )
    .bind(format!("att/{id}%"))
    .fetch_all(&pool)
    .await
    .unwrap();
    queued.sort();
    assert_eq!(
        queued,
        [
            format!("att/{id}"),
            format!("att/{id}/t400"),
            format!("att/{id}/t800")
        ]
    );
}

#[sqlx::test]
async fn existing_pictures_are_backfilled(pool: PgPool) {
    let (mut owner, mut member, channel) = owner_and_member(pool.clone()).await;
    let id = upload(
        &mut owner,
        &channel,
        "old.jpg",
        "image/jpeg",
        &phone_photo(),
    )
    .await;
    post(&mut owner, &channel, &[&id]).await;
    wait_for_worker(&pool, &id).await;
    // As a picture from before #574: no size, no thumbnails, version 0.
    sqlx::query(
        "UPDATE attachments SET width = NULL, height = NULL, placeholder = NULL, \
         placeholder_version = NULL, thumb_type = NULL, media_version = 0, media_attempts = 0, \
         media_retry_at = now() \
         WHERE id = $1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    wait_for_worker(&pool, &id).await;
    let listed = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    let attachment = &listed.body["messages"][0]["attachments"][0];
    assert_eq!(attachment["thumbnail"], true);
    assert_eq!(
        (attachment["width"].as_i64(), attachment["height"].as_i64()),
        (Some(900), Some(1200))
    );
}

#[sqlx::test]
async fn clients_hear_when_the_thumbnails_are_there(pool: PgPool) {
    let (mut owner, _member, channel) = owner_and_member(pool.clone()).await;
    let id = upload(
        &mut owner,
        &channel,
        "live.jpg",
        "image/jpeg",
        &phone_photo(),
    )
    .await;
    let sent = post(&mut owner, &channel, &[&id]).await;
    wait_for_worker(&pool, &id).await;
    let message = Uuid::parse_str(sent["id"].as_str().unwrap()).unwrap();
    // The worker sent the message again (an edit event), so its revision
    // moved on past the one the send answered with.
    let mut revision = 0;
    for _ in 0..100 {
        revision = sqlx::query_scalar("SELECT revision FROM messages WHERE id = $1")
            .bind(message)
            .fetch_one(&pool)
            .await
            .unwrap();
        if revision > sent["revision"].as_i64().unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(revision > sent["revision"].as_i64().unwrap(), "{sent}");
    let edited: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT edited_at FROM messages WHERE id = $1")
            .bind(message)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(edited, None, "thumbnails are no edit");
}

#[sqlx::test]
async fn a_failed_strip_put_keeps_the_upload(pool: PgPool) {
    let (mut owner, _member, channel) = owner_and_member(pool.clone()).await;
    let photo = phone_photo();
    let id = upload(&mut owner, &channel, "photo.jpg", "image/jpeg", &photo).await;
    // `prepare` recorded the stripped length, then its `put` failed: the
    // object is still the original.
    sqlx::query("UPDATE attachments SET stripped_size = $2 WHERE id = $1")
        .bind(Uuid::parse_str(&id).unwrap())
        .bind(photo.len() as i64 - 100)
        .execute(&pool)
        .await
        .unwrap();
    // A send that fails on another file discards its mismatches.
    let res = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({ "filename": "cut.jpg", "content_type": "image/jpeg", "size": 8 })),
        )
        .await;
    let cut = res.body["id"].as_str().unwrap().to_owned();
    owner
        .store
        .put(&format!("att/{cut}"), "image/jpeg", vec![1, 2, 3])
        .await
        .unwrap();
    let res = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": "", "attachment_ids": [&id, &cut] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    // The photo survived and sends on its own.
    let sent = post(&mut owner, &channel, &[&id]).await;
    assert_eq!(sent["attachments"][0]["id"], id.as_str());
}

#[sqlx::test]
async fn a_picture_that_keeps_killing_the_worker_is_given_up(pool: PgPool) {
    let (mut owner, mut member, channel) = owner_and_member(pool.clone()).await;
    let id = upload(
        &mut owner,
        &channel,
        "bad.jpg",
        "image/jpeg",
        &phone_photo(),
    )
    .await;
    post(&mut owner, &channel, &[&id]).await;
    wait_for_worker(&pool, &id).await;
    // As if claimed five times and the process died each time.
    sqlx::query(
        "UPDATE attachments SET thumb_type = NULL, media_version = 0, media_attempts = 5, \
         media_retry_at = now() WHERE id = $1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    wait_for_worker(&pool, &id).await;
    let listed = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    let attachment = &listed.body["messages"][0]["attachments"][0];
    assert!(attachment.get("thumbnail").is_none(), "{attachment}");
}

#[sqlx::test]
async fn heic_from_android_is_a_plain_file(pool: PgPool) {
    let (mut owner, mut member, channel) = owner_and_member(pool.clone()).await;
    let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic....";
    let id = upload(&mut owner, &channel, "PXL_2031.heic", "image/heic", heic).await;
    let message = post(&mut owner, &channel, &[&id]).await;
    assert_eq!(message["attachments"][0]["content_type"], "image/heic");
    assert!(message["attachments"][0].get("width").is_none());
    let (status, headers, bytes) = member
        .send_raw(Method::GET, &format!("/api/attachments/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, heic);
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
}
