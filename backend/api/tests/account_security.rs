//! Issue #188: sign-in only over a protected transport, failed logins counted
//! per account and address (GEL-44), HSTS behind TLS, password change and
//! signing out everywhere.

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

/// A login from `address`, as the proxy next to the API reports it.
async fn login_at(
    client: &mut Client,
    address: &str,
    email: &str,
    password: &str,
) -> common::Response {
    let address = address.to_owned();
    client
        .send_with(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": password })),
            move |builder| {
                builder
                    .headers_mut()
                    .unwrap()
                    .insert("x-forwarded-for", address.parse().unwrap());
            },
        )
        .await
}

/// A stranger's wrong guesses use up the stranger's address, not the
/// account: the owner still signs in from anywhere else (GEL-44).
#[sqlx::test]
async fn a_strangers_failed_logins_do_not_lock_out_the_owner(pool: PgPool) {
    let state = state_with(pool, &[("API_RATE_LOGIN_FAILURES_PER_ACCOUNT", "3")]);
    let mut owner = signed_up(state.clone()).await;
    owner.send(Method::POST, "/api/auth/logout", None).await;
    let mut stranger = Client::from_state(state);
    stranger.bootstrap().await;

    for _ in 0..3 {
        let res = login_at(
            &mut stranger,
            "203.0.113.7",
            "ADA@example.com",
            "wrong guess",
        )
        .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    }
    // The right password does not help from that address while its window
    // is full: otherwise the limit would only slow down, not stop, a guesser.
    let blocked = login_at(
        &mut stranger,
        "203.0.113.7",
        "ada@example.com",
        "correct horse",
    )
    .await;
    assert_eq!(
        blocked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        blocked.body
    );
    assert_eq!(blocked.body["error"], "rate_limited");
    assert!(blocked.headers.contains_key(RETRY_AFTER));

    let res = login_at(
        &mut owner,
        "198.51.100.20",
        "ada@example.com",
        "correct horse",
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

/// Spreading guesses over many addresses stops at the account's ceiling.
/// That holds the owner back at a new address too, but not at one the
/// account signed in from before, which keeps only its own count.
#[sqlx::test]
async fn failed_logins_from_many_addresses_stop_at_the_account_ceiling(pool: PgPool) {
    let state = state_with(
        pool,
        &[
            ("API_RATE_LOGIN_FAILURES_PER_ACCOUNT", "2"),
            ("API_RATE_LOGIN_FAILURES_ACCOUNT_CEILING", "5"),
        ],
    );
    let mut owner = signed_up(state.clone()).await;
    let home = "192.0.2.10";
    let res = login_at(&mut owner, home, "ada@example.com", "correct horse").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    owner.send(Method::POST, "/api/auth/logout", None).await;
    let mut stranger = Client::from_state(state);
    stranger.bootstrap().await;

    let mut statuses = Vec::new();
    for i in 0..4 {
        let address = format!("203.0.113.{i}");
        for _ in 0..2 {
            let res = login_at(&mut stranger, &address, "ada@example.com", "wrong guess").await;
            statuses.push(res.status);
        }
    }
    assert_eq!(
        tally(&statuses, StatusCode::UNAUTHORIZED),
        5,
        "{statuses:?}"
    );
    assert_eq!(
        tally(&statuses, StatusCode::TOO_MANY_REQUESTS),
        3,
        "{statuses:?}"
    );

    let elsewhere = login_at(
        &mut owner,
        "198.51.100.20",
        "ada@example.com",
        "correct horse",
    )
    .await;
    assert_eq!(
        elsewhere.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        elsewhere.body
    );
    // Another account is not affected.
    let other = login_at(
        &mut stranger,
        "203.0.113.0",
        "nobody@example.com",
        "wrong guess",
    )
    .await;
    assert_eq!(other.status, StatusCode::UNAUTHORIZED, "{}", other.body);

    let res = login_at(&mut owner, home, "ada@example.com", "correct horse").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    owner.send(Method::POST, "/api/auth/logout", None).await;
    for _ in 0..2 {
        let res = login_at(&mut owner, home, "ada@example.com", "wrong guess").await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    }
    let res = login_at(&mut owner, home, "ada@example.com", "correct horse").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
}

/// `count` POSTs to `path` with `body`, all in flight at once, the `i`th
/// from `address(i)` and each with the client's cookies and CSRF token.
async fn concurrently(
    client: &Client,
    path: &str,
    body: serde_json::Value,
    count: usize,
    address: impl Fn(usize) -> String,
) -> Vec<StatusCode> {
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::header::{CONTENT_TYPE, COOKIE};
    use tower::ServiceExt;

    let cookie = client
        .jar
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    let requests = (0..count).map(|i| {
        let mut builder = Request::post(path)
            .header(CONTENT_TYPE, "application/json")
            .header("x-forwarded-for", address(i));
        if !cookie.is_empty() {
            builder = builder.header(COOKIE, &cookie);
        }
        if let Some(token) = &client.csrf {
            builder = builder.header("x-csrf-token", token);
        }
        let request = builder.body(Body::from(body.to_string())).unwrap();
        client.app.clone().oneshot(request)
    });
    futures_util::future::join_all(requests)
        .await
        .into_iter()
        .map(|response| response.unwrap().status())
        .collect()
}

fn tally(statuses: &[StatusCode], wanted: StatusCode) -> usize {
    statuses.iter().filter(|status| **status == wanted).count()
}

fn spread(i: usize) -> String {
    format!("198.51.100.{i}")
}

/// Guesses that arrive together must not all pass the check before the
/// first failure is counted, from one address ...
#[sqlx::test]
async fn concurrent_login_guesses_stop_at_the_address_limit(pool: PgPool) {
    let mut client = signed_up(state_with(
        pool,
        &[("API_RATE_LOGIN_FAILURES_PER_ACCOUNT", "3")],
    ))
    .await;
    client.send(Method::POST, "/api/auth/logout", None).await;

    let statuses = concurrently(
        &client,
        "/api/auth/login",
        json!({ "email": "ada@example.com", "password": "wrong guess" }),
        12,
        |_| "203.0.113.7".to_owned(),
    )
    .await;
    assert_eq!(
        tally(&statuses, StatusCode::UNAUTHORIZED),
        3,
        "{statuses:?}"
    );
    assert_eq!(
        tally(&statuses, StatusCode::TOO_MANY_REQUESTS),
        9,
        "{statuses:?}"
    );
}

/// ... or from many.
#[sqlx::test]
async fn concurrent_login_guesses_stop_at_the_account_ceiling(pool: PgPool) {
    let mut client = signed_up(state_with(
        pool,
        &[("API_RATE_LOGIN_FAILURES_ACCOUNT_CEILING", "3")],
    ))
    .await;
    client.send(Method::POST, "/api/auth/logout", None).await;

    let statuses = concurrently(
        &client,
        "/api/auth/login",
        json!({ "email": "ada@example.com", "password": "wrong guess" }),
        12,
        spread,
    )
    .await;
    assert_eq!(
        tally(&statuses, StatusCode::UNAUTHORIZED),
        3,
        "{statuses:?}"
    );
    assert_eq!(
        tally(&statuses, StatusCode::TOO_MANY_REQUESTS),
        9,
        "{statuses:?}"
    );
}

/// A session's guesses count for the session wherever they come from.
#[sqlx::test]
async fn concurrent_password_change_guesses_stop_at_the_session_limit(pool: PgPool) {
    let client = signed_up(state_with(
        pool,
        &[("API_RATE_LOGIN_FAILURES_PER_ACCOUNT", "3")],
    ))
    .await;

    let statuses = concurrently(
        &client,
        "/api/auth/password",
        json!({ "current_password": "wrong guess", "password": "battery staple" }),
        12,
        spread,
    )
    .await;
    assert_eq!(
        tally(&statuses, StatusCode::UNPROCESSABLE_ENTITY),
        3,
        "{statuses:?}"
    );
    assert_eq!(
        tally(&statuses, StatusCode::TOO_MANY_REQUESTS),
        9,
        "{statuses:?}"
    );
}

/// Guesses through a session count for that session: whoever holds a
/// stolen cookie is stopped there, and the owner still signs in.
#[sqlx::test]
async fn a_sessions_guesses_do_not_lock_the_account_for_sign_in(pool: PgPool) {
    let state = state_with(pool, &[("API_RATE_LOGIN_FAILURES_PER_ACCOUNT", "2")]);
    let mut session = signed_up(state.clone()).await;
    let change = json!({ "current_password": "wrong guess", "password": "battery staple" });
    for _ in 0..2 {
        let res = session
            .send(Method::POST, "/api/auth/password", Some(change.clone()))
            .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    }
    let res = session
        .send(Method::POST, "/api/auth/password", Some(change))
        .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);

    let mut owner = Client::from_state(state);
    owner.bootstrap().await;
    let res = owner.login("ada@example.com", "correct horse").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

/// A right password leaves the failure count where it was, so a correct
/// login never uses up the account's allowance.
#[sqlx::test]
async fn correct_logins_do_not_count_as_failures(pool: PgPool) {
    let mut client = signed_up(state_with(
        pool,
        &[("API_RATE_LOGIN_FAILURES_PER_ACCOUNT", "2")],
    ))
    .await;
    for _ in 0..4 {
        let res = client.login("ada@example.com", "correct horse").await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }
    let wrong = client.login("ada@example.com", "wrong guess").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.body);
    let right = client.login("ada@example.com", "correct horse").await;
    assert_eq!(right.status, StatusCode::OK, "{}", right.body);
}

#[sqlx::test]
async fn made_up_session_cookies_share_the_address_window(pool: PgPool) {
    let state = state_with(pool, &[("API_RATE_API_PER_MIN", "3")]);
    let mut signed_in = signed_up(state.clone()).await;
    let mut forger = Client::from_state(state);
    forger.bootstrap().await;

    // The forger has a valid CSRF pair, so only the gate can refuse.
    let from_one_address = |builder: &mut axum::http::request::Builder| {
        builder
            .headers_mut()
            .unwrap()
            .insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
    };
    for i in 0..4 {
        forger
            .jar
            .insert(common::SESSION.to_owned(), format!("forged-{i}"));
        let res = forger
            .send_with(Method::POST, "/api/auth/logout", None, from_one_address)
            .await;
        if i < 3 {
            assert_eq!(res.status, StatusCode::OK, "{i}: {}", res.body);
        } else {
            assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
            assert_eq!(res.body["error"], "rate_limited");
            assert!(res.headers.contains_key(RETRY_AFTER));
        }
    }

    // A real session from the same address counts in a window of its own.
    let res = signed_in
        .send_with(Method::POST, "/api/auth/logout", None, from_one_address)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
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
