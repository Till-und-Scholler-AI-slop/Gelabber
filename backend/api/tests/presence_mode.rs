//! Manual presence (#590): Away, Do not disturb and Invisible as others see
//! them, the account's own sockets, typing while invisible, and the end of
//! a mode.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::gateway::protocol::REDIS_PREFIX;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::COOKIE;
use uuid::Uuid;

use common::{Client, SESSION};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct World {
    owner: Client,
    member: Client,
    server: Uuid,
    channel: Uuid,
}

async fn setup(pool: PgPool) -> World {
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
    let res = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Ruhe" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = res.body["id"].as_str().unwrap().parse().unwrap();
    let channel = res.body["channels"][0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    let code = res.body["code"].as_str().unwrap().to_owned();
    let res = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    World {
        owner,
        member,
        server,
        channel,
    }
}

async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let cookie = client.jar.get(SESSION).cloned().expect("session cookie");
    let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert(COOKIE, format!("{SESSION}={cookie}").parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws
}

async fn send_json(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .expect("ws send");
}

async fn recv_json(ws: &mut Ws) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("ws recv timeout")
        .expect("ws closed")
        .expect("ws error");
    match msg {
        Message::Text(text) => {
            let frame: Value = serde_json::from_str(text.as_str()).unwrap();
            if frame["op"] == "h" {
                send_json(ws, json!({ "op": "h" })).await;
            }
            frame
        }
        other => panic!("expected text frame, got {other:?}"),
    }
}

/// The first frame `pred` accepts; every frame skipped on the way is
/// returned too, to check nothing else came first.
async fn recv_until(ws: &mut Ws, pred: impl Fn(&Value) -> bool) -> (Value, Vec<Value>) {
    let mut skipped = Vec::new();
    loop {
        let frame = recv_json(ws).await;
        if pred(&frame) {
            return (frame, skipped);
        }
        skipped.push(frame);
    }
}

async fn set_mode(client: &mut Client, body: Value) -> Value {
    let res = client
        .send(Method::PUT, "/api/me/presence", Some(body))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

fn presence_of<'a>(user: &'a str) -> impl Fn(&Value) -> bool + 'a {
    move |f: &Value| f["op"] == "p" && f["u"] == user && f.get("st").is_some()
}

/// Subscribes and returns the server's snapshot.
async fn watch(ws: &mut Ws, server: Uuid) -> Value {
    send_json(ws, json!({ "op": "s", "s": server })).await;
    recv_until(ws, |f| f["op"] == "p" && f.get("snap").is_some())
        .await
        .0
}

fn listed(snap: &Value, user: &str) -> Option<String> {
    snap["snap"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["u"] == user)
        .map(|entry| entry["st"].as_str().unwrap().to_owned())
}

#[sqlx::test]
async fn others_see_the_mode_and_the_own_sockets_hear_it(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        ..
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let (addr, _state) = common::serve_ws(pool).await;
    let mut ada_ws = connect(addr, &owner).await;
    watch(&mut ada_ws, server).await;
    // Two of Bob's devices: both follow the account's mode.
    let mut bob_ws = connect(addr, &member).await;
    let mut bob_phone = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    watch(&mut bob_phone, server).await;
    recv_until(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "o").await;

    let saved = set_mode(&mut member, json!({ "mode": "dnd", "duration": "8h" })).await;
    assert_eq!(saved["mode"], "dnd");
    let until = saved["until"].as_str().expect("an end").to_owned();
    let (frame, _) = recv_until(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "d", "{frame}");
    for ws in [&mut bob_ws, &mut bob_phone] {
        let (frame, _) = recv_until(ws, |f| f["op"] == "pm").await;
        assert_eq!(frame, json!({ "op": "pm", "m": "dnd", "u": until }));
    }
    let res = member.send(Method::GET, "/api/me/presence", None).await;
    assert_eq!(res.body, saved);
    // Someone else's mode is not touched.
    let res = owner.send(Method::GET, "/api/me/presence", None).await;
    assert_eq!(res.body, json!({ "mode": "auto", "until": null }));

    // A socket going idle does not change what others see.
    send_json(&mut bob_ws, json!({ "op": "p", "st": "i" })).await;
    let mut late = connect(addr, &owner).await;
    assert_eq!(
        listed(&watch(&mut late, server).await, &bob).as_deref(),
        Some("d")
    );

    set_mode(&mut member, json!({ "mode": "idle", "duration": "1h" })).await;
    let (frame, _) = recv_until(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "i", "{frame}");
    // Away by hand stays away while used.
    send_json(&mut bob_ws, json!({ "op": "p", "st": "o" })).await;
    send_json(&mut bob_phone, json!({ "op": "p", "st": "o" })).await;

    let saved = set_mode(&mut member, json!({ "mode": "invisible" })).await;
    assert_eq!(saved, json!({ "mode": "invisible", "until": null }));
    let (frame, skipped) = recv_until(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "x", "{frame}");
    assert!(
        skipped.iter().all(|f| !presence_of(&bob)(f)),
        "away by hand never flipped back: {skipped:?}"
    );
    let mut late = connect(addr, &owner).await;
    assert_eq!(listed(&watch(&mut late, server).await, &bob), None);
    // One device leaving says nothing: Bob was already shown offline.
    bob_phone.close(None).await.ok();

    set_mode(&mut member, json!({ "mode": "auto" })).await;
    let (frame, _) = recv_until(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "o", "{frame}");
    // Bob's own socket heard every change, in order.
    let mut modes = Vec::new();
    while modes.last().map(String::as_str) != Some("auto") {
        let (frame, _) = recv_until(&mut bob_ws, |f| f["op"] == "pm").await;
        assert_eq!(frame.get("u").is_some(), frame["m"] == "idle", "{frame}");
        modes.push(frame["m"].as_str().unwrap().to_owned());
    }
    assert_eq!(modes, ["idle", "invisible", "auto"]);
}

