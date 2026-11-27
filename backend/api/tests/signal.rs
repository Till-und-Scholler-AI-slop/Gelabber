//! Voice signaling (issue #10 + #12): own `op: "sig"`, session + join_voice,
//! join/leave / mute/deafen / pub/unpub.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use gelabber_api::EventKind;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::ws::{self, Ws, recv, send, until};
use common::{Client, SESSION};

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
    ws::connect_to(addr, "/ws", Some(cookie), None).await
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

    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let err = until(&mut member_ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "forbidden");
    assert_eq!(err["c"], voice_id.to_string());

    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": text_id }),
    )
    .await;
    let err = until(&mut owner_ws, |f| f["op"] == "err").await;
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
    send(
        &mut stranger,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let err = until(&mut stranger, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "not_found");
}

#[sqlx::test]
async fn room_fanout_is_sig_not_chat(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Call").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, state) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    let mut member_ws = connect(addr, &session_cookie(&member)).await;

    send(
        &mut owner_ws,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    until(&mut owner_ws, |f| f["op"] == "ok").await;

    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let owner_join = until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    assert_eq!(owner_join["u"], ada.to_string());
    assert!(owner_join.get("n").is_none(), "{owner_join}");

    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let member_seen = until(&mut owner_ws, |f| {
        f["op"] == "sig" && f["t"] == "j" && f["u"] != ada.to_string()
    })
    .await;
    assert_eq!(member_seen["c"], voice_id.to_string());

    let snapshot = until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    assert_eq!(snapshot["u"], ada.to_string());

    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "a" }),
    )
    .await;
    let pubd = until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "p").await;
    assert_eq!(pubd["k"], "a");

    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "u", "s": server_id, "c": voice_id, "k": "a" }),
    )
    .await;
    let unpub = until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "u").await;
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
    let chat = until(&mut owner_ws, |f| f["op"] == "e").await;
    assert_eq!(chat["t"], "c");
    assert_ne!(chat["op"], "sig");

    let stray = ws::within(&mut member_ws, Duration::from_millis(120), |f| {
        f["op"] == "e"
    })
    .await;
    assert_eq!(stray, None, "voice socket must not get chat events");

    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "l", "s": server_id, "c": voice_id }),
    )
    .await;
    let left = until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "l").await;
    assert_eq!(left["c"], voice_id.to_string());
}

#[sqlx::test]
async fn disconnect_leaves_the_room(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Drop").await;
    let (server_id, _) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, _) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    let mut member_ws = connect(addr, &session_cookie(&member)).await;
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    owner_ws.close(None).await.ok();
    let left = until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "l").await;
    assert_eq!(left["u"], ada.to_string());
}

#[sqlx::test]
async fn camera_without_go_live_but_live_needs_it(pool: PgPool) {
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
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "v" }),
    )
    .await;
    let camera = until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "p").await;
    assert_eq!(camera["k"], "v");

    // Live is the screen: it and its sound need `go_live` to start.
    for kind in ["s", "sa"] {
        send(
            &mut member_ws,
            json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": kind }),
        )
        .await;
        let denied = until(&mut member_ws, |f| f["op"] == "err").await;
        assert_eq!(denied["e"], "forbidden", "{kind}");
        assert_eq!(denied["c"], voice_id.to_string());
    }
    // Stopping needs nothing.
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "u", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    let stopped = until(&mut member_ws, |f| {
        f["op"] == "err" || (f["op"] == "sig" && f["t"] == "u")
    })
    .await;
    assert_eq!(stopped["op"], "sig", "{stopped}");

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
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    let screen = until(&mut member_ws, |f| {
        f["op"] == "err" || (f["op"] == "sig" && f["t"] == "p" && f["k"] == "s")
    })
    .await;
    assert_eq!(screen["op"], "sig", "{screen}");
}

/// The socket handles frames in order and finishes each before it reads the
/// next one: once a mute is echoed, every frame before it is done.
async fn settle(ws: &mut Ws, server_id: Uuid, voice_id: Uuid, on: bool) {
    send(
        ws,
        json!({ "op": "sig", "t": "m", "s": server_id, "c": voice_id, "on": on }),
    )
    .await;
    until(ws, |f| f["op"] == "sig" && f["t"] == "m" && f["on"] == on).await;
}

