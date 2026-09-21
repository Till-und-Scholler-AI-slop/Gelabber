//! Voice signaling (issue #10 + #12): own `op: "sig"`, session + join_voice,
//! join/leave / mute/deafen / offer/answer / ice / pub/unpub. No LiveKit.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::EventKind;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::COOKIE;
use uuid::Uuid;

use common::{Client, SESSION};

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

async fn join_member(owner: &mut Client, member: &mut Client, server_id: &str) {
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = member
        .send(
            Method::POST,
            &format!("/api/invites/{}/join", res.body["code"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

async fn create_voice(client: &mut Client, server_id: Uuid) -> Uuid {
    let res = client
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/channels"),
            Some(json!({ "name": "Lounge", "kind": "voice" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().parse().unwrap()
}

fn session_cookie(client: &Client) -> String {
    client
        .jar
        .get(SESSION)
        .cloned()
        .expect("session cookie after login")
}

async fn connect(addr: std::net::SocketAddr, cookie: &str) -> Ws {
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

fn ids(server: &Value) -> (Uuid, Uuid) {
    let server_id = server["id"].as_str().unwrap().parse().unwrap();
    let channel_id = server["channels"][0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    (server_id, channel_id)
}

fn owner_id(server: &Value) -> Uuid {
    server["owner_id"].as_str().unwrap().parse().unwrap()
}

#[sqlx::test]
async fn join_requires_voice_channel_and_join_voice(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Voice").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;

    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({ "member_permissions": ["send_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let (addr, _) = common::serve_ws(pool).await;
    let mut member_ws = connect(addr, &session_cookie(&member)).await;

    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let err = recv_until(&mut member_ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "forbidden");
    assert_eq!(err["c"], voice_id.to_string());

    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": text_id }),
    )
    .await;
    let err = recv_until(&mut owner_ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "bad_request");
}

#[sqlx::test]
async fn join_unknown_or_foreign_is_not_found(pool: PgPool) {
    let (mut owner, member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Private").await;
    let (server_id, _) = ids(&server);
    let voice_id = create_voice(&mut owner, server_id).await;
    let (addr, _) = common::serve_ws(pool).await;

    let mut stranger = connect(addr, &session_cookie(&member)).await;
    send_json(
        &mut stranger,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let err = recv_until(&mut stranger, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "not_found");
}

#[sqlx::test]
async fn room_fanout_is_sig_not_chat_and_survives_offer_ice_pub(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Call").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, state) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    let mut member_ws = connect(addr, &session_cookie(&member)).await;

    send_json(
        &mut owner_ws,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "ok").await;

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let owner_join = recv_until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    assert_eq!(owner_join["u"], ada.to_string());
    assert!(owner_join.get("n").is_none(), "{owner_join}");

    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let member_seen = recv_until(&mut owner_ws, |f| {
        f["op"] == "sig" && f["t"] == "j" && f["u"] != ada.to_string()
    })
    .await;
    assert_eq!(member_seen["c"], voice_id.to_string());

    let snapshot = recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    assert_eq!(snapshot["u"], ada.to_string());

    send_json(
        &mut owner_ws,
        json!({
            "op": "sig",
            "t": "o",
            "s": server_id,
            "c": voice_id,
            "sdp": "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n"
        }),
    )
    .await;
    let rejected = recv_until(&mut owner_ws, |f| f["op"] == "err").await;
    assert_eq!(rejected["e"], "bad_request");

    send_json(
        &mut member_ws,
        json!({
            "op": "sig",
            "t": "a",
            "s": server_id,
            "c": voice_id,
            "sdp": "v=0\r\no=- 2 1 IN IP4 127.0.0.1\r\n"
        }),
    )
    .await;
    let rejected = recv_until(&mut member_ws, |f| f["op"] == "err").await;
    assert_eq!(rejected["e"], "bad_request");

    send_json(
        &mut owner_ws,
        json!({
            "op": "sig",
            "t": "i",
            "s": server_id,
            "c": voice_id,
            "ice": "candidate:1 1 UDP 1 127.0.0.1 9 typ host",
            "mid": "0"
        }),
    )
    .await;
    let rejected = recv_until(&mut owner_ws, |f| f["op"] == "err").await;
    assert_eq!(rejected["e"], "bad_request");

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "a" }),
    )
    .await;
    let pubd = recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "p").await;
    assert_eq!(pubd["k"], "a");

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "u", "s": server_id, "c": voice_id, "k": "a" }),
    )
    .await;
    let unpub = recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "u").await;
    assert_eq!(unpub["k"], "a");

    // Chat events stay on `op: e` and do not appear as sig.
    gelabber_api::publish_channel(
        &state,
        server_id,
        text_id,
        EventKind::C,
        Some(Uuid::from_u128(1)),
        Some(json!({ "b": "hi" })),
    )
    .await
    .unwrap();
    let chat = recv_until(&mut owner_ws, |f| f["op"] == "e").await;
    assert_eq!(chat["t"], "c");
    assert_ne!(chat["op"], "sig");

    let stray = tokio::time::timeout(Duration::from_millis(120), recv_json(&mut member_ws)).await;
    if let Ok(frame) = stray {
        assert_ne!(
            frame["op"], "e",
            "voice socket must not get chat events: {frame}"
        );
    }

    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "l", "s": server_id, "c": voice_id }),
    )
    .await;
    let left = recv_until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "l").await;
    assert_eq!(left["c"], voice_id.to_string());
}

#[sqlx::test]
async fn offer_before_join_is_rejected_and_disconnect_leaves(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Drop").await;
    let (server_id, _) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, _) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send_json(
        &mut owner_ws,
        json!({
            "op": "sig",
            "t": "o",
            "s": server_id,
            "c": voice_id,
            "sdp": "v=0"
        }),
    )
    .await;
    let err = recv_until(&mut owner_ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "bad_request");

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    let mut member_ws = connect(addr, &session_cookie(&member)).await;
    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    owner_ws.close(None).await.ok();
    let left = recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "l").await;
    assert_eq!(left["u"], ada.to_string());
}