#[sqlx::test]
async fn invisible_typing_reaches_nobody(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        channel,
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let _ = &mut owner;
    let (addr, _state) = common::serve_ws(pool).await;
    let mut ada_ws = connect(addr, &owner).await;
    send_json(&mut ada_ws, json!({ "op": "s", "s": server, "c": channel })).await;
    recv_until(&mut ada_ws, |f| {
        f["op"] == "ok" && f["c"] == channel.to_string()
    })
    .await;
    let mut bob_ws = connect(addr, &member).await;
    send_json(&mut bob_ws, json!({ "op": "s", "s": server, "c": channel })).await;
    recv_until(&mut bob_ws, |f| {
        f["op"] == "ok" && f["c"] == channel.to_string()
    })
    .await;

    set_mode(
        &mut member,
        json!({ "mode": "invisible", "duration": "1h" }),
    )
    .await;
    recv_until(&mut bob_ws, |f| f["op"] == "pm").await;
    send_json(
        &mut bob_ws,
        json!({ "op": "y", "s": server, "c": channel, "on": true }),
    )
    .await;
    // Frames of one socket are handled in order: once this subscribe is
    // answered, the typing frame was handled.
    send_json(&mut bob_ws, json!({ "op": "s", "s": server })).await;
    recv_until(&mut bob_ws, |f| f["op"] == "ok" && f.get("c").is_none()).await;

    set_mode(&mut member, json!({ "mode": "auto" })).await;
    recv_until(&mut bob_ws, |f| f["op"] == "pm" && f["m"] == "auto").await;
    send_json(
        &mut bob_ws,
        json!({ "op": "y", "s": server, "c": channel, "on": true }),
    )
    .await;
    let (frame, skipped) = recv_until(&mut ada_ws, |f| f["op"] == "y").await;
    assert_eq!(frame["u"], bob);
    // The typing above came first had it gone out; the frames before this
    // one are presence only.
    assert!(
        skipped.iter().all(|f| f["op"] != "y"),
        "typing while invisible went out: {skipped:?}"
    );
}

#[sqlx::test]
async fn a_mode_runs_out_and_is_announced(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        ..
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let bob_id: Uuid = bob.parse().unwrap();
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut ada_ws = connect(addr, &owner).await;
    watch(&mut ada_ws, server).await;
    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    recv_until(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "o").await;

    set_mode(&mut member, json!({ "mode": "dnd", "duration": "24h" })).await;
    recv_until(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "d").await;
    recv_until(&mut bob_ws, |f| f["op"] == "pm").await;

    // The end passes: the record reads automatic at once ...
    sqlx::query("UPDATE users SET presence_mode_until = now() - interval '1 second' WHERE id = $1")
        .bind(bob_id)
        .execute(&pool)
        .await
        .unwrap();
    let res = member.send(Method::GET, "/api/me/presence", None).await;
    assert_eq!(res.body, json!({ "mode": "auto", "until": null }));
    // ... and the sweep clears it and tells everyone.
    assert_eq!(gelabber_api::presence::sweep(&state).await.unwrap(), 1);
    assert_eq!(gelabber_api::presence::sweep(&state).await.unwrap(), 0);
    let (mode, until): (String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT presence_mode, presence_mode_until FROM users WHERE id = $1")
            .bind(bob_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((mode.as_str(), until), ("auto", None));
    let (frame, _) = recv_until(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "o", "{frame}");
    let (frame, _) = recv_until(&mut bob_ws, |f| f["op"] == "pm").await;
    assert_eq!(frame, json!({ "op": "pm", "m": "auto" }));
    let _ = &mut owner;
}

/// Redis lost the mode (a restart): the next socket of the account puts the
/// record's back.
#[sqlx::test]
async fn a_new_socket_restores_a_lost_mode(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        ..
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let (addr, _state) = common::serve_ws(pool).await;
    set_mode(&mut member, json!({ "mode": "dnd" })).await;
    let mut redis = redis::Client::open(common::redis_url())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: i64 = redis::cmd("DEL")
        .arg(format!("{REDIS_PREFIX}p:m:{bob}"))
        .query_async(&mut redis)
        .await
        .unwrap();

    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    let mut ada_ws = connect(addr, &owner).await;
    assert_eq!(
        listed(&watch(&mut ada_ws, server).await, &bob).as_deref(),
        Some("d")
    );
    let _ = &mut owner;
}

#[sqlx::test]
async fn a_bad_mode_or_duration_is_refused(pool: PgPool) {
    let World { mut member, .. } = setup(pool).await;
    for (body, field, code) in [
        (json!({}), "mode", "required"),
        (json!({ "mode": "busy" }), "mode", "invalid"),
        (
            json!({ "mode": "dnd", "duration": "2h" }),
            "duration",
            "invalid",
        ),
    ] {
        let res = member
            .send(Method::PUT, "/api/me/presence", Some(body.clone()))
            .await;
        assert_eq!(
            res.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}: {}",
            res.body
        );
        assert_eq!(res.body["fields"][field], code, "{body}: {}", res.body);
    }
    // Automatic never has an end, whatever duration came with it.
    let res = member
        .send(
            Method::PUT,
            "/api/me/presence",
            Some(json!({ "mode": "auto", "duration": "1h" })),
        )
        .await;
    assert_eq!(res.body, json!({ "mode": "auto", "until": null }));
}
