//! Native WS gateway (issue #6): session auth, subscribe, compact events,
//! heartbeat, Redis Pub/Sub fan-out, reconnect catch-up.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::{AppState, EventKind, publish_channel, publish_server};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{COOKIE, ORIGIN};
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

fn session_cookie(client: &Client) -> String {
    client
        .jar
        .get(SESSION)
        .cloned()
        .expect("session cookie after login")
}

async fn connect(addr: std::net::SocketAddr, cookie: &str, origin: Option<&str>) -> Ws {
    let mut request = format!("ws://{addr}/ws")
        .into_client_request()
        .expect("ws request");
    request.headers_mut().insert(
        COOKIE,
        format!("{SESSION}={cookie}")
            .parse()
            .expect("cookie header"),
    );
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert(ORIGIN, origin.parse().expect("origin"));
    }
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
        Message::Text(text) => {
            let frame: Value = serde_json::from_str(text.as_str()).expect("json frame");
            if frame["op"] == "h" {
                // A real client acknowledges server heartbeats even while it
                // waits for a different event. Liveness does not mark activity.
                send_json(ws, json!({ "op": "h" })).await;
            }
            frame
        }
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

async fn publish(
    state: &AppState,
    server_id: Uuid,
    channel_id: Uuid,
    kind: EventKind,
    body: &str,
) -> u64 {
    let event = publish_channel(
        state,
        server_id,
        channel_id,
        kind,
        Some(Uuid::from_u128(0xE01)),
        Some(json!({ "b": body })),
    )
    .await
    .expect("publish");
    event.n
}

#[sqlx::test]
async fn upgrade_requires_a_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let res = client.send(Method::GET, "/ws", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "unauthenticated");
}

#[sqlx::test]
async fn foreign_origin_is_rejected(pool: PgPool) {
    let (owner, _) = two_users(pool.clone()).await;
    let cookie = session_cookie(&owner);
    let (addr, _) = common::serve_ws(pool).await;

    let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert(COOKIE, format!("{SESSION}={cookie}").parse().unwrap());
    request
        .headers_mut()
        .insert(ORIGIN, "https://evil.example".parse().unwrap());
    let err = tokio_tungstenite::connect_async(request)
        .await
        .expect_err("cross-origin upgrade");
    let text = err.to_string();
    assert!(
        text.contains("403") || text.contains("Forbidden"),
        "expected 403, got {text}"
    );

    // Same-origin Origin (Vite/Caddy keep Host as the page host) is fine.
    let _ws = connect(addr, &cookie, Some(&format!("http://{addr}"))).await;
    let _ = owner;
}

