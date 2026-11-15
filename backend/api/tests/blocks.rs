//! One person blocks another (#565): the routes, every private path closed
//! both ways, what a block takes away, mentions, unblocking and the event.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::AppState;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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
    block(&mut w.ada, &bob).await;

    refused(
        &text(&mut w.bob, &dm, "nachher").await,
        StatusCode::FORBIDDEN,
        "dm_closed",
    );
    refused(
        &text(&mut w.ada, &dm, "nachher").await,
        StatusCode::FORBIDDEN,
        "blocked",
    );
    let edit = w
        .bob
        .send(
            Method::PATCH,
            &format!("/api/messages/{bobs}"),
            Some(json!({ "content": "geändert" })),
        )
        .await;
    refused(&edit, StatusCode::FORBIDDEN, "dm_closed");

    // Both still read it; each sees only their own side.
    let mine = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(mine.body["blocked"], true, "{}", mine.body);
    assert!(mine.body.get("closed").is_none());
    let theirs = w
        .bob
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert_eq!(theirs.body["closed"], true, "{}", theirs.body);
    assert!(theirs.body.get("blocked").is_none());
    let again = open_dm(&mut w.bob, &ada).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    let history = w
        .bob
        .send(Method::GET, &format!("/api/channels/{dm}/messages"), None)
        .await;
    assert_eq!(history.status, StatusCode::OK, "{}", history.body);
    assert_eq!(history.body["messages"].as_array().unwrap().len(), 2);

    // Unblocking opens it again; the friendship does not come back.
    unblock(&mut w.ada, &bob).await;
    assert_eq!(
        text(&mut w.bob, &dm, "wieder").await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        text(&mut w.ada, &dm, "ja").await.status,
        StatusCode::CREATED
    );
    let list = friends(&mut w.ada).await;
    assert_eq!(list["friends"], json!([]));
    let dm_now = w
        .ada
        .send(Method::GET, &format!("/api/dms/{dm}"), None)
        .await;
    assert!(dm_now.body.get("blocked").is_none());
    assert!(dm_now.body.get("request").is_none(), "still accepted");
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
    refused(
        &text(&mut w.bob, &dm, "noch was").await,
        StatusCode::FORBIDDEN,
        "dm_closed",
    );
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

/// The first frame `want` takes within `wait`, answering heartbeats.
async fn frame(ws: &mut Ws, wait: Duration, want: impl Fn(&Value) -> bool) -> Option<Value> {
    tokio::time::timeout(wait, async {
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
    .ok()
}

#[sqlx::test]
async fn only_the_blocker_hears_of_a_block(pool: PgPool) {
    let state = common::ws_state(pool);
    let addr = common::serve_ws_state(state.clone()).await;
    let mut w = world(state).await;
    let mut ada_ws = socket(addr, &w.ada).await;
    let mut bob_ws = socket(addr, &w.bob).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let bob = w.bob.user_id().to_owned();
    block(&mut w.ada, &bob).await;
    let bl = |f: &Value| f["op"] == "bl";
    assert!(
        frame(&mut ada_ws, Duration::from_secs(5), bl)
            .await
            .is_some()
    );
    unblock(&mut w.ada, &bob).await;
    assert!(
        frame(&mut ada_ws, Duration::from_secs(5), bl)
            .await
            .is_some()
    );
    assert!(
        frame(&mut bob_ws, Duration::from_millis(500), bl)
            .await
            .is_none(),
        "Bob is not told"
    );
}
