//! Friends, friend codes and message requests (#563): who may write whom,
//! the request limits, the friend request lifecycle and their events.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::AppState;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

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
    /// Shares no server with anyone.
    cara: Client,
}

/// Ada and Bob share a server; Cara is a stranger to both.
async fn world(pool: PgPool) -> World {
    world_with(common::state(pool)).await
}

async fn world_with(state: AppState) -> World {
    let mut ada = person(&state, "ada@example.com", "Ada").await;
    let mut bob = person(&state, "bob@example.com", "Bob").await;
    let cara = person(&state, "cara@example.com", "Cara").await;
    let server = ada
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Club" })),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED, "{}", server.body);
    let sid = server.body["id"].as_str().unwrap().to_owned();
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
    World { ada, bob, cara }
}

async fn open_dm(client: &mut Client, user_id: &str) -> Response {
    client
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": user_id })),
        )
        .await
}

async fn say(client: &mut Client, channel: &str, body: Value) -> Response {
    client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(body),
        )
        .await
}

async fn text(client: &mut Client, channel: &str, content: &str) -> Response {
    say(client, channel, json!({ "content": content })).await
}

async fn requests(client: &mut Client) -> Vec<Value> {
    let res = client.send(Method::GET, "/api/dms/requests", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body.as_array().unwrap().clone()
}

async fn dm_ids(client: &mut Client) -> Vec<String> {
    let res = client.send(Method::GET, "/api/dms", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
        .as_array()
        .unwrap()
        .iter()
        .map(|dm| dm["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn friends(client: &mut Client) -> Value {
    let res = client.send(Method::GET, "/api/friends", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

async fn ask(client: &mut Client, body: Value) -> Response {
    client
        .send(Method::POST, "/api/friends/requests", Some(body))
        .await
}

async fn code_of(client: &mut Client) -> String {
    let res = client.send(Method::GET, "/api/me/friend-code", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["code"].as_str().unwrap().to_owned()
}

fn uuid(raw: &str) -> Uuid {
    raw.parse().unwrap()
}

// ---- Who may open a DM (one test per row of the issue's table) ----------

#[sqlx::test]
async fn friends_open_an_ordinary_dm(pool: PgPool) {
    let World {
        mut ada, mut cara, ..
    } = world(pool.clone()).await;
    common::befriend(&pool, ada.user_id(), cara.user_id()).await;
    let res = open_dm(&mut ada, cara.user_id()).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert!(res.body.get("request").is_none(), "{}", res.body);
    let dm = res.body["id"].as_str().unwrap().to_owned();
    let sent = say(
        &mut ada,
        &dm,
        json!({ "content": "https://example.com @everyone" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    assert_eq!(dm_ids(&mut cara).await, vec![dm]);
}

#[sqlx::test]
async fn people_on_a_shared_server_start_a_message_request(pool: PgPool) {
    let World {
        mut ada, mut bob, ..
    } = world(pool).await;
    let res = open_dm(&mut ada, bob.user_id()).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(
        res.body["request"],
        json!({ "incoming": false, "remaining": 3 })
    );
    let dm = res.body["id"].as_str().unwrap().to_owned();
    // An empty request is nothing to answer yet.
    assert!(requests(&mut bob).await.is_empty());
    assert!(dm_ids(&mut bob).await.is_empty());

    assert_eq!(
        text(&mut ada, &dm, "Hi Bob!").await.status,
        StatusCode::CREATED
    );
    let incoming = requests(&mut bob).await;
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0]["id"], dm.as_str());
    assert_eq!(incoming[0]["peer"]["name"], "Ada");
    assert_eq!(incoming[0]["request"]["incoming"], true);
    assert!(dm_ids(&mut bob).await.is_empty(), "not in the DM list");
    // Ada sees it as her own DM with the count left.
    let mine = ada.send(Method::GET, &format!("/api/dms/{dm}"), None).await;
    assert_eq!(mine.body["request"]["remaining"], 2);
    // Unread badges leave the request out for Bob.
    let unread = bob.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(unread.status, StatusCode::OK, "{}", unread.body);
    assert!(!unread.body.to_string().contains(&dm), "{}", unread.body);
}

#[sqlx::test]
async fn strangers_cannot_open_a_dm(pool: PgPool) {
    let World {
        mut ada, mut cara, ..
    } = world(pool.clone()).await;
    let res = open_dm(&mut cara, ada.user_id()).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "dm_not_allowed");
    let res = open_dm(&mut ada, cara.user_id()).await;
    assert_eq!(res.body["error"], "dm_not_allowed");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM channels WHERE kind = 'dm'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn an_accepted_dm_stays_open_after_the_friendship_ends(pool: PgPool) {
    let World {
        mut ada, mut cara, ..
    } = world(pool.clone()).await;
    common::befriend(&pool, ada.user_id(), cara.user_id()).await;
    let dm = open_dm(&mut ada, cara.user_id()).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let res = cara
        .send(
            Method::DELETE,
            &format!("/api/friends/{}", ada.user_id()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    // Strangers again, but the DM was accepted: both may write, and opening
    // it again finds it.
    assert_eq!(
        text(&mut cara, &dm, "still here").await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        text(&mut ada, &dm, "me too").await.status,
        StatusCode::CREATED
    );
    let again = open_dm(&mut cara, ada.user_id()).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.body["id"], dm.as_str());
}

#[sqlx::test]
async fn dms_from_before_the_update_count_as_accepted(pool: PgPool) {
    let World {
        mut ada, mut cara, ..
    } = world(pool.clone()).await;
    // A DM row as 0004 made them, between people who are now strangers.
    let key = gelabber_api::dms::pair_key(uuid(ada.user_id()), uuid(cara.user_id()));
    let dm: Uuid = sqlx::query_scalar(
        "INSERT INTO channels (name, kind, pair_key) VALUES ('', 'dm', $1) RETURNING id",
    )
    .bind(&key)
    .fetch_one(&pool)
    .await
    .unwrap();
    for user in [ada.user_id(), cara.user_id()] {
        sqlx::query("INSERT INTO channel_members (channel_id, user_id) VALUES ($1, $2::uuid)")
            .bind(dm)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    let dm = dm.to_string();
    assert_eq!(dm_ids(&mut cara).await, vec![dm.clone()]);
    assert_eq!(
        say(&mut cara, &dm, json!({ "content": "https://example.com" }))
            .await
            .status,
        StatusCode::CREATED
    );
    assert_eq!(text(&mut ada, &dm, "hi").await.status, StatusCode::CREATED);
}

// ---- Message request limits ---------------------------------------------

async fn request_dm(world: &mut World) -> String {
    let bob = world.bob.user_id().to_owned();
    open_dm(&mut world.ada, &bob).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[sqlx::test]
async fn a_request_holds_three_messages(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    for n in 0..3 {
        let res = text(&mut w.ada, &dm, &format!("message {n}")).await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    }
    let res = text(&mut w.ada, &dm, "one more").await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "message_request_limit");
    let mine = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(mine.body["request"]["remaining"], 0);
}

#[sqlx::test]
async fn the_recipient_cannot_write_before_accepting(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    text(&mut w.ada, &dm, "hello").await;
    let res = text(&mut w.bob, &dm, "who is this?").await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "message_request_pending");
    // History is readable, to decide.
    let history = w
        .bob
        .send(Method::GET, &format!("/api/channels/{dm}/messages"), None)
        .await;
    assert_eq!(history.status, StatusCode::OK);
    assert_eq!(history.body["messages"].as_array().unwrap().len(), 1);
}

#[sqlx::test]
async fn a_request_refuses_links(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    for content in [
        "https://evil.example/login",
        "check www.example.org",
        "paypal-support.com",
        "[here](evil.com)",
    ] {
        let res = text(&mut w.ada, &dm, content).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{content}");
        assert_eq!(res.body["fields"]["content"], "request_link", "{content}");
    }
}

#[sqlx::test]
async fn a_request_refuses_mentions(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    let bob = w.bob.user_id().to_owned();
    for content in [format!("hey <@{bob}>"), "@everyone".into(), "@here".into()] {
        let res = text(&mut w.ada, &dm, &content).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{content}");
        assert_eq!(res.body["fields"]["content"], "request_mention");
    }
}

#[sqlx::test]
async fn a_request_refuses_attachments(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    let res = say(
        &mut w.ada,
        &dm,
        json!({ "content": "look", "attachment_ids": [Uuid::new_v4()] }),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["attachment_ids"], "request_text_only");
}

#[sqlx::test]
async fn a_request_refuses_gifs(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    let res = say(
        &mut w.ada,
        &dm,
        json!({ "gif": { "slug": "hello-wave", "query": "hi" } }),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["gif"], "request_text_only");
}

#[sqlx::test]
async fn an_edit_cannot_slip_a_link_into_a_request(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    let sent = text(&mut w.ada, &dm, "harmless").await;
    let id = sent.body["id"].as_str().unwrap().to_owned();
    let res = w
        .ada
        .send(
            Method::PATCH,
            &format!("/api/messages/{id}"),
            Some(json!({ "content": "now see evil.com" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["content"], "request_link");
}

// ---- Accept and decline ---------------------------------------------------

#[sqlx::test]
async fn accepting_makes_an_ordinary_dm_but_not_friends(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    text(&mut w.ada, &dm, "hello").await;
    // Only the recipient answers.
    let res = w
        .ada
        .send(Method::POST, &format!("/api/dms/{dm}/accept"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = w
        .bob
        .send(Method::POST, &format!("/api/dms/{dm}/accept"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(res.body.get("request").is_none(), "{}", res.body);
    assert!(requests(&mut w.bob).await.is_empty());
    assert_eq!(dm_ids(&mut w.bob).await, vec![dm.clone()]);
    assert_eq!(
        text(&mut w.bob, &dm, "hi Ada").await.status,
        StatusCode::CREATED
    );
    let res = say(&mut w.ada, &dm, json!({ "content": "https://example.com" })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let list = friends(&mut w.bob).await;
    assert!(list["friends"].as_array().unwrap().is_empty());
}

#[sqlx::test]
async fn a_decline_is_silent(pool: PgPool) {
    let mut w = world(pool.clone()).await;
    let dm = request_dm(&mut w).await;
    text(&mut w.ada, &dm, "hello").await;
    let res = w
        .bob
        .send(Method::POST, &format!("/api/dms/{dm}/decline"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert!(requests(&mut w.bob).await.is_empty());
    // Ada notices nothing: the DM and her count are as before, and what is
    // left of the limit is kept without showing the request again.
    let mine = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(mine.body["request"]["remaining"], 2);
    for n in 0..2 {
        let res = text(&mut w.ada, &dm, &format!("hello {n}?")).await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    }
    assert_eq!(
        text(&mut w.ada, &dm, "too many").await.body["error"],
        "message_request_limit"
    );
    assert!(requests(&mut w.bob).await.is_empty());
    // Later too: a declined request reads like one nobody answers.
    sqlx::query("UPDATE channels SET dm_declined_at = now() - interval '2 days' WHERE id = $1")
        .bind(uuid(&dm))
        .execute(&pool)
        .await
        .unwrap();
    let mine = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(mine.body["request"]["remaining"], 0, "{}", mine.body);
    // Bob can still accept it.
    let res = w
        .bob
        .send(Method::POST, &format!("/api/dms/{dm}/accept"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test]
async fn reacting_waits_for_the_request_to_be_accepted(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    let first = text(&mut w.ada, &dm, "hello").await;
    let id = first.body["id"].as_str().unwrap().to_owned();
    let react = format!("/api/messages/{id}/reactions/%F0%9F%91%8D");
    let res = w.bob.send(Method::PUT, &react, None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "message_request_pending");
    let res = w
        .bob
        .send(Method::POST, &format!("/api/dms/{dm}/accept"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = w.bob.send(Method::PUT, &react, None).await;
    assert!(res.status.is_success(), "{}", res.body);
}

#[sqlx::test]
async fn deleting_does_not_give_the_limit_back(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    let first = text(&mut w.ada, &dm, "hello").await;
    let id = first.body["id"].as_str().unwrap().to_owned();
    let res = w
        .ada
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    // Still shown to Bob, and still one of Ada's three.
    assert_eq!(requests(&mut w.bob).await.len(), 1);
    let mine = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(mine.body["request"]["remaining"], 2, "{}", mine.body);
    for n in 0..2 {
        let res = text(&mut w.ada, &dm, &format!("again {n}")).await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    }
    assert_eq!(
        text(&mut w.ada, &dm, "one more").await.body["error"],
        "message_request_limit"
    );
}

#[sqlx::test]
async fn an_empty_request_goes_to_whoever_writes_first(pool: PgPool) {
    let mut w = world(pool).await;
    // Ada opens the DM ("Neue Nachricht") but writes nothing.
    let dm = request_dm(&mut w).await;
    assert!(requests(&mut w.bob).await.is_empty());
    let bob_side = w
        .bob
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": w.ada.user_id() })),
        )
        .await;
    assert_eq!(bob_side.status, StatusCode::OK, "{}", bob_side.body);
    assert_eq!(bob_side.body["id"], dm.as_str());
    assert_eq!(
        bob_side.body["request"]["incoming"], false,
        "{}",
        bob_side.body
    );
    assert_eq!(
        text(&mut w.bob, &dm, "hallo Ada").await.status,
        StatusCode::CREATED
    );
    assert_eq!(requests(&mut w.ada).await.len(), 1);
    // Once written into, it stays Bob's.
    let ada_side = w
        .ada
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": w.bob.user_id() })),
        )
        .await;
    assert_eq!(
        ada_side.body["request"]["incoming"], true,
        "{}",
        ada_side.body
    );
}

#[sqlx::test]
async fn a_request_stays_pending_after_leaving_the_server(pool: PgPool) {
    let mut w = world(pool.clone()).await;
    let dm = request_dm(&mut w).await;
    text(&mut w.ada, &dm, "hello").await;
    sqlx::query("DELETE FROM server_members WHERE user_id = $1::uuid")
        .bind(w.bob.user_id())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(requests(&mut w.bob).await.len(), 1);
    assert_eq!(
        text(&mut w.ada, &dm, "still").await.status,
        StatusCode::CREATED
    );
    let res = w
        .bob
        .send(Method::POST, &format!("/api/dms/{dm}/accept"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

// ---- Friend requests ------------------------------------------------------

#[sqlx::test]
async fn friend_requests_by_code_and_by_profile(pool: PgPool) {
    let mut w = world(pool).await;
    // Cara knows nobody: only a code reaches Ada.
    let ada = w.ada.user_id().to_owned();
    let res = ask(&mut w.cara, json!({ "user_id": ada })).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let code = code_of(&mut w.ada).await;
    let typed = code.to_lowercase().replace('-', " ");
    let res = ask(&mut w.cara, json!({ "code": typed })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["status"], "pending");
    assert_eq!(res.body["peer"]["name"], "Ada");
    // Asking again changes nothing.
    let res = ask(&mut w.cara, json!({ "code": code })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Bob shares a server with Ada: the profile card's button works.
    let res = ask(&mut w.bob, json!({ "user_id": ada })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let list = friends(&mut w.ada).await;
    let incoming: Vec<&str> = list["incoming"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["user"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(incoming, vec!["Bob", "Cara"]);
    assert_eq!(
        friends(&mut w.cara).await["outgoing"][0]["user"]["name"],
        "Ada"
    );

    let res = ask(&mut w.ada, json!({ "code": code })).await;
    assert_eq!(res.body["fields"]["code"], "self");
    let res = ask(&mut w.ada, json!({ "code": "ZZZZ-ZZZZ" })).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = ask(&mut w.ada, json!({ "code": "nope" })).await;
    assert_eq!(res.body["fields"]["code"], "invalid");
}

#[sqlx::test]
async fn accept_makes_friends_and_opens_an_ordinary_dm(pool: PgPool) {
    let mut w = world(pool).await;
    let code = code_of(&mut w.ada).await;
    let res = ask(&mut w.cara, json!({ "code": code })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let cara = w.cara.user_id().to_owned();
    let res = w
        .ada
        .send(
            Method::POST,
            &format!("/api/friends/requests/{cara}/accept"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let list = friends(&mut w.ada).await;
    assert_eq!(list["friends"][0]["user"]["name"], "Cara");
    assert!(list["incoming"].as_array().unwrap().is_empty());
    assert_eq!(
        friends(&mut w.cara).await["friends"][0]["user"]["name"],
        "Ada"
    );
    let ada = w.ada.user_id().to_owned();
    let res = open_dm(&mut w.cara, &ada).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert!(res.body.get("request").is_none());
    // Friends see each other's card without a shared server.
    let card = w
        .cara
        .send(Method::GET, &format!("/api/users/{ada}"), None)
        .await;
    assert_eq!(card.status, StatusCode::OK, "{}", card.body);
    let res = ask(&mut w.cara, json!({ "user_id": ada })).await;
    assert_eq!(res.body["error"], "already_friends");
}

#[sqlx::test]
async fn asking_back_accepts(pool: PgPool) {
    let mut w = world(pool).await;
    let (ada, bob) = (w.ada.user_id().to_owned(), w.bob.user_id().to_owned());
    ask(&mut w.ada, json!({ "user_id": bob })).await;
    let res = ask(&mut w.bob, json!({ "user_id": ada })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["status"], "friends");
    assert_eq!(
        friends(&mut w.ada).await["friends"][0]["user"]["name"],
        "Bob"
    );
}

#[sqlx::test]
async fn becoming_friends_accepts_a_pending_message_request(pool: PgPool) {
    let mut w = world(pool).await;
    let dm = request_dm(&mut w).await;
    text(&mut w.ada, &dm, "hello").await;
    let (ada, bob) = (w.ada.user_id().to_owned(), w.bob.user_id().to_owned());
    ask(&mut w.ada, json!({ "user_id": bob })).await;
    let res = w
        .bob
        .send(
            Method::POST,
            &format!("/api/friends/requests/{ada}/accept"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(requests(&mut w.bob).await.is_empty());
    assert_eq!(dm_ids(&mut w.bob).await, vec![dm.clone()]);
    assert_eq!(
        text(&mut w.bob, &dm, "hi").await.status,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn a_declined_friend_request_waits_an_hour(pool: PgPool) {
    let mut w = world(pool.clone()).await;
    let (ada, bob) = (w.ada.user_id().to_owned(), w.bob.user_id().to_owned());
    ask(&mut w.ada, json!({ "user_id": bob })).await;
    let res = w
        .bob
        .send(
            Method::POST,
            &format!("/api/friends/requests/{ada}/decline"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(
        friends(&mut w.ada).await["outgoing"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        friends(&mut w.bob).await["incoming"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let res = ask(&mut w.ada, json!({ "user_id": bob })).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "friend_request_declined");
    let left = res.body["retry_after"].as_u64().unwrap();
    assert!((3500..=3600).contains(&left), "{left}");
    // Bob may still ask Ada.
    sqlx::query("UPDATE friend_requests SET closed_at = now() - interval '61 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    let res = ask(&mut w.ada, json!({ "user_id": bob })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

#[sqlx::test]
async fn withdraw_and_remove(pool: PgPool) {
    let mut w = world(pool).await;
    let (ada, bob) = (w.ada.user_id().to_owned(), w.bob.user_id().to_owned());
    ask(&mut w.ada, json!({ "user_id": bob })).await;
    let res = w
        .ada
        .send(
            Method::DELETE,
            &format!("/api/friends/requests/{bob}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(
        friends(&mut w.bob).await["incoming"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Withdrawn is no decline: asking again works at once.
    assert_eq!(
        ask(&mut w.ada, json!({ "user_id": bob })).await.status,
        StatusCode::CREATED
    );
    w.bob
        .send(
            Method::POST,
            &format!("/api/friends/requests/{ada}/accept"),
            None,
        )
        .await;
    let res = w
        .bob
        .send(Method::DELETE, &format!("/api/friends/{ada}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(
        friends(&mut w.ada).await["friends"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let res = w
        .bob
        .send(Method::DELETE, &format!("/api/friends/{ada}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn at_most_fifty_outgoing_requests(pool: PgPool) {
    let mut w = world(pool.clone()).await;
    let ada = w.ada.user_id().to_owned();
    sqlx::query(
        "WITH people AS ( \
           INSERT INTO users (email, name, password_hash) \
           SELECT 'p' || n || '@example.com', 'P' || n, 'x' FROM generate_series(1, 50) n \
           RETURNING id) \
         INSERT INTO friend_requests (sender_id, recipient_id) SELECT $1::uuid, id FROM people",
    )
    .bind(&ada)
    .execute(&pool)
    .await
    .unwrap();
    let bob = w.bob.user_id().to_owned();
    let res = ask(&mut w.ada, json!({ "user_id": bob })).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "friend_requests_full");
}

#[sqlx::test]
async fn friend_codes_regenerate_and_lookups_are_limited(pool: PgPool) {
    let mut w = world(pool).await;
    let first = code_of(&mut w.ada).await;
    assert_eq!(code_of(&mut w.ada).await, first, "stable until regenerated");
    let res = w.ada.send(Method::POST, "/api/me/friend-code", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let second = res.body["code"].as_str().unwrap().to_owned();
    assert_ne!(first, second);
    assert_ne!(code_of(&mut w.bob).await, second);
    let res = ask(&mut w.cara, json!({ "code": first })).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "old code is dead");

    // 20 lookups an hour, wrong or right.
    for _ in 0..19 {
        ask(&mut w.cara, json!({ "code": "ZZZZ-ZZZZ" })).await;
    }
    let res = ask(&mut w.cara, json!({ "code": second })).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
    assert_eq!(res.body["error"], "rate_limited");
}

#[sqlx::test]
async fn account_deletion_removes_friends_requests_and_code(pool: PgPool) {
    let mut w = world(pool.clone()).await;
    let (ada, bob) = (w.ada.user_id().to_owned(), w.bob.user_id().to_owned());
    common::befriend(&pool, &ada, w.cara.user_id()).await;
    ask(&mut w.ada, json!({ "user_id": bob })).await;
    code_of(&mut w.ada).await;
    // Ada owns the server, so Bob is the one who leaves.
    let code = code_of(&mut w.bob).await;
    ask(&mut w.cara, json!({ "code": code })).await;
    common::befriend(&pool, &bob, &ada).await;
    let res = w
        .bob
        .send(
            Method::DELETE,
            "/api/me",
            Some(json!({ "password": PASSWORD })),
        )
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let left: (i64, i64, Option<String>) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM friendships WHERE user_low = $1::uuid OR user_high = $1::uuid), \
                (SELECT count(*) FROM friend_requests WHERE sender_id = $1::uuid OR recipient_id = $1::uuid), \
                (SELECT friend_code FROM users WHERE id = $1::uuid)",
    )
    .bind(&bob)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, (0, 0, None));
    let list = friends(&mut w.ada).await;
    assert_eq!(list["friends"].as_array().unwrap().len(), 1, "Cara stays");
    let res = ask(&mut w.cara, json!({ "code": code })).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// ---- Live events ----------------------------------------------------------

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn socket(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    req.headers_mut().insert(
        "cookie",
        format!("{}={}", common::SESSION, client.jar[common::SESSION])
            .parse()
            .unwrap(),
    );
    let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws
}

async fn next_frame(ws: &mut Ws, want: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Message::Text(raw) = ws.next().await.expect("open socket").expect("frame") {
                let frame: Value = serde_json::from_str(&raw).unwrap();
                if frame["op"] == "h" {
                    ws.send(Message::Text(json!({ "op": "h" }).to_string().into()))
                        .await
                        .unwrap();
                }
                if want(&frame) {
                    return frame;
                }
            }
        }
    })
    .await
    .expect("frame")
}

#[sqlx::test]
async fn friend_and_request_changes_reach_every_socket(pool: PgPool) {
    let state = common::ws_state(pool);
    let addr = common::serve_ws_state(state.clone()).await;
    let mut w = world_with(state).await;
    let mut bob_ws = socket(addr, &w.bob).await;
    let mut ada_ws = socket(addr, &w.ada).await;
    // Let both sockets settle before the writes.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let bob = w.bob.user_id().to_owned();
    ask(&mut w.ada, json!({ "user_id": bob })).await;
    next_frame(&mut bob_ws, |f| f["op"] == "fr").await;
    next_frame(&mut ada_ws, |f| f["op"] == "fr").await;

    let dm = request_dm(&mut w).await;
    text(&mut w.ada, &dm, "hello").await;
    next_frame(&mut bob_ws, |f| f["op"] == "mr").await;
}