#[sqlx::test]
async fn events_only_reach_matching_subscribers(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Chat").await;
    let (server_id, channel_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;

    let other = create_server(&mut owner, "Other").await;
    let (other_server, other_channel) = ids(&other);

    let (addr, state) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner), None).await;
    let mut member_ws = connect(addr, &session_cookie(&member), None).await;

    send_json(
        &mut owner_ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    send_json(
        &mut member_ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;

    let owner_ok = recv_until(&mut owner_ws, |f| f["op"] == "ok").await;
    let member_ok = recv_until(&mut member_ws, |f| f["op"] == "ok").await;
    assert_eq!(owner_ok["n"], 0);
    assert_eq!(member_ok["c"], channel_id.to_string());

    let n = publish(&state, server_id, channel_id, EventKind::C, "hello").await;
    let owner_event = recv_until(&mut owner_ws, |f| f["op"] == "e").await;
    let member_event = recv_until(&mut member_ws, |f| f["op"] == "e").await;
    assert_eq!(owner_event["t"], "c");
    assert_eq!(owner_event["n"], n);
    assert_eq!(owner_event["d"]["b"], "hello");
    assert_eq!(member_event["n"], n);
    assert!(
        owner_event.to_string().len() < 220,
        "payload should stay compact: {}",
        owner_event
    );

    // Event on another channel must not arrive.
    publish(&state, other_server, other_channel, EventKind::C, "nope").await;
    let stray = tokio::time::timeout(Duration::from_millis(150), recv_json(&mut owner_ws)).await;
    if let Ok(frame) = stray {
        assert_ne!(frame["c"], other_channel.to_string(), "{frame}");
        assert_ne!(frame["op"], "e");
    }

    // Unsubscribe: further events on this channel stop.
    send_json(
        &mut member_ws,
        json!({ "op": "u", "s": server_id, "c": channel_id }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    publish(&state, server_id, channel_id, EventKind::E, "edit").await;
    let owner_edit = recv_until(&mut owner_ws, |f| f["op"] == "e" && f["t"] == "e").await;
    assert_eq!(owner_edit["d"]["b"], "edit");
    let member_stray =
        tokio::time::timeout(Duration::from_millis(150), recv_json(&mut member_ws)).await;
    if let Ok(frame) = member_stray {
        assert_ne!(frame["t"], "e", "{frame}");
    }
}

#[sqlx::test]
async fn subscribe_is_membership_scoped(pool: PgPool) {
    let (mut owner, member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Private").await;
    let (server_id, channel_id) = ids(&server);
    let (addr, _) = common::serve_ws(pool).await;

    let mut stranger = connect(addr, &session_cookie(&member), None).await;
    send_json(
        &mut stranger,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    let err = recv_until(&mut stranger, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "not_found");

    // Channel that does not belong to the server: also not_found.
    let mut owner_ws = connect(addr, &session_cookie(&owner), None).await;
    send_json(
        &mut owner_ws,
        json!({ "op": "s", "s": server_id, "c": Uuid::from_u128(99) }),
    )
    .await;
    let err = recv_until(&mut owner_ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "not_found");
}

#[sqlx::test]
async fn reconnect_replays_the_gap_without_duplicates(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Catchup").await;
    let (server_id, channel_id) = ids(&server);
    let cookie = session_cookie(&owner);
    let (addr, state) = common::serve_ws(pool).await;

    let mut ws = connect(addr, &cookie, None).await;
    send_json(
        &mut ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    recv_until(&mut ws, |f| f["op"] == "ok").await;

    let first = publish(&state, server_id, channel_id, EventKind::C, "one").await;
    let second = publish(&state, server_id, channel_id, EventKind::C, "two").await;
    recv_until(&mut ws, |f| f["n"] == first).await;
    recv_until(&mut ws, |f| f["n"] == second).await;
    ws.close(None).await.ok();

    let third = publish(&state, server_id, channel_id, EventKind::C, "three").await;
    let fourth = publish(&state, server_id, channel_id, EventKind::D, "gone").await;

    let mut ws = connect(addr, &cookie, None).await;
    send_json(
        &mut ws,
        json!({ "op": "s", "s": server_id, "c": channel_id, "n": second }),
    )
    .await;

    let replayed: Vec<Value> = {
        let mut out = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline {
            let frame = recv_json(&mut ws).await;
            let op = frame["op"].as_str().unwrap_or_default().to_owned();
            if op == "e" {
                out.push(frame);
            }
            if op == "ok" {
                break;
            }
        }
        out
    };
    let seqs: Vec<u64> = replayed.iter().map(|f| f["n"].as_u64().unwrap()).collect();
    assert_eq!(seqs, vec![third, fourth], "{replayed:?}");
    assert_eq!(replayed[0]["t"], "c");
    assert_eq!(replayed[1]["t"], "d");
}

#[sqlx::test]
async fn reconnect_signals_gap_when_the_buffer_cannot_fill_it(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Gap").await;
    let (server_id, channel_id) = ids(&server);
    let (addr, state) = common::serve_ws(pool).await;

    for i in 0..10 {
        publish(
            &state,
            server_id,
            channel_id,
            EventKind::C,
            &format!("m{i}"),
        )
        .await;
    }

    let mut ws = connect(addr, &session_cookie(&owner), None).await;
    send_json(
        &mut ws,
        json!({ "op": "s", "s": server_id, "c": channel_id, "n": 1 }),
    )
    .await;
    let gap = recv_until(&mut ws, |f| f["op"] == "gap" || f["op"] == "ok").await;
    assert_eq!(gap["op"], "gap", "{gap}");
    let ok = recv_until(&mut ws, |f| f["op"] == "ok").await;
    assert_eq!(ok["n"], 10);
}

#[sqlx::test]
async fn heartbeat_keeps_the_socket_and_silence_closes_it(pool: PgPool) {
    let (owner, _) = two_users(pool.clone()).await;
    let (addr, _) = common::serve_ws_with_dead(pool, 180).await;
    let mut ws = connect(addr, &session_cookie(&owner), None).await;

    let beat = recv_until(&mut ws, |f| f["op"] == "h").await;
    assert_eq!(beat, json!({ "op": "h" }));
    send_json(&mut ws, json!({ "op": "h" })).await;

    // Inbound `h` is liveness only — must not echo, or the browser client
    // (which replies to every server `h`) would ping-pong.
    let echoed = tokio::time::timeout(Duration::from_millis(20), recv_json(&mut ws)).await;
    assert!(
        echoed.is_err(),
        "server must not reply to a client heartbeat: {:?}",
        echoed.ok()
    );

    // Still alive after another server-initiated heartbeat cycle.
    let beat = tokio::time::timeout(
        Duration::from_millis(400),
        recv_until(&mut ws, |f| f["op"] == "h"),
    )
    .await
    .expect("second heartbeat");
    assert_eq!(beat["op"], "h");

    // Stop talking. Silent death is 180ms in the test config.
    let closed = tokio::time::timeout(Duration::from_millis(600), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) | None => return,
                Some(Ok(Message::Text(text))) if text.contains(r#""op":"h""#) => continue,
                Some(Ok(_)) => continue,
                Some(Err(_)) => return,
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "silent socket should be closed");
}

#[sqlx::test]
async fn server_topic_is_separate_from_channel_events(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Srv").await;
    let (server_id, channel_id) = ids(&server);
    let (addr, state) = common::serve_ws(pool).await;
    let mut ws = connect(addr, &session_cookie(&owner), None).await;

    send_json(&mut ws, json!({ "op": "s", "s": server_id })).await;
    recv_until(&mut ws, |f| f["op"] == "ok").await;

    publish_server(
        &state,
        server_id,
        EventKind::C,
        Some(channel_id),
        Some(json!({ "name": "voice" })),
    )
    .await
    .unwrap();
    let event = recv_until(&mut ws, |f| f["op"] == "e").await;
    assert_eq!(event["s"], server_id.to_string());
    assert!(event.get("c").is_none(), "{event}");

    publish(&state, server_id, channel_id, EventKind::C, "chat").await;
    let stray = tokio::time::timeout(Duration::from_millis(150), recv_json(&mut ws)).await;
    if let Ok(frame) = stray {
        assert_ne!(frame["c"], channel_id.to_string());
    }
}

#[sqlx::test]
async fn live_events_during_catch_up_are_queued(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Queue").await;
    let (server_id, channel_id) = ids(&server);
    let state = common::ws_state(pool);
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .expect("redis pub/sub");

    let topic = gelabber_api::gateway::Topic::Channel(channel_id);
    // Absorb the first Redis PUBLISH on a live socket so a late fan-out
    // cannot land in the catch-up queue (CI flake: queued_len 2 vs 1).
    let (drain_tx, mut drain_rx) = tokio::sync::mpsc::channel(128);
    let drain = state.gateway.attach(Uuid::from_u128(2), drain_tx).await;
    state.gateway.begin_catch_up(drain, topic).await;
    state.gateway.finish_catch_up(drain, topic, 0).await;

    let first = publish(&state, server_id, channel_id, EventKind::C, "old").await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let frame = drain_rx.recv().await.expect("drain closed");
            let json = serde_json::to_value(&frame).expect("drain json");
            if json["op"] == "e" && json["n"] == first {
                break;
            }
        }
    })
    .await
    .expect("first PUBLISH must reach Redis subscribers before catch-up");
    state.gateway.detach(drain).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let conn = state.gateway.attach(Uuid::from_u128(1), tx).await;
    state.gateway.begin_catch_up(conn, topic).await;

    let live = publish(&state, server_id, channel_id, EventKind::C, "live").await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if state.gateway.queued_len(conn, topic).await == 1 {
            break;
        }
    }
    assert_eq!(
        state.gateway.queued_len(conn, topic).await,
        1,
        "live Pub/Sub frame must be queued, not dropped"
    );

    state.gateway.finish_catch_up(conn, topic, first).await;
    let frame = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("flush timeout")
        .expect("channel closed");
    let json = serde_json::to_value(&frame).expect("frame json");
    assert_eq!(json["op"], "e");
    assert_eq!(json["n"], live);
    assert_eq!(json["d"]["b"], "live");
    assert!(rx.try_recv().is_err(), "watermark must drop seq {first}");
}

#[sqlx::test]
async fn concurrent_publish_keeps_seq_and_arrival_ordered(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Race").await;
    let (server_id, channel_id) = ids(&server);
    let (addr, state) = common::serve_ws(pool).await;
    let mut ws = connect(addr, &session_cookie(&owner), None).await;
    send_json(
        &mut ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    recv_until(&mut ws, |f| f["op"] == "ok").await;

    let mut tasks = Vec::new();
    for i in 0..16 {
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            publish(
                &state,
                server_id,
                channel_id,
                EventKind::C,
                &format!("m{i}"),
            )
            .await
        }));
    }
    let mut assigned = Vec::new();
    for task in tasks {
        assigned.push(task.await.expect("join"));
    }
    assigned.sort();
    assert_eq!(assigned, (1..=16).collect::<Vec<_>>());

    let mut arrived = Vec::new();
    while arrived.len() < 16 {
        let frame = recv_until(&mut ws, |f| f["op"] == "e").await;
        arrived.push(frame["n"].as_u64().unwrap());
    }
    assert_eq!(
        arrived, assigned,
        "PUBLISH order must match INCR so the client never skips a late seq"
    );
}