#[sqlx::test]
async fn camera_and_screen_pub_in_voice_without_go_live(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Live").await;
    let (server_id, _) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({ "member_permissions": ["join_voice", "send_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let (addr, _) = common::serve_ws(pool).await;
    let mut member_ws = connect(addr, &session_cookie(&member)).await;
    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "v" }),
    )
    .await;
    let camera = recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "p").await;
    assert_eq!(camera["k"], "v");

    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    let screen = recv_until(&mut member_ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "s"
    })
    .await;
    assert_eq!(screen["k"], "s");
}

#[sqlx::test]
async fn one_socket_leave_does_not_evict_another_seat(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Tabs").await;
    let (server_id, _) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, _) = common::serve_ws(pool).await;
    let cookie = session_cookie(&owner);
    let mut tab_a = connect(addr, &cookie).await;
    let mut tab_b = connect(addr, &cookie).await;
    let mut member_ws = connect(addr, &session_cookie(&member)).await;

    send_json(
        &mut tab_a,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut tab_a, |f| f["op"] == "sig" && f["t"] == "j").await;
    send_json(
        &mut tab_b,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut tab_b, |f| f["op"] == "sig" && f["t"] == "j").await;
    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    tab_a.close(None).await.ok();
    let stray = tokio::time::timeout(Duration::from_millis(200), recv_json(&mut member_ws)).await;
    if let Ok(frame) = stray {
        let ada = ada.to_string();
        assert!(
            !(frame["op"] == "sig" && frame["t"] == "l" && frame["u"] == ada),
            "first tab must not broadcast leave: {frame}"
        );
    }

    tab_b.close(None).await.ok();
    let left = recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "l").await;
    assert_eq!(left["u"], ada.to_string());
}

