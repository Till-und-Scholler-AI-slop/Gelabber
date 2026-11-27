//! Manual presence (#590): Away, Do not disturb and Invisible as others see
//! them, the account's own sockets, typing while invisible, and the end of
//! a mode.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use gelabber_api::AppState;
use gelabber_api::gateway::protocol::REDIS_PREFIX;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::ws::{self, Ws, send, until_skipping};
use common::{Client, SESSION};

struct World {
    owner: Client,
    member: Client,
    server: Uuid,
    channel: Uuid,
}

/// The worker that ends run-out modes is off: the test of that end runs its
/// sweep by hand.
fn state(pool: PgPool) -> AppState {
    let mut state = common::state(pool);
    state.workers.presence_modes = None;
    state
}

/// Presence that lasts a minute without a heartbeat. The sockets a test
/// does not read do not answer heartbeats, and with the usual 2 s a busy
/// machine let such a member drop offline before an `@here` was resolved.
fn ws_state(pool: PgPool) -> AppState {
    common::ws_state_with(pool, &[("API_WS_PRESENCE_TTL_MS", "60000".into())])
}

async fn serve(pool: PgPool) -> std::net::SocketAddr {
    common::serve_ws_state(ws_state(pool)).await
}

async fn setup(pool: PgPool) -> World {
    let mut owner = Client::from_state(state(pool.clone()));
    owner.bootstrap().await;
    let res = owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let mut member = Client::from_state(state(pool));
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

/// A client that knows `d` (#590).
async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    connect_to(addr, client, "/ws?caps=dnd").await
}

async fn connect_to(addr: std::net::SocketAddr, client: &Client, path: &str) -> Ws {
    ws::connect_to(addr, path, Some(&client.jar[SESSION]), None).await
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
    send(ws, json!({ "op": "s", "s": server })).await;
    until_skipping(ws, |f| f["op"] == "p" && f.get("snap").is_some())
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
    let addr = serve(pool).await;
    let mut ada_ws = connect(addr, &owner).await;
    watch(&mut ada_ws, server).await;
    // Two of Bob's devices: both follow the account's mode.
    let mut bob_ws = connect(addr, &member).await;
    let mut bob_phone = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    watch(&mut bob_phone, server).await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "o").await;

    let saved = set_mode(&mut member, json!({ "mode": "dnd", "duration": "8h" })).await;
    assert_eq!(saved["mode"], "dnd");
    let until = saved["until"].as_str().expect("an end").to_owned();
    let (frame, _) = until_skipping(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "d", "{frame}");
    for ws in [&mut bob_ws, &mut bob_phone] {
        let (frame, _) = until_skipping(ws, |f| f["op"] == "pm").await;
        assert_eq!(frame, json!({ "op": "pm", "m": "dnd", "u": until }));
    }
    let res = member.send(Method::GET, "/api/me/presence", None).await;
    assert_eq!(res.body, saved);
    // Someone else's mode is not touched.
    let res = owner.send(Method::GET, "/api/me/presence", None).await;
    assert_eq!(res.body, json!({ "mode": "auto", "until": null }));

    // A socket going idle does not change what others see.
    send(&mut bob_ws, json!({ "op": "p", "st": "i" })).await;
    let mut late = connect(addr, &owner).await;
    assert_eq!(
        listed(&watch(&mut late, server).await, &bob).as_deref(),
        Some("d")
    );

    set_mode(&mut member, json!({ "mode": "idle", "duration": "1h" })).await;
    let (frame, _) = until_skipping(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "i", "{frame}");
    // Away by hand stays away while used.
    send(&mut bob_ws, json!({ "op": "p", "st": "o" })).await;
    send(&mut bob_phone, json!({ "op": "p", "st": "o" })).await;

    let saved = set_mode(&mut member, json!({ "mode": "invisible" })).await;
    assert_eq!(saved, json!({ "mode": "invisible", "until": null }));
    let (frame, skipped) = until_skipping(&mut ada_ws, presence_of(&bob)).await;
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
    let (frame, _) = until_skipping(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "o", "{frame}");
    // Bob's own socket heard every change, in order.
    let mut modes = Vec::new();
    while modes.last().map(String::as_str) != Some("auto") {
        let (frame, _) = until_skipping(&mut bob_ws, |f| f["op"] == "pm").await;
        assert_eq!(frame.get("u").is_some(), frame["m"] == "idle", "{frame}");
        modes.push(frame["m"].as_str().unwrap().to_owned());
    }
    assert_eq!(modes, ["idle", "invisible", "auto"]);
}