#[sqlx::test]
async fn presence_online_idle_offline_without_bumping_chat_seq(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Pres").await;
    let (server_id, channel_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;

    let (addr, state) = common::serve_ws_idle(pool, 80).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner), None).await;
    let mut member_ws = connect(addr, &session_cookie(&member), None).await;

    send_json(&mut owner_ws, json!({ "op": "s", "s": server_id })).await;
    let owner_ok = recv_until(&mut owner_ws, |f| f["op"] == "ok").await;
    assert_eq!(owner_ok["n"], 0);
    let snap = recv_until(&mut owner_ws, |f| f["op"] == "p" && f.get("snap").is_some()).await;
    assert_eq!(snap["s"], server_id.to_string());
    assert!(snap.get("n").is_none(), "{snap}");

    send_json(&mut member_ws, json!({ "op": "s", "s": server_id })).await;
    recv_until(&mut member_ws, |f| f["op"] == "ok").await;
    let member_online = recv_until(&mut owner_ws, |f| {
        f["op"] == "p" && f["u"] == member.user_id() && f["st"] == "o"
    })
    .await;
    assert!(member_online.get("n").is_none(), "{member_online}");

    send_json(&mut member_ws, json!({ "op": "p", "st": "i" })).await;
    let idle = recv_until(&mut owner_ws, |f| {
        f["op"] == "p" && f["u"] == member.user_id() && f["st"] == "i"
    })
    .await;
    assert_eq!(idle["s"], server_id.to_string());

    send_json(&mut member_ws, json!({ "op": "p" })).await;
    recv_until(&mut owner_ws, |f| {
        f["op"] == "p" && f["u"] == member.user_id() && f["st"] == "o"
    })
    .await;

    // Server-side per-client idle: no activity frames, heartbeat still flows.
    let auto_idle = tokio::time::timeout(Duration::from_millis(400), async {
        recv_until(&mut owner_ws, |f| {
            f["op"] == "p" && f["u"] == member.user_id() && f["st"] == "i"
        })
        .await
    })
    .await
    .expect("idle timeout");
    assert_eq!(auto_idle["st"], "i");

    member_ws.close(None).await.ok();
    let offline = recv_until(&mut owner_ws, |f| {
        f["op"] == "p" && f["u"] == member.user_id() && f["st"] == "x"
    })
    .await;
    assert_eq!(offline["op"], "p");

    let n = publish(&state, server_id, channel_id, EventKind::C, "chat").await;
    assert_eq!(n, 1, "presence must not increment the chat seq");
}