#[sqlx::test]
async fn join_mute_deafen_reach_server_watchers_not_in_the_room(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Watch").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, _) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    let mut watcher = connect(addr, &session_cookie(&member)).await;

    send_json(
        &mut watcher,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    recv_until(&mut watcher, |f| f["op"] == "ok").await;
    let roster = recv_until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "r").await;
    assert_eq!(roster["s"], server_id.to_string());
    assert_eq!(roster["snap"].as_array().unwrap().len(), 0);

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let joined = recv_until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "j").await;
    assert_eq!(joined["u"], ada.to_string());
    assert_eq!(joined["c"], voice_id.to_string());

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "m", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    let muted = recv_until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "m").await;
    assert_eq!(muted["on"], true);
    assert_eq!(muted["u"], ada.to_string());

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "d", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    let deafened = recv_until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "d").await;
    assert_eq!(deafened["on"], true);

    send_json(
        &mut owner_ws,
        json!({
            "op": "sig",
            "t": "o",
            "s": server_id,
            "c": voice_id,
            "sdp": "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n"
        }),
    )
    .await;
    let rejected = recv_until(&mut owner_ws, |f| f["op"] == "err").await;
    assert_eq!(rejected["e"], "bad_request");
    let stray = tokio::time::timeout(Duration::from_millis(150), recv_json(&mut watcher)).await;
    if let Ok(frame) = stray {
        assert_ne!(frame["t"], "o", "watchers must not get SDP: {frame}");
        assert_ne!(frame["t"], "i", "watchers must not get ICE: {frame}");
    }

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "l", "s": server_id, "c": voice_id }),
    )
    .await;
    let left = recv_until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "l").await;
    assert_eq!(left["u"], ada.to_string());
}

#[sqlx::test]
async fn subscribe_replaces_voice_occupancy_and_mute_needs_a_seat(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Snap").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, _) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "m", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "m").await;

    let mut watcher = connect(addr, &session_cookie(&member)).await;
    send_json(
        &mut watcher,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    recv_until(&mut watcher, |f| f["op"] == "ok").await;
    let roster = recv_until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "r").await;
    let snap = roster["snap"].as_array().expect("snap");
    assert_eq!(snap.len(), 1, "{roster}");
    assert_eq!(snap[0]["u"], ada.to_string());
    assert_eq!(snap[0]["c"], voice_id.to_string());
    assert_eq!(snap[0]["m"], true);

    send_json(
        &mut watcher,
        json!({ "op": "sig", "t": "m", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    let err = recv_until(&mut watcher, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "bad_request");
}

#[sqlx::test]
async fn go_live_needs_permission_and_is_one_per_channel(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Stream").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({ "member_permissions": ["join_voice", "send_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let (addr, _) = common::serve_ws(pool).await;
    let mut member_ws = connect(addr, &session_cookie(&member)).await;
    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "l" }),
    )
    .await;
    let denied = recv_until(&mut member_ws, |f| f["op"] == "err").await;
    assert_eq!(denied["e"], "forbidden");

    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({
                "member_permissions": ["join_voice", "send_messages", "go_live"]
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "l" }),
    )
    .await;
    let live = recv_until(&mut owner_ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "l"
    })
    .await;
    assert_eq!(live["u"], ada.to_string());
    assert!(!live.to_string().contains("livekit"));

    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "l" }),
    )
    .await;
    let taken = recv_until(&mut member_ws, |f| f["op"] == "err").await;
    assert_eq!(taken["e"], "bad_request");

    let mut found = false;
    for _ in 0..20 {
        let hint = owner
            .send(
                Method::GET,
                &format!("/api/channels/{text_id}/messages"),
                None,
            )
            .await;
        assert_eq!(hint.status, StatusCode::OK, "{}", hint.body);
        let messages = hint.body["messages"].as_array().expect("messages");
        if messages
            .iter()
            .any(|row| row["content"].as_str().unwrap_or("").contains("ist live"))
        {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(found, "text channel should get a live hint");

    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "u", "s": server_id, "c": voice_id, "k": "l" }),
    )
    .await;
    recv_until(&mut member_ws, |f| {
        f["op"] == "sig" && f["t"] == "u" && f["k"] == "l"
    })
    .await;

    send_json(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "l" }),
    )
    .await;
    let second = recv_until(&mut member_ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "l"
    })
    .await;
    assert_eq!(second["k"], "l");
}

#[sqlx::test]
async fn go_live_shows_in_occupancy_snapshot(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Badge").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, _) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send_json(
        &mut owner_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "l" }),
    )
    .await;
    recv_until(&mut owner_ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "l"
    })
    .await;

    let mut watcher = connect(addr, &session_cookie(&member)).await;
    send_json(
        &mut watcher,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    recv_until(&mut watcher, |f| f["op"] == "ok").await;
    let roster = recv_until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "r").await;
    let snap = roster["snap"].as_array().expect("snap");
    assert_eq!(snap.len(), 1, "{roster}");
    assert_eq!(snap[0]["u"], ada.to_string());
    assert_eq!(snap[0]["l"], true);
}
