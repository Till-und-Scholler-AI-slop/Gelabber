//! Joins and leaves reach the server's members live. A join (#621):
//! invite, open listing and accepted request each publish one
//! `{"t": "c", "d": {"k": "j"}}` server event with the new member's list
//! row. A member leaving on their own (GEL-22) publishes one
//! `{"t": "c", "d": {"k": "l"}}`, never the removal event `t: "d"`.

mod common;

use axum::http::{Method, StatusCode};
use gelabber_api::AppState;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;
use common::ws::{self, Ws, until};

const PASSWORD: &str = "correct horse";

/// A served state with the directory on, as joins through it need.
async fn serve(pool: PgPool) -> (std::net::SocketAddr, AppState) {
    let mut state = common::ws_state(pool);
    state.directory = true;
    (common::serve_ws_state(state.clone()).await, state)
}

async fn registered(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::with_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn create_server(client: &mut Client) -> Uuid {
    let res = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Keller" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().parse().unwrap()
}

async fn me_id(client: &mut Client) -> String {
    client.send(Method::GET, "/api/me", None).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn invite_code(owner: &mut Client, server: Uuid) -> String {
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    invite.body["code"].as_str().unwrap().to_owned()
}

async fn list_with(owner: &mut Client, server: Uuid, join_mode: &str) {
    let res = owner
        .send(
            Method::PUT,
            &format!("/api/servers/{server}/listing"),
            Some(json!({
                "listed": true,
                "description": "Kommt vorbei.",
                "language": "de",
                "join_mode": join_mode,
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

/// The owner's socket, subscribed to the server's topic.
async fn watch(addr: std::net::SocketAddr, owner: &Client, server: Uuid) -> Ws {
    let mut ws = ws::connect(addr, owner).await;
    ws::send(&mut ws, json!({ "op": "s", "s": server })).await;
    until(&mut ws, |f| f["op"] == "ok").await;
    ws
}

/// The next event on the server topic itself (not a channel's).
async fn next_server_event(ws: &mut Ws) -> Value {
    until(ws, |f| f["op"] == "e" && f.get("c").is_none()).await
}

fn assert_joined(event: &Value, server: Uuid, user: &str, name: &str) {
    assert_eq!(event["t"], "c", "{event}");
    assert_eq!(event["s"], server.to_string(), "{event}");
    assert_eq!(event["i"], user, "{event}");
    assert_eq!(event["d"]["k"], "j", "{event}");
    let member = &event["d"]["member"];
    assert_eq!(member["user_id"], user, "{event}");
    assert_eq!(member["name"], name, "{event}");
    assert_eq!(member["role"], "member", "{event}");
    assert_eq!(member["role_ids"], json!([]), "{event}");
    assert!(member["joined_at"].is_string(), "{event}");
}

#[sqlx::test]
async fn an_invite_join_reaches_the_members_once(pool: PgPool) {
    let (addr, state) = serve(pool).await;
    let mut ada = registered(&state, "ada@example.com", "Ada").await;
    let mut bob = registered(&state, "bob@example.com", "Bob").await;
    let server = create_server(&mut ada).await;
    let code = invite_code(&mut ada, server).await;
    let mut ws = watch(addr, &ada, server).await;

    let res = bob
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let bob_id = me_id(&mut bob).await;
    let event = next_server_event(&mut ws).await;
    assert_joined(&event, server, &bob_id, "Bob");
    // The same row the detail lists.
    let detail = ada
        .send(Method::GET, &format!("/api/servers/{server}"), None)
        .await;
    let listed = detail.body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["user_id"] == bob_id.as_str())
        .cloned()
        .expect("listed");
    assert_eq!(event["d"]["member"], listed);

    // Following the link again as a member is no second join.
    let res = bob
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{server}/roles"),
            Some(json!({ "name": "Team", "color": "blue" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let next = next_server_event(&mut ws).await;
    assert_eq!(next["d"]["k"], "a", "no second join event: {next}");
}

#[sqlx::test]
async fn an_open_listing_join_reaches_the_members(pool: PgPool) {
    let (addr, state) = serve(pool).await;
    let mut ada = registered(&state, "ada@example.com", "Ada").await;
    let mut lena = registered(&state, "lena@example.com", "Lena").await;
    let server = create_server(&mut ada).await;
    list_with(&mut ada, server, "open").await;
    let mut ws = watch(addr, &ada, server).await;

    let res = lena
        .send(
            Method::POST,
            &format!("/api/directory/listings/{server}/join"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let lena_id = me_id(&mut lena).await;
    let event = next_server_event(&mut ws).await;
    assert_joined(&event, server, &lena_id, "Lena");
}

#[sqlx::test]
async fn an_accepted_request_reaches_the_members(pool: PgPool) {
    let (addr, state) = serve(pool).await;
    let mut ada = registered(&state, "ada@example.com", "Ada").await;
    let mut tom = registered(&state, "tom@example.com", "Tom").await;
    let server = create_server(&mut ada).await;
    list_with(&mut ada, server, "request").await;
    let res = tom
        .send(
            Method::POST,
            &format!("/api/directory/listings/{server}/request"),
            Some(json!({ "message": "Hallo!" })),
        )
        .await;
    assert!(res.status.is_success(), "{}", res.body);
    let tom_id = me_id(&mut tom).await;
    let mut ws = watch(addr, &ada, server).await;

    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{server}/join-requests/{tom_id}/accept"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let event = next_server_event(&mut ws).await;
    assert_joined(&event, server, &tom_id, "Tom");
}

#[sqlx::test]
async fn a_leave_reaches_the_members_once_and_is_no_removal(pool: PgPool) {
    let (addr, state) = serve(pool).await;
    let mut ada = registered(&state, "ada@example.com", "Ada").await;
    let mut bob = registered(&state, "bob@example.com", "Bob").await;
    let server = create_server(&mut ada).await;
    let code = invite_code(&mut ada, server).await;
    let res = bob
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let bob_id = me_id(&mut bob).await;
    let mut ws = watch(addr, &ada, server).await;

    let res = bob
        .send(Method::POST, &format!("/api/servers/{server}/leave"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let event = next_server_event(&mut ws).await;
    assert_eq!(event["t"], "c", "not the removal event: {event}");
    assert_eq!(event["s"], server.to_string(), "{event}");
    assert_eq!(event["i"], bob_id, "{event}");
    assert_eq!(event["d"], json!({ "k": "l" }), "{event}");
    // The detail agrees: Bob is gone.
    let detail = ada
        .send(Method::GET, &format!("/api/servers/{server}"), None)
        .await;
    assert!(
        !detail.body["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["user_id"] == bob_id.as_str()),
        "{}",
        detail.body
    );

    // Leaving again is refused and sends nothing.
    let res = bob
        .send(Method::POST, &format!("/api/servers/{server}/leave"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{server}/roles"),
            Some(json!({ "name": "Team", "color": "blue" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let next = next_server_event(&mut ws).await;
    assert_eq!(next["d"]["k"], "a", "no second leave event: {next}");
}