#[sqlx::test]
async fn two_clients_keep_user_online_until_the_last_goes_idle(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Tabs").await;
    let (server_id, _) = ids(&server);
    let cookie = session_cookie(&owner);
    let (addr, _) = common::serve_ws(pool).await;

    let mut a = connect(addr, &cookie, None).await;
    let mut b = connect(addr, &cookie, None).await;
    send_json(&mut a, json!({ "op": "s", "s": server_id })).await;
    send_json(&mut b, json!({ "op": "s", "s": server_id })).await;
    recv_until(&mut a, |f| f["op"] == "ok").await;
    recv_until(&mut b, |f| f["op"] == "ok").await;

    send_json(&mut a, json!({ "op": "p", "st": "i" })).await;
    // The other tab is still online — aggregate must stay online.
    let flipped = tokio::time::timeout(Duration::from_millis(150), async {
        recv_until(&mut b, |f| f["op"] == "p" && f["st"] == "i").await
    })
    .await;
    assert!(
        flipped.is_err(),
        "one idle client must not mark the user idle: {:?}",
        flipped.ok()
    );

    send_json(&mut b, json!({ "op": "p", "st": "i" })).await;
    let idle = recv_until(&mut a, |f| f["op"] == "p" && f["st"] == "i").await;
    assert_eq!(idle["st"], "i");
}

#[sqlx::test]
async fn typing_broadcasts_fast_and_clears_on_stop(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Type").await;
    let (server_id, channel_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;

    let (addr, state) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner), None).await;
    let mut member_ws = connect(addr, &session_cookie(&member), None).await;
    send_json(
        &mut owner_ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    send_json(
        &mut member_ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "ok").await;
    recv_until(&mut member_ws, |f| f["op"] == "ok").await;

    let start = std::time::Instant::now();
    send_json(
        &mut member_ws,
        json!({ "op": "y", "s": server_id, "c": channel_id, "on": true }),
    )
    .await;
    let seen = recv_until(&mut owner_ws, |f| {
        f["op"] == "y" && f["on"] == true && f["u"] == member.user_id()
    })
    .await;
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "typing must be visible in under 100ms, took {:?}",
        start.elapsed()
    );
    assert_eq!(seen["c"], channel_id.to_string());
    assert!(seen.get("n").is_none(), "{seen}");

    send_json(
        &mut member_ws,
        json!({ "op": "y", "s": server_id, "c": channel_id, "on": false }),
    )
    .await;
    let stop = recv_until(&mut owner_ws, |f| {
        f["op"] == "y" && f["on"] == false && f["u"] == member.user_id()
    })
    .await;
    assert_eq!(stop["on"], false);

    let n = publish(&state, server_id, channel_id, EventKind::C, "hi").await;
    assert_eq!(n, 1, "typing must not increment the chat seq");
}

