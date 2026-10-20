//! Web Push (#243): subscriptions, who gets a push and what it carries. A
//! fake push service on loopback plays the browser vendor.
mod common;

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::{Router, routing};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use common::{Client, SESSION};
use futures_util::SinkExt;
use gelabber_api::push::webpush::{Vapid, decrypt};
use gelabber_api::{AppState, Config, app};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::COOKIE;
use uuid::Uuid;

struct Hit {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

/// Answers 201, 410 for an endpoint under `/gone/`, 500 under `/fail/`,
/// 400 under `/bad/`, 403 under `/denied/`.
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
    let redis = common::redis_url();
    let private = vapid_private.map(str::to_owned);
    let config = Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(redis.clone()),
        "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
        "API_WS_HEARTBEAT_MS" => Some("40".to_owned()),
        "API_WS_DEAD_MS" => Some("5000".to_owned()),
        "VAPID_PRIVATE_KEY" => private.clone(),
        "VAPID_SUBJECT" => Some("mailto:admin@test.example".to_owned()),
        "PUSH_ALLOWED_HOSTS" => Some("127.0.0.1".to_owned()),
        _ => None,
    })
    .expect("push test config");
    AppState::with_pool(&config, pool).expect("state")
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

    fn open(&self, hit: &Hit) -> Value {
        let plain = decrypt(&self.private, &self.auth, &hit.body).expect("decrypts");
        serde_json::from_slice(&plain).unwrap()
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
    let (private, vapid_public) = Vapid::generate();
    let state = push_state(pool, Some(&private));
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

#[sqlx::test]
async fn in_a_server_only_mentions_push_and_settings_hold_them_back(pool: PgPool) {
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
    // No mention: nothing. The DM after it is the first to arrive.
    post(&mut a, &channel, "plain").await;
    post(&mut a, &dm, "marker 1").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "marker 1");

    post(&mut a, &channel, &format!("<@{bob}> schau mal")).await;
    let payload = browser.open(&next(&mut hits).await);
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
/// a month; a recent one stays.
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
    subscribe(&mut b, &Browser::new(&service, "fail/old")).await;
    subscribe(&mut b, &Browser::new(&service, "fail/new")).await;
    sqlx::query(
        "UPDATE push_subscriptions SET created_at=now()-interval '40 days' \
         WHERE endpoint LIKE '%/fail/old'",
    )
    .execute(&pool)
    .await
    .unwrap();
    post(&mut a, &dm, "eins").await;
    next(&mut hits).await;
    next(&mut hits).await;
    let mut left = Vec::new();
    for _ in 0..50 {
        left = sqlx::query_scalar::<_, String>("SELECT endpoint FROM push_subscriptions")
            .fetch_all(&pool)
            .await
            .unwrap();
        if left.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(left.len(), 1);
    assert!(left[0].ends_with("/fail/new"), "{left:?}");
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
/// rejected VAPID token, a clock off): a refusal counts as a failure, and
/// only a subscription silent for a month goes.
#[sqlx::test]
async fn a_refusal_of_every_push_keeps_the_subscriptions(pool: PgPool) {
    let World {
        mut a,
        mut b,
        dm,
        mut hits,
        service,
        ..
    } = setup(pool.clone()).await;
    subscribe(&mut b, &Browser::new(&service, "denied/new")).await;
    subscribe(&mut b, &Browser::new(&service, "denied/old")).await;
    sqlx::query(
        "UPDATE push_subscriptions SET created_at=now()-interval '40 days' \
         WHERE endpoint LIKE '%/denied/old'",
    )
    .execute(&pool)
    .await
    .unwrap();
    post(&mut a, &dm, "eins").await;
    next(&mut hits).await;
    next(&mut hits).await;
    // The old one going shows the cleanup ran; the recent one stays.
    let left = endpoints_left(&pool, 1).await;
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(left[0].ends_with("/denied/new"), "{left:?}");
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
    request.headers_mut().insert(
        COOKIE,
        format!("{SESSION}={}", client.jar[SESSION])
            .parse()
            .unwrap(),
    );
    tokio_tungstenite::connect_async(request).await.unwrap().0
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

    let mut ws = connect(addr, &b).await;
    ws.send(Message::Text(
        json!({"op":"f","on":true}).to_string().into(),
    ))
    .await
    .unwrap();
    focus_is(&state, b.user_id(), true).await;
    post(&mut a, &dm, "gesehen").await;
    // The push runs after the answer; let it look at the focus first.
    tokio::time::sleep(Duration::from_millis(500)).await;
    ws.send(Message::Text(
        json!({"op":"f","on":false}).to_string().into(),
    ))
    .await
    .unwrap();
    focus_is(&state, b.user_id(), false).await;
    post(&mut a, &dm, "weg").await;
    assert_eq!(browser.open(&next(&mut hits).await)["body"], "weg");

    // A closed socket gives its focus up.
    ws.send(Message::Text(
        json!({"op":"f","on":true}).to_string().into(),
    ))
    .await
    .unwrap();
    focus_is(&state, b.user_id(), true).await;
    drop(ws);
    focus_is(&state, b.user_id(), false).await;
}
