//! Kick, ban, invite-block and moderation delete (issue #15).

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::COOKIE;

use common::{Client, SESSION, serve_ws};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn two_users(pool: PgPool) -> (Client, Client) {
    let mut owner = Client::new(pool.clone());
    owner.bootstrap().await;
    let res = owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let mut member = Client::new(pool);
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

async fn invite_and_join(owner: &mut Client, member: &mut Client, server_id: &str) -> String {
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let code = invite.body["code"].as_str().unwrap().to_owned();
    let res = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    code
}

async fn server_with_member(owner: &mut Client, member: &mut Client) -> Value {
    let server = create_server(owner, "Team").await;
    let id = server["id"].as_str().unwrap();
    invite_and_join(owner, member, id).await;
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

async fn me_id(client: &mut Client) -> String {
    client.send(Method::GET, "/api/me", None).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[sqlx::test]
async fn owner_can_delete_someone_elses_message(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);

    let posted = member
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "bitte weg" })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    let id = posted.body["id"].as_str().unwrap();

    let deleted = owner
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);

    let page = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(page.body["messages"], json!([]));
}

#[sqlx::test]
async fn manage_messages_lets_a_member_delete_foreign_rows(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap();
    let channel_id = text_channel_id(&server);

    let posted = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "Ada only" })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED);
    let id = posted.body["id"].as_str().unwrap();

    let denied = member
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.body["error"], "forbidden");

    let granted = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({
                "member_permissions": ["send_messages", "manage_messages"]
            })),
        )
        .await;
    assert_eq!(granted.status, StatusCode::OK, "{}", granted.body);

    let deleted = member
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
}

#[sqlx::test]
async fn owner_can_kick_and_member_can_rejoin(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap().to_owned();
    let member_id = me_id(&mut member).await;

    let kicked = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/kick"),
            Some(json!({ "user_id": member_id })),
        )
        .await;
    assert_eq!(kicked.status, StatusCode::NO_CONTENT, "{}", kicked.body);

    let gone = member
        .send(Method::GET, &format!("/api/servers/{server_id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    let code = invite_and_join(&mut owner, &mut member, &server_id).await;
    let again = member
        .send(Method::GET, &format!("/api/servers/{server_id}"), None)
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.body["role"], "member");
    let _ = code;
}

#[sqlx::test]
async fn ban_blocks_invite_rejoin(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap().to_owned();
    let member_id = me_id(&mut member).await;

    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED);
    let code = invite.body["code"].as_str().unwrap().to_owned();

    let banned = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/ban"),
            Some(json!({ "user_id": member_id })),
        )
        .await;
    assert_eq!(banned.status, StatusCode::NO_CONTENT, "{}", banned.body);

    let preview = member
        .send(Method::GET, &format!("/api/invites/{code}"), None)
        .await;
    assert_eq!(preview.status, StatusCode::FORBIDDEN);
    assert_eq!(preview.body["error"], "banned");

    let join = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(join.status, StatusCode::FORBIDDEN);
    assert_eq!(join.body["error"], "banned");

    let list = owner
        .send(Method::GET, &format!("/api/servers/{server_id}/bans"), None)
        .await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body.as_array().unwrap().len(), 1);
    assert_eq!(list.body[0]["user_id"], member_id);

    let unban = owner
        .send(
            Method::DELETE,
            &format!("/api/servers/{server_id}/bans/{member_id}"),
            None,
        )
        .await;
    assert_eq!(unban.status, StatusCode::NO_CONTENT, "{}", unban.body);

    let rejoined = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(rejoined.status, StatusCode::OK, "{}", rejoined.body);
}

#[sqlx::test]
async fn kick_and_ban_are_admin_only_and_cannot_target_owner(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap();
    let owner_id = me_id(&mut owner).await;
    let member_id = me_id(&mut member).await;

    let denied = member
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/kick"),
            Some(json!({ "user_id": owner_id })),
        )
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);

    let owner_kick = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/kick"),
            Some(json!({ "user_id": owner_id })),
        )
        .await;
    assert_eq!(owner_kick.status, StatusCode::FORBIDDEN);

    let self_kick = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/kick"),
            Some(json!({ "user_id": member_id })),
        )
        .await;
    // Owner kicking the member is allowed — this is the control that
    // manage_server works. The member-trying-to-kick-owner case is above.
    assert_eq!(self_kick.status, StatusCode::NO_CONTENT);

    let missing = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/ban"),
            Some(json!({ "user_id": "" })),
        )
        .await;
    assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing.body["fields"]["user_id"], "required");
}