#[sqlx::test]
async fn last_close_and_idle_reach_every_server_the_user_is_on(pool: PgPool) {
    let (mut owner, mut watcher) = two_users(pool.clone()).await;
    let first = create_server(&mut owner, "One").await;
    let second = create_server(&mut owner, "Two").await;
    let (s1, _) = ids(&first);
    let (s2, _) = ids(&second);
    join_member(&mut owner, &mut watcher, &s1.to_string()).await;
    join_member(&mut owner, &mut watcher, &s2.to_string()).await;

    let cookie = session_cookie(&owner);
    let (addr, _) = common::serve_ws(pool).await;
    let mut tab_a = connect(addr, &cookie, None).await;
    let mut tab_b = connect(addr, &cookie, None).await;
    let mut watch_s1 = connect(addr, &session_cookie(&watcher), None).await;
    let mut watch_s2 = connect(addr, &session_cookie(&watcher), None).await;

    send_json(&mut tab_a, json!({ "op": "s", "s": s1 })).await;
    send_json(&mut tab_b, json!({ "op": "s", "s": s2 })).await;
    recv_until(&mut tab_a, |f| f["op"] == "ok").await;
    recv_until(&mut tab_b, |f| f["op"] == "ok").await;
    // `ok` is sent before the gateway stores presence. Wait for each
    // owner's snapshot before asking observers for theirs.
    recv_until(&mut tab_a, |f| f["op"] == "p" && f.get("snap").is_some()).await;
    recv_until(&mut tab_b, |f| f["op"] == "p" && f.get("snap").is_some()).await;
    send_json(&mut watch_s1, json!({ "op": "s", "s": s1 })).await;
    send_json(&mut watch_s2, json!({ "op": "s", "s": s2 })).await;
    recv_until(&mut watch_s1, |f| f["op"] == "ok").await;
    recv_until(&mut watch_s2, |f| f["op"] == "ok").await;
    let snap1 = recv_until(&mut watch_s1, |f| f["op"] == "p" && f.get("snap").is_some()).await;
    let snap2 = recv_until(&mut watch_s2, |f| f["op"] == "p" && f.get("snap").is_some()).await;
    assert!(
        snap1["snap"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| e["u"] == owner.user_id() && e["st"] == "o"),
        "{snap1}"
    );
    assert!(
        snap2["snap"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| e["u"] == owner.user_id() && e["st"] == "o"),
        "{snap2}"
    );

    send_json(&mut tab_a, json!({ "op": "p", "st": "i" })).await;
    let no_idle = tokio::time::timeout(Duration::from_millis(120), async {
        recv_until(&mut watch_s2, |f| f["op"] == "p" && f["st"] == "i").await
    })
    .await;
    assert!(
        no_idle.is_err(),
        "one idle tab must not flip the user: {:?}",
        no_idle.ok()
    );

    send_json(&mut tab_b, json!({ "op": "p", "st": "i" })).await;
    let idle_s1 = recv_until(&mut watch_s1, |f| {
        f["op"] == "p" && f["u"] == owner.user_id() && f["st"] == "i"
    })
    .await;
    let idle_s2 = recv_until(&mut watch_s2, |f| {
        f["op"] == "p" && f["u"] == owner.user_id() && f["st"] == "i"
    })
    .await;
    assert_eq!(idle_s1["s"], s1.to_string());
    assert_eq!(idle_s2["s"], s2.to_string());

    tab_a.close(None).await.ok();
    tab_b.close(None).await.ok();
    let off_s1 = recv_until(&mut watch_s1, |f| {
        f["op"] == "p" && f["u"] == owner.user_id() && f["st"] == "x"
    })
    .await;
    let off_s2 = recv_until(&mut watch_s2, |f| {
        f["op"] == "p" && f["u"] == owner.user_id() && f["st"] == "x"
    })
    .await;
    assert_eq!(off_s1["s"], s1.to_string());
    assert_eq!(off_s2["s"], s2.to_string());
}

#[sqlx::test]
async fn disconnect_stops_typing_immediately(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Stop").await;
    let (server_id, channel_id) = ids(&server);
    join_member(&mut owner, &mut member, &server_id.to_string()).await;

    let (addr, _) = common::serve_ws(pool).await;
    let mut owner_ws = connect(addr, &session_cookie(&owner), None).await;
    let mut member_ws = connect(addr, &session_cookie(&member), None).await;
    send_json(
        &mut owner_ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    send_json(
        &mut member_ws,
        json!({ "op": "s", "s": server_id, "c": channel_id }),
    )
    .await;
    recv_until(&mut owner_ws, |f| f["op"] == "ok").await;
    recv_until(&mut member_ws, |f| f["op"] == "ok").await;

    send_json(
        &mut member_ws,
        json!({ "op": "y", "s": server_id, "c": channel_id, "on": true }),
    )
    .await;
    recv_until(&mut owner_ws, |f| {
        f["op"] == "y" && f["on"] == true && f["u"] == member.user_id()
    })
    .await;

    let start = std::time::Instant::now();
    member_ws.close(None).await.ok();
    let stop = recv_until(&mut owner_ws, |f| {
        f["op"] == "y" && f["on"] == false && f["u"] == member.user_id()
    })
    .await;
    assert_eq!(stop["c"], channel_id.to_string());
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "typing must stop on detach, not wait for TTL, took {:?}",
        start.elapsed()
    );
}

