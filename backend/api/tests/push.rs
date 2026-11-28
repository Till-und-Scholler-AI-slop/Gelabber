//! Web Push (#243): subscriptions, who gets a push and what it carries. A
//! fake push service on loopback plays the browser vendor.
mod common;

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::{Router, routing};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use common::Client;
use common::ws::{self, Ws};
use gelabber_api::push::webpush::{Vapid, decrypt};
use gelabber_api::{AppState, Config, app};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

struct Hit {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

/// Answers 201, 410 for an endpoint under `/gone/`, 500 under `/fail/`,
/// 400 under `/bad/`, 403 under `/denied/`, 413 under `/big/`.
async fn fake_service() -> (String, mpsc::UnboundedReceiver<Hit>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let router = Router::new()
        .route(
            "/{*path}",
            routing::post(
                |State(tx): State<mpsc::UnboundedSender<Hit>>,
                 Path(path): Path<String>,
                 headers: HeaderMap,
                 body: axum::body::Bytes| async move {
                    let gone = path.starts_with("gone/");
                    let fail = path.starts_with("fail/");
                    let bad = path.starts_with("bad/");
                    let denied = path.starts_with("denied/");
                    let big = path.starts_with("big/");
                    let _ = tx.send(Hit {
                        path,
                        headers,
                        body: body.to_vec(),
                    });
                    if gone {
                        StatusCode::GONE
                    } else if bad {
                        StatusCode::BAD_REQUEST
                    } else if denied {
                        StatusCode::FORBIDDEN
                    } else if big {
                        StatusCode::PAYLOAD_TOO_LARGE
                    } else if fail {
                        StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        StatusCode::CREATED
                    }
                },
            ),
        )
        .with_state(tx);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), rx)
}

fn push_state(pool: PgPool, vapid_private: Option<&str>) -> AppState {
    push_state_at(pool, vapid_private, None)
}

/// As [`push_state`], for an instance that knows its public address.
fn push_state_at(pool: PgPool, vapid_private: Option<&str>, public_url: Option<&str>) -> AppState {
    push_state_for(pool, vapid_private, public_url, 2)
}

/// As [`push_state_at`], with sessions that live `ttl_hours`; the idle
/// window is the default one, or as long where that is shorter.
fn push_state_for(
    pool: PgPool,
    vapid_private: Option<&str>,
    public_url: Option<&str>,
    ttl_hours: u64,
) -> AppState {
    let redis = common::redis_url();
    let private = vapid_private.map(str::to_owned);
    let public_url = public_url.map(str::to_owned);
    let config = Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(redis.clone()),
        "API_SESSION_TTL_HOURS" => Some(ttl_hours.to_string()),
        "API_WS_HEARTBEAT_MS" => Some("40".to_owned()),
        "API_WS_DEAD_MS" => Some(common::WS_DEAD_MS.to_string()),
        "VAPID_PRIVATE_KEY" => private.clone(),
        "VAPID_SUBJECT" => Some("mailto:admin@test.example".to_owned()),
        "PUSH_ALLOWED_HOSTS" => Some("127.0.0.1".to_owned()),
        "APP_PUBLIC_URL" => public_url.clone(),
        _ => None,
    })
    .expect("push test config");
    let mut state = AppState::with_pool(&config, pool).expect("state");
    // The last-call tests run its sweep by hand.
    state.workers.last_call = None;
    state
}

/// A browser's subscription keys; the private half decrypts what it gets.
struct Browser {
    endpoint: String,
    private: String,
    public: String,
    auth: String,
}

impl Browser {
    fn new(service: &str, path: &str) -> Self {
        let (private, public) = Vapid::generate();
        Self {
            endpoint: format!("{service}/{path}"),
            private,
            public,
            auth: B64.encode(rand::random::<[u8; 16]>()),
        }
    }

    fn json(&self) -> Value {
        json!({"endpoint": self.endpoint, "keys": {"p256dh": self.public, "auth": self.auth}})
    }

    /// The push message as it arrives, declared notification and all.
    fn message(&self, hit: &Hit) -> Value {
        let plain = decrypt(&self.private, &self.auth, &hit.body).expect("decrypts");
        serde_json::from_slice(&plain).unwrap()
    }

    /// What the service worker reads of it (`pushed` in `web/public/sw.js`):
    /// the declared notification's `data`, with its `title` and `body`.
    fn open(&self, hit: &Hit) -> Value {
        let message = self.message(hit);
        assert_eq!(message["web_push"], 8030, "{message}");
        let declared = &message["notification"];
        let mut read = declared["data"].clone();
        assert!(read.is_object(), "{message}");
        read["title"] = declared["title"].clone();
        read["body"] = declared["body"].clone();
        read
    }
}

struct World {
    state: AppState,
    a: Client,
    b: Client,
    server: String,
    channel: String,
    dm: String,
    hits: mpsc::UnboundedReceiver<Hit>,
    service: String,
    vapid_public: String,
}

async fn user(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let r = client.register(email, "password123", name).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    client
}

async fn setup(pool: PgPool) -> World {
    setup_at(pool, None).await
}

async fn setup_at(pool: PgPool, public_url: Option<&str>) -> World {
    setup_for(pool, public_url, 2).await
}

/// A world whose sessions live 30 days and end after 14 unused, as a real
/// one's do.
async fn setup_long(pool: PgPool) -> World {
    setup_for(pool, None, 30 * 24).await
}

async fn setup_for(pool: PgPool, public_url: Option<&str>, ttl_hours: u64) -> World {
    let (private, vapid_public) = Vapid::generate();
    let state = push_state_for(pool, Some(&private), public_url, ttl_hours);
    let (service, hits) = fake_service().await;
    let mut a = user(&state, "a@test.example", "Ada").await;
    let mut b = user(&state, "b@test.example", "Bob").await;
    let server = a
        .send(Method::POST, "/api/servers", Some(json!({"name":"Chat"})))
        .await;
    let sid = server.body["id"].as_str().unwrap().to_owned();
    let cid = server.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let invite = a
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let joined = b
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    a.befriend(b.user_id()).await;
    let dm = a
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({"user_id": b.user_id()})),
        )
        .await;
    assert!(dm.status.is_success(), "{}", dm.body);
    World {
        state,
        a,
        b,
        server: sid,
        channel: cid,
        dm: dm.body["id"].as_str().unwrap().to_owned(),
        hits,
        service,
        vapid_public,
    }
}

async fn subscribe(client: &mut Client, browser: &Browser) {
    let r = client
        .send(
            Method::POST,
            "/api/push/subscriptions",
            Some(browser.json()),
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
}

async fn post(client: &mut Client, channel: &str, text: &str) -> Value {
    let r = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({"content": text})),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body
}

async fn next(hits: &mut mpsc::UnboundedReceiver<Hit>) -> Hit {
    tokio::time::timeout(Duration::from_secs(5), hits.recv())
        .await
        .expect("a push arrives")
        .unwrap()
}