/// A client from before #590 never hears `d`: Do not disturb reaches it
/// as online, in snapshots and live.
#[sqlx::test]
async fn an_old_client_hears_do_not_disturb_as_online(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        ..
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let addr = serve(pool).await;
    let mut old = connect_to(addr, &owner, "/ws").await;
    watch(&mut old, server).await;
    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    until_skipping(&mut old, |f| presence_of(&bob)(f) && f["st"] == "o").await;

    // Each change reaches the old client, Do not disturb as online.
    for (mode, heard) in [("dnd", "o"), ("idle", "i"), ("dnd", "o")] {
        set_mode(&mut member, json!({ "mode": mode })).await;
        let (frame, _) = until_skipping(&mut old, presence_of(&bob)).await;
        assert_eq!(frame["st"], heard, "{mode}: {frame}");
    }
    let mut late = connect_to(addr, &owner, "/ws").await;
    assert_eq!(
        listed(&watch(&mut late, server).await, &bob).as_deref(),
        Some("o")
    );
    let mut new = connect(addr, &owner).await;
    assert_eq!(
        listed(&watch(&mut new, server).await, &bob).as_deref(),
        Some("d")
    );
    let _ = &mut owner;
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
    let addr = serve(pool).await;
    let mut ada_ws = connect(addr, &owner).await;
    send(&mut ada_ws, json!({ "op": "s", "s": server, "c": channel })).await;
    until_skipping(&mut ada_ws, |f| {
        f["op"] == "ok" && f["c"] == channel.to_string()
    })
    .await;
    let mut bob_ws = connect(addr, &member).await;
    send(&mut bob_ws, json!({ "op": "s", "s": server, "c": channel })).await;
    until_skipping(&mut bob_ws, |f| {
        f["op"] == "ok" && f["c"] == channel.to_string()
    })
    .await;

    set_mode(
        &mut member,
        json!({ "mode": "invisible", "duration": "1h" }),
    )
    .await;
    until_skipping(&mut bob_ws, |f| f["op"] == "pm").await;
    send(
        &mut bob_ws,
        json!({ "op": "y", "s": server, "c": channel, "on": true }),
    )
    .await;
    // Frames of one socket are handled in order: once this subscribe is
    // answered, the typing frame was handled.
    send(&mut bob_ws, json!({ "op": "s", "s": server })).await;
    until_skipping(&mut bob_ws, |f| f["op"] == "ok" && f.get("c").is_none()).await;

    set_mode(&mut member, json!({ "mode": "auto" })).await;
    until_skipping(&mut bob_ws, |f| f["op"] == "pm" && f["m"] == "auto").await;
    send(
        &mut bob_ws,
        json!({ "op": "y", "s": server, "c": channel, "on": true }),
    )
    .await;
    let (frame, skipped) = until_skipping(&mut ada_ws, |f| f["op"] == "y").await;
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
    let mut state = ws_state(pool.clone());
    state.workers.presence_modes = None;
    let addr = common::serve_ws_state(state.clone()).await;
    let mut ada_ws = connect(addr, &owner).await;
    watch(&mut ada_ws, server).await;
    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "o").await;

    set_mode(&mut member, json!({ "mode": "dnd", "duration": "24h" })).await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "d").await;
    until_skipping(&mut bob_ws, |f| f["op"] == "pm").await;

    // The end passes: the record reads automatic at once ...
    sqlx::query("UPDATE users SET presence_mode_until = now() - interval '1 second' WHERE id = $1")
        .bind(bob_id)
        .execute(&pool)
        .await
        .unwrap();
    let res = member.send(Method::GET, "/api/me/presence", None).await;
    assert_eq!(res.body, json!({ "mode": "auto", "until": null }));
    // The Redis copy ran out with it, as it does in production.
    del(&[format!("{REDIS_PREFIX}p:m:{bob}")]).await;
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
    let (frame, _) = until_skipping(&mut ada_ws, presence_of(&bob)).await;
    assert_eq!(frame["st"], "o", "{frame}");
    let (frame, _) = until_skipping(&mut bob_ws, |f| f["op"] == "pm").await;
    assert_eq!(frame, json!({ "op": "pm", "m": "auto" }));
    let _ = &mut owner;
}