#[sqlx::test]
async fn dm_uses_the_same_subscribe_and_message_paths(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut ada = Client::with_state(state.clone());
    ada.bootstrap().await;
    let res = ada
        .register("ada-ws@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let mut bob = Client::with_state(state.clone());
    bob.bootstrap().await;
    let res = bob
        .register("bob-ws@example.com", "password123", "Bob")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let opened = ada
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": bob.user_id() })),
        )
        .await;
    assert_eq!(opened.status, StatusCode::CREATED, "{}", opened.body);
    let channel_id = opened.body["id"].as_str().unwrap();

    let mut ada_ws = connect(addr, &session_cookie(&ada), None).await;
    let mut bob_ws = connect(addr, &session_cookie(&bob), None).await;
    send_json(
        &mut ada_ws,
        json!({ "op": "s", "s": channel_id, "c": channel_id }),
    )
    .await;
    send_json(
        &mut bob_ws,
        json!({ "op": "s", "s": channel_id, "c": channel_id }),
    )
    .await;
    recv_until(&mut ada_ws, |f| f["op"] == "ok" && f["c"] == channel_id).await;
    recv_until(&mut bob_ws, |f| f["op"] == "ok" && f["c"] == channel_id).await;

    let posted = ada
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "dm ping" })),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);

    let event = recv_until(&mut bob_ws, |f| {
        f["op"] == "e" && f["t"] == "c" && f["c"] == channel_id
    })
    .await;
    assert_eq!(event["s"], channel_id);
    assert_eq!(event["i"], posted.body["id"]);
    assert_eq!(event["d"]["content"], "dm ping");

    send_json(
        &mut ada_ws,
        json!({ "op": "y", "s": channel_id, "c": channel_id, "on": true }),
    )
    .await;
    let typing = recv_until(&mut bob_ws, |f| {
        f["op"] == "y" && f["on"] == true && f["u"] == ada.user_id()
    })
    .await;
    assert_eq!(typing["s"], channel_id);
    assert_eq!(typing["c"], channel_id);

    let mut cara = Client::with_state(state);
    cara.bootstrap().await;
    let res = cara
        .register("cara-ws@example.com", "password123", "Cara")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let mut cara_ws = connect(addr, &session_cookie(&cara), None).await;
    send_json(
        &mut cara_ws,
        json!({ "op": "s", "s": channel_id, "c": channel_id }),
    )
    .await;
    let err = recv_until(&mut cara_ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "not_found");
}

#[sqlx::test]
async fn redis_topic_reset_changes_epoch_and_old_cursor_gets_gap(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Epoch").await;
    let (s, c) = ids(&server);
    let (addr, state) = common::serve_ws(pool).await;
    let mut ws = connect(addr, &session_cookie(&owner), None).await;
    send_json(&mut ws, json!({"op":"s","s":s,"c":c})).await;
    let old = recv_until(&mut ws, |f| f["op"] == "ok").await;
    let epoch = old["ep"].clone();
    for _ in 0..3 {
        publish(&state, s, c, EventKind::C, "before").await;
        recv_until(&mut ws, |f| f["op"] == "e").await;
    }
    let topic = gelabber_api::gateway::Topic::Channel(c);
    let mut redis = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    redis::cmd("DEL")
        .arg(topic.seq_key())
        .arg(topic.log_key())
        .arg(topic.epoch_key())
        .arg(topic.delivery_key())
        .query_async::<i32>(&mut redis)
        .await
        .unwrap();
    send_json(&mut ws, json!({"op":"s","s":s,"c":c,"n":3,"ep":epoch})).await;
    let gap = recv_until(&mut ws, |f| f["op"] == "gap").await;
    assert_ne!(gap["ep"], epoch);
    let reset = recv_until(&mut ws, |f| f["op"] == "ok").await;
    assert_eq!(reset["n"], 0);
    assert_eq!(reset["ep"], gap["ep"]);
    publish(&state, s, c, EventKind::C, "after").await;
    let event = recv_until(&mut ws, |f| f["op"] == "e").await;
    assert_eq!(event["n"], 1);
    assert_eq!(event["ep"], reset["ep"]);
    // Epoch mismatch matters even when the numeric cursor happens to match.
    send_json(&mut ws, json!({"op":"s","s":s,"c":c,"n":1,"ep":epoch})).await;
    recv_until(&mut ws, |f| f["op"] == "gap").await;
    recv_until(&mut ws, |f| f["op"] == "ok").await;
}