async fn subscriptions(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM push_subscriptions")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn without_a_key_there_is_no_push(pool: PgPool) {
    let state = push_state(pool, None);
    let mut a = user(&state, "a@test.example", "Ada").await;
    let key = a.send(Method::GET, "/api/push/key", None).await;
    assert_eq!(key.status, StatusCode::OK);
    assert_eq!(key.body, json!({"key": null}));
    let browser = Browser::new("http://127.0.0.1:9", "x");
    let r = a
        .send(
            Method::POST,
            "/api/push/subscriptions",
            Some(browser.json()),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn a_subscription_is_checked_and_can_be_removed(pool: PgPool) {
    let World {
        mut a,
        service,
        vapid_public,
        ..
    } = setup(pool.clone()).await;
    let key = a.send(Method::GET, "/api/push/key", None).await;
    assert_eq!(key.body["key"], vapid_public);

    let good = Browser::new(&service, "sub/1");
    for (body, field) in [
        (
            json!({"endpoint": "https://evil.example/x", "keys": good.json()["keys"]}),
            "endpoint",
        ),
        (
            json!({"endpoint": "http://10.0.0.1/x", "keys": good.json()["keys"]}),
            "endpoint",
        ),
        (
            json!({"endpoint": good.endpoint, "keys": {"p256dh": "nope", "auth": good.auth}}),
            "keys",
        ),
    ] {
        let r = a
            .send(Method::POST, "/api/push/subscriptions", Some(body.clone()))
            .await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(r.body.to_string().contains(field), "{}", r.body);
    }
    assert_eq!(subscriptions(&pool).await, 0);
    subscribe(&mut a, &good).await;
    // Again: still one.
    subscribe(&mut a, &good).await;
    assert_eq!(subscriptions(&pool).await, 1);
    let r = a
        .send(
            Method::DELETE,
            "/api/push/subscriptions",
            Some(json!({"endpoint": good.endpoint})),
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(subscriptions(&pool).await, 0);
}

#[sqlx::test]
async fn a_dm_pushes_an_encrypted_preview(pool: PgPool) {
    let World {
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        vapid_public,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let message = post(
        &mut a,
        &dm,
        &format!("hallo <@{}>,\n  wie gehts?", b.user_id()),
    )
    .await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert_eq!(hit.headers["content-encoding"], "aes128gcm");
    assert_eq!(hit.headers["urgency"], "high");
    assert!(hit.headers["ttl"].to_str().unwrap().parse::<u32>().unwrap() > 0);
    let auth = hit.headers["authorization"].to_str().unwrap();
    let (token, key) = auth
        .strip_prefix("vapid t=")
        .unwrap()
        .split_once(", k=")
        .unwrap();
    assert_eq!(key, vapid_public);
    let claims: Value =
        serde_json::from_slice(&B64.decode(token.split('.').nth(1).unwrap()).unwrap()).unwrap();
    assert_eq!(claims["aud"], service);
    assert_eq!(claims["sub"], "mailto:admin@test.example");

    let payload = browser.open(&hit);
    assert_eq!(payload["title"], "Ada");
    assert_eq!(payload["body"], "hallo @Bob, wie gehts?");
    assert_eq!(payload["path"], format!("/d/{dm}"));
    assert_eq!(payload["user"], b.user_id());
    assert_eq!(payload["channel"], dm);
    assert_eq!(payload["message"], message["id"]);
    // Never to the author.
    let ada = Browser::new(&service, "sub/ada");
    subscribe(&mut a, &ada).await;
    post(&mut a, &dm, "noch was").await;
    assert_eq!(next(&mut hits).await.path, "sub/bob");
}

async fn level(client: &mut Client, scope: &str, id: &str, level: Value) {
    let r = client
        .send(
            Method::PUT,
            &format!("/api/{scope}/{id}/notification-settings"),
            Some(json!({ "level": level })),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
}

async fn channel(client: &mut Client, server: &str, name: &str) -> String {
    let r = client
        .send(
            Method::POST,
            &format!("/api/servers/{server}/channels"),
            Some(json!({ "name": name })),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body["id"].as_str().unwrap().to_owned()
}

#[sqlx::test]
async fn a_push_declares_its_notification(pool: PgPool) {
    let World {
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();
    let message = post(&mut a, &dm, "hallo").await;
    let id = message["id"].as_str().unwrap();
    let payload = browser.message(&next(&mut hits).await);
    // A declarative push message: a browser that knows them shows this
    // without the worker's help.
    assert_eq!(payload["web_push"], 8030);
    assert_eq!(payload["mutable"], true);
    assert_eq!(
        payload["notification"],
        json!({
            "title": "Ada",
            "body": "hallo",
            // No public address is set: the path, for the app's own.
            "navigate": format!("/d/{dm}"),
            "tag": format!("gelabber:{bob}:{dm}"),
            "lang": "de",
            "renotify": true,
            "silent": false,
            "data": {
                "path": format!("/d/{dm}"),
                "user": bob,
                "channel": dm,
                "message": id,
                "at": message["created_at"],
            },
        })
    );

    // A recall replaces it without a sound.
    let r = a
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    let recalled = browser.message(&next(&mut hits).await);
    let declared = &recalled["notification"];
    assert_eq!(declared["body"], "Nachricht gelöscht");
    assert_eq!(declared["tag"], format!("gelabber:{bob}:{dm}"));
    assert_eq!(declared["renotify"], false);
    assert_eq!(declared["silent"], true);
    assert_eq!(declared["data"]["recall"], true);
    assert_eq!(declared["data"]["message"], id);
}

#[sqlx::test]
async fn a_declared_notification_leads_to_the_public_address(pool: PgPool) {
    let World {
        mut a,
        mut b,
        server,
        channel,
        mut hits,
        service,
        ..
    } = setup_at(pool, Some("https://chat.test.example/")).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    post(&mut a, &channel, "plain").await;
    let payload = browser.message(&next(&mut hits).await);
    assert_eq!(
        payload["notification"]["navigate"],
        format!("https://chat.test.example/s/{server}/c/{channel}")
    );
    // The worker still gets the path: it opens it in the running app.
    assert_eq!(
        payload["notification"]["data"]["path"],
        format!("/s/{server}/c/{channel}")
    );
}

#[sqlx::test]
async fn in_a_server_the_level_decides_what_pushes(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        channel,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();
    // "All" is what a member has without a setting: a message that names
    // nobody pushes too, without waking a sleeping phone for it.
    post(&mut a, &channel, "plain").await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.headers["urgency"], "normal");
    let payload = browser.open(&hit);
    assert_eq!(payload["title"], "Ada · #allgemein");
    assert_eq!(payload["body"], "plain");
    assert_eq!(payload["path"], format!("/s/{server}/c/{channel}"));

    // At "mentions" it does not. The DM after it is the next to arrive.
    let second = self::channel(&mut a, &server, "zwei").await;
    level(&mut b, "servers", &server, json!("mentions")).await;
    post(&mut a, &second, "plain").await;
    settled(&state).await;
    post(&mut a, &dm, "marker 1").await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.headers["urgency"], "high");
    assert_eq!(browser.open(&hit)["body"], "marker 1");
    // A channel's own "all" wins over its server's "mentions".
    let third = self::channel(&mut a, &server, "drei").await;
    level(&mut b, "channels", &third, json!("all")).await;
    post(&mut a, &third, "hier schon").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "hier schon");

    post(&mut a, &channel, &format!("<@{bob}> schau mal")).await;
    let hit = next(&mut hits).await;
    // A mention wakes the phone, as a direct message does.
    assert_eq!(hit.headers["urgency"], "high");
    let payload = browser.open(&hit);
    assert_eq!(payload["title"], "Ada · #allgemein");
    assert_eq!(payload["path"], format!("/s/{server}/c/{channel}"));
    post(&mut a, &channel, "@everyone Essen").await;
    assert_eq!(
        browser.open(&next(&mut hits).await)["body"],
        "@everyone Essen"
    );

    // Muted channel, then a server set to nothing: no push.
    let r = b
        .send(
            Method::PUT,
            &format!("/api/channels/{channel}/notification-settings"),
            Some(json!({"muted": true})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    post(&mut a, &channel, &format!("<@{bob}> stumm")).await;
    // The push is sent in the background; let it pass the mute check
    // before the mute is lifted.
    settled(&state).await;
    post(&mut a, &dm, "marker 1b").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 1b");
    b.send(
        Method::PUT,
        &format!("/api/channels/{channel}/notification-settings"),
        Some(json!({"muted": false})),
    )
    .await;
    b.send(
        Method::PUT,
        &format!("/api/servers/{server}/notification-settings"),
        Some(json!({"level": "none"})),
    )
    .await;
    post(&mut a, &channel, &format!("<@{bob}> nichts")).await;
    settled(&state).await;
    post(&mut a, &dm, "marker 2").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 2");
}

#[sqlx::test]
async fn hidden_content_pushes_only_who_wrote(pool: PgPool) {
    let World {
        mut a,
        mut b,
        server,
        channel,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();

    let r = b.send(Method::GET, "/api/notification-privacy", None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body, json!({"hide_content": false}));
    let r = b
        .send(Method::PUT, "/api/notification-privacy", Some(json!({})))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
    let r = b
        .send(
            Method::PUT,
            "/api/notification-privacy",
            Some(json!({"hide_content": true})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body, json!({"hide_content": true}));
    let r = b.send(Method::GET, "/api/notification-privacy", None).await;
    assert_eq!(r.body, json!({"hide_content": true}));
    // Someone else's switch stays as it was.
    let r = a.send(Method::GET, "/api/notification-privacy", None).await;
    assert_eq!(r.body, json!({"hide_content": false}));

    let message = post(&mut a, &dm, "geheim").await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["title"], "Neue Nachricht von Ada");
    assert_eq!(payload["body"], "");
    // A tap still leads to the conversation, and the page still dedupes.
    assert_eq!(payload["path"], format!("/d/{dm}"));
    assert_eq!(payload["message"], message["id"]);

    post(&mut a, &channel, &format!("<@{bob}> auch geheim")).await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["title"], "Neue Nachricht von Ada");
    assert_eq!(payload["body"], "");
    assert_eq!(payload["path"], format!("/s/{server}/c/{channel}"));

    b.send(
        Method::PUT,
        "/api/notification-privacy",
        Some(json!({"hide_content": false})),
    )
    .await;
    post(&mut a, &dm, "wieder lesbar").await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["title"], "Ada");
    assert_eq!(payload["body"], "wieder lesbar");
}

#[sqlx::test]
async fn a_gone_subscription_is_removed_and_a_logout_ends_pushes(pool: PgPool) {
    let World {
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    subscribe(&mut b, &Browser::new(&service, "gone/1")).await;
    post(&mut a, &dm, "eins").await;
    assert_eq!(next(&mut hits).await.path, "gone/1");
    let mut left = 1;
    for _ in 0..50 {
        left = subscriptions(&pool).await;
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(left, 0);

    subscribe(&mut b, &Browser::new(&service, "sub/2")).await;
    let r = b.send(Method::POST, "/api/auth/logout", None).await;
    assert!(r.status.is_success(), "{}", r.body);
    assert_eq!(subscriptions(&pool).await, 0);
}

/// Someone who only knows an endpoint cannot move it to their account; the
/// same browser signing in as someone else (same keys) can.
#[sqlx::test]
async fn an_endpoint_moves_only_with_its_keys(pool: PgPool) {
    let World {
        mut a,
        mut b,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let thief = Browser::new(&service, "sub/bob");
    subscribe(&mut a, &thief).await;
    let (owner, key): (Uuid, String) =
        sqlx::query_as("SELECT user_id, p256dh FROM push_subscriptions")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(owner.to_string(), b.user_id());
    assert_eq!(key, browser.public);
    subscribe(&mut a, &browser).await;
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM push_subscriptions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner.to_string(), a.user_id());
}

/// A subscription that keeps failing goes once it has not taken a push for
/// a month and was not re-registered since; a recent one stays, and an
/// outage on the service's side (5xx) costs none.
#[sqlx::test]
async fn a_long_failing_subscription_is_removed(pool: PgPool) {
    let World {
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    for path in ["big/old", "big/new", "big/resynced", "fail/old", "sub/1"] {
        subscribe(&mut b, &Browser::new(&service, path)).await;
    }
    sqlx::query(
        "UPDATE push_subscriptions SET created_at=now()-interval '40 days' \
         WHERE endpoint LIKE '%/old'",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Took its last push long ago, but the app re-subscribed it just now.
    sqlx::query(
        "UPDATE push_subscriptions SET last_ok_at=now()-interval '40 days' \
         WHERE endpoint LIKE '%/resynced'",
    )
    .execute(&pool)
    .await
    .unwrap();
    post(&mut a, &dm, "eins").await;
    for _ in 0..5 {
        next(&mut hits).await;
    }
    let left = endpoints_left(&pool, 4).await;
    assert_eq!(left.len(), 4, "{left:?}");
    assert!(!left.iter().any(|e| e.ends_with("/big/old")), "{left:?}");
}

/// When no push of a message went out, the failures are more likely the
/// server's own (DNS, egress): even a month-old subscription stays.
#[sqlx::test]
async fn a_message_no_service_took_costs_no_subscription(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    for path in ["big/old", "fail/old", "gone/1"] {
        subscribe(&mut b, &Browser::new(&service, path)).await;
    }
    sqlx::query(
        "UPDATE push_subscriptions SET created_at=now()-interval '40 days' \
         WHERE endpoint LIKE '%/old'",
    )
    .execute(&pool)
    .await
    .unwrap();
    post(&mut a, &dm, "eins").await;
    for _ in 0..3 {
        next(&mut hits).await;
    }
    let left = endpoints_settled(&state, &pool).await;
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left[0].ends_with("/big/old"), "{left:?}");
    assert!(left[1].ends_with("/fail/old"), "{left:?}");
}

async fn endpoints_left(pool: &PgPool, expected: usize) -> Vec<String> {
    let mut left = Vec::new();
    for _ in 0..50 {
        left = sqlx::query_scalar::<_, String>(
            "SELECT endpoint FROM push_subscriptions ORDER BY endpoint",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        if left.len() == expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    left
}

/// The endpoints left once every push decided so far is done ([`settled`]):
/// a delivery deletes the gone and failed rows before it ends.
async fn endpoints_settled(state: &AppState, pool: &PgPool) -> Vec<String> {
    settled(state).await;
    sqlx::query_scalar::<_, String>("SELECT endpoint FROM push_subscriptions ORDER BY endpoint")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// A service that refuses a subscription (400, 403 after a key change)
/// while it takes another push of the same message will not take it later
/// either: it goes on that refusal, while a recent one that only failed
/// stays.
#[sqlx::test]
async fn a_refused_subscription_is_removed_at_once(pool: PgPool) {
    let World {
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    for path in ["bad/1", "denied/1", "fail/1", "sub/1"] {
        subscribe(&mut b, &Browser::new(&service, path)).await;
    }
    post(&mut a, &dm, "eins").await;
    for _ in 0..4 {
        next(&mut hits).await;
    }
    let left = endpoints_left(&pool, 2).await;
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left[0].ends_with("/fail/1"), "{left:?}");
    assert!(left[1].ends_with("/sub/1"), "{left:?}");
}

/// Refused by the service across the board, the fault is the server's (a
/// rejected VAPID token, a clock off): no subscription goes, not even one
/// silent for a month.
#[sqlx::test]
async fn a_refusal_of_every_push_keeps_the_subscriptions(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    for path in ["denied/new", "denied/old", "gone/1"] {
        subscribe(&mut b, &Browser::new(&service, path)).await;
    }
    sqlx::query(
        "UPDATE push_subscriptions SET created_at=now()-interval '40 days' \
         WHERE endpoint LIKE '%/denied/old'",
    )
    .execute(&pool)
    .await
    .unwrap();
    post(&mut a, &dm, "eins").await;
    for _ in 0..3 {
        next(&mut hits).await;
    }
    // The gone one goes; both refused ones stay.
    let left = endpoints_settled(&state, &pool).await;
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left[0].ends_with("/denied/new"), "{left:?}");
    assert!(left[1].ends_with("/denied/old"), "{left:?}");
}

#[sqlx::test]
async fn a_lively_channel_pushes_once_in_a_while(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        channel,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();
    post(&mut a, &channel, "eins").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "eins");
    // The next lines of the conversation stay quiet; the DM is the next
    // push to arrive.
    post(&mut a, &channel, "zwei").await;
    post(&mut a, &channel, "drei").await;
    settled(&state).await;
    post(&mut a, &dm, "marker").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker");
    // A mention is not held back by the lines before it.
    post(&mut a, &channel, &format!("<@{bob}> vier")).await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "@Bob vier");
}

/// Adds members to the server until it has `total`, without accounts
/// anyone could sign in to.
async fn crowd(pool: &PgPool, server: &str, total: i64) {
    let server = Uuid::parse_str(server).unwrap();
    sqlx::query(
        "WITH people AS (
            INSERT INTO users (email, name, password_hash)
            SELECT 'crowd-' || gen_random_uuid() || '@test.example', 'Crowd ' || n, 'unused'
            FROM generate_series(1, $2 - (SELECT count(*) FROM server_members WHERE server_id=$1)) AS n
            RETURNING id
         )
         INSERT INTO server_members (server_id, user_id) SELECT $1, id FROM people",
    )
    .bind(server)
    .bind(total)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn a_large_server_pushes_ordinary_messages_only_where_someone_chose_all(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        channel,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();
    // 50 members are still a group: the default "all" pushes.
    crowd(&pool, &server, 50).await;
    post(&mut a, &channel, "klein").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "klein");

    // One more, and a message that names nobody pushes to nobody who did
    // not ask for it.
    crowd(&pool, &server, 51).await;
    let second = self::channel(&mut a, &server, "zwei").await;
    post(&mut a, &second, "gross").await;
    settled(&state).await;
    post(&mut a, &dm, "marker 1").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 1");
    // A mention still does.
    post(&mut a, &second, &format!("<@{bob}> du")).await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "@Bob du");

    // Bob's own "all" for the server brings them back, but not into a
    // channel he set to "mentions".
    level(&mut b, "servers", &server, json!("all")).await;
    let third = self::channel(&mut a, &server, "drei").await;
    post(&mut a, &third, "gewollt").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "gewollt");
    let fourth = self::channel(&mut a, &server, "vier").await;
    level(&mut b, "channels", &fourth, json!("mentions")).await;
    post(&mut a, &fourth, "nicht hier").await;
    settled(&state).await;
    post(&mut a, &dm, "marker 2").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 2");
    // An "all" on one channel alone is enough for that channel.
    level(&mut b, "servers", &server, Value::Null).await;
    let fifth = self::channel(&mut a, &server, "fuenf").await;
    level(&mut b, "channels", &fifth, json!("all")).await;
    post(&mut a, &fifth, "nur hier").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "nur hier");
}

/// Until every push decided so far has gone out or been dropped: what
/// was not pushed by then is not pushed at all.
async fn settled(state: &AppState) {
    let push = state.push.as_ref().expect("push on");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while push.deliveries_under_way() > 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "pushes still under way after 5 s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn focus_is(state: &AppState, user: &str, on: bool) {
    let user = Uuid::parse_str(user).unwrap();
    for _ in 0..100 {
        if state.gateway.has_focus(user).await.unwrap() == on {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("focus never became {on}");
}

#[sqlx::test]
async fn a_page_in_front_holds_pushes_back(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        channel,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .unwrap();

    let mut ws = ws::connect(addr, &b).await;
    ws::send(&mut ws, json!({"op":"f","on":true})).await;
    focus_is(&state, b.user_id(), true).await;
    // In front, but showing something else: the page does not subscribe to
    // this conversation, so the push still goes.
    post(&mut a, &dm, "anderswo").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "anderswo");

    // A page in front that follows the conversation holds the push back.
    ws::send(&mut ws, json!({"op":"s","s":dm,"c":dm})).await;
    ws::until(&mut ws, |f| f["op"] == "ok").await;
    post(&mut a, &dm, "gesehen").await;
    // The push runs after the answer; it has looked at the focus once it
    // is settled.
    settled(&state).await;
    ws::send(&mut ws, json!({"op":"f","on":false})).await;
    focus_is(&state, b.user_id(), false).await;
    post(&mut a, &dm, "weg").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "weg");

    // A closed socket gives its focus up.
    ws::send(&mut ws, json!({"op":"f","on":true})).await;
    focus_is(&state, b.user_id(), true).await;
    // While it is in front, wherever: an ordinary message of a server
    // channel stays off the phone, a mention there does not.
    post(&mut a, &channel, "nebenbei").await;
    let bob = b.user_id().to_owned();
    post(&mut a, &channel, &format!("<@{bob}> doch")).await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "@Bob doch");
    drop(ws);
    focus_is(&state, b.user_id(), false).await;
}

async fn receipts(pool: &PgPool, message: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM push_receipts WHERE message_id=$1::uuid")
        .bind(message)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The receipts are written once the push services answered, after the
/// pushes themselves arrived.
async fn receipts_are(pool: &PgPool, message: &str, expected: i64) {
    for _ in 0..100 {
        if receipts(pool, message).await == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{expected} receipts expected for {message}");
}

#[sqlx::test]
async fn a_deleted_message_recalls_its_push(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    subscribe(&mut b, &Browser::new(&service, "fail/bob")).await;
    let message = post(&mut a, &dm, "geheim").await;
    let id = message["id"].as_str().unwrap();
    let topic = Uuid::parse_str(id).unwrap().simple().to_string();
    let mut first = [next(&mut hits).await, next(&mut hits).await];
    first.sort_by(|x, y| x.path.cmp(&y.path));
    assert_eq!(first[1].path, "sub/bob");
    assert_eq!(first[1].headers["topic"], topic.as_str());
    assert_eq!(browser.open(&first[1])["body"], "geheim");
    // Only the subscription whose service took it.
    receipts_are(&pool, id, 1).await;

    let r = a
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    // The same topic: a push service still holding the first replaces it.
    assert_eq!(hit.headers["topic"], topic.as_str());
    let payload = browser.open(&hit);
    assert_eq!(payload["recall"], true);
    assert_eq!(payload["body"], "Nachricht gelöscht");
    assert_eq!(payload["title"], "Ada");
    assert_eq!(payload["message"], id);
    assert_eq!(payload["channel"], dm.as_str());
    assert_eq!(payload["user"], b.user_id());
    assert_eq!(receipts(&pool, id).await, 0);

    // Past the TTL the push service holds nothing any more: no recall.
    let old = post(&mut a, &dm, "alt").await;
    let old = old["id"].as_str().unwrap();
    next(&mut hits).await;
    next(&mut hits).await;
    receipts_are(&pool, old, 1).await;
    sqlx::query("UPDATE push_receipts SET sent_at=now() - interval '2 hours'")
        .execute(&pool)
        .await
        .unwrap();
    let r = a
        .send(Method::DELETE, &format!("/api/messages/{old}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    settled(&state).await;
    assert!(hits.try_recv().is_err(), "no recall past the TTL");
}

#[sqlx::test]
async fn a_recall_shows_who_wrote_only_where_content_is_hidden(pool: PgPool) {
    let World {
        mut a,
        mut b,
        server,
        channel,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let message = post(&mut a, &channel, &format!("<@{}> geheim", b.user_id())).await;
    let id = message["id"].as_str().unwrap();
    next(&mut hits).await;
    receipts_are(&pool, id, 1).await;
    sqlx::query("UPDATE users SET notifications_hide_content=true WHERE id=$1::uuid")
        .bind(b.user_id())
        .execute(&pool)
        .await
        .unwrap();
    let r = a
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["recall"], true);
    assert_eq!(payload["title"], "Neue Nachricht von Ada");
    assert_eq!(payload["body"], "Nachricht gelöscht");
    assert_eq!(payload["path"], format!("/s/{server}/c/{channel}"));
}

/// Issue #515: a push is in its recipient's language, also a recall.
#[sqlx::test]
async fn a_push_speaks_the_recipient_language(pool: PgPool) {
    let World {
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let bob_browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &bob_browser).await;
    let ada_browser = Browser::new(&service, "sub/ada");
    subscribe(&mut a, &ada_browser).await;
    let r = b
        .send(
            Method::PUT,
            "/api/me/locale",
            Some(json!({ "locale": "en" })),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    sqlx::query("UPDATE users SET notifications_hide_content=true")
        .execute(&pool)
        .await
        .unwrap();

    let message = post(&mut a, &dm, "secret").await;
    let id = message["id"].as_str().unwrap();
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    let payload = bob_browser.open(&hit);
    assert_eq!(payload["title"], "New message from Ada");
    assert_eq!(payload["body"], "");
    receipts_are(&pool, id, 1).await;
    let r = a
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    let payload = bob_browser.open(&next(&mut hits).await);
    assert_eq!(payload["recall"], true);
    assert_eq!(payload["title"], "New message from Ada");
    assert_eq!(payload["body"], "Message deleted");

    // Ada still reads German.
    let message = post(&mut b, &dm, "geheim").await;
    let id = message["id"].as_str().unwrap();
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/ada");
    let payload = ada_browser.open(&hit);
    assert_eq!(payload["title"], "Neue Nachricht von Bob");
    receipts_are(&pool, id, 1).await;
    let r = b
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    let payload = ada_browser.open(&next(&mut hits).await);
    assert_eq!(payload["title"], "Neue Nachricht von Bob");
    assert_eq!(payload["body"], "Nachricht gelöscht");
}

#[sqlx::test]
async fn a_hidden_channel_pushes_nothing(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        channel,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    // The channel is limited to a role Bob does not hold (#516).
    let role = a
        .send(
            Method::POST,
            &format!("/api/servers/{server}/roles"),
            Some(json!({"name": "Stammgäste", "color": "green"})),
        )
        .await;
    assert_eq!(role.status, StatusCode::CREATED, "{}", role.body);
    let r = a
        .send(
            Method::PATCH,
            &format!("/api/channels/{channel}"),
            Some(json!({"role_ids": [role.body["id"]]})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);

    let bob = b.user_id().to_owned();
    post(&mut a, &channel, "@everyone Essen").await;
    post(&mut a, &channel, &format!("<@{bob}> schau mal")).await;
    settled(&state).await;
    post(&mut a, &dm, "marker").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker");
}

/// A message request rings once, however its messages are deleted and
/// sent again, and a friend request taken back and sent again within the
/// hour rings no second time (#563).
#[sqlx::test]
async fn requests_ring_once(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let mut cara = user(&state, "c@test.example", "Cara").await;
    let invite = a
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let joined = cara
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();

    let request = cara
        .send(Method::POST, "/api/dms", Some(json!({"user_id": bob})))
        .await;
    let request = request.body["id"].as_str().unwrap().to_owned();
    let first = post(&mut cara, &request, "hallo").await;
    assert_eq!(browser.open(&next(&mut hits).await)["path"], "/d/requests");
    let id = first["id"].as_str().unwrap();
    let r = cara
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    post(&mut cara, &request, "hallo nochmal").await;

    let send = json!({"user_id": bob});
    let r = cara
        .send(Method::POST, "/api/friends/requests", Some(send.clone()))
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    assert_eq!(browser.open(&next(&mut hits).await)["path"], "/d/friends");
    let r = cara
        .send(
            Method::DELETE,
            &format!("/api/friends/requests/{bob}"),
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);
    let r = cara
        .send(Method::POST, "/api/friends/requests", Some(send))
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);

    settled(&state).await;
    post(&mut a, &dm, "marker").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker");
}

/// Nothing from someone the recipient blocks (#565): not their mentions,
/// not `@everyone`. Cara's mention after them is the only push.
#[sqlx::test]
async fn nothing_pushes_from_someone_blocked(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        channel,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let mut c = user(&state, "c@test.example", "Cara").await;
    let invite = a
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap().to_owned();
    let joined = c
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();
    let r = b
        .send(
            Method::PUT,
            &format!("/api/users/me/blocks/{}", a.user_id()),
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);

    post(&mut a, &channel, &format!("<@{bob}> hallo")).await;
    post(&mut a, &channel, "@everyone Essen").await;
    settled(&state).await;
    post(&mut c, &channel, &format!("<@{bob}> marker")).await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["body"], "@Bob marker");
    settled(&state).await;
    assert!(hits.try_recv().is_err(), "nothing from Ada");
}

/// Do not disturb (#590) holds back every push, DMs and mentions alike,
/// until it is lifted or runs out.
#[sqlx::test]
async fn do_not_disturb_holds_back_every_push_until_it_ends(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        channel,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let bob_browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &bob_browser).await;
    let ada_browser = Browser::new(&service, "sub/ada");
    subscribe(&mut a, &ada_browser).await;
    let bob = b.user_id().to_owned();

    let r = b
        .send(
            Method::PUT,
            "/api/me/presence",
            Some(json!({"mode": "dnd", "duration": "1h"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    post(&mut a, &dm, "leise dm").await;
    post(&mut a, &channel, &format!("<@{bob}> leise erwähnt")).await;
    post(&mut a, &channel, "@everyone leise alle").await;
    // Bob's pushes are decided in the background; Ada's marker lets them
    // pass the check before the mode ends.
    settled(&state).await;
    post(&mut b, &dm, "marker").await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/ada");
    assert_eq!(ada_browser.open(&hit)["body"], "marker");

    // Runs out: pushes resume, though nobody swept yet.
    sqlx::query("UPDATE users SET presence_mode_until = now() - interval '1 second' WHERE id = $1")
        .bind(Uuid::parse_str(&bob).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    post(&mut a, &dm, "wieder laut").await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert_eq!(bob_browser.open(&hit)["body"], "wieder laut");

    // Lifted by hand after a new one.
    b.send(
        Method::PUT,
        "/api/me/presence",
        Some(json!({"mode": "dnd", "duration": "forever"})),
    )
    .await;
    post(&mut a, &dm, "wieder leise").await;
    settled(&state).await;
    post(&mut b, &dm, "marker 2").await;
    assert_eq!(ada_browser.open(&next(&mut hits).await)["body"], "marker 2");
    let r = b
        .send(
            Method::PUT,
            "/api/me/presence",
            Some(json!({"mode": "auto"})),
        )
        .await;
    assert_eq!(r.body, json!({"mode": "auto", "until": null}));
    post(&mut a, &dm, "laut").await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert_eq!(bob_browser.open(&hit)["body"], "laut");
}

/// `@here` pushes to an invisible member it reached (GEL-43), although the
/// message others read does not name them.
#[sqlx::test]
async fn here_pushes_to_an_invisible_member(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        channel,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    // Only what names Bob pushes, not every message of the server.
    level(&mut b, "servers", &server, json!("mentions")).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .unwrap();
    let r = b
        .send(
            Method::PUT,
            "/api/me/presence",
            Some(json!({"mode": "invisible"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let mut ws = ws::connect(addr, &b).await;
    ws::send(&mut ws, json!({"op":"s","s":server})).await;
    let server_id = Uuid::parse_str(&server).unwrap();
    let bob = Uuid::parse_str(b.user_id()).unwrap();
    for _ in 0..100 {
        let present = state.gateway.present_members(server_id).await.unwrap();
        if present.iter().any(|entry| entry.u == bob) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let message = post(&mut a, &channel, "@here kurz?").await;
    assert_eq!(message["mentions"], json!([]));
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert_eq!(browser.open(&hit)["body"], "@here kurz?");
    drop(ws);
}

/// Do not disturb (#590) holds back friend and message requests (#563)
/// too.
#[sqlx::test]
async fn do_not_disturb_holds_back_requests(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool).await;
    let mut cara = user(&state, "c@test.example", "Cara").await;
    let invite = a
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let joined = cara
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    subscribe(&mut b, &Browser::new(&service, "sub/bob")).await;
    let ada_browser = Browser::new(&service, "sub/ada");
    subscribe(&mut a, &ada_browser).await;
    let bob = b.user_id().to_owned();
    let r = b
        .send(
            Method::PUT,
            "/api/me/presence",
            Some(json!({"mode": "dnd", "duration": "8h"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);

    let r = cara
        .send(
            Method::POST,
            "/api/friends/requests",
            Some(json!({"user_id": bob})),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let request = cara
        .send(Method::POST, "/api/dms", Some(json!({"user_id": bob})))
        .await;
    let request = request.body["id"].as_str().unwrap().to_owned();
    post(&mut cara, &request, "hallo").await;
    // Bob's marker reaches Ada; nothing reached Bob before it.
    settled(&state).await;
    post(&mut b, &dm, "marker").await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/ada");
    assert_eq!(ada_browser.open(&hit)["body"], "marker");
}

/// Moves the end of every session of `user` to `ends` from now (an
/// interval), as time passing without a use would.
async fn session_ends_in(pool: &PgPool, user: &str, ends: &str) {
    let done = sqlx::query(
        "UPDATE sessions SET expires_at = now() + $2::text::interval WHERE user_id = $1::uuid",
    )
    .bind(user)
    .bind(ends)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(done.rows_affected(), 1);
}

async fn warned(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM push_subscriptions WHERE warned_at IS NOT NULL")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A session nobody used takes its push subscription with it (#616): two
/// days before, the device hears that once, and again only after a use.
#[sqlx::test]
async fn a_session_about_to_end_says_so_once(pool: PgPool) {
    use gelabber_api::push::last_call::sweep;
    let World {
        state,
        mut a,
        mut b,
        mut hits,
        service,
        ..
    } = setup_long(pool.clone()).await;
    let bob_browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &bob_browser).await;
    subscribe(&mut a, &Browser::new(&service, "sub/ada")).await;
    let bob = b.user_id().to_owned();

    // Fresh, and with three days left: nothing to say.
    assert_eq!(sweep(&state).await.unwrap(), 0);
    session_ends_in(&pool, &bob, "3 days").await;
    assert_eq!(sweep(&state).await.unwrap(), 0);

    session_ends_in(&pool, &bob, "47 hours").await;
    assert_eq!(sweep(&state).await.unwrap(), 1);
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert_eq!(hit.headers["urgency"], "normal");
    // Kept by the push service while the session lasts.
    let ttl: i64 = hit.headers["ttl"].to_str().unwrap().parse().unwrap();
    assert!((46 * 3600..=47 * 3600).contains(&ttl), "{ttl}");
    let payload = bob_browser.open(&hit);
    assert_eq!(payload["title"], "Benachrichtigungen enden bald");
    assert_eq!(
        payload["body"],
        "Du warst länger nicht hier. Öffne Gelabber, damit Benachrichtigungen auf diesem Gerät weiter ankommen."
    );
    assert_eq!(payload["path"], "/");
    assert_eq!(bob_browser.message(&hit)["notification"]["navigate"], "/");
    // A notification of its own: no conversation's tag.
    assert_eq!(payload["channel"], Uuid::nil().to_string());
    assert_eq!(payload.get("recall"), None);

    // Once.
    assert_eq!(sweep(&state).await.unwrap(), 0);
    assert_eq!(warned(&pool).await, 1);

    // Bob opens the app: the session is renewed, and says so again the next
    // time it runs low.
    let r = b.send(Method::GET, "/api/auth/session", None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(sweep(&state).await.unwrap(), 0);
    session_ends_in(&pool, &bob, "1 hour").await;
    assert_eq!(sweep(&state).await.unwrap(), 1);
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert!(hits.try_recv().is_err());
}

/// At the end of a session's life opening the app no longer helps: the
/// notice asks for a new sign-in, in the reader's language, once.
#[sqlx::test]
async fn at_the_end_of_its_life_a_session_asks_for_a_new_sign_in(pool: PgPool) {
    use gelabber_api::push::last_call::sweep;
    let World {
        state,
        mut b,
        mut hits,
        service,
        ..
    } = setup_long(pool.clone()).await;
    let bob_browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &bob_browser).await;
    let r = b
        .send(
            Method::PUT,
            "/api/me/locale",
            Some(json!({ "locale": "en" })),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    // Signed in 29 days ago and used ever since.
    sqlx::query(
        "UPDATE sessions SET created_at = now() - interval '29 days', \
         expires_at = now() + interval '1 day', last_used_at = now() WHERE user_id = $1::uuid",
    )
    .bind(b.user_id())
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(sweep(&state).await.unwrap(), 1);
    let hit = next(&mut hits).await;
    let payload = bob_browser.open(&hit);
    assert_eq!(payload["title"], "Notifications are about to stop");
    assert_eq!(
        payload["body"],
        "The sign-in on this device ends soon. Sign in again to keep notifications coming."
    );
    assert_eq!(bob_browser.message(&hit)["notification"]["lang"], "en");

    // Using the app renews nothing now, and brings no second notice.
    let r = b.send(Method::GET, "/api/auth/session", None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(sweep(&state).await.unwrap(), 0);
    assert!(hits.try_recv().is_err());

    // A new sign-in on the same browser starts over.
    let mut again = Client::from_state(state.clone());
    again.bootstrap().await;
    let r = again.login("b@test.example", "password123").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    subscribe(&mut again, &bob_browser).await;
    assert_eq!(warned(&pool).await, 0);
    assert_eq!(sweep(&state).await.unwrap(), 0);
}

/// Do not disturb (#590) holds the notice back until it ends; one no push
/// service took comes again.
#[sqlx::test]
async fn a_last_call_waits_for_do_not_disturb_and_is_tried_again(pool: PgPool) {
    use gelabber_api::push::last_call::sweep;
    let World {
        state,
        mut b,
        mut hits,
        service,
        ..
    } = setup_long(pool.clone()).await;
    subscribe(&mut b, &Browser::new(&service, "fail/bob")).await;
    let bob = b.user_id().to_owned();
    let r = b
        .send(
            Method::PUT,
            "/api/me/presence",
            Some(json!({"mode": "dnd", "duration": "8h"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    session_ends_in(&pool, &bob, "10 hours").await;
    assert_eq!(sweep(&state).await.unwrap(), 0);
    assert!(hits.try_recv().is_err());

    sqlx::query(
        "UPDATE users SET presence_mode_until = now() - interval '1 second' WHERE id = $1::uuid",
    )
    .bind(&bob)
    .execute(&pool)
    .await
    .unwrap();
    // The push service is down: asked, but not counted as told.
    assert_eq!(sweep(&state).await.unwrap(), 0);
    assert_eq!(next(&mut hits).await.path, "fail/bob");
    assert_eq!(warned(&pool).await, 0);
    assert_eq!(subscriptions(&pool).await, 1);
    assert_eq!(sweep(&state).await.unwrap(), 0);
    assert_eq!(next(&mut hits).await.path, "fail/bob");
}

/// With the push service down, a round of the worker ends after one batch:
/// it does not claim the same subscriptions again and again until the service
/// is back. The next round tries them again.
#[sqlx::test]
async fn a_last_call_round_ends_while_the_push_service_is_down(pool: PgPool) {
    use gelabber_api::push::last_call::round;
    let World {
        state,
        mut b,
        mut hits,
        service,
        ..
    } = setup_long(pool.clone()).await;
    subscribe(&mut b, &Browser::new(&service, "fail/bob")).await;
    // More than one batch (200) of subscriptions, all of them due.
    sqlx::query(
        "INSERT INTO push_subscriptions (endpoint, user_id, session_hash, p256dh, auth)
         SELECT endpoint || '-' || n, user_id, session_hash, p256dh, auth
         FROM push_subscriptions, generate_series(1, 250) AS n",
    )
    .execute(&pool)
    .await
    .unwrap();
    session_ends_in(&pool, b.user_id(), "10 hours").await;

    let asked = |hits: &mut mpsc::UnboundedReceiver<Hit>| {
        let mut asked = 0;
        while hits.try_recv().is_ok() {
            asked += 1;
        }
        asked
    };
    tokio::time::timeout(Duration::from_secs(30), round(&state))
        .await
        .expect("the round ends");
    assert_eq!(asked(&mut hits), 200);
    assert_eq!(warned(&pool).await, 0);
    assert_eq!(subscriptions(&pool).await, 251);
    tokio::time::timeout(Duration::from_secs(30), round(&state))
        .await
        .expect("the round ends");
    assert_eq!(asked(&mut hits), 200);
}

async fn voice(client: &mut Client, server: &str, name: &str) -> String {
    let r = client
        .send(
            Method::POST,
            &format!("/api/servers/{server}/channels"),
            Some(json!({"name": name, "kind": "voice"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body["id"].as_str().unwrap().to_owned()
}

/// Sends a voice frame and waits until the socket hears it back: the server
/// has handled it then.
async fn sig(ws: &mut Ws, server: &str, channel: &str, t: &str, k: Option<&str>) {
    let mut frame = json!({"op": "sig", "t": t, "s": server, "c": channel});
    if let Some(k) = k {
        frame["k"] = json!(k);
    }
    ws::send(ws, frame).await;
    ws::until(ws, |heard| {
        heard["op"] == "sig"
            && heard["t"] == t
            && heard["c"] == channel
            && k.is_none_or(|k| heard["k"] == k)
    })
    .await;
}

/// Joins the voice channel and goes live there.
async fn go_live(ws: &mut Ws, server: &str, channel: &str) {
    sig(ws, server, channel, "j", None).await;
    sig(ws, server, channel, "p", Some("s")).await;
}

/// A Live reaches the closed apps as the pages announce it (#616): at the
/// server's level "all", once in a while, and not who is in voice there.
#[sqlx::test]
async fn a_live_pushes_at_the_level_all(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    subscribe(&mut a, &Browser::new(&service, "sub/ada")).await;
    let lounge = voice(&mut a, &server, "Lounge").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .unwrap();
    let mut ada = ws::connect(addr, &a).await;

    go_live(&mut ada, &server, &lounge).await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert_eq!(hit.headers["urgency"], "normal");
    // Not kept for long: a Live heard of late is over.
    assert_eq!(hit.headers["ttl"], "600");
    let payload = browser.open(&hit);
    assert_eq!(payload["title"], "Ada · Chat");
    assert_eq!(payload["body"], "Ada ist live in Lounge.");
    assert_eq!(payload["path"], format!("/s/{server}/c/{lounge}"));
    assert_eq!(payload["channel"], lounge);

    // Off and on again within the cooldown: not again.
    sig(&mut ada, &server, &lounge, "u", Some("s")).await;
    sig(&mut ada, &server, &lounge, "p", Some("s")).await;
    settled(&state).await;
    post(&mut a, &dm, "marker 1").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 1");

    // At "mentions" for the server a Live, which names nobody, stays quiet.
    level(&mut b, "servers", &server, json!("mentions")).await;
    let second = voice(&mut a, &server, "Zwei").await;
    go_live(&mut ada, &server, &second).await;
    settled(&state).await;
    post(&mut a, &dm, "marker 2").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 2");

    // With content hidden: who, not where.
    level(&mut b, "servers", &server, Value::Null).await;
    sqlx::query("UPDATE users SET notifications_hide_content=true")
        .execute(&pool)
        .await
        .unwrap();
    let third = voice(&mut a, &server, "Drei").await;
    go_live(&mut ada, &server, &third).await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["title"], "Ada ist live");
    assert_eq!(payload["body"], "");

    // Bob sits in a voice channel of the server: his app shows it.
    let mut bob = ws::connect(addr, &b).await;
    sig(&mut bob, &server, &lounge, "j", None).await;
    let fourth = voice(&mut a, &server, "Vier").await;
    go_live(&mut ada, &server, &fourth).await;
    settled(&state).await;
    post(&mut a, &dm, "marker 3").await;
    assert_eq!(
        browser.open(&next(&mut hits).await)["title"],
        "Neue Nachricht von Ada"
    );
    assert!(hits.try_recv().is_err());
}

/// Someone sits down in an empty voice channel (#616): a push for those who
/// asked for it on the server's setting, and it wakes the phone.
#[sqlx::test]
async fn a_call_that_begins_pushes_who_asked(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    subscribe(&mut a, &Browser::new(&service, "sub/ada")).await;
    let lounge = voice(&mut a, &server, "Lounge").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .unwrap();
    let mut ada = ws::connect(addr, &a).await;
    let asks = |voice: bool, level: Value| json!({"level": level, "muted": false, "voice": voice});
    let server_path = format!("/api/servers/{server}/notification-settings");

    // Nobody asked: quiet, whatever the level.
    sig(&mut ada, &server, &lounge, "j", None).await;
    settled(&state).await;
    post(&mut a, &dm, "marker 1").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 1");

    // Bob asks, and wants nothing else from this server.
    let r = b
        .send(Method::PUT, &server_path, Some(asks(true, json!("none"))))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let second = voice(&mut a, &server, "Zwei").await;
    sig(&mut ada, &server, &second, "j", None).await;
    let hit = next(&mut hits).await;
    assert_eq!(hit.path, "sub/bob");
    assert_eq!(hit.headers["urgency"], "high");
    assert_eq!(hit.headers["ttl"], "600");
    let payload = browser.open(&hit);
    assert_eq!(payload["title"], "Ada · Chat");
    assert_eq!(payload["body"], "Ada ist im Sprachkanal Zwei.");
    assert_eq!(payload["path"], format!("/s/{server}/c/{second}"));
    assert_eq!(payload["channel"], second);

    // Out and back in within the cooldown: no second ring.
    // (Frames are handled in order: the join's echo says the leave is done.)
    ws::send(
        &mut ada,
        json!({"op": "sig", "t": "l", "s": server, "c": second}),
    )
    .await;
    sig(&mut ada, &server, &second, "j", None).await;
    // Someone who joins a call under way starts none: Cara sits down in a
    // channel of her own, which rings, and Ada joining her rings nobody.
    let mut cara = user(&state, "c@test.example", "Cara").await;
    let invite = a
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let joined = cara
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    let third = voice(&mut a, &server, "Drei").await;
    let mut cara_ws = ws::connect(addr, &cara).await;
    sig(&mut cara_ws, &server, &third, "j", None).await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["body"], "Cara ist im Sprachkanal Drei.");
    sig(&mut ada, &server, &third, "j", None).await;
    settled(&state).await;
    post(&mut a, &dm, "marker 2").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 2");

    // With content hidden: who, not where.
    sqlx::query("UPDATE users SET notifications_hide_content=true")
        .execute(&pool)
        .await
        .unwrap();
    let fourth = voice(&mut a, &server, "Vier").await;
    sig(&mut ada, &server, &fourth, "j", None).await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["title"], "Ada ist in einem Sprachkanal");
    assert_eq!(payload["body"], "");

    // Bob no longer asks.
    let r = b
        .send(Method::PUT, &server_path, Some(asks(false, Value::Null)))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let fifth = voice(&mut a, &server, "Fuenf").await;
    sig(&mut ada, &server, &fifth, "j", None).await;
    settled(&state).await;
    post(&mut a, &dm, "marker 3").await;
    assert_eq!(
        browser.open(&next(&mut hits).await)["title"],
        "Neue Nachricht von Ada"
    );
    assert!(hits.try_recv().is_err());
}

/// A second seat of someone already in the call (another device, or a
/// rejoin after a reconnect while the old seat lasts) begins no call: the
/// channel was not empty.
#[sqlx::test]
async fn a_second_seat_in_the_call_rings_nobody(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let r = b
        .send(
            Method::PUT,
            &format!("/api/servers/{server}/notification-settings"),
            Some(json!({"level": null, "muted": false, "voice": true})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let lounge = voice(&mut a, &server, "Lounge").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .unwrap();

    let mut phone = ws::connect(addr, &a).await;
    sig(&mut phone, &server, &lounge, "j", None).await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["body"], "Ada ist im Sprachkanal Lounge.");

    // As if the cooldown had run out: Ada's second device sits down beside
    // the first, and nobody hears of a call beginning.
    state.limiter.refund("push-call", &lounge);
    let mut laptop = ws::connect(addr, &a).await;
    sig(&mut laptop, &server, &lounge, "j", None).await;
    settled(&state).await;
    post(&mut a, &dm, "marker").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker");
    assert!(hits.try_recv().is_err());
}

/// Gives every member of the server but its owner a setting of `level` and
/// `voice` for it.
async fn crowd_settings(pool: &PgPool, server: &str, level: &str, voice: bool) {
    sqlx::query(
        "INSERT INTO notification_settings (user_id, scope_kind, scope_id, level, voice)
         SELECT m.user_id, 'server', m.server_id, $2, $3 FROM server_members m
         JOIN servers s ON s.id=m.server_id AND s.owner_id<>m.user_id
         WHERE m.server_id=$1::uuid
         ON CONFLICT (user_id, scope_kind, scope_id)
         DO UPDATE SET level=EXCLUDED.level, voice=EXCLUDED.voice",
    )
    .bind(server)
    .bind(level)
    .bind(voice)
    .execute(pool)
    .await
    .unwrap();
}

/// Takes the settings of crowd members for the server away until `left`
/// members have one.
async fn crowd_settings_left(pool: &PgPool, server: &str, left: i64) {
    sqlx::query(
        "DELETE FROM notification_settings WHERE scope_id=$1::uuid AND user_id IN (
           SELECT n.user_id FROM notification_settings n JOIN users u ON u.id=n.user_id
           WHERE n.scope_id=$1::uuid AND u.email LIKE 'crowd-%'
           ORDER BY n.user_id
           LIMIT GREATEST((SELECT count(*) FROM notification_settings
                           WHERE scope_id=$1::uuid) - $2, 0))",
    )
    .bind(server)
    .bind(left)
    .execute(pool)
    .await
    .unwrap();
    let have: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notification_settings WHERE scope_id=$1::uuid")
            .bind(server)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(have, left);
}

/// More than 500 members who chose "all" are a crowd, as for `@everyone`: an
/// ordinary message pages none of them; a mention still reaches whom it
/// names.
#[sqlx::test]
async fn an_ordinary_message_pages_no_crowd(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    let bob = b.user_id().to_owned();
    crowd(&pool, &server, 600).await;
    crowd_settings(&pool, &server, "all", false).await;
    let first = self::channel(&mut a, &server, "eins").await;
    post(&mut a, &first, "an alle").await;
    settled(&state).await;
    post(&mut a, &dm, "marker").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker");
    post(&mut a, &first, &format!("<@{bob}> du")).await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "@Bob du");

    // 500 are still pushed to.
    crowd_settings_left(&pool, &server, 500).await;
    let second = self::channel(&mut a, &server, "zwei").await;
    post(&mut a, &second, "an fast alle").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "an fast alle");
    assert!(hits.try_recv().is_err());
}

/// The same for a voice push: more than 500 who asked to hear of a call
/// beginning are a crowd, and nobody is rung.
#[sqlx::test]
async fn a_call_rings_no_crowd(pool: PgPool) {
    let World {
        state,
        mut a,
        mut b,
        server,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    let browser = Browser::new(&service, "sub/bob");
    subscribe(&mut b, &browser).await;
    crowd(&pool, &server, 600).await;
    crowd_settings(&pool, &server, "mentions", true).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state
        .gateway
        .wait_ready(Duration::from_secs(2))
        .await
        .unwrap();
    let mut ada = ws::connect(addr, &a).await;

    let lounge = voice(&mut a, &server, "Lounge").await;
    sig(&mut ada, &server, &lounge, "j", None).await;
    settled(&state).await;
    post(&mut a, &dm, "marker").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker");

    // 500 are still rung.
    crowd_settings_left(&pool, &server, 500).await;
    let second = voice(&mut a, &server, "Zwei").await;
    sig(&mut ada, &server, &second, "j", None).await;
    let payload = browser.open(&next(&mut hits).await);
    assert_eq!(payload["body"], "Ada ist im Sprachkanal Zwei.");
    assert!(hits.try_recv().is_err());
}