async fn redis() -> redis::aio::MultiplexedConnection {
    redis::Client::open(common::redis_url())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

async fn del(keys: &[String]) {
    let _: i64 = redis::cmd("DEL")
        .arg(keys)
        .query_async(&mut redis().await)
        .await
        .unwrap();
}

/// The last socket of an invisible account closes: nobody hears it leave,
/// as nobody saw it there.
#[sqlx::test]
async fn an_invisible_account_leaves_without_a_word(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        ..
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let addr = serve(pool).await;
    let mut ada_ws = connect(addr, &owner).await;
    watch(&mut ada_ws, server).await;
    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "o").await;
    set_mode(&mut member, json!({ "mode": "invisible" })).await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "x").await;

    bob_ws.close(None).await.ok();
    drop(bob_ws);
    // Bob's status is gone once the server let the socket go.
    let key = format!("{REDIS_PREFIX}p:u:{bob}");
    let mut conn = redis().await;
    let start = std::time::Instant::now();
    loop {
        let exists: i64 = redis::cmd("EXISTS")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .unwrap();
        if exists == 0 {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "socket not dropped"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Ada's own change comes after anything Bob's leaving sent.
    set_mode(&mut owner, json!({ "mode": "dnd" })).await;
    let (_, skipped) = until_skipping(&mut ada_ws, |f| f["op"] == "pm").await;
    assert!(
        skipped.iter().all(|f| !presence_of(&bob)(f)),
        "Bob's leaving was announced: {skipped:?}"
    );
}

/// Redis restarted under open sockets: they only subscribe again
/// (`resync`), and the record's mode is back before they announce.
#[sqlx::test]
async fn a_subscribe_after_a_redis_restart_restores_the_mode(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        ..
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let addr = serve(pool).await;
    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    set_mode(&mut member, json!({ "mode": "dnd" })).await;
    until_skipping(&mut bob_ws, |f| f["op"] == "pm").await;
    // What a restart loses: the copy and its check marker.
    del(&[
        format!("{REDIS_PREFIX}p:m:{bob}"),
        format!("{REDIS_PREFIX}p:mc:{bob}"),
    ])
    .await;

    watch(&mut bob_ws, server).await;
    let mut ada_ws = connect(addr, &owner).await;
    assert_eq!(
        listed(&watch(&mut ada_ws, server).await, &bob).as_deref(),
        Some("d")
    );
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
    let addr = serve(pool).await;
    set_mode(&mut member, json!({ "mode": "dnd" })).await;
    del(&[
        format!("{REDIS_PREFIX}p:m:{bob}"),
        format!("{REDIS_PREFIX}p:mc:{bob}"),
    ])
    .await;

    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    let mut ada_ws = connect(addr, &owner).await;
    assert_eq!(
        listed(&watch(&mut ada_ws, server).await, &bob).as_deref(),
        Some("d")
    );
    let _ = &mut owner;
}

/// Two sockets of an account subscribe again at once after a restart: the
/// one that finds another's check still under way checks too, rather than
/// announcing itself before the copy is back.
#[sqlx::test]
async fn a_socket_does_not_skip_a_check_still_under_way(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        ..
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let addr = serve(pool).await;
    set_mode(&mut member, json!({ "mode": "dnd" })).await;
    del(&[format!("{REDIS_PREFIX}p:m:{bob}")]).await;
    // Another socket claimed the check and has not written the copy yet.
    let mut conn = redis().await;
    redis::cmd("SET")
        .arg(format!("{REDIS_PREFIX}p:mc:{bob}"))
        .arg("pending")
        .arg("PX")
        .arg(30_000)
        .query_async::<()>(&mut conn)
        .await
        .unwrap();

    let mut bob_ws = connect(addr, &member).await;
    watch(&mut bob_ws, server).await;
    let mut ada_ws = connect(addr, &owner).await;
    assert_eq!(
        listed(&watch(&mut ada_ws, server).await, &bob).as_deref(),
        Some("d")
    );
    let marker: String = redis::cmd("GET")
        .arg(format!("{REDIS_PREFIX}p:mc:{bob}"))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(marker, "done");
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

/// A third member, Cem, on the world's server.
async fn join(pool: &PgPool, owner: &mut Client, server: Uuid) -> Client {
    let mut cem = Client::new(pool.clone());
    cem.bootstrap().await;
    let res = cem.register("cem@example.com", "password123", "Cem").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    let code = res.body["code"].as_str().unwrap().to_owned();
    let res = cem
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    cem
}

async fn follow(ws: &mut Ws, server: Uuid, channel: Uuid) {
    send(ws, json!({ "op": "s", "s": server, "c": channel })).await;
    until_skipping(ws, |f| f["op"] == "ok" && f["c"] == channel.to_string()).await;
}

async fn post(client: &mut Client, channel: Uuid, content: &str) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": content })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

async fn edit(client: &mut Client, message: &str, content: &str) -> Value {
    let res = client
        .send(
            Method::PATCH,
            &format!("/api/messages/{message}"),
            Some(json!({ "content": content })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

/// The mentions of message `id` as `client` reads them in the history.
async fn listed_mentions(client: &mut Client, channel: Uuid, id: &str) -> Value {
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
        .expect("message listed")["mentions"]
        .clone()
}

/// The delta of the `t` event of message `id` as this socket hears it.
async fn heard(ws: &mut Ws, t: &str, id: &str) -> Value {
    until_skipping(ws, |f| f["op"] == "e" && f["t"] == t && f["i"] == id)
        .await
        .0["d"]
        .clone()
}

/// `@here` reaches an invisible member (GEL-43), but only they learn that
/// it did: what others read of the message, live, in the history, on an
/// edit or in a replay, names only the members shown online.
#[sqlx::test]
async fn here_reaches_an_invisible_member_without_showing_them(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        channel,
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let mut cem = join(&pool, &mut owner, server).await;
    let cem_id = cem.user_id().to_owned();
    let addr = serve(pool).await;
    let mut ada_ws = connect(addr, &owner).await;
    follow(&mut ada_ws, server, channel).await;
    let mut bob_ws = connect(addr, &member).await;
    follow(&mut bob_ws, server, channel).await;
    let mut cem_ws = connect(addr, &cem).await;
    follow(&mut cem_ws, server, channel).await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "o").await;
    until_skipping(&mut ada_ws, |f| presence_of(&cem_id)(f) && f["st"] == "o").await;
    set_mode(&mut member, json!({ "mode": "invisible" })).await;
    until_skipping(&mut bob_ws, |f| f["op"] == "pm").await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "x").await;

    let shown = json!([cem_id]);
    let mut own = vec![bob.clone(), cem_id.clone()];
    own.sort();
    let own = json!(own);
    let sent = post(&mut owner, channel, "@here kurz?").await;
    assert_eq!(sent["mentions"], shown);
    let id = sent["id"].as_str().unwrap().to_owned();
    for (ws, expected) in [
        (&mut ada_ws, &shown),
        (&mut cem_ws, &shown),
        (&mut bob_ws, &own),
    ] {
        let d = heard(ws, "c", &id).await;
        assert_eq!(&d["mentions"], expected, "{d}");
        assert!(d.get("hidden_mentions").is_none(), "{d}");
    }
    assert_eq!(listed_mentions(&mut owner, channel, &id).await, shown);
    assert_eq!(listed_mentions(&mut cem, channel, &id).await, shown);
    assert_eq!(listed_mentions(&mut member, channel, &id).await, own);
    let res = member.send(Method::GET, "/api/messages/unread", None).await;
    let row = res
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["channel_id"] == channel.to_string())
        .unwrap()
        .clone();
    assert_eq!(row["mention_count"], 1, "{row}");
    let res = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/search?q=kurz"),
            None,
        )
        .await;
    assert_eq!(res.body["messages"][0]["mentions"], shown, "{}", res.body);
    let res = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/{id}/context"),
            None,
        )
        .await;
    assert_eq!(res.body["messages"][0]["mentions"], shown, "{}", res.body);

    // An edit keeping `@here` keeps whom it reached, Bob still unseen.
    let edited = edit(&mut owner, &id, "@here kurz? (Tippfehler)").await;
    assert_eq!(edited["mentions"], shown);
    for (ws, expected) in [(&mut ada_ws, &shown), (&mut bob_ws, &own)] {
        let d = heard(ws, "e", &id).await;
        assert_eq!(&d["mentions"], expected, "{d}");
        assert!(d.get("hidden_mentions").is_none(), "{d}");
    }
    // Bob's reaction sends the message out again, as everyone reads it,
    // and his answer still shows him his mention.
    let res = member
        .send(
            Method::PUT,
            &format!("/api/messages/{id}/reactions/%F0%9F%91%8D"),
            None,
        )
        .await;
    assert!(res.status.is_success(), "{}", res.body);
    let d = heard(&mut ada_ws, "e", &id).await;
    assert_eq!(d["mentions"], shown, "{d}");

    // A socket that resumes from the start replays the same.
    let mut late = connect(addr, &owner).await;
    send(
        &mut late,
        json!({ "op": "s", "s": server, "c": channel, "n": 0 }),
    )
    .await;
    let (_, replayed) = until_skipping(&mut late, |f| f["op"] == "ok").await;
    let events: Vec<&Value> = replayed
        .iter()
        .filter(|f| f["op"] == "e" && f["i"] == id.as_str())
        .collect();
    assert!(!events.is_empty(), "nothing replayed: {replayed:?}");
    for event in events {
        assert_eq!(event["d"]["mentions"], shown, "{event}");
        assert!(event["d"].get("hidden_mentions").is_none(), "{event}");
    }
}