#[sqlx::test]
async fn committed_message_retries_after_redis_failure_and_ack_retry_is_deduplicated(pool: PgPool) {
    use gelabber_api::gateway::{Topic, delivery};
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Outbox").await;
    let (s, c) = ids(&server);
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut ws = connect(addr, &session_cookie(&owner), None).await;
    send_json(&mut ws, json!({"op":"s","s":s,"c":c})).await;
    recv_until(&mut ws, |f| f["op"] == "ok").await;
    let mut redis = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let topic = Topic::Channel(c);
    redis::cmd("SET")
        .arg(topic.seq_key())
        .arg("invalid counter")
        .query_async::<()>(&mut redis)
        .await
        .unwrap();
    let created = owner
        .send(
            Method::POST,
            &format!("/api/channels/{c}/messages"),
            Some(json!({"content":"durable"})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let revision = created.body["revision"].as_i64().unwrap();
    assert!(revision > 0);
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gateway_outbox WHERE id=$1")
        .bind(revision)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(queued, 1, "publish failure retains committed outbox");
    redis::cmd("DEL")
        .arg(topic.seq_key())
        .query_async::<i32>(&mut redis)
        .await
        .unwrap();
    sqlx::query("UPDATE gateway_outbox SET next_attempt=now() WHERE id=$1")
        .bind(revision)
        .execute(&pool)
        .await
        .unwrap();
    let recovery = common::ws_state(pool.clone());
    let _ = delivery::deliver_pending(&recovery, 32).await;
    let event = recv_until(&mut ws, |f| f["op"] == "e").await;
    assert_eq!(event["r"], revision);
    assert_eq!(event["d"], created.body);
    let head = state.gateway.current_seq(topic).await.unwrap();
    // Simulate process death after Redis publication but before DB acknowledgement.
    sqlx::query("INSERT INTO gateway_outbox(id,channel_id,server_id,kind,entity_id,delta) VALUES($1,$2,$3,'c',$4,$5) ON CONFLICT(id) DO UPDATE SET next_attempt=now()")
        .bind(revision).bind(c).bind(s).bind(created.body["id"].as_str().unwrap().parse::<Uuid>().unwrap()).bind(created.body.clone()).execute(&pool).await.unwrap();
    let _ = delivery::deliver_pending(&recovery, 32).await;
    assert_eq!(state.gateway.current_seq(topic).await.unwrap(), head);
    assert_eq!(state.gateway.load_log(topic).await.unwrap().len(), 1);
}

#[sqlx::test]
async fn dm_discovery_reaches_already_connected_recipient_and_no_other_user(pool: PgPool) {
    let (mut owner, mut recipient) = two_users(pool.clone()).await;
    let mut foreign = Client::new(pool.clone());
    foreign.bootstrap().await;
    assert_eq!(
        foreign
            .register("foreign@example.com", "password123", "Foreign")
            .await
            .status,
        StatusCode::CREATED
    );
    let (addr, _) = common::serve_ws(pool).await;
    let mut bob = connect(addr, &session_cookie(&recipient), None).await;
    let mut cara = connect(addr, &session_cookie(&foreign), None).await;
    let created = owner
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({"user_id":recipient.user_id()})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let c = created.body["id"].clone();
    let first = owner
        .send(
            Method::POST,
            &format!("/api/channels/{}/messages", c.as_str().unwrap()),
            Some(json!({"content":"first"})),
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED);
    let discovery = recv_until(&mut bob, |f| f["op"] == "dm").await;
    assert_eq!(discovery["c"], c);
    send_json(&mut bob, json!({"op":"s","s":c,"c":c,"n":0})).await;
    let event = recv_until(&mut bob, |f| f["op"] == "e").await;
    assert_eq!(event["d"]["id"], first.body["id"]);
    assert!(
        tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                let f = recv_json(&mut cara).await;
                assert_ne!(f["op"], "dm", "foreign discovery leak");
            }
        })
        .await
        .is_err()
    );
    let list = recipient.send(Method::GET, "/api/dms", None).await;
    assert_eq!(list.body.as_array().unwrap().len(), 1);
}

