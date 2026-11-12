//! Deleting an account (#519): `DELETE /api/me` and the purge after it.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::AppState;
use gelabber_api::account::deletion;
use gelabber_api::mail::Mailer;
use gelabber_api::storage::StoreError;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "password123";

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn signed_up(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

/// A second browser signed in to the same account.
async fn signed_in(state: &AppState, email: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.login(email, PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    client
}

async fn delete_me(client: &mut Client, body: Value) -> common::Response {
    client.send(Method::DELETE, "/api/me", Some(body)).await
}

async fn create_server(client: &mut Client, name: &str) -> Value {
    let res = client
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

async fn invite(client: &mut Client, server_id: &str) -> String {
    let res = client
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["code"].as_str().unwrap().to_owned()
}

async fn join(client: &mut Client, code: &str) {
    let res = client
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

fn id_of(value: &Value) -> String {
    value["id"].as_str().unwrap().to_owned()
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

async fn post(client: &mut Client, channel_id: &str, body: Value) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(body),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

/// Runs purge passes until the account's row is gone (the app's own worker
/// may get there first).
async fn purged(state: &AppState, user_id: Uuid) {
    for _ in 0..500 {
        deletion::purge_pending(state).await.unwrap();
        let left: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        if !left {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("account {user_id} was not purged");
}

async fn count(pool: &PgPool, sql: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

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

/// The next frame `want` accepts, answering heartbeats on the way.
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

/// Whether the socket ends (or errors) within a few seconds.
async fn closes(ws: &mut Ws) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return true,
                Some(Ok(Message::Text(raw))) => {
                    let frame: Value = serde_json::from_str(&raw).unwrap();
                    if frame["op"] == "h" {
                        let _ = ws
                            .send(Message::Text(json!({ "op": "h" }).to_string().into()))
                            .await;
                    }
                }
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[sqlx::test]
async fn an_owner_is_told_which_servers_to_hand_over_first(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_up(&state, "bob@example.com", "Bob").await;
    let home = create_server(&mut ada, "Wohnzimmer").await;
    let code = invite(&mut ada, &id_of(&home)).await;
    join(&mut bob, &code).await;
    // A blocked server cannot be handed over but still counts.
    let blocked = create_server(&mut ada, "Keller").await;
    sqlx::query("UPDATE servers SET blocked_at = now() WHERE id = $1")
        .bind(Uuid::parse_str(&id_of(&blocked)).unwrap())
        .execute(&pool)
        .await
        .unwrap();

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "owns_servers");
    assert_eq!(
        res.body["servers"],
        json!([
            { "id": id_of(&home), "name": "Wohnzimmer", "member_count": 2 },
            { "id": id_of(&blocked), "name": "Keller", "member_count": 1 },
        ])
    );
    // Nothing changed.
    let res = ada.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    let marked = count(
        &pool,
        "SELECT COUNT(*) FROM users WHERE id = $1 AND deleted_at IS NOT NULL",
        ada_id,
    )
    .await;
    assert_eq!(marked, 0);
}

#[sqlx::test]
async fn a_wrong_password_is_refused_and_counts_like_a_failed_login(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;

    let res = delete_me(&mut ada, json!({})).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["password"], "required");

    for _ in 0..10 {
        let res = delete_me(&mut ada, json!({ "password": "not the one" })).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["fields"]["password"], "wrong");
    }
    let res = delete_me(&mut ada, json!({ "password": "not the one" })).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
    assert!(res.headers.contains_key("retry-after"));
    // The same budget as signing in.
    let mut other = Client::from_state(state.clone());
    other.bootstrap().await;
    let res = other.login("ada@example.com", PASSWORD).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
    let res = ada.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[sqlx::test]
async fn deleting_ends_every_session_and_leaves_every_server_at_once(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut ada_phone = signed_in(&state, "ada@example.com").await;
    let mut bob = signed_up(&state, "bob@example.com", "Bob").await;
    let server = create_server(&mut bob, "Team").await;
    let server_id = id_of(&server);
    let code = invite(&mut bob, &server_id).await;
    join(&mut ada, &code).await;
    let ada_id = ada.user_id().to_owned();

    let mut bob_ws = socket(addr, &bob).await;
    bob_ws
        .send(Message::Text(
            json!({ "op": "s", "s": server_id }).to_string().into(),
        ))
        .await
        .unwrap();
    next_frame(&mut bob_ws, |f| f["op"] == "ok").await;
    let mut phone_ws = socket(addr, &ada_phone).await;
    next_frame(&mut phone_ws, |f| f["op"] == "h" || f["op"] == "hello").await;

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert!(res.body["csrf_token"].is_string());
    assert!(
        res.set_cookie(common::SESSION)
            .is_some_and(|c| c.contains("Max-Age=0")),
        "{:?}",
        res.headers
    );
    assert!(!ada.jar.contains_key(common::SESSION));

    // Members see the person leave.
    let frame = next_frame(&mut bob_ws, |f| f["op"] == "e" && f["d"]["k"] == "x").await;
    assert_eq!(frame["t"], "d", "{frame}");
    assert_eq!(frame["s"], server_id.as_str(), "{frame}");
    assert_eq!(frame["i"], ada_id.as_str(), "{frame}");
    let detail = bob
        .send(Method::GET, &format!("/api/servers/{server_id}"), None)
        .await;
    let members: Vec<&str> = detail.body["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["user_id"].as_str().unwrap())
        .collect();
    assert!(!members.contains(&ada_id.as_str()), "{members:?}");

    // The other device is signed out and its socket closed.
    assert!(closes(&mut phone_ws).await, "the phone's socket stays open");
    let res = ada_phone.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    // Nobody can sign in, read the card or open a DM any more.
    let mut again = Client::from_state(state.clone());
    again.bootstrap().await;
    let res = again.login("ada@example.com", PASSWORD).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    assert_eq!(res.body["error"], "invalid_credentials");
    let res = bob
        .send(Method::GET, &format!("/api/users/{ada_id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = bob
        .send(Method::POST, "/api/dms", Some(json!({ "user_id": ada_id })))
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
}

#[sqlx::test]
async fn the_purge_removes_messages_files_reactions_and_the_row(pool: PgPool) {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x40\0\0\0\x40\x08\x06\0\0\0\0\0\0\0";
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_up(&state, "bob@example.com", "Bob").await;
    let mut cara = signed_up(&state, "cara@example.com", "Cara").await;
    let server = create_server(&mut bob, "Team").await;
    let server_id = id_of(&server);
    let sid = Uuid::parse_str(&server_id).unwrap();
    let channel_id = text_channel_id(&server);
    let code = invite(&mut bob, &server_id).await;
    join(&mut ada, &code).await;
    join(&mut cara, &code).await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    let cara_id = Uuid::parse_str(cara.user_id()).unwrap();

    // Profile images.
    let (status, body) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let avatar_url = body["avatar_url"].as_str().unwrap().to_owned();
    let (status, body) = ada
        .send_bytes(Method::PUT, "/api/me/banner", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let banner_url = body["banner_url"].as_str().unwrap().to_owned();

    // A message with a file, one without, and an upload never sent.
    let presign =
        |name: &str| json!({ "filename": name, "content_type": "image/png", "size": PNG.len() });
    let signed = ada
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(presign("cat.png")),
        )
        .await;
    assert_eq!(signed.status, StatusCode::CREATED, "{}", signed.body);
    let sent_file = id_of(&signed.body);
    ada.store
        .put(&format!("att/{sent_file}"), "image/png", PNG.to_vec())
        .await
        .unwrap();
    post(
        &mut ada,
        &channel_id,
        json!({ "content": "look", "attachment_ids": [sent_file] }),
    )
    .await;
    post(&mut ada, &channel_id, json!({ "content": "hallo" })).await;
    let pending = ada
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(presign("later.png")),
        )
        .await;
    assert_eq!(pending.status, StatusCode::CREATED, "{}", pending.body);
    let pending_file = id_of(&pending.body);

    // A reply to her, and her reaction on Bob's message.
    let bobs = post(&mut bob, &channel_id, json!({ "content": "moin" })).await;
    let res = ada
        .send(
            Method::PUT,
            &format!("/api/messages/{}/reactions/%F0%9F%91%8D", id_of(&bobs)),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Rows the purge must leave: a ban she issued, a report she filed and
    // the beta invitation she used. And an identity, which goes.
    sqlx::query("INSERT INTO server_bans (server_id, user_id, banned_by) VALUES ($1, $2, $3)")
        .bind(sid)
        .bind(cara_id)
        .bind(ada_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM server_members WHERE server_id = $1 AND user_id = $2")
        .bind(sid)
        .bind(cara_id)
        .execute(&pool)
        .await
        .unwrap();
    let report: Uuid = sqlx::query_scalar(
        "INSERT INTO reports (server_id, reporter_id, reason) VALUES ($1, $2, 'spam') RETURNING id",
    )
    .bind(sid)
    .bind(ada_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let invitation: Uuid = sqlx::query_scalar(
        "INSERT INTO beta_invitations (code, used_by, used_at) VALUES ('ABCDEFGHJKMN', $1, now()) \
         RETURNING id",
    )
    .bind(ada_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id, email) \
         VALUES ('google', 'ada-sub', $1, 'ada@example.com')",
    )
    .bind(ada_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    purged(&state, ada_id).await;

    for (sql, what) in [
        (
            "SELECT COUNT(*) FROM messages WHERE author_id = $1",
            "messages",
        ),
        (
            "SELECT COUNT(*) FROM attachments WHERE uploader_id = $1",
            "attachments",
        ),
        (
            "SELECT COUNT(*) FROM message_reactions WHERE user_id = $1",
            "reactions",
        ),
        (
            "SELECT COUNT(*) FROM user_identities WHERE user_id = $1",
            "identities",
        ),
        (
            "SELECT COUNT(*) FROM upload_daily_usage WHERE uploader_id = $1",
            "ledger",
        ),
        (
            "SELECT COUNT(*) FROM sessions WHERE user_id = $1",
            "sessions",
        ),
    ] {
        assert_eq!(count(&pool, sql, ada_id).await, 0, "{what}");
    }
    // The files are queued (kept as rows until their upload links have
    // expired), the unsent upload without a reservation left to settle.
    for key in [format!("att/{sent_file}"), format!("att/{pending_file}")] {
        let reserved: Option<i64> =
            sqlx::query_scalar("SELECT reserved_bytes FROM storage_cleanup WHERE object_key = $1")
                .bind(&key)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert_eq!(reserved, Some(0), "{key}");
    }
    // Every object is deleted by the storage worker.
    let avatar_key = avatar_url.trim_start_matches("/api/").to_owned();
    let banner_key = banner_url.trim_start_matches("/api/").to_owned();
    for key in [format!("att/{sent_file}"), avatar_key, banner_key] {
        let mut gone = false;
        for _ in 0..250 {
            if matches!(state.store.head(&key).await, Err(StoreError::NotFound)) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(gone, "{key} is still stored");
    }

    // Bob's own message stays, without her reaction.
    let page = bob
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    let messages = page.body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1, "{}", page.body);
    assert_eq!(messages[0]["id"], id_of(&bobs).as_str());
    assert_eq!(messages[0]["reactions"], json!([]));

    // The ban stays without its issuer; report and invitation stay too.
    let bans = bob
        .send(Method::GET, &format!("/api/servers/{server_id}/bans"), None)
        .await;
    assert_eq!(bans.status, StatusCode::OK, "{}", bans.body);
    assert_eq!(bans.body[0]["user_id"], cara_id.to_string());
    assert_eq!(bans.body[0]["banned_by"], Value::Null);
    let reporter: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT reporter_id FROM reports WHERE id = $1")
            .bind(report)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(reporter, Some(None));
    let used_by: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT used_by FROM beta_invitations WHERE id = $1")
            .bind(invitation)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(used_by, Some(None));
}

#[sqlx::test]
async fn invites_go_at_once_and_the_address_is_free_after_the_purge(pool: PgPool) {
    let state = common::state(pool);
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_up(&state, "bob@example.com", "Bob").await;
    let server = create_server(&mut bob, "Team").await;
    let code = invite(&mut bob, &id_of(&server)).await;
    join(&mut ada, &code).await;
    let hers = invite(&mut ada, &id_of(&server)).await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let mut cara = signed_up(&state, "cara@example.com", "Cara").await;
    let res = cara
        .send(Method::POST, &format!("/api/invites/{hers}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let res = cara
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    purged(&state, ada_id).await;
    let mut again = Client::from_state(state.clone());
    again.bootstrap().await;
    let res = again
        .register("ada@example.com", "a new password", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_ne!(again.user_id(), ada_id.to_string());
}

#[sqlx::test]
async fn a_dm_goes_on_both_sides_and_the_peer_is_told(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_up(&state, "bob@example.com", "Bob").await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    let res = ada
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": bob.user_id() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let dm = id_of(&res.body);
    post(&mut ada, &dm, json!({ "content": "hi" })).await;
    post(&mut bob, &dm, json!({ "content": "hey" })).await;

    let mut bob_ws = socket(addr, &bob).await;
    next_frame(&mut bob_ws, |f| f["op"] == "h" || f["op"] == "hello").await;

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    purged(&state, ada_id).await;

    // Delivered although Bob can no longer read the channel.
    let frame = next_frame(&mut bob_ws, |f| f["op"] == "dm").await;
    assert_eq!(frame["c"], dm.as_str(), "{frame}");
    let list = bob.send(Method::GET, "/api/dms", None).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body, json!([]), "{}", list.body);
    let res = bob.send(Method::GET, &format!("/api/dms/{dm}"), None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let dm_id = Uuid::parse_str(&dm).unwrap();
    for sql in [
        "SELECT COUNT(*) FROM channels WHERE id = $1",
        "SELECT COUNT(*) FROM messages WHERE channel_id = $1",
    ] {
        assert_eq!(count(&pool, sql, dm_id).await, 0, "{sql}");
    }
}

#[sqlx::test]
async fn the_purge_mails_the_old_address_in_its_language(pool: PgPool) {
    let mut state = common::state(pool.clone());
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    // With mail on, a registration must be confirmed before it signs in.
    let mut ada = Client::from_state(state.clone());
    ada.bootstrap().await;
    let res = ada.register("ada@example.com", PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    sqlx::query(
        "UPDATE users SET email_verified_at = now(), registration_pending = false, locale = 'en' \
         WHERE email = 'ada@example.com'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let res = ada.login("ada@example.com", PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    let before = mailer.outbox().len();

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    purged(&state, ada_id).await;
    for _ in 0..200 {
        if mailer.outbox().len() > before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let sent = mailer.outbox();
    let last = sent.last().expect("a mail");
    assert_eq!(sent.len(), before + 1, "{sent:?}");
    assert_eq!(last.to, "ada@example.com");
    assert_eq!(last.subject, "Your Gelabber account has been deleted");
}

#[sqlx::test]
async fn an_account_without_a_password_confirms_with_its_address(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    sqlx::query("UPDATE users SET password_hash = NULL WHERE id = $1")
        .bind(ada_id)
        .execute(&pool)
        .await
        .unwrap();

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["email"], "required");
    let res = delete_me(&mut ada, json!({ "email": "bob@example.com" })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["email"], "wrong");

    // A sign-in older than the window must be repeated first.
    sqlx::query(
        "UPDATE sessions SET created_at = now() - interval '11 minutes' WHERE user_id = $1",
    )
    .bind(ada_id)
    .execute(&pool)
    .await
    .unwrap();
    let res = delete_me(&mut ada, json!({ "email": "ada@example.com" })).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "reauth_required");

    sqlx::query("UPDATE sessions SET created_at = now() WHERE user_id = $1")
        .bind(ada_id)
        .execute(&pool)
        .await
        .unwrap();
    let res = delete_me(&mut ada, json!({ "email": " ADA@example.com " })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let res = ada.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn a_provider_sign_in_does_not_find_an_account_under_deletion(pool: PgPool) {
    use gelabber_api::auth::oidc::link::{self, Identity, SignIn};

    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id, email) \
         VALUES ('google', 'ada-sub', $1, 'ada@example.com')",
    )
    .bind(ada_id)
    .execute(&pool)
    .await
    .unwrap();
    // Keep the row from being purged while the lookups run.
    let mut hold = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('account-purge'), hashtext($1::text))")
        .bind(ada_id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let marked = count(
        &pool,
        "SELECT COUNT(*) FROM users WHERE id = $1 AND deleted_at IS NOT NULL",
        ada_id,
    )
    .await;
    assert_eq!(marked, 1);

    let mut conn = pool.acquire().await.unwrap();
    let identity = Identity {
        provider: "google",
        subject: "ada-sub",
        email: Some("ada@example.com"),
        email_verified: true,
    };
    let found = link::sign_in(&mut conn, &identity).await.unwrap();
    assert_eq!(found, SignIn::New { takeover: None });
    hold.rollback().await.unwrap();
}

/// A server created while the deletion runs is either seen by it (409) or
/// refused, never left owned by a deleted account.
#[sqlx::test]
async fn a_server_created_during_the_deletion_is_never_left_behind(pool: PgPool) {
    let state = common::state(pool.clone());
    let ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    let browser = |client: &Client| {
        let mut other = Client::from_state(state.clone());
        other.jar = client.jar.clone();
        other.csrf = client.csrf.clone();
        other
    };

    // The deletion holds the row first: the server is refused.
    let mut deleting = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(ada_id)
        .execute(&mut *deleting)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(ada_id)
        .execute(&mut *deleting)
        .await
        .unwrap();
    let mut creator = browser(&ada);
    let create = tokio::spawn(async move {
        creator
            .send(
                Method::POST,
                "/api/servers",
                Some(json!({ "name": "Spät" })),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!create.is_finished(), "the create waits for the row");
    deleting.rollback().await.unwrap();
    let res = create.await.unwrap();
    // Rolled back, so this time the account is alive and the server made.
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server_id = id_of(&res.body);

    // Committed instead: refused.
    let mut deleting = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(ada_id)
        .execute(&mut *deleting)
        .await
        .unwrap();
    // Phase one would refuse while she owns a server; hand it away first.
    sqlx::query("DELETE FROM servers WHERE id = $1")
        .bind(Uuid::parse_str(&server_id).unwrap())
        .execute(&mut *deleting)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(ada_id)
        .execute(&mut *deleting)
        .await
        .unwrap();
    let mut creator = browser(&ada);
    let create = tokio::spawn(async move {
        creator
            .send(
                Method::POST,
                "/api/servers",
                Some(json!({ "name": "Spät" })),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    deleting.commit().await.unwrap();
    let res = create.await.unwrap();
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    let owned = count(
        &pool,
        "SELECT COUNT(*) FROM servers WHERE owner_id = $1",
        ada_id,
    )
    .await;
    assert_eq!(owned, 0);
}

/// The other order: a server created first holds the row, and the deletion
/// that waited for it then answers 409.
#[sqlx::test]
async fn a_deletion_waits_for_a_server_being_created(pool: PgPool) {
    let state = common::state(pool.clone());
    let ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();

    let mut creating = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR KEY SHARE")
        .bind(ada_id)
        .execute(&mut *creating)
        .await
        .unwrap();
    sqlx::query("INSERT INTO servers (name, owner_id, member_permissions) VALUES ('Neu', $1, 0)")
        .bind(ada_id)
        .execute(&mut *creating)
        .await
        .unwrap();
    let mut deleter = Client::from_state(state.clone());
    deleter.jar = ada.jar.clone();
    deleter.csrf = ada.csrf.clone();
    let delete =
        tokio::spawn(async move { delete_me(&mut deleter, json!({ "password": PASSWORD })).await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!delete.is_finished(), "the deletion waits for the row");
    creating.commit().await.unwrap();
    let res = delete.await.unwrap();
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["servers"][0]["name"], "Neu");
}

/// A queued object deletion whose reservation was released goes through
/// without a ledger row to settle against. One that still holds reserved
/// bytes waits for its ledger row and stays queued, which is why the purge
/// releases the person's reservations before it deletes their ledger.
#[sqlx::test]
async fn a_released_reservation_needs_no_ledger_row(pool: PgPool) {
    use gelabber_api::attachments::cleanup;

    let state = common::state(pool.clone());
    let uploader = Uuid::new_v4();
    for key in ["att/orphan", "att/reserved"] {
        state
            .store
            .put(key, "image/png", vec![1, 2, 3])
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, retain_until, quota_uploader, quota_day, \
             reserved_bytes) \
         VALUES ('att/orphan', now(), $1, CURRENT_DATE, 0), \
                ('att/reserved', now(), $1, CURRENT_DATE, 3)",
    )
    .bind(uploader)
    .execute(&pool)
    .await
    .unwrap();
    let queued = |key: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM storage_cleanup WHERE object_key = $1)",
            )
            .bind(key)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let mut deleted = false;
    for _ in 0..100 {
        cleanup::delete_pending(&state, 32).await.unwrap();
        if !queued("att/orphan").await {
            deleted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(deleted, "the queued object was not deleted");
    // Past its retry delay, the reserved one is still waiting.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    cleanup::delete_pending(&state, 32).await.unwrap();
    assert!(queued("att/reserved").await);
    assert!(state.store.fetch("att/reserved").await.is_ok());
}

/// Pending requests to join a listed server go with phase one, like the
/// memberships: the queue no longer lists the person and its handlers hear
/// the smaller count. A declined request stays until the purge.
#[sqlx::test]
async fn pending_join_requests_go_at_once_and_the_queue_hears_it(pool: PgPool) {
    let mut state = common::ws_state(pool.clone());
    state.directory = true;
    let addr = common::serve_ws_state(state.clone()).await;
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut lena = signed_up(&state, "lena@example.com", "Lena").await;
    let lena_id = Uuid::parse_str(lena.user_id()).unwrap();
    let keller = id_of(&create_server(&mut ada, "Modellbau-Keller").await);
    let werkstatt = id_of(&create_server(&mut ada, "Werkstatt").await);
    let res = ada
        .send(
            Method::PUT,
            &format!("/api/servers/{keller}/listing"),
            Some(json!({
                "listed": true,
                "description": "Kommt vorbei.",
                "language": "de",
                "join_mode": "request",
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    sqlx::query(
        "INSERT INTO server_join_requests (server_id, user_id, status, decided_at, decided_by) \
         VALUES ($1, $2, 'declined', now(), $3)",
    )
    .bind(Uuid::parse_str(&werkstatt).unwrap())
    .bind(lena_id)
    .bind(Uuid::parse_str(ada.user_id()).unwrap())
    .execute(&pool)
    .await
    .unwrap();

    let mut ada_ws = socket(addr, &ada).await;
    ada_ws
        .send(Message::Text(json!({ "op": "h" }).to_string().into()))
        .await
        .unwrap();
    next_frame(&mut ada_ws, |f| f["op"] == "h").await;
    let res = lena
        .send(
            Method::POST,
            &format!("/api/directory/listings/{keller}/request"),
            Some(json!({ "message": "Darf ich?" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let frame = next_frame(&mut ada_ws, |f| f["op"] == "jq").await;
    assert_eq!(frame, json!({ "op": "jq", "s": keller, "n": 1 }));

    let res = delete_me(&mut lena, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let frame = next_frame(&mut ada_ws, |f| f["op"] == "jq").await;
    assert_eq!(frame, json!({ "op": "jq", "s": keller, "n": 0 }));
    let res = ada
        .send(
            Method::GET,
            &format!("/api/servers/{keller}/join-requests"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!([]));
    // Before the purge: the pending row is gone, the declined one stays.
    let left: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT server_id, status FROM server_join_requests WHERE user_id = $1")
            .bind(lena_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        left,
        vec![(Uuid::parse_str(&werkstatt).unwrap(), "declined".to_owned())]
    );
}

/// Feedback outlives its sender's account (#542): the entry stays for the
/// operators, unlinked and without its image; an operator's deleted
/// account leaves its replies as operator replies without an author.
#[sqlx::test]
async fn feedback_stays_unlinked_and_its_image_is_queued(pool: PgPool) {
    let mut state = common::state(pool.clone());
    state.operators = std::sync::Arc::new(std::collections::HashSet::from([
        "lena@example.com".to_owned(),
        "max@example.com".to_owned(),
    ]));
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut lena = signed_up(&state, "lena@example.com", "Lena").await;
    let mut max = signed_up(&state, "max@example.com", "Max").await;
    let ada_id: Uuid = ada.user_id().parse().unwrap();
    let lena_id: Uuid = lena.user_id().parse().unwrap();

    let key = format!("feedback/{ada_id}/{}", Uuid::new_v4());
    let entry: Uuid = sqlx::query_scalar(
        "INSERT INTO feedback (kind, user_id, client, os, app_version, locale, page, body, \
             image_key, image_type, image_bytes) \
         VALUES ('idea', $1, 'web', 'linux', '0.10.0', 'de', '/', 'Bitte mehr', $2, \
             'image/png', 100) RETURNING id",
    )
    .bind(ada_id)
    .bind(&key)
    .fetch_one(&pool)
    .await
    .unwrap();
    let res = lena
        .send(
            Method::POST,
            &format!("/api/operator/feedback/{entry}/messages"),
            Some(json!({ "text": "Danke" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let res = delete_me(&mut ada, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    purged(&state, ada_id).await;
    let (user_id, image_key): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT user_id, image_key FROM feedback WHERE id = $1")
            .bind(entry)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((user_id, image_key), (None, None));
    let queued: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM storage_cleanup WHERE object_key = $1)")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(queued);
    let res = max
        .send(
            Method::GET,
            &format!("/api/operator/feedback/{entry}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["sender"], json!({ "type": "deleted" }));
    assert_eq!(res.body["image"], Value::Null);
    assert_eq!(res.body["text"], "Bitte mehr");

    let res = delete_me(&mut lena, json!({ "password": PASSWORD })).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    purged(&state, lena_id).await;
    let res = max
        .send(
            Method::GET,
            &format!("/api/operator/feedback/{entry}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["messages"][0]["from"], "operator");
    assert_eq!(res.body["messages"][0]["author"], Value::Null);
    let kind: String =
        sqlx::query_scalar("SELECT author_kind FROM feedback_messages WHERE feedback_id = $1")
            .bind(entry)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kind, "operator");
}