/// A member named in the text is shown as mentioned, invisible or not; an
/// invisible one counts as offline, reached by the name and not by `@here`.
/// So an edit that drops the name but keeps `@here` drops the mention, as
/// for an offline member, and shows nobody that `@here` had reached them
/// (GEL-43).
#[sqlx::test]
async fn an_invisible_member_is_shown_only_while_named(pool: PgPool) {
    let World {
        mut owner,
        mut member,
        server,
        channel,
    } = setup(pool.clone()).await;
    let bob = member.user_id().to_owned();
    let addr = serve(pool).await;
    let mut ada_ws = connect(addr, &owner).await;
    follow(&mut ada_ws, server, channel).await;
    let mut bob_ws = connect(addr, &member).await;
    follow(&mut bob_ws, server, channel).await;
    until_skipping(&mut ada_ws, |f| presence_of(&bob)(f) && f["st"] == "o").await;
    set_mode(&mut member, json!({ "mode": "invisible" })).await;
    until_skipping(&mut bob_ws, |f| f["op"] == "pm").await;

    let named = format!("@here <@{bob}> kurz?");
    let sent = post(&mut owner, channel, &named).await;
    assert_eq!(sent["mentions"], json!([bob]));
    let id = sent["id"].as_str().unwrap().to_owned();
    let edited = edit(&mut owner, &id, "@here kurz?").await;
    assert_eq!(edited["mentions"], json!([]));
    assert_eq!(heard(&mut ada_ws, "e", &id).await["mentions"], json!([]));
    assert_eq!(heard(&mut bob_ws, "e", &id).await["mentions"], json!([]));
    assert_eq!(listed_mentions(&mut owner, channel, &id).await, json!([]));
    assert_eq!(listed_mentions(&mut member, channel, &id).await, json!([]));
    // Named again, Bob is shown again.
    let edited = edit(&mut owner, &id, &named).await;
    assert_eq!(edited["mentions"], json!([bob]));
}
