//! Issue #515: every account has a language. Registration picks it, the
//! account changes it with `PUT /api/me/locale`, and the account's sockets
//! hear about it (`lc`).

mod common;

use std::time::Duration;

use axum::http::header::ACCEPT_LANGUAGE;
use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::COOKIE;

use common::{Client, SESSION};

const PASSWORD: &str = "password123";

fn body(email: &str, locale: Option<&str>) -> Value {
    let mut body = json!({ "email": email, "password": PASSWORD, "name": "Ada" });
    if let Some(locale) = locale {
        body["locale"] = json!(locale);
    }
    body
}

async fn register(
    pool: &PgPool,
    email: &str,
    locale: Option<&str>,
    accept_language: Option<&str>,
) -> (Client, common::Response) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let res = client
        .send_with(
            Method::POST,
            "/api/auth/register",
            Some(body(email, locale)),
            |request| {
                if let Some(value) = accept_language {
                    request
                        .headers_mut()
                        .unwrap()
                        .insert(ACCEPT_LANGUAGE, value.parse().unwrap());
                }
            },
        )
        .await;
    (client, res)
}

async fn put_locale(client: &mut Client, body: Value) -> common::Response {
    client.send(Method::PUT, "/api/me/locale", Some(body)).await
}

#[sqlx::test]
async fn registration_takes_the_field_then_the_header_then_german(pool: PgPool) {
    let (_, res) = register(&pool, "a@example.com", Some("en"), Some("de-DE")).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["user"]["locale"], "en");

    let (_, res) = register(&pool, "b@example.com", None, Some("en-US")).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["user"]["locale"], "en");

    let (_, res) = register(&pool, "c@example.com", None, Some("fr-FR, de;q=0.5")).await;
    assert_eq!(res.body["user"]["locale"], "de");

    let (_, res) = register(&pool, "d@example.com", None, Some("fr-FR")).await;
    assert_eq!(res.body["user"]["locale"], "en");

    // Clients older than v0.10 send neither.
    let (mut client, res) = register(&pool, "e@example.com", None, None).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["user"]["locale"], "de");
    let session = client.bootstrap().await;
    assert_eq!(session.body["user"]["locale"], "de");
}

#[sqlx::test]
async fn an_unknown_language_fails_registration(pool: PgPool) {
    for locale in ["fr", "", "EN", "en-GB"] {
        let (client, res) = register(&pool, "a@example.com", Some(locale), Some("en")).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["error"], "validation_failed");
        assert_eq!(res.body["fields"], json!({ "locale": "invalid" }));
        assert!(client.user_id.is_none());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn the_account_changes_its_language(pool: PgPool) {
    let (mut ada, _) = register(&pool, "ada@example.com", None, None).await;
    let (mut bob, _) = register(&pool, "bob@example.com", None, None).await;

    let res = put_locale(&mut ada, json!({ "locale": "en" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!({ "locale": "en" }));
    let session = ada.bootstrap().await;
    assert_eq!(session.body["user"]["locale"], "en");
    let me = ada.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.body["locale"], "en");
    // Only her own account.
    let session = bob.bootstrap().await;
    assert_eq!(session.body["user"]["locale"], "de");
    // Nobody else sees it.
    let profile = bob
        .send(Method::GET, &format!("/api/users/{}", ada.user_id()), None)
        .await;
    assert!(profile.body.get("locale").is_none(), "{}", profile.body);

    let res = put_locale(&mut ada, json!({ "locale": "de" })).await;
    assert_eq!(res.body, json!({ "locale": "de" }));
    let session = ada.bootstrap().await;
    assert_eq!(session.body["user"]["locale"], "de");
}

#[sqlx::test]
async fn a_language_change_is_validated_and_guarded(pool: PgPool) {
    let (mut ada, _) = register(&pool, "ada@example.com", None, None).await;

    for (body, code) in [
        (json!({}), "required"),
        (json!({ "locale": null }), "required"),
        (json!({ "locale": "fr" }), "invalid"),
        (json!({ "locale": "" }), "invalid"),
    ] {
        let res = put_locale(&mut ada, body.clone()).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(res.body["error"], "validation_failed");
        assert_eq!(res.body["fields"], json!({ "locale": code }), "{body}");
    }

    let csrf = ada.csrf.take();
    let res = put_locale(&mut ada, json!({ "locale": "en" })).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "csrf_invalid");
    ada.csrf = csrf;

    let mut stranger = Client::new(pool.clone());
    stranger.bootstrap().await;
    let res = put_locale(&mut stranger, json!({ "locale": "en" })).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    assert_eq!(res.body["error"], "unauthenticated");

    let session = ada.bootstrap().await;
    assert_eq!(session.body["user"]["locale"], "de");
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
    let cookie = client.jar.get(SESSION).expect("session cookie");
    request
        .headers_mut()
        .insert(COOKIE, format!("{SESSION}={cookie}").parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws
}

/// The next frame; server heartbeats are answered on the way, like a client.
async fn recv(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("ws recv timeout")
            .expect("ws closed")
            .expect("ws error");
        if let Message::Text(text) = msg {
            let frame: Value = serde_json::from_str(text.as_str()).unwrap();
            if frame["op"] == "h" {
                ws.send(Message::Text(json!({ "op": "h" }).to_string().into()))
                    .await
                    .unwrap();
            }
            return frame;
        }
    }
}

async fn recv_until(ws: &mut Ws, pred: impl Fn(&Value) -> bool) -> Value {
    loop {
        let frame = recv(ws).await;
        if pred(&frame) {
            return frame;
        }
    }
}

#[sqlx::test]
async fn the_account_sockets_hear_the_new_language(pool: PgPool) {
    let (mut ada, _) = register(&pool, "ada@example.com", None, None).await;
    let (bob, _) = register(&pool, "bob@example.com", None, None).await;
    let (addr, _) = common::serve_ws(pool).await;
    let mut first = connect(addr, &ada).await;
    let mut other = connect(addr, &bob).await;
    // Every socket is registered once it answers a heartbeat.
    for ws in [&mut first, &mut other] {
        ws.send(Message::Text(json!({ "op": "h" }).to_string().into()))
            .await
            .unwrap();
        recv_until(ws, |f| f["op"] == "h").await;
    }

    let res = put_locale(&mut ada, json!({ "locale": "en" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let frame = recv_until(&mut first, |f| f["op"] == "lc").await;
    assert_eq!(frame, json!({ "op": "lc", "l": "en" }));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), async {
            loop {
                let f = recv(&mut other).await;
                assert_ne!(f["op"], "lc", "language of another account");
            }
        })
        .await
        .is_err()
    );
}
