//! Channels limited to roles (v0.10, issue #516): `role_ids` on channel
//! create and PATCH, the filtered server detail, and the 404 every lookup
//! gives a member who cannot see the channel.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::{AppState, EventKind, publish_channel};
use gelabber_shared::ticket::member_authority_key;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

use common::{Client, SESSION};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn register(state: &AppState, name: &str) -> Client {
    let mut client = Client::with_state(state.clone());
    client.bootstrap().await;
    let res = client
        .register(&format!("{name}@example.test"), "password123", name)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

fn id(value: &Value) -> Uuid {
    value.as_str().unwrap().parse().unwrap()
}

/// The owner Ada with Bob (no roles) and Cara (who gets the role) as
/// members. Returns the server id and its first text channel.
struct Team {
    ada: Client,
    bob: Client,
    cara: Client,
    sid: Uuid,
    text: Uuid,
}

async fn team(state: &AppState) -> Team {
    let mut ada = register(state, "ada").await;
    let mut bob = register(state, "bob").await;
    let mut cara = register(state, "cara").await;
    let res = ada
        .send(Method::POST, "/api/servers", Some(json!({"name": "Team"})))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let sid = id(&res.body["id"]);
    let text = id(&res.body["channels"][0]["id"]);
    let invite = ada
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let code = invite.body["code"].as_str().unwrap().to_owned();
    for client in [&mut bob, &mut cara] {
        let res = client
            .send(Method::POST, &format!("/api/invites/{code}/join"), None)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }
    Team {
        ada,
        bob,
        cara,
        sid,
        text,
    }
}

async fn role(owner: &mut Client, sid: Uuid, name: &str) -> Uuid {
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/roles"),
            Some(json!({"name": name, "color": "green"})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    id(&res.body["id"])
}

async fn set_roles(owner: &mut Client, sid: Uuid, user: &str, roles: Value) {
    let res = owner
        .send(
            Method::PUT,
            &format!("/api/servers/{sid}/members/{user}/roles"),
            Some(json!({"role_ids": roles})),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

async fn patch_access(client: &mut Client, channel: Uuid, roles: Value) -> common::Response {
    client
        .send(
            Method::PATCH,
            &format!("/api/channels/{channel}"),
            Some(json!({"role_ids": roles})),
        )
        .await
}

async fn create_channel(client: &mut Client, sid: Uuid, body: Value) -> common::Response {
    client
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/channels"),
            Some(body),
        )
        .await
}

async fn detail(client: &mut Client, sid: Uuid) -> Value {
    let res = client
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

fn has(list: &Value, wanted: Uuid) -> bool {
    list.as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == wanted.to_string())
}

#[sqlx::test]
async fn role_ids_limit_a_channel_on_create_and_patch(pool: PgPool) {
    let state = common::state(pool.clone());
    let Team {
        mut ada,
        mut bob,
        sid,
        ..
    } = team(&state).await;
    let r = role(&mut ada, sid, "Moderation").await;
    let s = role(&mut ada, sid, "Stammgäste").await;
    let mut wanted = vec![r.to_string(), s.to_string()];
    wanted.sort();

    let res = create_channel(
        &mut ada,
        sid,
        json!({"name": "spiele", "kind": "text", "role_ids": [s, r, r]}),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["limited"], true);
    assert_eq!(
        res.body["role_ids"],
        json!(wanted),
        "sorted, duplicates gone"
    );
    let limited = id(&res.body["id"]);

    for body in [
        json!({"name": "offen", "kind": "text"}),
        json!({"name": "auch-offen", "kind": "voice", "role_ids": []}),
    ] {
        let res = create_channel(&mut ada, sid, body).await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        assert_eq!(res.body["limited"], false);
        assert_eq!(res.body["role_ids"], json!([]));
    }

    // A name alone leaves access as it is.
    let res = ada
        .send(
            Method::PATCH,
            &format!("/api/channels/{limited}"),
            Some(json!({"name": "brettspiele"})),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["limited"], true);
    assert_eq!(res.body["role_ids"], json!(wanted));

    let res = patch_access(&mut ada, limited, json!([r])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "brettspiele");
    assert_eq!(res.body["limited"], true);
    assert_eq!(res.body["role_ids"], json!([r]));

    let res = patch_access(&mut ada, limited, json!([])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["limited"], false);
    assert_eq!(res.body["role_ids"], json!([]));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_roles WHERE channel_id = $1")
        .bind(limited)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    // Unknown, malformed and foreign role ids.
    let other = ada
        .send(Method::POST, "/api/servers", Some(json!({"name": "Other"})))
        .await;
    let foreign = role(&mut ada, id(&other.body["id"]), "Fremd").await;
    for bad in [
        json!([Uuid::new_v4()]),
        json!(["nope"]),
        json!([r, foreign]),
    ] {
        let res = patch_access(&mut ada, limited, bad.clone()).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(res.body["fields"]["role_ids"], "invalid", "{}", res.body);
        let res = create_channel(
            &mut ada,
            sid,
            json!({"name": "neu", "kind": "text", "role_ids": bad}),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(res.body["fields"]["role_ids"], "invalid", "{}", res.body);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channels WHERE name = 'neu'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "a rejected create leaves no channel");

    // manage_channels, as for the other channel fields.
    let res = patch_access(&mut bob, limited, json!([r])).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    let res = create_channel(
        &mut bob,
        sid,
        json!({"name": "neu", "kind": "text", "role_ids": [r]}),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
}

async fn category(owner: &mut Client, sid: Uuid, name: &str) -> Uuid {
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/categories"),
            Some(json!({"name": name})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    id(&res.body["id"])
}

async fn channel_in(
    owner: &mut Client,
    sid: Uuid,
    name: &str,
    category: Uuid,
    roles: Value,
) -> Uuid {
    let res = create_channel(
        owner,
        sid,
        json!({"name": name, "kind": "text", "category_id": category, "role_ids": roles}),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    id(&res.body["id"])
}

#[sqlx::test]
async fn the_detail_leaves_out_hidden_channels_and_their_categories(pool: PgPool) {
    let state = common::state(pool.clone());
    let Team {
        mut ada,
        mut bob,
        mut cara,
        sid,
        text,
    } = team(&state).await;
    let r = role(&mut ada, sid, "Stammgäste").await;
    set_roles(&mut ada, sid, cara.user_id(), json!([r])).await;
    let secret = category(&mut ada, sid, "Geheim").await;
    let mixed = category(&mut ada, sid, "Gemischt").await;
    let empty = category(&mut ada, sid, "Leer").await;
    let hidden = channel_in(&mut ada, sid, "geheim", secret, json!([r])).await;
    let hidden_mixed = channel_in(&mut ada, sid, "intern", mixed, json!([r])).await;
    let open_mixed = channel_in(&mut ada, sid, "offen", mixed, json!([])).await;

    let seen = detail(&mut bob, sid).await;
    assert!(!has(&seen["channels"], hidden));
    assert!(!has(&seen["channels"], hidden_mixed));
    assert!(has(&seen["channels"], open_mixed));
    assert!(has(&seen["channels"], text));
    assert!(!has(&seen["categories"], secret), "all its channels hidden");
    assert!(has(&seen["categories"], mixed), "one channel still visible");
    assert!(has(&seen["categories"], empty), "empty categories stay");

    for client in [&mut ada, &mut cara] {
        let seen = detail(client, sid).await;
        for channel in [hidden, hidden_mixed, open_mixed, text] {
            assert!(has(&seen["channels"], channel));
        }
        for category in [secret, mixed, empty] {
            assert!(has(&seen["categories"], category));
        }
    }

    // Deleting the role leaves the channel to the owner alone.
    let res = ada
        .send(Method::DELETE, &format!("/api/roles/{r}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let seen = detail(&mut cara, sid).await;
    assert!(!has(&seen["channels"], hidden));
    assert!(!has(&seen["categories"], secret));
    let seen = detail(&mut ada, sid).await;
    let kept = seen["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["id"] == hidden.to_string())
        .expect("the owner still sees it");
    assert_eq!(kept["limited"], true);
    assert_eq!(kept["role_ids"], json!([]));
}

#[sqlx::test]
async fn a_hidden_channel_is_not_found_everywhere(pool: PgPool) {
    let state = common::state(pool.clone());
    let Team {
        mut ada,
        mut bob,
        sid,
        text,
        ..
    } = team(&state).await;
    // Bob may manage channels and messages, but that does not reveal them.
    let res = ada
        .send(
            Method::PATCH,
            &format!("/api/servers/{sid}"),
            Some(json!({"member_permissions": [
                "send_messages", "send_files", "join_voice", "go_live",
                "manage_channels", "manage_messages",
            ]})),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = ada
        .send(
            Method::POST,
            &format!("/api/channels/{text}/messages"),
            Some(json!({"content": "geheim"})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let message = id(&res.body["id"]);
    let res = ada
        .send(
            Method::POST,
            &format!("/api/channels/{text}/attachments"),
            Some(json!({"filename": "a.png", "content_type": "image/png", "size": 16})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let attachment = id(&res.body["id"]);
    sqlx::query("UPDATE attachments SET message_id = $2 WHERE id = $1")
        .bind(attachment)
        .bind(message)
        .execute(&pool)
        .await
        .unwrap();
    let voice = create_channel(&mut ada, sid, json!({"name": "Raum", "kind": "voice"})).await;
    let voice = id(&voice.body["id"]);

    let r = role(&mut ada, sid, "Stammgäste").await;
    for channel in [text, voice] {
        let res = patch_access(&mut ada, channel, json!([r])).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }

    let requests: Vec<(Method, String, Option<Value>)> = vec![
        (Method::GET, format!("/api/channels/{text}/messages"), None),
        (
            Method::POST,
            format!("/api/channels/{text}/messages"),
            Some(json!({"content": "hallo"})),
        ),
        (
            Method::PATCH,
            format!("/api/messages/{message}"),
            Some(json!({"content": "anders"})),
        ),
        (Method::DELETE, format!("/api/messages/{message}"), None),
        (
            Method::PUT,
            format!("/api/messages/{message}/reactions/%F0%9F%98%80"),
            None,
        ),
        (
            Method::POST,
            format!("/api/channels/{text}/attachments"),
            Some(json!({"filename": "b.png", "content_type": "image/png", "size": 16})),
        ),
        (Method::GET, format!("/api/attachments/{attachment}"), None),
        (
            Method::GET,
            format!("/api/channels/{text}/messages/search?q=geheim"),
            None,
        ),
        (
            Method::GET,
            format!("/api/channels/{text}/messages/{message}/context"),
            None,
        ),
        (
            Method::PUT,
            format!("/api/channels/{text}/read"),
            Some(json!({})),
        ),
        (
            Method::PUT,
            format!("/api/channels/{text}/notification-settings"),
            Some(json!({"level": "none"})),
        ),
        (
            Method::PATCH,
            format!("/api/channels/{text}"),
            Some(json!({"name": "offen"})),
        ),
        (
            Method::PATCH,
            format!("/api/channels/{text}"),
            Some(json!({"role_ids": []})),
        ),
        (Method::DELETE, format!("/api/channels/{voice}"), None),
        (
            Method::POST,
            format!("/api/channels/{voice}/media-ticket"),
            None,
        ),
    ];
    for (method, path, body) in &requests {
        let res = bob.send(method.clone(), path, body.clone()).await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            res.body
        );
        assert_eq!(res.body["error"], "not_found", "{method} {path}");
    }

    // The owner still reads it; a member given the role does at once.
    let res = ada
        .send(Method::GET, &format!("/api/channels/{text}/messages"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    set_roles(&mut ada, sid, bob.user_id(), json!([r])).await;
    let res = bob
        .send(Method::GET, &format!("/api/channels/{text}/messages"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = bob
        .send(
            Method::POST,
            &format!("/api/channels/{voice}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test]
async fn mentions_reach_only_members_who_see_the_channel(pool: PgPool) {
    let state = common::state(pool.clone());
    let Team {
        mut ada,
        bob,
        cara,
        sid,
        text,
    } = team(&state).await;
    let r = role(&mut ada, sid, "Stammgäste").await;
    set_roles(&mut ada, sid, cara.user_id(), json!([r])).await;
    let res = patch_access(&mut ada, text, json!([r])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let (b, c) = (bob.user_id().to_owned(), cara.user_id().to_owned());
    let res = ada
        .send(
            Method::POST,
            &format!("/api/channels/{text}/messages"),
            Some(json!({"content": format!("<@{b}> <@{c}>")})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["mentions"], json!([c]));
}

/// An upload started before the channel was limited cannot finish, and its
/// pending file cannot be fetched.
#[sqlx::test]
async fn a_pending_upload_stops_once_the_channel_is_limited(pool: PgPool) {
    let state = common::state(pool.clone());
    let Team {
        mut ada,
        mut bob,
        sid,
        text,
        ..
    } = team(&state).await;
    let png = b"\x89PNG\r\n\x1a\n0123456789abcdef".to_vec();
    let signed = bob
        .send(
            Method::POST,
            &format!("/api/channels/{text}/attachments"),
            Some(json!({"filename": "a.png", "content_type": "image/png", "size": png.len()})),
        )
        .await;
    assert_eq!(signed.status, StatusCode::CREATED, "{}", signed.body);
    let attachment = id(&signed.body["id"]);

    let r = role(&mut ada, sid, "Stammgäste").await;
    let res = patch_access(&mut ada, text, json!([r])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let (status, body) = bob.put_upload(&signed.body, png, |_| {}).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let res = bob
        .send(Method::GET, &format!("/api/attachments/{attachment}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
}

/// `@here` reaches only the online members who see the channel.
#[sqlx::test]
async fn here_reaches_only_members_who_see_the_channel(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let Team {
        mut ada,
        bob,
        cara,
        sid,
        text,
    } = team(&state).await;
    let r = role(&mut ada, sid, "Stammgäste").await;
    set_roles(&mut ada, sid, cara.user_id(), json!([r])).await;
    let res = patch_access(&mut ada, text, json!([r])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Bob and Cara are both online.
    let mut watch = connect(addr, &ada).await;
    send(&mut watch, json!({"op": "s", "s": sid})).await;
    until(&mut watch, |f| f["op"] == "ok").await;
    let mut online = Vec::new();
    for client in [&bob, &cara] {
        let mut ws = connect(addr, client).await;
        send(&mut ws, json!({"op": "s", "s": sid})).await;
        until(&mut ws, |f| f["op"] == "ok").await;
        let user = client.user_id().to_owned();
        until(&mut watch, |f| {
            f["op"] == "p" && f["u"] == user.as_str() && f["st"] == "o"
        })
        .await;
        online.push(ws);
    }

    let res = ada
        .send(
            Method::POST,
            &format!("/api/channels/{text}/messages"),
            Some(json!({"content": "@here Essen"})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["mentions"], json!([cara.user_id()]));
}

async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    req.headers_mut().insert(
        "cookie",
        format!("{SESSION}={}", client.jar[SESSION])
            .parse()
            .unwrap(),
    );
    tokio_tungstenite::connect_async(req)
        .await
        .expect("connect")
        .0
}

async fn send(ws: &mut Ws, frame: Value) {
    ws.send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
}

async fn until(ws: &mut Ws, pred: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await.expect("open socket").expect("frame") {
                Message::Text(raw) => {
                    let frame: Value = serde_json::from_str(&raw).unwrap();
                    if frame["op"] == "h" {
                        send(ws, json!({"op": "h"})).await;
                    }
                    if pred(&frame) {
                        return frame;
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    })
    .await
    .expect("matching frame")
}

/// No `e` frame for `channel` arrives within a second.
async fn quiet(ws: &mut Ws, channel: Uuid) {
    let got = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(Ok(Message::Text(raw))) = ws.next().await {
                let frame: Value = serde_json::from_str(&raw).unwrap();
                if frame["op"] == "h" {
                    send(ws, json!({"op": "h"})).await;
                }
                if frame["op"] == "e" && frame["c"] == channel.to_string() {
                    return frame;
                }
            }
        }
    })
    .await;
    assert!(got.is_err(), "event in a hidden channel: {got:?}");
}

async fn subscribe(ws: &mut Ws, sid: Uuid, cid: Uuid) {
    send(ws, json!({"op": "s", "s": sid, "c": cid})).await;
    until(ws, |f| f["op"] == "ok" && f["c"] == cid.to_string()).await;
}

async fn not_found(ws: &mut Ws, frame: Value, cid: Uuid) {
    send(ws, frame).await;
    let err = until(ws, |f| f["op"] == "err").await;
    assert_eq!(err["e"], "not_found", "{err}");
    assert_eq!(err["c"], cid.to_string(), "{err}");
}

async fn redis_get(key: String) -> Option<String> {
    let mut redis = redis::Client::open(common::redis_url())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    redis::cmd("GET")
        .arg(key)
        .query_async(&mut redis)
        .await
        .unwrap()
}

#[sqlx::test]
async fn the_socket_loses_a_channel_once_it_is_limited(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let Team {
        mut ada,
        mut bob,
        sid,
        text,
        ..
    } = team(&state).await;
    let voice = create_channel(&mut ada, sid, json!({"name": "Raum", "kind": "voice"})).await;
    let voice = id(&voice.body["id"]);
    let r = role(&mut ada, sid, "Stammgäste").await;

    let mut ws = connect(addr, &bob).await;
    subscribe(&mut ws, sid, text).await;
    send(
        &mut ws,
        json!({"op": "sig", "t": "j", "s": sid, "c": voice}),
    )
    .await;
    until(&mut ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    let ticket = bob
        .send(
            Method::POST,
            &format!("/api/channels/{voice}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(ticket.status, StatusCode::OK, "{}", ticket.body);
    let key = member_authority_key(sid, bob.user_id().parse().unwrap());
    let generation = redis_get(key.clone()).await;
    assert!(generation.is_some());

    // Limiting the voice room he sits in cuts the seat and the authority.
    let res = patch_access(&mut ada, voice, json!([r])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(state.gateway.voice_snapshot(sid).await.unwrap().is_empty());
    assert_ne!(redis_get(key).await, generation);

    // Limiting the text channel stops its events.
    let res = patch_access(&mut ada, text, json!([r])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    publish_channel(&state, sid, text, EventKind::C, Some(Uuid::new_v4()), None)
        .await
        .unwrap();
    quiet(&mut ws, text).await;

    not_found(&mut ws, json!({"op": "s", "s": sid, "c": text}), text).await;
    not_found(
        &mut ws,
        json!({"op": "y", "s": sid, "c": text, "on": true}),
        text,
    )
    .await;
    not_found(
        &mut ws,
        json!({"op": "sig", "t": "j", "s": sid, "c": voice}),
        voice,
    )
    .await;

    // Given the role, the same socket gets the channel back.
    set_roles(&mut ada, sid, bob.user_id(), json!([r])).await;
    subscribe(&mut ws, sid, text).await;
    publish_channel(&state, sid, text, EventKind::C, Some(Uuid::new_v4()), None)
        .await
        .unwrap();
    until(&mut ws, |f| f["op"] == "e" && f["c"] == text.to_string()).await;
}

#[sqlx::test]
async fn unread_and_read_all_skip_a_hidden_channel(pool: PgPool) {
    let state = common::state(pool.clone());
    let Team {
        mut ada,
        mut bob,
        mut cara,
        sid,
        text,
    } = team(&state).await;
    let res = ada
        .send(
            Method::POST,
            &format!("/api/channels/{text}/messages"),
            Some(json!({"content": "@everyone Essen"})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let unread_row = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row["channel_id"] == text.to_string())
            .cloned()
    };
    let rows = bob.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(rows.status, StatusCode::OK, "{}", rows.body);
    assert_eq!(unread_row(&rows.body).unwrap()["mention_count"], 1);

    let r = role(&mut ada, sid, "Stammgäste").await;
    set_roles(&mut ada, sid, cara.user_id(), json!([r])).await;
    let res = patch_access(&mut ada, text, json!([r])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Bob no longer counts it, not even the @everyone; Cara still does.
    let rows = bob.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(rows.status, StatusCode::OK, "{}", rows.body);
    assert!(unread_row(&rows.body).is_none(), "{}", rows.body);
    let rows = cara.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(unread_row(&rows.body).unwrap()["mention_count"], 1);

    // Reading everything writes no position for the hidden channel.
    for (path, body) in [
        ("/api/messages/read-all".to_owned(), Some(json!({}))),
        (format!("/api/servers/{sid}/read"), None),
    ] {
        let res = bob.send(Method::PUT, &path, body).await;
        assert_eq!(res.status, StatusCode::OK, "{path}: {}", res.body);
        assert_eq!(res.body, json!([]), "{path}");
    }
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM channel_read_state WHERE channel_id = $1 AND user_id = $2",
    )
    .bind(text)
    .bind(bob.user_id().parse::<Uuid>().unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
}

fn seated(frame: &Value, user: &str) -> bool {
    frame["snap"]
        .as_array()
        .unwrap()
        .iter()
        .any(|seat| seat["u"] == user)
}

async fn join_voice(addr: std::net::SocketAddr, client: &Client, sid: Uuid, room: Uuid) -> Ws {
    let mut ws = connect(addr, client).await;
    send(&mut ws, json!({"op": "sig", "t": "j", "s": sid, "c": room})).await;
    until(&mut ws, |f| {
        f["op"] == "sig" && f["t"] == "j" && f["u"] == client.user_id()
    })
    .await;
    ws
}

#[sqlx::test]
async fn the_voice_roster_leaves_out_hidden_rooms(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let Team {
        mut ada,
        bob,
        cara,
        sid,
        ..
    } = team(&state).await;
    let r = role(&mut ada, sid, "Stammgäste").await;
    set_roles(&mut ada, sid, cara.user_id(), json!([r])).await;
    let hidden = create_channel(
        &mut ada,
        sid,
        json!({"name": "Hinterzimmer", "kind": "voice", "role_ids": [r]}),
    )
    .await;
    assert_eq!(hidden.status, StatusCode::CREATED, "{}", hidden.body);
    let hidden = id(&hidden.body["id"]);
    let lobby = create_channel(&mut ada, sid, json!({"name": "Lobby", "kind": "voice"})).await;
    let lobby = id(&lobby.body["id"]);
    let _cara_seat = join_voice(addr, &cara, sid, hidden).await;
    let _ada_seat = join_voice(addr, &ada, sid, lobby).await;

    // The subscribe snapshot shows the open room, not the hidden one.
    let mut ws = connect(addr, &bob).await;
    send(&mut ws, json!({"op": "s", "s": sid})).await;
    let snap = until(&mut ws, |f| f["op"] == "sig" && f["t"] == "r").await;
    assert!(seated(&snap, ada.user_id()), "{snap}");
    assert!(!seated(&snap, cara.user_id()), "{snap}");

    // Neither the roster passes nor Cara's signaling tell Bob about her.
    let cara_id = cara.user_id().to_owned();
    let leaked = tokio::time::timeout(Duration::from_millis(2500), async {
        loop {
            state.gateway.refresh_rosters().await;
            let frame = until(&mut ws, |f| f["op"] == "sig").await;
            if frame["u"] == cara_id.as_str() || (frame["t"] == "r" && seated(&frame, &cara_id)) {
                return frame;
            }
        }
    })
    .await;
    // A quiet socket also ends here: the outer timeout comes before `until`'s.
    assert!(leaked.is_err(), "hidden seat leaked: {leaked:?}");

    // Given the role, Bob's next roster shows Cara without a reconnect.
    set_roles(&mut ada, sid, bob.user_id(), json!([r])).await;
    let roster = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            state.gateway.refresh_rosters().await;
            let got = tokio::time::timeout(
                Duration::from_millis(300),
                until(&mut ws, |f| f["op"] == "sig" && f["t"] == "r"),
            )
            .await;
            if let Ok(frame) = got
                && seated(&frame, &cara_id)
            {
                return frame;
            }
        }
    })
    .await
    .expect("a roster with the room");
    assert!(seated(&roster, ada.user_id()), "{roster}");
}

/// The `g` claim of a media ticket for `room`, read back from Redis.
async fn ticket_go_live(client: &mut Client, room: Uuid) -> bool {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{room}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let ticket = res.body["ticket"].as_str().unwrap();
    let raw = redis_get(gelabber_shared::ticket::redis_key(ticket))
        .await
        .expect("ticket in Redis");
    let claim: Value = serde_json::from_str(&raw).unwrap();
    claim["g"].as_bool().unwrap_or(false)
}

#[sqlx::test]
async fn go_live_through_a_role_reaches_the_ticket_and_the_gateway(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let Team {
        mut ada,
        mut bob,
        sid,
        ..
    } = team(&state).await;
    let res = ada
        .send(
            Method::PATCH,
            &format!("/api/servers/{sid}"),
            Some(json!({"member_permissions": ["join_voice", "send_messages"]})),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let room = create_channel(&mut ada, sid, json!({"name": "Raum", "kind": "voice"})).await;
    let room = id(&room.body["id"]);
    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/roles"),
            Some(json!({"name": "Streamer", "color": "pink", "permissions": ["go_live"]})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let streamer = id(&res.body["id"]);

    let mut ws = join_voice(addr, &bob, sid, room).await;
    let live = json!({"op": "sig", "t": "p", "s": sid, "c": room, "k": "s"});
    assert!(!ticket_go_live(&mut bob, room).await);
    send(&mut ws, live.clone()).await;
    let denied = until(&mut ws, |f| f["op"] == "err").await;
    assert_eq!(denied["e"], "forbidden", "{denied}");

    // The role's go_live adds to the base role: ticket and gateway agree.
    set_roles(&mut ada, sid, bob.user_id(), json!([streamer])).await;
    assert!(ticket_go_live(&mut bob, room).await);
    send(&mut ws, live).await;
    let screen = until(&mut ws, |f| {
        f["op"] == "err" || (f["op"] == "sig" && f["t"] == "p" && f["k"] == "s")
    })
    .await;
    assert_eq!(screen["op"], "sig", "{screen}");
}
