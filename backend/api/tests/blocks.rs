//! One person blocks another (#565): the routes, every private path closed
//! both ways, what a block takes away, mentions, unblocking and the event.
//! The one blocked writes on into an existing DM, unseen by the blocker
//! (#619).

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::SinkExt;
use gelabber_api::AppState;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;

use common::ws::{self, connect, within};
use common::{Client, Response};

const PASSWORD: &str = "password123";

async fn person(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

struct World {
    ada: Client,
    bob: Client,
    /// The server's text channel.
    channel: String,
}

/// Ada and Bob share a server.
async fn world(state: AppState) -> World {
    let mut ada = person(&state, "ada@example.com", "Ada").await;
    let mut bob = person(&state, "bob@example.com", "Bob").await;
    let server = ada
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Club" })),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED, "{}", server.body);
    let sid = server.body["id"].as_str().unwrap().to_owned();
    let channel = server.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let invite = ada
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap().to_owned();
    let joined = bob
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    World { ada, bob, channel }
}

async fn block(client: &mut Client, user: &str) -> Response {
    client
        .send(Method::PUT, &format!("/api/users/me/blocks/{user}"), None)
        .await
}

async fn unblock(client: &mut Client, user: &str) -> Response {
    client
        .send(
            Method::DELETE,
            &format!("/api/users/me/blocks/{user}"),
            None,
        )
        .await
}

async fn blocks(client: &mut Client) -> Vec<Value> {
    let res = client.send(Method::GET, "/api/users/me/blocks", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body.as_array().unwrap().clone()
}

async fn open_dm(client: &mut Client, user: &str) -> Response {
    client
        .send(Method::POST, "/api/dms", Some(json!({ "user_id": user })))
        .await
}

async fn text(client: &mut Client, channel: &str, content: &str) -> Response {
    client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": content })),
        )
        .await
}

async fn ask(client: &mut Client, body: Value) -> Response {
    client
        .send(Method::POST, "/api/friends/requests", Some(body))
        .await
}

async fn friends(client: &mut Client) -> Value {
    let res = client.send(Method::GET, "/api/friends", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

/// A message with one small PNG; its id and the picture's.
async fn picture_message(client: &mut Client, channel: &str) -> (String, String) {
    let mut png = Vec::new();
    image::RgbImage::from_pixel(4, 4, image::Rgb([200, 120, 40]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({ "filename": "dog.png", "content_type": "image/png", "size": png.len() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let picture = res.body["id"].as_str().unwrap().to_owned();
    client
        .store
        .put(&format!("att/{picture}"), "image/png", png)
        .await
        .unwrap();
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": "", "attachment_ids": [&picture] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    (res.body["id"].as_str().unwrap().to_owned(), picture)
}

fn refused(res: &Response, status: StatusCode, code: &str) {
    assert_eq!(res.status, status, "{}", res.body);
    assert_eq!(res.body["error"], code, "{}", res.body);
}

#[sqlx::test]
async fn blocking_is_idempotent_listed_and_undone(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    assert_eq!(block(&mut w.ada, &bob).await.status, StatusCode::NO_CONTENT);
    assert_eq!(block(&mut w.ada, &bob).await.status, StatusCode::NO_CONTENT);
    let list = blocks(&mut w.ada).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], bob.as_str());
    assert_eq!(list[0]["name"], "Bob");
    assert!(list[0]["created_at"].is_string());
    // Bob's own list says nothing about Ada's block.
    assert!(blocks(&mut w.bob).await.is_empty());

    let res = block(&mut w.ada, &ada).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["user_id"], "self");
    let res = block(&mut w.ada, "00000000-0000-0000-0000-000000000099").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    assert_eq!(
        unblock(&mut w.ada, &bob).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        unblock(&mut w.ada, &bob).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(blocks(&mut w.ada).await.is_empty());
}

#[sqlx::test]
async fn a_thousand_blocks_are_the_most(pool: PgPool) {
    let mut w = world(common::state(pool.clone())).await;
    let ada = w.ada.user_id().to_owned();
    sqlx::query(
        "WITH made AS (INSERT INTO users (email, name, password_hash) \
                       SELECT 'b' || n || '@example.com', 'B' || n, 'x' \
                       FROM generate_series(1, 1000) n RETURNING id) \
         INSERT INTO user_blocks (blocker_id, blocked_id) SELECT $1::uuid, id FROM made",
    )
    .bind(&ada)
    .execute(&pool)
    .await
    .unwrap();
    let bob = w.bob.user_id().to_owned();
    refused(
        &block(&mut w.ada, &bob).await,
        StatusCode::CONFLICT,
        "blocks_full",
    );
}

#[sqlx::test]
async fn new_ways_in_are_closed_both_ways(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    block(&mut w.ada, &bob).await;

    // A DM: Bob hears what a stranger would, Ada why.
    refused(
        &open_dm(&mut w.bob, &ada).await,
        StatusCode::FORBIDDEN,
        "dm_not_allowed",
    );
    refused(
        &open_dm(&mut w.ada, &bob).await,
        StatusCode::FORBIDDEN,
        "blocked",
    );

    // A friend request, by profile and by code: Bob as if Ada were unknown.
    refused(
        &ask(&mut w.bob, json!({ "user_id": ada })).await,
        StatusCode::NOT_FOUND,
        "not_found",
    );
    let code = w.ada.send(Method::GET, "/api/me/friend-code", None).await;
    let code = code.body["code"].as_str().unwrap().to_owned();
    refused(
        &ask(&mut w.bob, json!({ "code": code })).await,
        StatusCode::NOT_FOUND,
        "not_found",
    );
    refused(
        &ask(&mut w.ada, json!({ "user_id": bob })).await,
        StatusCode::FORBIDDEN,
        "blocked",
    );
}

async fn history(client: &mut Client, channel: &str) -> Vec<String> {
    let res = client
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test]
async fn an_existing_dm_stays_readable_and_closes_for_writing(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    w.ada.befriend(&bob).await;
    let dm = open_dm(&mut w.ada, &bob).await;
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    let bobs = text(&mut w.bob, &dm, "vorher").await;
    let bobs = bobs.body["id"].as_str().unwrap().to_owned();
    text(&mut w.ada, &dm, "auch vorher").await;
    let (photo, picture) = picture_message(&mut w.bob, &dm).await;
    block(&mut w.ada, &bob).await;

    // Bob writes on as if nothing happened (#619); Ada is refused.
    let hidden = text(&mut w.bob, &dm, "nachher").await;
    assert_eq!(hidden.status, StatusCode::CREATED, "{}", hidden.body);
    let hidden = hidden.body["id"].as_str().unwrap().to_owned();
    refused(
        &text(&mut w.ada, &dm, "nachher").await,
        StatusCode::FORBIDDEN,
        "blocked",
    );
    // What only Bob reads he may change; an edit of what Ada has seen
    // stays his (GEL-21, `an_edit_of_a_seen_message_stays_the_senders`).
    let fixed = w
        .bob
        .send(
            Method::PATCH,
            &format!("/api/messages/{hidden}"),
            Some(json!({ "content": "nachher!" })),
        )
        .await;
    assert_eq!(fixed.status, StatusCode::OK, "{}", fixed.body);
    let edit = w
        .bob
        .send(
            Method::PATCH,
            &format!("/api/messages/{bobs}"),
            Some(json!({ "content": "geändert" })),
        )
        .await;
    assert_eq!(edit.status, StatusCode::OK, "{}", edit.body);
    // An alt text alone (the text unchanged) is an edit too (#575).
    let alt = w
        .bob
        .send(
            Method::PATCH,
            &format!("/api/messages/{photo}"),
            Some(json!({
                "content": "",
                "attachment_descriptions": { picture: "Ein Hund" },
            })),
        )
        .await;
    assert_eq!(alt.status, StatusCode::OK, "{}", alt.body);

    // Both still read it; each sees only their own side.
    let mine = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(mine.body["blocked"], true, "{}", mine.body);
    assert!(mine.body.get("closed").is_none());
    // Bob's DM looks as before: nothing says it is closed.
    let theirs = w
        .bob
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert!(theirs.body.get("closed").is_none(), "{}", theirs.body);
    assert!(theirs.body.get("blocked").is_none());
    let again = open_dm(&mut w.bob, &ada).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(
        history(&mut w.bob, &dm).await,
        ["geändert", "auch vorher", "", "nachher!"]
    );
    assert_eq!(
        history(&mut w.ada, &dm).await,
        ["vorher", "auch vorher", ""]
    );
    // Ada cannot reach it by id either.
    let jump = w
        .ada
        .send(
            Method::GET,
            &format!("/api/channels/{dm}/messages/{hidden}/context"),
            None,
        )
        .await;
    assert_eq!(jump.status, StatusCode::NOT_FOUND, "{}", jump.body);
    let delete = w
        .ada
        .send(Method::DELETE, &format!("/api/messages/{hidden}"), None)
        .await;
    refused(&delete, StatusCode::NOT_FOUND, "not_found");

    // Unblocking opens it again; the friendship does not come back, and
    // what Bob wrote meanwhile stays his.
    unblock(&mut w.ada, &bob).await;
    assert_eq!(
        text(&mut w.bob, &dm, "wieder").await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        text(&mut w.ada, &dm, "ja").await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        history(&mut w.ada, &dm).await,
        ["vorher", "auch vorher", "", "wieder", "ja"]
    );
    // An answer to the hidden message does not quote it to Ada.
    let answer = w
        .bob
        .send(
            Method::POST,
            &format!("/api/channels/{dm}/messages"),
            Some(json!({ "content": "siehe oben", "reply_to": hidden })),
        )
        .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
    assert!(answer.body.get("reply").is_none(), "{}", answer.body);
    let list = friends(&mut w.ada).await;
    assert_eq!(list["friends"], json!([]));
    let dm_now = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert!(dm_now.body.get("blocked").is_none());
    assert!(dm_now.body.get("request").is_none(), "still accepted");
}

/// The message `id` as `client` reads it in the history of `channel`, or
/// `Null` when it is not there.
async fn read(client: &mut Client, channel: &str, id: &str) -> Value {
    let res = client
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}

/// The message `id` as `client` reads it around it (jump to message).
async fn jump_to(client: &mut Client, channel: &str, id: &str) -> Value {
    let res = client
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/{id}/context"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .cloned()
        .unwrap()
}

async fn edit(client: &mut Client, id: &str, body: Value) -> Response {
    client
        .send(Method::PATCH, &format!("/api/messages/{id}"), Some(body))
        .await
}

/// The ids `client` finds for `q` in `channel`.
async fn found(client: &mut Client, channel: &str, q: &str) -> Vec<String> {
    let res = client
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/search?q={q}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect()
}

/// No alt text on the first attachment of `message`.
fn undescribed(message: &Value) -> bool {
    message["attachments"][0]
        .get("description")
        .is_none_or(Value::is_null)
}

/// GEL-21: the one blocked edits a message the blocker has already seen.
/// The edit answers as ever and is stored as their own view; the blocker
/// keeps what they saw, also after an unblock.
#[sqlx::test]
async fn an_edit_of_a_seen_message_stays_the_senders(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    w.ada.befriend(&bob).await;
    let dm = open_dm(&mut w.ada, &bob).await;
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    let sent = text(&mut w.bob, &dm, "alter text").await;
    let seen = sent.body["id"].as_str().unwrap().to_owned();
    let (photo, picture) = picture_message(&mut w.bob, &dm).await;
    let before = read(&mut w.ada, &dm, &seen).await;
    block(&mut w.ada, &bob).await;

    // Bob's edit answers like any edit.
    let bobs_text = format!("<@{ada}> neuer text");
    let edited = edit(&mut w.bob, &seen, json!({ "content": bobs_text })).await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(
        edited.body["content"],
        bobs_text.as_str(),
        "{}",
        edited.body
    );
    assert!(edited.body["edited_at"].is_string(), "{}", edited.body);
    assert!(
        edited.body["revision"].as_i64().unwrap() > before["revision"].as_i64().unwrap(),
        "{}",
        edited.body
    );
    // He reads his text everywhere, by history and by id ...
    let his = read(&mut w.bob, &dm, &seen).await;
    assert_eq!(his["content"], bobs_text.as_str(), "{his}");
    assert_eq!(his["edited_at"], edited.body["edited_at"], "{his}");
    assert_eq!(his["revision"], edited.body["revision"], "{his}");
    assert_eq!(
        jump_to(&mut w.bob, &dm, &seen).await["content"],
        bobs_text.as_str()
    );
    // ... and Ada what she saw, down to the revision.
    assert_eq!(read(&mut w.ada, &dm, &seen).await, before);
    assert_eq!(
        jump_to(&mut w.ada, &dm, &seen).await["content"],
        "alter text"
    );
    // Search goes by the text each reads.
    assert_eq!(
        found(&mut w.bob, &dm, "neuer").await,
        std::slice::from_ref(&seen)
    );
    assert!(found(&mut w.bob, &dm, "alter").await.is_empty());
    assert!(found(&mut w.ada, &dm, "neuer").await.is_empty());
    assert_eq!(
        found(&mut w.ada, &dm, "alter").await,
        std::slice::from_ref(&seen)
    );
    // The mention he added pings nobody.
    assert_eq!(unread_row(&mut w.ada, &dm).await["mention_count"], 0);

    // An alt text alone (#575) goes the same way.
    let alt = edit(
        &mut w.bob,
        &photo,
        json!({ "content": "", "attachment_descriptions": { &picture: "Ein Hund" } }),
    )
    .await;
    assert_eq!(alt.status, StatusCode::OK, "{}", alt.body);
    assert_eq!(alt.body["attachments"][0]["description"], "Ein Hund");
    assert_eq!(
        read(&mut w.bob, &dm, &photo).await["attachments"][0]["description"],
        "Ein Hund"
    );
    assert!(undescribed(&read(&mut w.ada, &dm, &photo).await));
    // His answer to it quotes his own text to him.
    let answer = w
        .bob
        .send(
            Method::POST,
            &format!("/api/channels/{dm}/messages"),
            Some(json!({ "content": "siehe oben", "reply_to": seen })),
        )
        .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
    assert_eq!(
        answer.body["reply"]["excerpt"],
        bobs_text.as_str(),
        "{}",
        answer.body
    );
    // A reaction's answer shows his text too.
    let reacted = react(&mut w.bob, &seen).await;
    assert_eq!(reacted.status, StatusCode::OK, "{}", reacted.body);
    assert_eq!(reacted.body["content"], bobs_text.as_str());
    // Edited again, and once with the text unchanged: all stays his.
    let again = edit(&mut w.bob, &seen, json!({ "content": "noch neuer" })).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.body["content"], "noch neuer");
    let same = edit(&mut w.bob, &seen, json!({ "content": "noch neuer" })).await;
    assert_eq!(same.body["content"], "noch neuer", "{}", same.body);

    // Unblocking shows Ada none of it.
    unblock(&mut w.ada, &bob).await;
    let hers = read(&mut w.ada, &dm, &seen).await;
    assert_eq!(hers["content"], "alter text", "{hers}");
    assert!(hers["edited_at"].is_null(), "{hers}");
    assert!(undescribed(&read(&mut w.ada, &dm, &photo).await));
    assert_eq!(unread_row(&mut w.ada, &dm).await["mention_count"], 0);
    assert_eq!(read(&mut w.bob, &dm, &seen).await["content"], "noch neuer");
    assert_eq!(
        read(&mut w.bob, &dm, &photo).await["attachments"][0]["description"],
        "Ein Hund"
    );
    // Her reaction's answer keeps her text, and he keeps his.
    let hers = react(&mut w.ada, &seen).await;
    assert_eq!(hers.status, StatusCode::OK, "{}", hers.body);
    assert_eq!(hers.body["content"], "alter text", "{}", hers.body);
    assert_eq!(read(&mut w.bob, &dm, &seen).await["content"], "noch neuer");

    // An edit after the unblock is everyone's again: back to the text she
    // reads is an edit for him all the same.
    let back = edit(&mut w.bob, &seen, json!({ "content": "alter text" })).await;
    assert_eq!(back.status, StatusCode::OK, "{}", back.body);
    for client in [&mut w.ada, &mut w.bob] {
        let message = read(client, &dm, &seen).await;
        assert_eq!(message["content"], "alter text", "{message}");
        assert!(message["edited_at"].is_string(), "{message}");
    }
    // Dropping only his own alt text changes nothing she reads, and sends
    // her no event: her copy does not take its revision. (Hers may still
    // move by the picture's own processing, which runs on its own time.)
    let mut photo_before = read(&mut w.ada, &dm, &photo).await;
    let dropped = edit(
        &mut w.bob,
        &photo,
        json!({ "attachment_descriptions": { &picture: null } }),
    )
    .await;
    assert_eq!(dropped.status, StatusCode::OK, "{}", dropped.body);
    assert!(undescribed(&dropped.body), "{}", dropped.body);
    assert!(undescribed(&read(&mut w.bob, &dm, &photo).await));
    let mut photo_after = read(&mut w.ada, &dm, &photo).await;
    assert_ne!(photo_after["revision"], dropped.body["revision"]);
    for photo in [&mut photo_before, &mut photo_after] {
        photo.as_object_mut().unwrap().remove("revision");
    }
    assert_eq!(photo_after, photo_before);
    let open = edit(&mut w.bob, &seen, json!({ "content": "für alle" })).await;
    assert_eq!(open.status, StatusCode::OK, "{}", open.body);
    for client in [&mut w.ada, &mut w.bob] {
        let message = read(client, &dm, &seen).await;
        assert_eq!(message["content"], "für alle", "{message}");
        assert!(message["edited_at"].is_string(), "{message}");
    }
    let cat = edit(
        &mut w.bob,
        &photo,
        json!({ "attachment_descriptions": { &picture: "Eine Katze" } }),
    )
    .await;
    assert_eq!(cat.status, StatusCode::OK, "{}", cat.body);
    for client in [&mut w.ada, &mut w.bob] {
        assert_eq!(
            read(client, &dm, &photo).await["attachments"][0]["description"],
            "Eine Katze"
        );
    }

    // Under a new block, deleting a message Ada saw is a delete as ever,
    // also after an edit of his own: it is gone for both.
    block(&mut w.ada, &bob).await;
    let private = edit(&mut w.bob, &photo, json!({ "content": "weg damit" })).await;
    assert_eq!(private.status, StatusCode::OK, "{}", private.body);
    let gone = w
        .bob
        .send(Method::DELETE, &format!("/api/messages/{photo}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    assert!(read(&mut w.bob, &dm, &photo).await.is_null());
    assert!(read(&mut w.ada, &dm, &photo).await.is_null());
}

#[sqlx::test]
async fn blocking_ends_friendship_and_requests(pool: PgPool) {
    let state = common::state(pool);
    let mut w = world(state.clone()).await;
    let mut cara = person(&state, "cara@example.com", "Cara").await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    let cara_id = cara.user_id().to_owned();
    w.ada.befriend(&bob).await;
    // Cara asked Ada; Ada blocks her, and the request goes.
    let code = w.ada.send(Method::GET, "/api/me/friend-code", None).await;
    let code = code.body["code"].as_str().unwrap().to_owned();
    assert_eq!(
        ask(&mut cara, json!({ "code": code })).await.status,
        StatusCode::CREATED
    );
    block(&mut w.ada, &bob).await;
    block(&mut w.ada, &cara_id).await;
    let list = friends(&mut w.ada).await;
    assert_eq!(list["friends"], json!([]));
    assert_eq!(list["incoming"], json!([]));
    let theirs = friends(&mut cara).await;
    assert_eq!(theirs["outgoing"], json!([]));
    assert_eq!(friends(&mut w.bob).await["friends"], json!([]));

    // A message request from Bob leaves Ada's requests when she blocks.
    unblock(&mut w.ada, &bob).await;
    let dm = open_dm(&mut w.bob, &ada).await;
    assert_eq!(dm.status, StatusCode::CREATED, "{}", dm.body);
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    text(&mut w.bob, &dm, "hallo").await;
    let res = w.ada.send(Method::GET, "/api/dms/requests", None).await;
    assert_eq!(res.body.as_array().unwrap().len(), 1);
    block(&mut w.ada, &bob).await;
    let res = w.ada.send(Method::GET, "/api/dms/requests", None).await;
    assert_eq!(res.body, json!([]));
    // Bob writes on into his request, within its limit, unseen (#619).
    assert_eq!(
        text(&mut w.bob, &dm, "noch was").await.status,
        StatusCode::CREATED
    );
    let mine = w
        .bob
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(mine.body["request"]["remaining"], 1, "{}", mine.body);
    refused(
        &text(&mut w.bob, &dm, "https://example.com").await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
    );
    assert_eq!(
        text(&mut w.bob, &dm, "letzte").await.status,
        StatusCode::CREATED
    );
    refused(
        &text(&mut w.bob, &dm, "zu viel").await,
        StatusCode::CONFLICT,
        "message_request_limit",
    );
    let res = w.ada.send(Method::GET, "/api/dms/requests", None).await;
    assert_eq!(res.body, json!([]));
}

async fn react(client: &mut Client, message: &str) -> Response {
    client
        .send(
            Method::PUT,
            &format!("/api/messages/{message}/reactions/%F0%9F%91%8D"),
            None,
        )
        .await
}

#[sqlx::test]
async fn reactions_in_a_closed_dm_reach_nobody(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let bob = w.bob.user_id().to_owned();
    w.ada.befriend(&bob).await;
    let dm = open_dm(&mut w.ada, &bob).await;
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    let adas = text(&mut w.ada, &dm, "von ada").await;
    let adas = adas.body["id"].as_str().unwrap().to_owned();
    let bobs = text(&mut w.bob, &dm, "von bob").await;
    let bobs = bobs.body["id"].as_str().unwrap().to_owned();
    block(&mut w.ada, &bob).await;

    // Bob's reaction on Ada's message sticks for him and only him (#619).
    let hidden = react(&mut w.bob, &adas).await;
    assert_eq!(hidden.status, StatusCode::OK, "{}", hidden.body);
    assert_eq!(
        hidden.body["reactions"][0]["emoji"], "👍",
        "{}",
        hidden.body
    );
    let reactions_on = |page: &Response, id: &str| {
        page.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()["reactions"]
            .clone()
    };
    let history = format!("/api/channels/{dm}/messages");
    let seen = w.ada.send(Method::GET, &history, None).await;
    assert_eq!(reactions_on(&seen, &adas), json!([]), "{}", seen.body);
    let own = w.bob.send(Method::GET, &history, None).await;
    assert_eq!(
        reactions_on(&own, &adas)[0]["user_ids"],
        json!([bob]),
        "{}",
        own.body
    );
    // On his own hidden message it sticks, for him.
    let hidden = text(&mut w.bob, &dm, "nur für mich").await;
    let hidden = hidden.body["id"].as_str().unwrap().to_owned();
    let own = react(&mut w.bob, &hidden).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(own.body["reactions"][0]["emoji"], "👍", "{}", own.body);
    refused(
        &react(&mut w.ada, &bobs).await,
        StatusCode::FORBIDDEN,
        "blocked",
    );
    // Given again after the unblock, it is open.
    unblock(&mut w.ada, &bob).await;
    let removed = w
        .bob
        .send(
            Method::DELETE,
            &format!("/api/messages/{adas}/reactions/%F0%9F%91%8D"),
            None,
        )
        .await;
    assert_eq!(removed.body["reactions"], json!([]), "{}", removed.body);
    assert_eq!(react(&mut w.bob, &adas).await.status, StatusCode::OK);
    let seen = w.ada.send(Method::GET, &history, None).await;
    assert_eq!(
        reactions_on(&seen, &adas)[0]["user_ids"],
        json!([bob]),
        "{}",
        seen.body
    );
    block(&mut w.ada, &bob).await;
    // On a shared server reactions stay open: the clients fold there.
    let open = text(&mut w.bob, &w.channel.clone(), "im server").await;
    let open = open.body["id"].as_str().unwrap().to_owned();
    assert_eq!(react(&mut w.ada, &open).await.status, StatusCode::OK);
}

fn reaction_path(message: &str, emoji: &str) -> String {
    let encoded: String = emoji.bytes().map(|b| format!("%{b:02X}")).collect();
    format!("/api/messages/{message}/reactions/{encoded}")
}

#[sqlx::test]
async fn hidden_reactions_leave_the_blockers_emoji_slots_free(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let bob = w.bob.user_id().to_owned();
    w.ada.befriend(&bob).await;
    let dm = open_dm(&mut w.ada, &bob).await;
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    let adas = text(&mut w.ada, &dm, "von ada").await;
    let adas = adas.body["id"].as_str().unwrap().to_owned();
    block(&mut w.ada, &bob).await;

    // Bob fills all 20 slots on Ada's message, hidden (#619).
    let emojis = [
        "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "😊", "😇", "🙂", "🙃", "😉", "😌", "🥰",
        "😗", "😙", "😚", "😋", "😍",
    ];
    for emoji in emojis {
        let res = w
            .bob
            .send(Method::PUT, &reaction_path(&adas, emoji), None)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{emoji}: {}", res.body);
    }
    // His limit holds for him.
    refused(
        &w.bob
            .send(Method::PUT, &reaction_path(&adas, "😘"), None)
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
    );
    // They stay hidden after the unblock, so they must not use up Ada's
    // slots: she still reacts on her message and sees only her reaction.
    unblock(&mut w.ada, &bob).await;
    let own = w
        .ada
        .send(Method::PUT, &reaction_path(&adas, "😎"), None)
        .await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(
        own.body["reactions"],
        json!([{"emoji": "😎", "user_ids": [w.ada.user_id()]}]),
        "{}",
        own.body
    );
}

#[sqlx::test]
async fn the_blockers_own_request_waits_out_the_block(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let bob = w.bob.user_id().to_owned();
    // Ada writes Bob a message request, then blocks him.
    let dm = open_dm(&mut w.ada, &bob).await;
    assert_eq!(dm.status, StatusCode::CREATED, "{}", dm.body);
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    assert_eq!(
        text(&mut w.ada, &dm, "hallo").await.status,
        StatusCode::CREATED
    );
    block(&mut w.ada, &bob).await;

    // Bob neither sees nor accepts it while the block stands.
    let res = w.bob.send(Method::GET, "/api/dms/requests", None).await;
    assert_eq!(res.body, json!([]));
    let accept = w
        .bob
        .send(Method::POST, &format!("/api/dms/{dm}/accept"), None)
        .await;
    refused(&accept, StatusCode::NOT_FOUND, "not_found");

    // After the unblock it is live again, not silently declined.
    unblock(&mut w.ada, &bob).await;
    assert_eq!(
        text(&mut w.ada, &dm, "nochmal").await.status,
        StatusCode::CREATED
    );
    let res = w.bob.send(Method::GET, "/api/dms/requests", None).await;
    assert_eq!(res.body.as_array().unwrap().len(), 1, "{}", res.body);
    let accept = w
        .bob
        .send(Method::POST, &format!("/api/dms/{dm}/accept"), None)
        .await;
    assert_eq!(accept.status, StatusCode::OK, "{}", accept.body);
}

async fn unread_row(client: &mut Client, channel: &str) -> Value {
    let res = client.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["channel_id"] == channel)
        .cloned()
        .unwrap_or(Value::Null)
}

#[sqlx::test]
async fn mentions_from_someone_blocked_do_not_count(pool: PgPool) {
    let mut w = world(common::state(pool)).await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    block(&mut w.ada, &bob).await;
    let channel = w.channel.clone();
    assert_eq!(
        text(&mut w.bob, &channel, &format!("<@{ada}> hallo"))
            .await
            .status,
        StatusCode::CREATED
    );
    text(&mut w.bob, &channel, "essen").await;
    let row = unread_row(&mut w.ada, &channel).await;
    // The messages are there (the client folds them); they ping nobody.
    assert_eq!(row["unread_count"], 2, "{row}");
    assert_eq!(row["mention_count"], 0, "{row}");
    unblock(&mut w.ada, &bob).await;
    let row = unread_row(&mut w.ada, &channel).await;
    assert_eq!(row["mention_count"], 1, "{row}");
}

#[sqlx::test]
async fn account_deletion_takes_its_blocks_both_ways(pool: PgPool) {
    let mut w = world(common::state(pool.clone())).await;
    let bob = w.bob.user_id().to_owned();
    let ada = w.ada.user_id().to_owned();
    block(&mut w.ada, &bob).await;
    block(&mut w.bob, &ada).await;
    let res = w
        .bob
        .send(
            Method::DELETE,
            "/api/me",
            Some(json!({ "password": PASSWORD })),
        )
        .await;
    assert!(res.status.is_success(), "{}", res.body);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM user_blocks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

// ---- Live events ----------------------------------------------------------

#[sqlx::test]
async fn only_the_blocker_hears_of_a_block(pool: PgPool) {
    let state = common::ws_state(pool);
    let addr = common::serve_ws_state(state.clone()).await;
    let mut w = world(state).await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    // Bob's message request to Ada is declined by the block.
    let dm = open_dm(&mut w.bob, &ada).await;
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    text(&mut w.bob, &dm, "hallo").await;
    let mut ada_ws = connect(addr, &w.ada).await;
    let mut bob_ws = connect(addr, &w.bob).await;
    ws::registered(&mut ada_ws).await;
    ws::registered(&mut bob_ws).await;

    block(&mut w.ada, &bob).await;
    let bl = |f: &Value| f["op"] == "bl";
    let mr = |f: &Value| f["op"] == "mr";
    assert!(
        within(&mut ada_ws, Duration::from_secs(5), bl)
            .await
            .is_some()
    );
    assert!(
        within(&mut ada_ws, Duration::from_secs(5), mr)
            .await
            .is_some(),
        "Ada's incoming requests changed"
    );
    assert!(
        within(&mut bob_ws, Duration::from_millis(500), mr)
            .await
            .is_none(),
        "Bob's did not"
    );

    // Bob's typing in the closed DM reaches nobody.
    for ws in [&mut ada_ws, &mut bob_ws] {
        ws.send(Message::Text(
            json!({ "op": "s", "s": dm, "c": dm }).to_string().into(),
        ))
        .await
        .unwrap();
        assert!(
            within(ws, Duration::from_secs(5), |f| f["op"] == "ok")
                .await
                .is_some()
        );
    }
    bob_ws
        .send(Message::Text(
            json!({ "op": "y", "s": dm, "c": dm, "on": true })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert!(
        within(&mut ada_ws, Duration::from_millis(700), |f| f["op"] == "y")
            .await
            .is_none(),
        "no typing across a block"
    );
    unblock(&mut w.ada, &bob).await;
    assert!(
        within(&mut ada_ws, Duration::from_secs(5), bl)
            .await
            .is_some()
    );
    assert!(
        within(&mut bob_ws, Duration::from_millis(500), bl)
            .await
            .is_none(),
        "Bob is not told"
    );
}

#[sqlx::test]
async fn the_blocker_hears_nothing_of_hidden_messages(pool: PgPool) {
    let state = common::ws_state(pool);
    let addr = common::serve_ws_state(state.clone()).await;
    let mut w = world(state.clone()).await;
    let mut cara = person(&state, "cara@example.com", "Cara").await;
    let ada = w.ada.user_id().to_owned();
    let bob = w.bob.user_id().to_owned();
    let cara_id = cara.user_id().to_owned();
    w.ada.befriend(&bob).await;
    w.ada.befriend(&cara_id).await;
    let with_bob = open_dm(&mut w.ada, &bob).await;
    let with_bob = with_bob.body["id"].as_str().unwrap().to_owned();
    text(&mut w.ada, &with_bob, "hallo bob").await;
    let with_cara = open_dm(&mut w.ada, &cara_id).await;
    let with_cara = with_cara.body["id"].as_str().unwrap().to_owned();
    text(&mut cara, &with_cara, "hallo ada").await;
    // Ada reads both, then blocks Bob.
    for dm in [&with_bob, &with_cara] {
        let res = w
            .ada
            .send(
                Method::PUT,
                &format!("/api/channels/{dm}/read"),
                Some(json!({})),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }
    block(&mut w.ada, &bob).await;
    let mut ada_ws = connect(addr, &w.ada).await;
    let mut bob_ws = connect(addr, &w.bob).await;
    for (ws, dm) in [(&mut ada_ws, &with_bob), (&mut bob_ws, &with_bob)] {
        ws.send(Message::Text(
            json!({ "op": "s", "s": dm, "c": dm }).to_string().into(),
        ))
        .await
        .unwrap();
        assert!(
            within(ws, Duration::from_secs(5), |f| f["op"] == "ok")
                .await
                .is_some()
        );
    }

    let sent = text(&mut w.bob, &with_bob, &format!("<@{ada}> geheim")).await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    let id = sent.body["id"].as_str().unwrap().to_owned();
    let edited = w
        .bob
        .send(
            Method::PATCH,
            &format!("/api/messages/{id}"),
            Some(json!({ "content": "geheim!" })),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    let adas: Vec<Value> = w
        .ada
        .send(
            Method::GET,
            &format!("/api/channels/{with_bob}/messages"),
            None,
        )
        .await
        .body["messages"]
        .as_array()
        .unwrap()
        .clone();
    let adas = adas[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(react(&mut w.bob, &adas).await.status, StatusCode::OK);
    let event = |f: &Value| f["op"] == "e";
    assert!(
        within(&mut ada_ws, Duration::from_millis(800), event)
            .await
            .is_none(),
        "no event reaches Ada"
    );

    // Nothing unread, no mention, and Bob's DM does not move up for Ada.
    let row = unread_row(&mut w.ada, &with_bob).await;
    assert_eq!(row["unread_count"], 0, "{row}");
    assert_eq!(row["mention_count"], 0, "{row}");
    let list = w.ada.send(Method::GET, "/api/dms", None).await;
    let order: Vec<&str> = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        [with_cara.as_str(), with_bob.as_str()],
        "{}",
        list.body
    );
    let list = w.bob.send(Method::GET, "/api/dms", None).await;
    assert_eq!(list.body[0]["id"], with_bob.as_str(), "{}", list.body);

    // Search finds it for Bob only.
    let search = format!("/api/channels/{with_bob}/messages/search?q=geheim");
    let theirs = w.ada.send(Method::GET, &search, None).await;
    assert_eq!(theirs.body["messages"], json!([]), "{}", theirs.body);
    let mine = w.bob.send(Method::GET, &search, None).await;
    assert_eq!(
        mine.body["messages"].as_array().unwrap().len(),
        1,
        "{}",
        mine.body
    );

    // Deleting it tells nobody either.
    let gone = w
        .bob
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    assert!(
        within(&mut ada_ws, Duration::from_millis(800), event)
            .await
            .is_none(),
        "no delete event reaches Ada"
    );
}

#[sqlx::test]
async fn an_edit_of_a_seen_message_reaches_his_sockets_alone(pool: PgPool) {
    let state = common::ws_state(pool);
    let addr = common::serve_ws_state(state.clone()).await;
    let mut w = world(state).await;
    let bob = w.bob.user_id().to_owned();
    w.ada.befriend(&bob).await;
    let dm = open_dm(&mut w.ada, &bob).await;
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    let sent = text(&mut w.bob, &dm, "alt").await;
    let seen = sent.body["id"].as_str().unwrap().to_owned();
    block(&mut w.ada, &bob).await;
    let mut ada_ws = connect(addr, &w.ada).await;
    let mut bob_ws = connect(addr, &w.bob).await;
    let mut head = Value::Null;
    for ws in [&mut ada_ws, &mut bob_ws] {
        ws.send(Message::Text(
            json!({ "op": "s", "s": dm, "c": dm }).to_string().into(),
        ))
        .await
        .unwrap();
        head = within(ws, Duration::from_secs(5), |f| f["op"] == "ok")
            .await
            .expect("subscribed");
    }

    // His edit of what she saw goes out to his sockets alone.
    let edited = edit(&mut w.bob, &seen, json!({ "content": "neu" })).await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    let event = |f: &Value| f["op"] == "e";
    let his = within(&mut bob_ws, Duration::from_secs(5), event)
        .await
        .expect("his edit event");
    assert_eq!(his["t"], "e", "{his}");
    assert_eq!(his["d"]["content"], "neu", "{his}");
    assert_eq!(his["d"]["edited_at"], edited.body["edited_at"], "{his}");
    assert_eq!(his["r"], edited.body["revision"], "{his}");
    assert!(his["d"].get("author_only").is_none(), "{his}");
    assert!(
        within(&mut ada_ws, Duration::from_millis(800), event)
            .await
            .is_none(),
        "no event for her"
    );
    // Nor when she resumes from before it.
    for (client, expect) in [(&w.ada, false), (&w.bob, true)] {
        let mut ws = connect(addr, client).await;
        ws.send(Message::Text(
            json!({ "op": "s", "s": dm, "c": dm, "n": head["n"], "ep": head["ep"] })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let first = within(&mut ws, Duration::from_secs(5), |f| {
            f["op"] == "e" || f["op"] == "ok"
        })
        .await
        .expect("resumed");
        assert_eq!(first["op"] == "e", expect, "{first}");
    }
    // From the start she still replays what she saw: his edit rewrote
    // none of the earlier entries.
    let mut ws = connect(addr, &w.ada).await;
    ws.send(Message::Text(
        json!({ "op": "s", "s": dm, "c": dm, "n": 0, "ep": head["ep"] })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let replayed = within(&mut ws, Duration::from_secs(5), |f| {
        (f["op"] == "e" && f["i"] == seen) || f["op"] == "ok"
    })
    .await
    .expect("replayed");
    assert_eq!(replayed["op"], "e", "{replayed}");
    assert_eq!(replayed["d"]["content"], "alt", "{replayed}");

    // After the unblock her reaction goes out to both, each with the text
    // they read.
    unblock(&mut w.ada, &bob).await;
    assert_eq!(react(&mut w.ada, &seen).await.status, StatusCode::OK);
    let hers = within(&mut ada_ws, Duration::from_secs(5), event)
        .await
        .expect("her reaction event");
    assert_eq!(hers["d"]["content"], "alt", "{hers}");
    assert!(hers["d"]["edited_at"].is_null(), "{hers}");
    assert_eq!(hers["d"]["reactions"][0]["emoji"], "👍", "{hers}");
    let his = within(&mut bob_ws, Duration::from_secs(5), event)
        .await
        .expect("her reaction event, for him");
    assert_eq!(his["d"]["content"], "neu", "{his}");
    assert_eq!(his["d"]["edited_at"], edited.body["edited_at"], "{his}");
    assert_eq!(his["d"]["reactions"][0]["emoji"], "👍", "{his}");
    assert_eq!(his["r"], hers["r"]);
    for frame in [&hers, &his] {
        assert!(frame["d"].get("author_view").is_none(), "{frame}");
    }

    // Her answer to it quotes to each the text they read.
    let answer = w
        .ada
        .send(
            Method::POST,
            &format!("/api/channels/{dm}/messages"),
            Some(json!({ "content": "worauf?", "reply_to": seen })),
        )
        .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
    assert_eq!(answer.body["reply"]["excerpt"], "alt", "{}", answer.body);
    let created = |f: &Value| f["op"] == "e" && f["t"] == "c";
    let hers = within(&mut ada_ws, Duration::from_secs(5), created)
        .await
        .expect("her answer");
    assert_eq!(hers["d"]["reply"]["excerpt"], "alt", "{hers}");
    let his = within(&mut bob_ws, Duration::from_secs(5), created)
        .await
        .expect("her answer, for him");
    assert_eq!(his["d"]["reply"]["excerpt"], "neu", "{his}");
    for frame in [&hers, &his] {
        assert!(frame["d"].get("reply_author_view").is_none(), "{frame}");
    }
    assert_eq!(
        read(&mut w.bob, &dm, answer.body["id"].as_str().unwrap()).await["reply"]["excerpt"],
        "neu"
    );
}

/// GEL-21: his own text shows the previews of its own links only.
#[sqlx::test]
async fn an_edit_of_a_seen_message_keeps_only_its_own_previews(pool: PgPool) {
    let mut w = world(common::state(pool.clone())).await;
    let bob = w.bob.user_id().to_owned();
    w.ada.befriend(&bob).await;
    let dm = open_dm(&mut w.ada, &bob).await;
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    let sent = text(&mut w.bob, &dm, "siehe https://example.org/").await;
    let seen = sent.body["id"].as_str().unwrap().to_owned();
    sqlx::query(
        "WITH p AS (INSERT INTO link_previews (url_hash, url, status, title) \
         VALUES (sha256('https://example.org/'), 'https://example.org/', 'ok', 'Beispiel') RETURNING id) \
         INSERT INTO message_previews (message_id, preview_id, position) SELECT $1::uuid, id, 0 FROM p",
    )
    .bind(&seen)
    .execute(&pool)
    .await
    .unwrap();
    block(&mut w.ada, &bob).await;

    let edited = edit(&mut w.bob, &seen, json!({ "content": "ohne link" })).await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert!(edited.body.get("previews").is_none(), "{}", edited.body);
    assert!(read(&mut w.bob, &dm, &seen).await.get("previews").is_none());
    assert_eq!(
        read(&mut w.ada, &dm, &seen).await["previews"][0]["url"],
        "https://example.org/"
    );
    // With the link back in his text, so is its preview.
    let again = edit(
        &mut w.bob,
        &seen,
        json!({ "content": "doch https://example.org/" }),
    )
    .await;
    assert_eq!(again.body["previews"][0]["url"], "https://example.org/");
}
