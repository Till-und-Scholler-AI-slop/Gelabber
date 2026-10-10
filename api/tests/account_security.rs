//! Issue #188: sign-in only over a protected transport, failed logins counted
//! per account, HSTS behind TLS, password change and signing out everywhere.

mod common;

use std::collections::HashMap;

use axum::http::header::{HOST, RETRY_AFTER, STRICT_TRANSPORT_SECURITY};
use axum::http::{Method, StatusCode};
use gelabber_api::{AppState, Config};
use serde_json::json;
use sqlx::PgPool;

use common::Client;

fn state_with(pool: PgPool, extra: &[(&str, &str)]) -> AppState {
    let extra: HashMap<String, String> = extra
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let config = Config::from_source(|key| {
        if let Some(value) = extra.get(key) {
            return Some(value.clone());
        }
        match key {
            "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
            "REDIS_URL" => Some(common::redis_url()),
            "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
            _ => None,
        }
    })
    .expect("test config");
    AppState::with_pool(&config, pool).expect("state")
}

async fn login_from(client: &mut Client, host: &'static str) -> common::Response {
    client
        .send_with(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": "ada@example.com", "password": "correct horse" })),
            |builder| {
                builder
                    .headers_mut()
                    .unwrap()
                    .insert(HOST, host.parse().unwrap());
            },
        )
        .await
}

async fn signed_up(state: AppState) -> Client {
    let mut client = Client::from_state(state);
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "correct horse", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

#[sqlx::test]
async fn plain_http_sign_in_is_refused_off_loopback(pool: PgPool) {
    let mut client = signed_up(state_with(pool, &[])).await;
    client.send(Method::POST, "/api/auth/logout", None).await;

    let refused = login_from(&mut client, "192.168.1.20").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(refused.body["error"], "insecure_transport");
    assert!(refused.set_cookie(common::SESSION).is_none());

    for host in ["localhost:8080", "127.0.0.1", "[::1]:80"] {
        let res = login_from(&mut client, host).await;
        assert_eq!(res.status, StatusCode::OK, "{host}: {}", res.body);
    }
}

#[sqlx::test]
async fn plain_http_sign_in_can_be_allowed_for_a_trusted_network(pool: PgPool) {
    let mut client = signed_up(state_with(pool, &[("API_ALLOW_PLAIN_HTTP", "true")])).await;
    let res = login_from(&mut client, "192.168.1.20").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test]
async fn behind_tls_any_host_signs_in_and_gets_hsts(pool: PgPool) {
    let mut client = signed_up(state_with(pool, &[("API_COOKIE_SECURE", "true")])).await;
    let res = login_from(&mut client, "chat.example.com").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.headers.get(STRICT_TRANSPORT_SECURITY).unwrap(),
        "max-age=31536000"
    );
    let session = client.bootstrap().await;
    assert!(session.headers.contains_key(STRICT_TRANSPORT_SECURITY));
}

#[sqlx::test]
async fn plain_http_sends_no_hsts(pool: PgPool) {
    let mut client = Client::from_state(state_with(pool, &[]));
    let res = client.bootstrap().await;
    assert!(!res.headers.contains_key(STRICT_TRANSPORT_SECURITY));
}

#[sqlx::test]
async fn failed_logins_are_limited_per_account_across_addresses(pool: PgPool) {
    let mut client = signed_up(state_with(
        pool,
        &[("API_RATE_LOGIN_FAILURES_PER_ACCOUNT", "3")],
    ))
    .await;
    client.send(Method::POST, "/api/auth/logout", None).await;

    for i in 0..3 {
        let res = client
            .send_with(
                Method::POST,
                "/api/auth/login",
                Some(json!({ "email": "ADA@example.com", "password": "wrong guess" })),
                |builder| {
                    let ip = format!("203.0.113.{i}");
                    builder
                        .headers_mut()
                        .unwrap()
                        .insert("x-forwarded-for", ip.parse().unwrap());
                },
            )
            .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    }

    // The right password does not help while the account's window is full:
    // otherwise the limit would only slow down, not stop, a guesser.
    let blocked = client.login("ada@example.com", "correct horse").await;
    assert_eq!(
        blocked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        blocked.body
    );
    assert_eq!(blocked.body["error"], "rate_limited");
    assert!(blocked.headers.contains_key(RETRY_AFTER));

    // Another account is not affected.
    let other = client.login("nobody@example.com", "wrong guess").await;
    assert_eq!(other.status, StatusCode::UNAUTHORIZED, "{}", other.body);
}

#[sqlx::test]
async fn password_change_ends_every_other_session(pool: PgPool) {
    let state = state_with(pool, &[]);
    let mut first = signed_up(state.clone()).await;
    let mut second = Client::from_state(state);
    second.bootstrap().await;
    let res = second.login("ada@example.com", "correct horse").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let wrong = first
        .send(
            Method::POST,
            "/api/auth/password",
            Some(json!({ "current_password": "not it", "password": "battery staple" })),
        )
        .await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.body
    );
    assert_eq!(wrong.body["fields"]["current_password"], "wrong");

    let short = first
        .send(
            Method::POST,
            "/api/auth/password",
            Some(json!({ "current_password": "correct horse", "password": "short" })),
        )
        .await;
    assert_eq!(
        short.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        short.body
    );
    assert_eq!(short.body["fields"]["password"], "too_short");

    let changed = first
        .send(
            Method::POST,
            "/api/auth/password",
            Some(json!({ "current_password": "correct horse", "password": "battery staple" })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["user"]["email"], "ada@example.com");
    assert!(changed.set_cookie(common::SESSION).is_some());

    // The caller carries on with the fresh session; the other device is out.
    assert_eq!(
        first.send(Method::GET, "/api/me", None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        second.send(Method::GET, "/api/me", None).await.status,
        StatusCode::UNAUTHORIZED
    );

    let old = second.login("ada@example.com", "correct horse").await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED, "{}", old.body);
    let new = second.login("ada@example.com", "battery staple").await;
    assert_eq!(new.status, StatusCode::OK, "{}", new.body);
}

#[sqlx::test]
async fn password_change_needs_a_session(pool: PgPool) {
    let mut client = Client::from_state(state_with(pool, &[]));
    client.bootstrap().await;
    let res = client
        .send(
            Method::POST,
            "/api/auth/password",
            Some(json!({ "current_password": "x", "password": "battery staple" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
}

#[sqlx::test]
async fn logout_all_ends_every_session(pool: PgPool) {
    let state = state_with(pool, &[]);
    let mut first = signed_up(state.clone()).await;
    let mut second = Client::from_state(state);
    second.bootstrap().await;
    second.login("ada@example.com", "correct horse").await;

    let res = second
        .send(Method::POST, "/api/auth/logout-all", None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(res.body["csrf_token"].is_string());
    let cleared = res.set_cookie(common::SESSION).expect("session cookie");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");

    for client in [&mut first, &mut second] {
        assert_eq!(
            client.send(Method::GET, "/api/me", None).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}