#[sqlx::test]
async fn parallel_message_edits_deliver_in_database_revision_order(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Revision").await;
    let (s, c) = ids(&server);
    let (addr, _) = common::serve_ws(pool).await;
    let mut ws = connect(addr, &session_cookie(&owner), None).await;
    send_json(&mut ws, json!({"op":"s","s":s,"c":c})).await;
    recv_until(&mut ws, |f| f["op"] == "ok").await;
    let created = owner
        .send(
            Method::POST,
            &format!("/api/channels/{c}/messages"),
            Some(json!({"content":"initial"})),
        )
        .await;
    recv_until(&mut ws, |f| f["op"] == "e").await;
    let path = format!("/api/messages/{}", created.body["id"].as_str().unwrap());
    let mut tab = Client {
        app: owner.app.clone(),
        store: owner.store.clone(),
        jar: owner.jar.clone(),
        csrf: owner.csrf.clone(),
        user_id: owner.user_id.clone(),
    };
    let (a, b) = tokio::join!(
        owner.send(Method::PATCH, &path, Some(json!({"content":"A"}))),
        tab.send(Method::PATCH, &path, Some(json!({"content":"B"})))
    );
    assert_eq!((a.status, b.status), (StatusCode::OK, StatusCode::OK));
    let one = recv_until(&mut ws, |f| f["op"] == "e").await;
    let two = recv_until(&mut ws, |f| f["op"] == "e").await;
    assert!(one["r"].as_i64().unwrap() < two["r"].as_i64().unwrap());
    let latest = owner
        .send(Method::GET, &format!("/api/channels/{c}/messages"), None)
        .await;
    assert_eq!(two["d"], latest.body["messages"][0]);
}

/// Isolated TCP fault proxy; interrupts only this test Gateway's Pub/Sub
/// connections. Command connections and every other Redis client remain live.
async fn pubsub_proxy() -> (redis::Client, tokio::sync::watch::Sender<bool>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let redis = redis::Client::open(common::redis_url()).unwrap();
    let actual = match &redis.get_connection_info().addr() {
        redis::ConnectionAddr::Tcp(host, port) => format!("{host}:{port}"),
        _ => panic!("local test Redis is TCP"),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (control, _) = tokio::sync::watch::channel(false);
    let control_copy = control.clone();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let actual = actual.clone();
            let mut control = control_copy.subscribe();
            tokio::spawn(async move {
                let server = tokio::net::TcpStream::connect(actual).await.unwrap();
                let (mut cr, mut cw) = client.into_split();
                let (mut sr, mut sw) = server.into_split();
                let to_server = async {
                    let mut data = [0u8; 4096];
                    let mut recent = Vec::new();
                    let mut subscribed = false;
                    loop {
                        let n = tokio::select! {n=cr.read(&mut data)=>n?,_=control.changed()=>{if subscribed && *control.borrow(){return Ok::<(),std::io::Error>(());}continue;}};
                        if n == 0 {
                            return Ok(());
                        }
                        recent.extend_from_slice(&data[..n]);
                        if recent
                            .windows(10)
                            .any(|chunk| chunk.eq_ignore_ascii_case(b"PSUBSCRIBE"))
                        {
                            subscribed = true;
                        }
                        if subscribed {
                            while *control.borrow() {
                                if control.changed().await.is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        sw.write_all(&data[..n]).await?;
                        if recent.len() > 32 {
                            recent.drain(..recent.len() - 32);
                        }
                    }
                };
                tokio::select! {_=to_server=>{},_=tokio::io::copy(&mut sr,&mut cw)=>{}}
            });
        }
    });
    (
        redis::Client::open(format!("redis://{addr}")).unwrap(),
        control,
    )
}

#[sqlx::test]
async fn pubsub_outage_preserves_open_socket_and_requests_rest_resync(pool: PgPool) {
    let (mut owner, _) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Subscriber outage").await;
    let (s, c) = ids(&server);
    let (proxy, paused) = pubsub_proxy().await;
    let mut state = common::ws_state(pool);
    let gateway = gelabber_api::gateway::Gateway::new(
        proxy,
        8,
        Duration::from_secs(2),
        Duration::from_millis(400),
    );
    state.gateway = gateway.clone();
    state.connections = gateway.connections.clone();
    state.events = gateway.events.clone();
    state.voice = gateway.voice.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = gelabber_api::app(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    gateway.wait_ready(Duration::from_secs(2)).await.unwrap();
    let mut ws = connect(addr, &session_cookie(&owner), None).await;
    send_json(&mut ws, json!({"op":"s","s":s,"c":c})).await;
    let previous = recv_until(&mut ws, |f| f["op"] == "ok").await;
    paused.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), async {
        while gateway.is_ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let missed = owner
        .send(
            Method::POST,
            &format!("/api/channels/{c}/messages"),
            Some(json!({"content":"during outage"})),
        )
        .await;
    assert_eq!(missed.status, StatusCode::CREATED);
    paused.send_replace(false);
    recv_until(&mut ws, |f| f["op"] == "resync").await;
    let rest = owner
        .send(Method::GET, &format!("/api/channels/{c}/messages"), None)
        .await;
    assert_eq!(rest.body["messages"][0]["id"], missed.body["id"]);
    // Same socket is still authorized; catching up converges without reconnect.
    send_json(
        &mut ws,
        json!({"op":"s","s":s,"c":c,"n":previous["n"],"ep":previous["ep"]}),
    )
    .await;
    let recovered = recv_until(&mut ws, |f| f["op"] == "e").await;
    assert_eq!(recovered["d"]["id"], missed.body["id"]);
    recv_until(&mut ws, |f| f["op"] == "ok").await;
}
