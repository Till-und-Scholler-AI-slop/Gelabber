//! Adversarial authorization transitions with sockets deliberately left open.
mod common;

use axum::http::{Method, StatusCode};
use common::{Client, SESSION};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::auth::session;
use gelabber_api::media;
use gelabber_api::{AppState, EventKind, publish_channel};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

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
fn duplicate(client: &Client) -> Client {
    Client {
        app: client.app.clone(),
        store: client.store.clone(),
        jar: client.jar.clone(),
        csrf: client.csrf.clone(),
        user_id: client.user_id.clone(),
    }
}
async fn server(client: &mut Client, name: &str) -> (Uuid, Uuid, Uuid, String) {
    let res = client
        .send(Method::POST, "/api/servers", Some(json!({"name":name})))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let sid: Uuid = res.body["id"].as_str().unwrap().parse().unwrap();
    let text = res.body["channels"][0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let voice = client
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/channels"),
            Some(json!({"name":"Voice", "kind":"voice"})),
        )
        .await;
    assert_eq!(voice.status, StatusCode::CREATED, "{}", voice.body);
    let invite = client
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED);
    (
        sid,
        text,
        voice.body["id"].as_str().unwrap().parse().unwrap(),
        invite.body["code"].as_str().unwrap().to_owned(),
    )
}
async fn join(client: &mut Client, code: &str) {
    let res = client
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
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
                        send(ws, json!({"op":"h"})).await;
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
async fn subscribe(ws: &mut Ws, sid: Uuid, cid: Uuid) {
    send(ws, json!({"op":"s", "s":sid, "c":cid})).await;
    until(ws, |f| f["op"] == "ok" && f["c"] == cid.to_string()).await;
}
async fn voice_join(ws: &mut Ws, sid: Uuid, cid: Uuid) {
    send(ws, json!({"op":"sig", "t":"j", "s":sid, "c":cid})).await;
    until(ws, |f| f["op"] == "sig" && f["t"] == "j").await;
}
async fn closed_without_events(ws: &mut Ws) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(frame) = ws.next().await {
            match frame {
                Ok(Message::Close(_)) | Err(_) => return,
                Ok(Message::Text(raw)) => {
                    let frame: Value = serde_json::from_str(&raw).unwrap();
                    assert_ne!(frame["op"], "e", "data after revoke: {frame}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("session socket closed promptly");
}
async fn claim(client: &mut Client, cid: Uuid) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{cid}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let mut redis = redis::Client::open(common::redis_url())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let raw: String = redis::cmd("GET")
        .arg(format!("gb:mt:{}", res.body["ticket"].as_str().unwrap()))
        .query_async(&mut redis)
        .await
        .unwrap();
    serde_json::from_str(&raw).unwrap()
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
async fn publish(state: &AppState, sid: Uuid, cid: Uuid) {
    publish_channel(state, sid, cid, EventKind::C, Some(Uuid::new_v4()), None)
        .await
        .unwrap();
}

#[sqlx::test]
async fn logout_closes_only_exact_session_across_api_instances(pool: PgPool) {
    let (addr, socket_state) = common::serve_ws(pool.clone()).await;
    let state = common::ws_state(pool);
    let mut owner = register(&state, "owner").await;
    let (sid, text, _, _) = server(&mut owner, "Sessions").await;
    let mut other_session = Client::with_state(state.clone());
    other_session.bootstrap().await;
    assert_eq!(
        other_session
            .login("owner@example.test", "password123")
            .await
            .status,
        StatusCode::OK
    );
    let mut first = connect(addr, &owner).await;
    let mut second = connect(addr, &other_session).await;
    subscribe(&mut first, sid, text).await;
    subscribe(&mut second, sid, text).await;
    assert_eq!(
        owner
            .send(Method::POST, "/api/auth/logout", None)
            .await
            .status,
        StatusCode::OK
    );
    publish(&socket_state, sid, text).await;
    closed_without_events(&mut first).await;
    until(&mut second, |f| f["op"] == "e").await;
    assert_eq!(
        other_session.bootstrap().await.body["user"]["name"],
        "owner"
    );
}

#[sqlx::test]
async fn session_expiry_closes_open_socket_without_client_action(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut owner = register(&state, "owner").await;
    let (sid, text, _, _) = server(&mut owner, "Expiry").await;
    sqlx::query("UPDATE sessions SET expires_at = now() + interval '400 milliseconds' WHERE token_hash = $1")
        .bind(gelabber_api::token::hash(&owner.jar[SESSION])).execute(&pool).await.unwrap();
    let mut ws = connect(addr, &owner).await;
    subscribe(&mut ws, sid, text).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    publish(&state, sid, text).await;
    closed_without_events(&mut ws).await;
}

async fn removal(pool: PgPool, mode: &str) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mutation_state = common::ws_state(pool);
    let mut owner = register(&mutation_state, "owner").await;
    let mut member = register(&mutation_state, "member").await;
    let (sid, text, voice, code) = server(&mut owner, "Revoked").await;
    let (control, control_text, _, control_code) = server(&mut owner, "Control").await;
    join(&mut member, &code).await;
    join(&mut member, &control_code).await;
    let old = claim(&mut member, voice).await;
    let uid: Uuid = member.user_id().parse().unwrap();
    let mut ws = connect(addr, &member).await;
    subscribe(&mut ws, sid, text).await;
    subscribe(&mut ws, control, control_text).await;
    voice_join(&mut ws, sid, voice).await;
    let response = if mode == "leave" {
        member
            .send(Method::POST, &format!("/api/servers/{sid}/leave"), None)
            .await
    } else {
        owner
            .send(
                Method::POST,
                &format!("/api/servers/{sid}/{mode}"),
                Some(json!({"user_id":uid})),
            )
            .await
    };
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.body);
    assert_ne!(
        redis_get(media::member_key(sid, uid)).await.as_deref(),
        old["auth"]["member"].as_str()
    );
    publish(&state, sid, text).await;
    publish(&state, control, control_text).await;
    let received = until(&mut ws, |f| {
        assert!(
            !(f["op"] == "e" && f["s"] == sid.to_string()),
            "revoked event leaked: {f}"
        );
        f["op"] == "e" && f["s"] == control.to_string()
    })
    .await;
    assert_eq!(received["c"], control_text.to_string());
    assert!(state.gateway.voice_snapshot(sid).await.unwrap().is_empty());
    send(
        &mut ws,
        json!({"op":"sig", "t":"m", "s":sid, "c":voice, "on":true}),
    )
    .await;
    until(&mut ws, |f| f["op"] == "err" && f["e"] == "not_found").await;
    let denied = member
        .send(
            Method::POST,
            &format!("/api/channels/{voice}/media-ticket"),
            None,
        )
        .await;
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
    if mode != "ban" {
        join(&mut member, &code).await;
        let fresh = claim(&mut member, voice).await;
        assert_ne!(old["auth"]["member"], fresh["auth"]["member"]);
        send(
            &mut ws,
            json!({"op":"sig", "t":"m", "s":sid, "c":voice, "on":true}),
        )
        .await;
        until(&mut ws, |f| f["op"] == "err" && f["e"] == "bad_request").await;
    }
}
#[sqlx::test]
async fn leave_revokes_existing_subscriptions_and_old_tickets(pool: PgPool) {
    removal(pool, "leave").await;
}
#[sqlx::test]
async fn kick_revokes_existing_subscriptions_and_old_tickets(pool: PgPool) {
    removal(pool, "kick").await;
}
#[sqlx::test]
async fn ban_revokes_existing_subscriptions_and_old_tickets(pool: PgPool) {
    removal(pool, "ban").await;
}

#[sqlx::test]
async fn voice_followups_cannot_forge_server_channel_pair(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool).await;
    let mut owner = register(&state, "owner").await;
    let (sid, _, voice, _) = server(&mut owner, "Actual").await;
    let (wrong, _, _, _) = server(&mut owner, "Forged").await;
    let mut ws = connect(addr, &owner).await;
    voice_join(&mut ws, sid, voice).await;
    for frame in [
        json!({"t":"p", "k":"l"}),
        json!({"t":"u", "k":"v"}),
        json!({"t":"m", "on":true}),
        json!({"t":"d", "on":true}),
        json!({"t":"l"}),
    ] {
        let mut frame = frame;
        frame["op"] = json!("sig");
        frame["s"] = json!(wrong);
        frame["c"] = json!(voice);
        send(&mut ws, frame).await;
        until(&mut ws, |f| f["op"] == "err" && f["s"] == wrong.to_string()).await;
        let actual = state.gateway.voice_snapshot(sid).await.unwrap();
        assert_eq!(actual.len(), 1);
        assert!(!actual[0].m && !actual[0].d && !actual[0].l);
        assert!(
            state
                .gateway
                .voice_snapshot(wrong)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

async fn deletion(pool: PgPool, whole_server: bool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut owner = register(&common::ws_state(pool), "owner").await;
    let (sid, text, voice, _) = server(&mut owner, "Deleted").await;
    let (control, control_text, _, _) = server(&mut owner, "Control").await;
    let old = claim(&mut owner, voice).await;
    let mut ws = connect(addr, &owner).await;
    subscribe(&mut ws, sid, text).await;
    subscribe(&mut ws, control, control_text).await;
    voice_join(&mut ws, sid, voice).await;
    let path = if whole_server {
        format!("/api/servers/{sid}")
    } else {
        format!("/api/channels/{voice}")
    };
    assert_eq!(
        owner.send(Method::DELETE, &path, None).await.status,
        StatusCode::NO_CONTENT
    );
    publish(&state, control, control_text).await;
    until(&mut ws, |f| f["op"] == "e" && f["s"] == control.to_string()).await;
    assert!(state.gateway.voice_snapshot(sid).await.unwrap().is_empty());
    let key = if whole_server {
        media::member_key(sid, owner.user_id().parse().unwrap())
    } else {
        media::channel_key(voice)
    };
    let old_revision = if whole_server {
        &old["auth"]["member"]
    } else {
        &old["auth"]["channel"]
    };
    assert_ne!(redis_get(key).await.as_deref(), old_revision.as_str());
    send(
        &mut ws,
        json!({"op":"sig", "t":"p", "s":sid, "c":voice, "k":"l"}),
    )
    .await;
    until(&mut ws, |f| f["op"] == "err" && f["e"] == "not_found").await;
}
#[sqlx::test]
async fn deleting_channel_cleans_open_voice_and_ticket_authority(pool: PgPool) {
    deletion(pool, false).await;
}
#[sqlx::test]
async fn deleting_server_cleans_open_voice_and_ticket_authority(pool: PgPool) {
    deletion(pool, true).await;
}

#[sqlx::test]
async fn concurrent_invites_for_same_user_consume_one_use(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    let mut owner = register(&state, "owner").await;
    let member = register(&state, "member").await;
    let (sid, _, _, code) = server(&mut owner, "Race").await;
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({"max_uses":1})),
        )
        .await;
    let code2 = invite.body["code"].as_str().unwrap();
    let mut a = duplicate(&member);
    let mut b = duplicate(&member);
    let path_a = format!("/api/invites/{code}/join");
    let path_b = format!("/api/invites/{code2}/join");
    let (a, b) = tokio::join!(
        a.send(Method::POST, &path_a, None),
        b.send(Method::POST, &path_b, None)
    );
    assert_eq!(a.status, StatusCode::OK, "{}", a.body);
    assert_eq!(b.status, StatusCode::OK, "{}", b.body);
    let uses: i64 =
        sqlx::query_scalar("SELECT sum(uses)::bigint FROM invites WHERE server_id = $1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(uses, 1);
}

#[sqlx::test]
async fn ban_serializes_with_join_after_ban_check_before_insert(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    let mut owner = register(&state, "owner").await;
    let mut member = register(&state, "member").await;
    let uid: Uuid = member.user_id().parse().unwrap();
    let (sid, _, _, code) = server(&mut owner, "Race").await;
    // Force the exact old TOCTOU window: insert has passed the ban check but
    // waits on a BEFORE INSERT trigger. This uses test-local DB objects only.
    let lock = (Uuid::new_v4().as_u128() & 0x7fff_ffff) as i64;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE FUNCTION pause_join() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({lock}); RETURN NEW; END $$; CREATE TRIGGER pause_join BEFORE INSERT ON server_members FOR EACH ROW EXECUTE FUNCTION pause_join();"))).execute(&pool).await.unwrap();
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let join_task = tokio::spawn(async move {
        member
            .send(Method::POST, &format!("/api/invites/{code}/join"), None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event = 'advisory')").fetch_one(&pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("join reached insert barrier");
    let mut ban_task = tokio::spawn(async move {
        owner
            .send(
                Method::POST,
                &format!("/api/servers/{sid}/ban"),
                Some(json!({"user_id":uid})),
            )
            .await
    });
    let prematurely_finished = tokio::time::timeout(Duration::from_millis(100), &mut ban_task)
        .await
        .ok();
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock)
        .execute(&mut *blocker)
        .await
        .unwrap();
    drop(blocker);
    let joined = join_task.await.unwrap();
    // The insert is serialized before Ban, but join() rechecks membership after
    // COMMIT. Ban may remove it before that final read; 404 is then intentional.
    match joined.status {
        StatusCode::OK => assert_eq!(joined.body["id"], sid.to_string()),
        StatusCode::NOT_FOUND => assert_eq!(joined.body["error"], "not_found"),
        status => panic!("unexpected join response {status}: {}", joined.body),
    }
    let ban_waited_for_join = prematurely_finished.is_none();
    let banned = match prematurely_finished {
        Some(result) => result.unwrap(),
        None => ban_task.await.unwrap(),
    };
    assert_eq!(banned.status, StatusCode::NO_CONTENT);
    assert!(
        ban_waited_for_join,
        "Ban must wait for the held Join transaction"
    );
    let uses: i32 = sqlx::query_scalar("SELECT uses FROM invites WHERE server_id=$1")
        .bind(sid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uses, 1, "Join must have committed exactly once before Ban");
    let inconsistent: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM server_members m JOIN server_bans b USING(server_id,user_id) WHERE m.server_id = $1 AND m.user_id = $2)").bind(sid).bind(uid).fetch_one(&pool).await.unwrap();
    assert!(!inconsistent, "ban and membership cannot coexist");
}

#[sqlx::test]
async fn utf8_avatar_prefix_is_422_and_does_not_kill_request(pool: PgPool) {
    let mut owner = register(&common::state(pool), "owner").await;
    for raw in ["aaaaaaaéx", "🦀🦀🦀"] {
        let response = owner
            .send(Method::PATCH, "/api/me", Some(json!({"avatar_url":raw})))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.body["fields"]["avatar_url"], "invalid");
    }
}

#[sqlx::test]
async fn logout_stays_revoked_when_redis_cleanup_fails(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    let owner = register(&state, "owner").await;
    let raw = owner.jar[SESSION].clone();
    let mut unavailable = state;
    unavailable.redis = redis::Client::open("redis://127.0.0.1:1").unwrap();
    session::revoke_current(&unavailable, Some(&raw))
        .await
        .unwrap();
    assert!(session::resolve(&pool, &raw).await.unwrap().is_none());
}

#[sqlx::test]
async fn media_session_lease_dies_when_renewal_stops(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    let mut owner = register(&state, "owner").await;
    let (_, _, voice, _) = server(&mut owner, "Lease").await;
    let ticket = claim(&mut owner, voice).await;
    let key = media::session_key(ticket["auth"]["session"].as_str().unwrap());
    assert_eq!(
        redis_get(key.clone()).await.as_deref(),
        Some(owner.user_id())
    );
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(gelabber_api::token::hash(&owner.jar[SESSION]))
        .execute(&pool)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while redis_get(key.clone()).await.is_some() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("lease expires without logout notification");
}

#[sqlx::test]
async fn one_connection_pool_supports_concurrent_auth_join_ticket_and_cleanup(pool: PgPool) {
    use sqlx::postgres::PgPoolOptions;
    let small = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(2))
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let state = common::ws_state(small.clone());
    let mut owner = register(&state, "owner").await;
    let member = register(&state, "member").await;
    let (sid, _, voice, code) = server(&mut owner, "Small pool").await;
    let mut tasks = Vec::new();
    for _ in 0..5 {
        let mut member = duplicate(&member);
        let code = code.clone();
        tasks.push(tokio::spawn(async move {
            join(&mut member, &code).await;
            claim(&mut member, voice).await;
            assert_eq!(member.bootstrap().await.body["user"]["name"], "member");
        }));
    }
    tokio::time::timeout(Duration::from_secs(6), async {
        for task in tasks {
            task.await.unwrap();
        }
    })
    .await
    .expect("no pool starvation");
    assert_eq!(
        owner
            .send(
                Method::POST,
                &format!("/api/servers/{sid}/ban"),
                Some(json!({"user_id":member.user_id()}))
            )
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        owner
            .send(Method::DELETE, &format!("/api/channels/{voice}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        owner
            .send(Method::DELETE, &format!("/api/servers/{sid}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        owner
            .send(Method::POST, "/api/auth/logout", None)
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn idle_media_authority_stops_and_a_new_mint_restarts_it(pool: PgPool) {
    let state = common::ws_state(pool);
    let mut owner = register(&state, "owner").await;
    let (_, _, voice, _) = server(&mut owner, "Idle").await;
    let old = claim(&mut owner, voice).await;
    let key = media::session_key(old["auth"]["session"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(40), async {
        while redis_get(key.clone()).await.is_some() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("one ticket does not renew authority for a 30-day session");
    let fresh = claim(&mut owner, voice).await;
    assert_eq!(old["auth"]["session"], fresh["auth"]["session"]);
    assert_eq!(redis_get(key).await.as_deref(), Some(owner.user_id()));
}

#[sqlx::test]
async fn permission_extension_keeps_voice_but_reduction_revokes_it(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool.clone()).await;
    let mut owner = register(&state, "owner").await;
    let mut member = register(&state, "member").await;
    let (sid, _, voice, code) = server(&mut owner, "Permissions").await;
    join(&mut member, &code).await;
    let mut ws = connect(addr, &member).await;
    voice_join(&mut ws, sid, voice).await;
    let old = claim(&mut member, voice).await;
    assert_eq!(owner.send(Method::PATCH, &format!("/api/servers/{sid}"), Some(json!({"member_permissions":["send_messages","send_files","join_voice","go_live","manage_channels"]}))).await.status, StatusCode::OK);
    send(
        &mut ws,
        json!({"op":"sig","t":"m","s":sid,"c":voice,"on":true}),
    )
    .await;
    until(&mut ws, |f| f["op"] == "sig" && f["t"] == "m").await;
    assert_eq!(
        redis_get(media::member_key(sid, member.user_id().parse().unwrap()))
            .await
            .as_deref(),
        old["auth"]["member"].as_str()
    );
    assert_eq!(
        owner
            .send(
                Method::PATCH,
                &format!("/api/servers/{sid}"),
                Some(json!({"member_permissions":["send_messages"]}))
            )
            .await
            .status,
        StatusCode::OK
    );
    assert!(state.gateway.voice_snapshot(sid).await.unwrap().is_empty());
    assert_ne!(
        redis_get(media::member_key(sid, member.user_id().parse().unwrap()))
            .await
            .as_deref(),
        old["auth"]["member"].as_str()
    );
}

#[sqlx::test]
async fn go_live_with_one_pool_connection_keeps_bootstrap_responsive_and_persists_hint(
    pool: PgPool,
) {
    let small = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(500))
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let state = common::ws_state(small);
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .unwrap();
    let mut owner = register(&state, "owner").await;
    let (sid, text, voice, _) = server(&mut owner, "Live hint").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = gelabber_api::app(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut ws = connect(addr, &owner).await;
    voice_join(&mut ws, sid, voice).await;
    send(
        &mut ws,
        json!({"op":"sig", "t":"p", "s":sid, "c":voice, "k":"l"}),
    )
    .await;
    until(&mut ws, |f| {
        f["op"] == "sig" && f["t"] == "p" && f["k"] == "l"
    })
    .await;
    let bootstrap = tokio::time::timeout(Duration::from_millis(250), owner.bootstrap())
        .await
        .expect("no nested pool acquire");
    assert_eq!(bootstrap.status, StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let hint: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE channel_id = $1 AND content LIKE '%ist live%')")
                .bind(text).fetch_one(&pool).await.unwrap();
            if hint { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("authorized Live hint persists");
}