/// Going live shows in the voice room and roster only: nothing in the chat.
#[sqlx::test]
async fn going_live_posts_nothing_in_the_chat(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Quiet").await;
    let (server_id, _) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;

    let (addr, _) = common::serve_ws(pool.clone()).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    let screen = |t: &str| json!({ "op": "sig", "t": t, "s": server_id, "c": voice_id, "k": "s" });
    send(&mut owner_ws, screen("p")).await;
    until(&mut owner_ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "s"
    })
    .await;
    // Off and on again, then a re-announcement.
    send(&mut owner_ws, screen("u")).await;
    send(&mut owner_ws, screen("p")).await;
    send(&mut owner_ws, screen("p")).await;
    settle(&mut owner_ws, server_id, voice_id, true).await;

    let mut member_ws = connect(addr, &session_cookie(&member)).await;
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut member_ws, |f| {
        f["op"] == "sig" && f["t"] == "j" && f["u"] != owner_id(&server).to_string()
    })
    .await;
    send(&mut member_ws, screen("p")).await;
    settle(&mut member_ws, server_id, voice_id, true).await;

    let messages: i64 = sqlx::query_scalar("SELECT count(*) FROM messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(messages, 0, "no message in any channel");
}

#[sqlx::test]
async fn screen_live_shows_in_occupancy_snapshot(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Badge").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let ada = owner_id(&server);

    let (addr, _) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner)).await;
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    until(&mut owner_ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "s"
    })
    .await;

    // Someone outside the room sees who is live from the snapshot alone.
    let mut watcher = connect(addr, &session_cookie(&member)).await;
    send(
        &mut watcher,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    until(&mut watcher, |f| f["op"] == "ok").await;
    let roster = until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "r").await;
    let snap = roster["snap"].as_array().expect("snap");
    assert_eq!(snap.len(), 1, "{roster}");
    assert_eq!(snap[0]["u"], ada.to_string());
    assert_eq!(snap[0]["s"], true);
    assert!(snap[0].get("l").is_none(), "{roster}");
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

    send(
        &mut tab_a,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut tab_a, |f| f["op"] == "sig" && f["t"] == "j").await;
    send(
        &mut tab_b,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut tab_b, |f| f["op"] == "sig" && f["t"] == "j").await;
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;

    tab_a.close(None).await.ok();
    let ada_id = ada.to_string();
    let stray = ws::within(&mut member_ws, Duration::from_millis(200), |f| {
        f["op"] == "sig" && f["t"] == "l" && f["u"] == ada_id
    })
    .await;
    assert_eq!(stray, None, "first tab must not broadcast leave");

    tab_b.close(None).await.ok();
    let left = until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "l").await;
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

    send(
        &mut watcher,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    until(&mut watcher, |f| f["op"] == "ok").await;
    let roster = until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "r").await;
    assert_eq!(roster["s"], server_id.to_string());
    assert_eq!(roster["snap"].as_array().unwrap().len(), 0);

    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let joined = until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "j").await;
    assert_eq!(joined["u"], ada.to_string());
    assert_eq!(joined["c"], voice_id.to_string());

    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "m", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    let muted = until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "m").await;
    assert_eq!(muted["on"], true);
    assert_eq!(muted["u"], ada.to_string());

    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "d", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    let deafened = until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "d").await;
    assert_eq!(deafened["on"], true);

    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "l", "s": server_id, "c": voice_id }),
    )
    .await;
    let left = until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "l").await;
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
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "m", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "m").await;

    let mut watcher = connect(addr, &session_cookie(&member)).await;
    send(
        &mut watcher,
        json!({ "op": "s", "s": server_id, "c": text_id }),
    )
    .await;
    until(&mut watcher, |f| f["op"] == "ok").await;
    let roster = until(&mut watcher, |f| f["op"] == "sig" && f["t"] == "r").await;
    let snap = roster["snap"].as_array().expect("snap");
    assert_eq!(snap.len(), 1, "{roster}");
    assert_eq!(snap[0]["u"], ada.to_string());
    assert_eq!(snap[0]["c"], voice_id.to_string());
    assert_eq!(snap[0]["m"], true);

    send(
        &mut watcher,
        json!({ "op": "sig", "t": "m", "s": server_id, "c": voice_id, "on": true }),
    )
    .await;
    let err = until(&mut watcher, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "bad_request");
}

