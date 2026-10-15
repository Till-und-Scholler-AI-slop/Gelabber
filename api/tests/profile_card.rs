//! Profile card customisation: bio, pronouns, status text, colors, avatar
//! decoration and an uploaded banner on `PATCH /api/me`, and who may read
//! someone else's card on `GET /api/users/{id}`.

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

async fn signed_in(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, "password123", name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn patch_me(client: &mut Client, body: Value) -> common::Response {
    client.send(Method::PATCH, "/api/me", Some(body)).await
}

#[sqlx::test(migrations = "./migrations")]
async fn card_fields_are_saved_normalised_and_cleared(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;

    let me = ada.send(Method::GET, "/api/me", None).await;
    for field in [
        "bio",
        "pronouns",
        "status_text",
        "accent_color",
        "banner_color",
        "banner_url",
        "avatar_decoration",
    ] {
        assert_eq!(me.body[field], Value::Null, "{field} starts unset");
    }

    let res = patch_me(
        &mut ada,
        json!({
            "bio": "  Mathematikerin.\r\nSchreibt Programme. ",
            "pronouns": " sie/ihr ",
            "status_text": "Rechnet gerade",
            "accent_color": "#FF8800",
            "banner_color": "#1a2B3c",
            "avatar_decoration": "rainbow",
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["bio"], "Mathematikerin.\nSchreibt Programme.");
    assert_eq!(res.body["pronouns"], "sie/ihr");
    assert_eq!(res.body["status_text"], "Rechnet gerade");
    assert_eq!(res.body["accent_color"], "#ff8800");
    assert_eq!(res.body["banner_color"], "#1a2b3c");
    assert_eq!(res.body["avatar_decoration"], "rainbow");
    assert_eq!(res.body["name"], "Ada", "untouched fields stay");

    let me = ada.send(Method::GET, "/api/me", None).await;
    assert_eq!(
        me.body["accent_color"], "#ff8800",
        "the session reads it too"
    );

    // A partial patch leaves the rest alone; empty strings clear.
    let res = patch_me(&mut ada, json!({ "status_text": "", "accent_color": "" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["status_text"], Value::Null);
    assert_eq!(res.body["accent_color"], Value::Null);
    assert_eq!(res.body["pronouns"], "sie/ihr");
    assert_eq!(res.body["avatar_decoration"], "rainbow");

    let res = patch_me(&mut ada, json!({ "avatar_decoration": "" })).await;
    assert_eq!(res.body["avatar_decoration"], Value::Null);
}

#[sqlx::test(migrations = "./migrations")]
async fn invalid_card_fields_are_rejected_as_a_whole(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;

    let res = patch_me(
        &mut ada,
        json!({
            "bio": "x".repeat(191),
            "pronouns": "x".repeat(41),
            "status_text": "zwei\nZeilen",
            "accent_color": "orange",
            "banner_color": "#abc",
            "avatar_decoration": "crown",
            "banner_url": "https://cdn.example/banner.png",
            "name": "Ada L",
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    let fields = &res.body["fields"];
    assert_eq!(fields["bio"], "too_long");
    assert_eq!(fields["pronouns"], "too_long");
    assert_eq!(fields["status_text"], "invalid");
    assert_eq!(fields["accent_color"], "invalid");
    assert_eq!(fields["banner_color"], "invalid");
    assert_eq!(fields["avatar_decoration"], "invalid");
    assert_eq!(fields["banner_url"], "invalid", "banners are never links");

    let me = ada.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.body["name"], "Ada", "nothing of a rejected patch lands");
}

#[sqlx::test(migrations = "./migrations")]
async fn banner_upload_replace_and_remove(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;

    let (status, avatar) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{avatar}");
    let avatar_url = avatar["avatar_url"].as_str().unwrap().to_owned();

    let (status, first) = ada
        .send_bytes(Method::PUT, "/api/me/banner", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_url = first["banner_url"].as_str().unwrap().to_owned();
    assert!(first_url.starts_with(&format!("/api/banners/{}/", ada.user_id())));
    assert_eq!(first["avatar_url"], avatar_url.as_str(), "avatar untouched");

    let (status, headers, bytes) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["x-content-type-options"], "nosniff");

    // The profile form echoes the current banner: no change, no delete.
    let res = patch_me(&mut ada, json!({ "banner_url": first_url, "bio": "Hi" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let (status, _, _) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, second) = ada
        .send_bytes(Method::PUT, "/api/me/banner", "image/webp", WEBP.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let second_url = second["banner_url"].as_str().unwrap().to_owned();
    assert_ne!(second_url, first_url);
    let (status, _, _) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "replaced banner is deleted");
    let (status, _, _) = ada.send_raw(Method::GET, &avatar_url, None).await;
    assert_eq!(status, StatusCode::OK, "a banner never deletes the avatar");

    // A stale tab echoing the earlier banner is refused.
    let res = patch_me(&mut ada, json!({ "banner_url": first_url })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["banner_url"], "stale");
    // So is pointing the banner at the avatar object.
    let res = patch_me(&mut ada, json!({ "banner_url": avatar_url })).await;
    assert_eq!(res.body["fields"]["banner_url"], "invalid");

    let res = patch_me(&mut ada, json!({ "banner_url": "" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["banner_url"], Value::Null);
    let (status, _, _) = ada.send_raw(Method::GET, &second_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "removed banner is deleted");
    let (status, _, _) = ada.send_raw(Method::GET, &avatar_url, None).await;
    assert_eq!(status, StatusCode::OK);
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM storage_cleanup")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pending, 0, "nothing left for the cleanup worker");
}

#[sqlx::test(migrations = "./migrations")]
async fn banner_upload_refuses_non_images_and_oversize(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let (status, _) = ada
        .send_bytes(
            Method::PUT,
            "/api/me/banner",
            "image/png",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = ada
        .send_bytes(
            Method::PUT,
            "/api/me/banner",
            "image/png",
            [PNG, &vec![0; 1024 * 1024]].concat(),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    // Larger than an avatar may be, within the banner limit.
    let (status, body) = ada
        .send_bytes(
            Method::PUT,
            "/api/me/banner",
            "image/png",
            [PNG, &vec![0; 700 * 1024]].concat(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut anonymous = Client::from_state(state);
    let url = body["banner_url"].as_str().unwrap();
    let (status, _, _) = anonymous.send_raw(Method::GET, url, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn cards_are_visible_to_server_members_and_dm_peers_only(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_in(&state, "bob@example.com", "Bob").await;
    let mut cara = signed_in(&state, "cara@example.com", "Cara").await;
    let ada_id = ada.user_id().to_owned();
    let bob_id = bob.user_id().to_owned();
    let ada_card = format!("/api/users/{ada_id}");

    let res = patch_me(
        &mut ada,
        json!({ "bio": "Hallo", "pronouns": "sie/ihr", "status_text": "Da",
                "accent_color": "#00aa88", "avatar_decoration": "glow" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let own = ada.send(Method::GET, &ada_card, None).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(own.body["bio"], "Hallo");

    // Strangers see nothing, same as for an unknown or malformed id.
    let res = bob.send(Method::GET, &ada_card, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = bob
        .send(
            Method::GET,
            "/api/users/00000000-0000-0000-0000-000000000001",
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = bob.send(Method::GET, "/api/users/nope", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // A shared server makes the card visible, without the e-mail address.
    let server = ada
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    let server_id = server.body["id"].as_str().unwrap().to_owned();
    let invite = ada
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let res = bob
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let card = bob.send(Method::GET, &ada_card, None).await;
    assert_eq!(card.status, StatusCode::OK, "{}", card.body);
    assert_eq!(card.body["id"], ada_id.as_str());
    assert_eq!(card.body["name"], "Ada");
    assert_eq!(card.body["bio"], "Hallo");
    assert_eq!(card.body["pronouns"], "sie/ihr");
    assert_eq!(card.body["accent_color"], "#00aa88");
    assert!(card.body.get("email").is_none(), "{}", card.body);

    // The member list carries the short parts of the card.
    let detail = bob
        .send(Method::GET, &format!("/api/servers/{server_id}"), None)
        .await;
    let row = detail.body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["user_id"] == ada_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(row["status_text"], "Da");
    assert_eq!(row["accent_color"], "#00aa88");
    assert_eq!(row["avatar_decoration"], "glow");
    assert!(row.get("bio").is_none(), "the bio loads with the card only");

    // Cara has no server with Ada; a DM with Bob does not reveal Ada.
    let res = cara
        .send(Method::POST, "/api/dms", Some(json!({ "user_id": bob_id })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = cara.send(Method::GET, &ada_card, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = cara
        .send(Method::GET, &format!("/api/users/{bob_id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "a DM peer's card is visible");

    // Leaving the only shared server hides the card again.
    let res = bob
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/leave"),
            None,
        )
        .await;
    assert!(res.status.is_success(), "{}", res.body);
    let res = bob.send(Method::GET, &ada_card, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

/// Decompression bombs: a file under the byte limit may declare a huge
/// canvas. The API refuses anything beyond what the web client produces,
/// and does not serve such an object even if it is already stored.
#[sqlx::test(migrations = "./migrations")]
async fn uploads_beyond_the_client_pixel_size_are_refused(pool: PgPool) {
    use gelabber_api::image_size::fixtures;

    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let refused = [
        ("/api/me/banner", "image/png", fixtures::png(15000, 15000)),
        ("/api/me/banner", "image/png", fixtures::png(961, 320)),
        ("/api/me/banner", "image/webp", fixtures::webp(960, 321)),
        ("/api/me/banner", "image/jpeg", fixtures::jpeg(16000, 16000)),
        ("/api/me/avatar", "image/png", fixtures::png(257, 256)),
        ("/api/me/avatar", "image/webp", fixtures::webp(960, 320)),
        ("/api/me/avatar", "image/jpeg", fixtures::jpeg(256, 0)),
    ];
    for (path, content_type, bytes) in refused {
        let (status, body) = ada.send_bytes(Method::PUT, path, content_type, bytes).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path} {body}");
    }
    let me = ada.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.body["avatar_url"], Value::Null);
    assert_eq!(me.body["banner_url"], Value::Null);
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM storage_cleanup")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pending, 0, "refused before anything was stored");

    for (path, content_type, bytes) in [
        ("/api/me/banner", "image/jpeg", fixtures::jpeg(960, 320)),
        ("/api/me/banner", "image/webp", fixtures::webp(480, 160)),
        ("/api/me/avatar", "image/jpeg", fixtures::jpeg(256, 256)),
    ] {
        let (status, body) = ada.send_bytes(Method::PUT, path, content_type, bytes).await;
        assert_eq!(status, StatusCode::OK, "{path} {body}");
    }

    // Stored before the check existed (or by another write path).
    let id = ada.user_id().to_owned();
    let file = uuid::Uuid::new_v4();
    state
        .store
        .put(
            &format!("avatars/{id}/{file}"),
            "image/png",
            fixtures::png(15000, 15000),
        )
        .await
        .unwrap();
    let (status, _, _) = ada
        .send_raw(Method::GET, &format!("/api/avatars/{id}/{file}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A banner appears only on the profile card, so its bytes follow the
/// card's visibility; avatars stay readable for every signed-in account.
#[sqlx::test(migrations = "./migrations")]
async fn banner_downloads_follow_card_visibility(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_in(&state, "bob@example.com", "Bob").await;
    let (_, me) = ada
        .send_bytes(Method::PUT, "/api/me/banner", "image/webp", WEBP.to_vec())
        .await;
    let banner = me["banner_url"].as_str().unwrap().to_owned();
    let (_, me) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    let avatar = me["avatar_url"].as_str().unwrap().to_owned();

    let (status, _, _) = ada.send_raw(Method::GET, &banner, None).await;
    assert_eq!(status, StatusCode::OK, "own banner");
    let (status, _, _) = bob.send_raw(Method::GET, &banner, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a stranger's banner");
    let (status, _, _) = bob.send_raw(Method::GET, &avatar, None).await;
    assert_eq!(status, StatusCode::OK, "avatars are not gated");

    let res = bob
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": ada.user_id() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let (status, _, bytes) = bob.send_raw(Method::GET, &banner, None).await;
    assert_eq!(status, StatusCode::OK, "a DM peer sees the card");
    assert_eq!(bytes, WEBP);
}

/// One `PATCH /api/me` clearing both uploads deletes both objects.
#[sqlx::test(migrations = "./migrations")]
async fn clearing_avatar_and_banner_together_deletes_both(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com", "Ada").await;
    let (_, me) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    let avatar = me["avatar_url"].as_str().unwrap().to_owned();
    let (_, me) = ada
        .send_bytes(Method::PUT, "/api/me/banner", "image/webp", WEBP.to_vec())
        .await;
    let banner = me["banner_url"].as_str().unwrap().to_owned();

    let res = patch_me(&mut ada, json!({ "avatar_url": "", "banner_url": "" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["avatar_url"], Value::Null);
    assert_eq!(res.body["banner_url"], Value::Null);
    for url in [&avatar, &banner] {
        let (status, _, _) = ada.send_raw(Method::GET, url, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{url} deleted");
    }
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM storage_cleanup")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pending, 0);
}