#[sqlx::test]
async fn manage_server_member_can_kick(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let mut other = Client::new(pool);
    other.bootstrap().await;
    let res = other
        .register("other@example.com", "password123", "Cara")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap();
    invite_and_join(&mut owner, &mut other, server_id).await;
    let other_id = me_id(&mut other).await;

    let granted = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({
                "member_permissions": ["manage_server", "send_messages"]
            })),
        )
        .await;
    assert_eq!(granted.status, StatusCode::OK, "{}", granted.body);

    let kicked = member
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/kick"),
            Some(json!({ "user_id": other_id })),
        )
        .await;
    assert_eq!(kicked.status, StatusCode::NO_CONTENT, "{}", kicked.body);
}

async fn connect_ws(addr: std::net::SocketAddr, cookie: &str) -> Ws {
    let mut request = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("ws request");
    request.headers_mut().insert(
        COOKIE,
        format!("{SESSION}={cookie}")
            .parse()
            .expect("cookie header"),
    );
    let (ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .unwrap_or_else(|err| panic!("ws connect failed: {err}"));
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    ws
}

async fn recv_json(ws: &mut Ws) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("ws recv timeout")
        .expect("ws closed")
        .expect("ws error");
    match msg {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("json frame"),
        other => panic!("expected text frame, got {other:?}"),
    }
}

async fn recv_until(ws: &mut Ws, pred: impl Fn(&Value) -> bool) -> Value {
    loop {
        let frame = recv_json(ws).await;
        if pred(&frame) {
            return frame;
        }
    }
}

async fn send_json(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .expect("ws send");
}

#[sqlx::test]
async fn delete_fans_out_on_the_channel_topic(pool: PgPool) {
    let (addr, state) = serve_ws(pool).await;
    let mut owner = Client::with_state(state.clone());
    owner.bootstrap().await;
    owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    let mut member = Client::with_state(state);
    member.bootstrap().await;
    member
        .register("member@example.com", "password123", "Bob")
        .await;

    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap();
    let channel_id = text_channel_id(&server);
    let posted = member
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "weg damit" })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED);
    let message_id = posted.body["id"].as_str().unwrap();

    let cookie = member.jar.get(SESSION).cloned().expect("session");
    let mut ws = connect_ws(addr, &cookie).await;
    send_json(
        &mut ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    let _ok = recv_until(&mut ws, |f| f["op"] == "ok").await;

    let deleted = owner
        .send(Method::DELETE, &format!("/api/messages/{message_id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let event = recv_until(&mut ws, |f| f["op"] == "e" && f["t"] == "d").await;
    assert_eq!(event["i"], message_id);
    assert_eq!(event["c"], channel_id);
}

#[sqlx::test]
async fn kick_revokes_the_socket_and_blocks_resubscribe(pool: PgPool) {
    let (addr, state) = serve_ws(pool).await;
    let mut owner = Client::with_state(state.clone());
    owner.bootstrap().await;
    owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    let mut member = Client::with_state(state);
    member.bootstrap().await;
    member
        .register("member@example.com", "password123", "Bob")
        .await;

    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap().to_owned();
    let member_id = me_id(&mut member).await;

    let cookie = member.jar.get(SESSION).cloned().expect("session");
    let mut ws = connect_ws(addr, &cookie).await;
    send_json(&mut ws, json!({ "op": "s", "s": server_id })).await;
    let _ok = recv_until(&mut ws, |f| f["op"] == "ok").await;

    let kicked = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/kick"),
            Some(json!({ "user_id": member_id })),
        )
        .await;
    assert_eq!(kicked.status, StatusCode::NO_CONTENT, "{}", kicked.body);

    let err = recv_until(&mut ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "kicked");
    assert_eq!(err["s"], server_id);

    send_json(&mut ws, json!({ "op": "s", "s": server_id })).await;
    let denied = recv_until(&mut ws, |f| f["op"] == "err").await;
    assert_eq!(denied["e"], "not_found");
}