#[sqlx::test]
async fn live_needs_permission_and_is_several_per_channel(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Stream").await;
    let (server_id, _) = ids(&server);
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
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut member_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    let denied = until(&mut member_ws, |f| f["op"] == "err").await;
    assert_eq!(denied["e"], "forbidden");

    send(
        &mut member_ws,
        json!({"op":"sig","t":"p","s":server_id,"c":voice_id,"k":"sa"}),
    )
    .await;
    assert_eq!(
        until(&mut member_ws, |f| f["op"] == "err").await["e"],
        "forbidden"
    );

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
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut owner_ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    send(
        &mut owner_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    let live = until(&mut owner_ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "s"
    })
    .await;
    assert_eq!(live["u"], ada.to_string());
    assert!(live.get("lc").is_none(), "{live}");

    // Several can be live in one channel.
    send(
        &mut member_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    let second = until(&mut member_ws, |f| {
        f["op"] == "err"
            || (f["op"] == "sig" && f["t"] == "p" && f["k"] == "s" && f["u"] != ada.to_string())
    })
    .await;
    assert_eq!(second["op"], "sig", "{second}");
}

#[sqlx::test]
async fn source_audio_requires_its_same_socket_parent_and_parent_stop_cleans_it(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Source audio").await;
    let (server_id, _) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let channel = create_voice(&mut owner, server_id).await;
    let (addr, _) = common::serve_ws(pool).await;
    let mut publisher = connect(addr, &session_cookie(&owner)).await;
    let mut other_tab = connect(addr, &session_cookie(&owner)).await;
    for ws in [&mut publisher, &mut other_tab] {
        send(ws, json!({"op":"sig","t":"j","s":server_id,"c":channel})).await;
        until(ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    }
    send(
        &mut publisher,
        json!({"op":"sig","t":"p","k":"sa","s":server_id,"c":channel}),
    )
    .await;
    let rejected = until(&mut publisher, |f| f["op"] == "err").await;
    assert_eq!(rejected["e"], "bad_request");
    for kind in ["a", "s", "sa"] {
        send(
            &mut publisher,
            json!({"op":"sig","t":"p","k":kind,"s":server_id,"c":channel}),
        )
        .await;
        until(&mut publisher, |f| {
            f["op"] == "sig" && f["t"] == "p" && f["k"] == kind
        })
        .await;
    }
    send(
        &mut other_tab,
        json!({"op":"sig","t":"p","k":"sa","s":server_id,"c":channel}),
    )
    .await;
    assert_eq!(
        until(&mut other_tab, |f| f["op"] == "err").await["e"],
        "bad_request"
    );
    send(
        &mut publisher,
        json!({"op":"sig","t":"u","k":"s","s":server_id,"c":channel}),
    )
    .await;
    let mut removed = std::collections::HashSet::new();
    while removed.len() < 2 {
        let frame = until(&mut publisher, |f| f["op"] == "sig" && f["t"] == "u").await;
        removed.insert(frame["k"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        removed,
        std::collections::HashSet::from(["s".into(), "sa".into()])
    );
    let mut viewer = connect(addr, &session_cookie(&member)).await;
    send(
        &mut viewer,
        json!({"op":"sig","t":"j","s":server_id,"c":channel}),
    )
    .await;
    let mut snapshot = std::collections::HashSet::new();
    loop {
        let frame = recv(&mut viewer).await;
        if frame["op"] == "sig" && frame["t"] == "p" {
            snapshot.insert(frame["k"].as_str().unwrap().to_owned());
        }
        if frame["op"] == "sig" && frame["t"] == "j" && frame["u"] != owner_id(&server).to_string()
        {
            break;
        }
    }
    assert_eq!(snapshot, std::collections::HashSet::from(["a".into()]));
}

async fn another_user(pool: PgPool, owner: &mut Client, server_id: Uuid, name: &str) -> Client {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let res = client
        .register(&format!("{name}@example.com"), "password123", name)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    join_member(owner, &mut client, &server_id.to_string()).await;
    client
}

async fn seat(addr: std::net::SocketAddr, client: &Client, server_id: Uuid, voice_id: Uuid) -> Ws {
    let mut ws = connect(addr, &session_cookie(client)).await;
    send(
        &mut ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut ws, |f| {
        f["op"] == "sig" && f["t"] == "j" && f["u"] == client.user_id()
    })
    .await;
    ws
}

fn watch(server_id: Uuid, channel_id: Uuid, owner: &str, on: bool) -> Value {
    json!({ "op": "sig", "t": "w", "s": server_id, "c": channel_id, "o": owner, "k": "s", "on": on })
}

async fn watch_set(ws: &mut Ws, watcher: &str) -> Value {
    until(ws, |f| {
        f["op"] == "sig" && f["t"] == "w" && f["u"] == watcher
    })
    .await
}

/// Who watches a Live reaches everyone in the room, and a late joiner gets
/// every occupant's set in its replay.
#[sqlx::test]
async fn watching_a_live_reaches_the_room_and_late_joiners(pool: PgPool) {
    let (mut owner, member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Kino").await;
    let (server_id, _) = ids(&server);
    let mut member = member;
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let cara = another_user(pool.clone(), &mut owner, server_id, "Cara").await;
    let dan = another_user(pool.clone(), &mut owner, server_id, "Dan").await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let (ada, bob) = (owner.user_id().to_owned(), member.user_id().to_owned());

    let (addr, _) = common::serve_ws(pool).await;
    let mut ada_ws = seat(addr, &owner, server_id, voice_id).await;
    let mut bob_ws = seat(addr, &member, server_id, voice_id).await;
    let mut cara_ws = seat(addr, &cara, server_id, voice_id).await;
    send(
        &mut ada_ws,
        json!({ "op": "sig", "t": "p", "s": server_id, "c": voice_id, "k": "s" }),
    )
    .await;
    send(&mut bob_ws, watch(server_id, voice_id, &ada, true)).await;
    for ws in [&mut ada_ws, &mut bob_ws, &mut cara_ws] {
        let set = watch_set(ws, &bob).await;
        assert_eq!(set["w"], json!([ada]), "{set}");
        assert_eq!(set["s"], server_id.to_string());
        assert_eq!(set["c"], voice_id.to_string());
        assert!(set.get("replay").is_none(), "{set}");
    }

    // Dan joins late: one replayed set per occupant, also the empty ones.
    let mut dan_ws = connect(addr, &session_cookie(&dan)).await;
    send(
        &mut dan_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let mut sets = std::collections::HashMap::new();
    while sets.len() < 3 {
        let frame = until(&mut dan_ws, |f| f["op"] == "sig" && f["t"] == "w").await;
        assert_eq!(frame["replay"], true, "{frame}");
        sets.insert(frame["u"].as_str().unwrap().to_owned(), frame["w"].clone());
    }
    assert_eq!(sets[&bob], json!([ada]));
    assert_eq!(sets[&ada], json!([]));
    assert_eq!(sets[cara.user_id()], json!([]));

    // Bob leaves: the room hears `l`, and Bob's rejoin starts with nothing.
    send(
        &mut bob_ws,
        json!({ "op": "sig", "t": "l", "s": server_id, "c": voice_id }),
    )
    .await;
    let left = until(&mut cara_ws, |f| f["op"] == "sig" && f["u"] == bob).await;
    assert_eq!(left["t"], "l", "{left}");
    send(
        &mut bob_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    until(&mut cara_ws, |f| {
        f["op"] == "sig" && f["t"] == "j" && f["u"] == bob
    })
    .await;
    drop(dan_ws);
    let mut dan_ws = connect(addr, &session_cookie(&dan)).await;
    send(
        &mut dan_ws,
        json!({ "op": "sig", "t": "j", "s": server_id, "c": voice_id }),
    )
    .await;
    let again = until(&mut dan_ws, |f| {
        f["op"] == "sig" && f["t"] == "w" && f["u"] == bob
    })
    .await;
    assert_eq!(again["w"], json!([]), "{again}");
    assert_eq!(again["replay"], true, "{again}");
}

async fn refused(ws: &mut Ws) -> Value {
    until(ws, |f| f["op"] == "err").await["e"].clone()
}

#[sqlx::test]
async fn watch_needs_a_seat_another_owner_and_join_voice(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Regeln").await;
    let (server_id, text_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;
    let voice_id = create_voice(&mut owner, server_id).await;
    let (ada, bob) = (owner.user_id().to_owned(), member.user_id().to_owned());

    let (addr, _) = common::serve_ws(pool).await;

    // No seat in the room yet.
    let mut bob_ws = connect(addr, &session_cookie(&member)).await;
    send(&mut bob_ws, watch(server_id, voice_id, &ada, true)).await;
    assert_eq!(refused(&mut bob_ws).await, "bad_request");
    // A text channel is no voice room.
    send(&mut bob_ws, watch(server_id, text_id, &ada, true)).await;
    assert_eq!(refused(&mut bob_ws).await, "bad_request");

    drop(bob_ws);
    let mut bob_ws = seat(addr, &member, server_id, voice_id).await;
    // Oneself, a missing owner or `on`, and a kind other than the screen.
    for frame in [
        watch(server_id, voice_id, &bob, true),
        json!({ "op": "sig", "t": "w", "s": server_id, "c": voice_id, "k": "s", "on": true }),
        json!({ "op": "sig", "t": "w", "s": server_id, "c": voice_id, "o": ada, "k": "s" }),
        json!({ "op": "sig", "t": "w", "s": server_id, "c": voice_id, "o": ada, "k": "v", "on": true }),
    ] {
        send(&mut bob_ws, frame.clone()).await;
        assert_eq!(refused(&mut bob_ws).await, "bad_request", "{frame}");
    }
    // Without `k` it is the screen; the owner need not be in the room.
    send(
        &mut bob_ws,
        json!({ "op": "sig", "t": "w", "s": server_id, "c": voice_id, "o": ada, "on": true }),
    )
    .await;
    let set = until(&mut bob_ws, |f| {
        f["op"] == "err" || (f["op"] == "sig" && f["t"] == "w")
    })
    .await;
    assert_eq!(set["w"], json!([ada]), "{set}");

    // Without join_voice the seat is no longer enough.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({ "member_permissions": ["send_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    send(&mut bob_ws, watch(server_id, voice_id, &ada, false)).await;
    assert_eq!(refused(&mut bob_ws).await, "forbidden");
}
