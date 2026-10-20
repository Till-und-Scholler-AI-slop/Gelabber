//! Issue #195: address confirmation and password reset through account mail,
//! with the same answer whether or not an address has an account.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use gelabber_api::AppState;
use gelabber_api::mail::{Mailer, Sent};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

const ADA: &str = "ada@example.com";
const PASSWORD: &str = "correct horse";

fn mail_state(pool: PgPool) -> (AppState, Mailer) {
    let mut state = common::state(pool);
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    (state, mailer)
}

/// Mails go out in the background; wait until `count` have been taken.
async fn outbox(mailer: &Mailer, count: usize) -> Vec<Sent> {
    for _ in 0..200 {
        let sent = mailer.outbox();
        if sent.len() >= count {
            return sent;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("expected {count} mails, got {:?}", mailer.outbox());
}

/// The token of the link in `mail` (`…#token=<64 hex>`).
fn token(mail: &Sent) -> String {
    let (_, rest) = mail.body.split_once("#token=").expect("link in mail");
    rest.chars().take(64).collect()
}

async fn client(state: &AppState) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    client
}

async fn post(client: &mut Client, path: &str, body: Value) -> common::Response {
    client.send(Method::POST, path, Some(body)).await
}

/// Registers Ada and opens her confirmation link.
async fn confirmed_ada(state: &AppState, mailer: &Mailer) -> Client {
    let mut client = client(state).await;
    let res = client.register(ADA, PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(mailer, 1).await;
    let res = post(
        &mut client,
        "/api/auth/verify-email",
        json!({ "token": token(sent.last().unwrap()), "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    client
}

#[sqlx::test]
async fn registration_waits_for_the_confirmation_link(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    let mut client = client(&state).await;

    let res = client.register(ADA, PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(res.body["verification_sent"], true);
    assert!(res.body["user"].is_null());
    assert!(res.body["csrf_token"].is_string());
    assert!(!client.jar.contains_key(common::SESSION));
    let session = client.bootstrap().await;
    assert!(session.body["user"].is_null());

    let sent = outbox(&mailer, 1).await;
    assert_eq!(sent[0].to, ADA);
    assert!(
        sent[0]
            .body
            .contains("https://chat.example.org/verify-email#token="),
        "{}",
        sent[0].body
    );
    // What the registrant typed never reaches the mail.
    assert!(!sent[0].body.contains("Ada"));

    // The right password alone does not sign in; a fresh link goes out.
    let res = client.login(ADA, PASSWORD).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "email_unverified");
    assert!(res.set_cookie(common::SESSION).is_none());
    let sent = outbox(&mailer, 2).await;

    // Only the newest link works, and only once.
    let old = post(
        &mut client,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[0]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(old.status, StatusCode::GONE, "{}", old.body);
    assert_eq!(old.body["error"], "token_invalid");
    let res = post(
        &mut client,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[1]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["email"], ADA);
    assert!(res.set_cookie(common::SESSION).is_some());
    let again = post(
        &mut client,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[1]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(again.status, StatusCode::GONE, "{}", again.body);

    client.send(Method::POST, "/api/auth/logout", None).await;
    let res = client.login(ADA, PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test]
async fn a_known_address_gets_the_same_answer_and_keeps_its_account(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    confirmed_ada(&state, &mailer).await;

    let mut stranger = client(&state).await;
    let res = stranger.register(ADA, "another password", "Mallory").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(res.body["verification_sent"], true);
    assert!(res.body["user"].is_null());

    let sent = outbox(&mailer, 2).await;
    assert_eq!(sent[1].to, ADA);
    assert!(!sent[1].body.contains("#token="), "{}", sent[1].body);
    assert!(
        sent[1]
            .body
            .contains("https://chat.example.org/forgot-password")
    );

    let res = stranger.login(ADA, "another password").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    let res = stranger.login(ADA, PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["name"], "Ada");
}

#[sqlx::test]
async fn whoever_confirms_a_pending_address_gets_the_password_they_typed(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    // A stranger registers the address first; its owner ignores that mail.
    let mut stranger = client(&state).await;
    let res = stranger.register(ADA, "stranger password", "Mallory").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 1).await;
    let stranger_link = token(&sent[0]);

    // Clicking the stranger's link takes the stranger's password.
    let mut owner = client(&state).await;
    let res = post(
        &mut owner,
        "/api/auth/verify-email",
        json!({ "token": stranger_link, "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["password"], "wrong");
    assert!(res.set_cookie(common::SESSION).is_none());
    let res = post(
        &mut owner,
        "/api/auth/verify-email",
        json!({ "token": stranger_link }),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["password"], "required");

    let mut ada = client(&state).await;
    let res = ada.register(ADA, PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 2).await;
    assert!(
        sent[1].body.contains("/verify-email#token="),
        "{}",
        sent[1].body
    );

    // The first link died with the takeover.
    let res = post(
        &mut ada,
        "/api/auth/verify-email",
        json!({ "token": stranger_link }),
    )
    .await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    let res = post(
        &mut ada,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[1]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["name"], "Ada");

    let res = stranger.login(ADA, "stranger password").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    let res = stranger.login(ADA, PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Confirmed now: registering again changes nothing.
    let mut late = client(&state).await;
    late.register(ADA, "late password", "Late").await;
    let res = late.login(ADA, "late password").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
}

#[sqlx::test]
async fn a_reset_link_sets_the_password_once_and_ends_every_session(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    let mut ada = confirmed_ada(&state, &mailer).await;
    let mut other = client(&state).await;

    // Unknown address: same answer, no mail.
    let res = post(
        &mut other,
        "/api/auth/password-reset",
        json!({ "email": "nobody@example.com" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let res = post(
        &mut other,
        "/api/auth/password-reset",
        json!({ "email": " Ada@Example.com " }),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 2).await;
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[1].to, ADA);
    assert!(
        sent[1]
            .body
            .contains("https://chat.example.org/reset-password#token=")
    );
    let link = token(&sent[1]);

    let res = post(
        &mut other,
        "/api/auth/password-reset/confirm",
        json!({ "token": link, "password": "short" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["password"], "too_short");

    let res = post(
        &mut other,
        "/api/auth/password-reset/confirm",
        json!({ "token": link, "password": "a brand new one" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["email"], ADA);
    assert!(res.set_cookie(common::SESSION).is_some());

    // Ada's earlier session is gone, the old password too.
    let session = ada.bootstrap().await;
    assert!(session.body["user"].is_null(), "{}", session.body);
    let res = ada.login(ADA, PASSWORD).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    let res = ada.login(ADA, "a brand new one").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let res = post(
        &mut other,
        "/api/auth/password-reset/confirm",
        json!({ "token": link, "password": "yet another one" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    assert_eq!(res.body["error"], "token_invalid");
}

#[sqlx::test]
async fn a_reset_confirms_an_unconfirmed_address(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    let mut client = client(&state).await;
    let res = client.register(ADA, PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    outbox(&mailer, 1).await;

    post(
        &mut client,
        "/api/auth/password-reset",
        json!({ "email": ADA }),
    )
    .await;
    let sent = outbox(&mailer, 2).await;
    let res = post(
        &mut client,
        "/api/auth/password-reset/confirm",
        json!({ "token": token(&sent[1]), "password": "a brand new one" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    client.send(Method::POST, "/api/auth/logout", None).await;
    let res = client.login(ADA, "a brand new one").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test]
async fn changing_the_password_kills_open_reset_links(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    let mut ada = confirmed_ada(&state, &mailer).await;
    post(
        &mut ada,
        "/api/auth/password-reset",
        json!({ "email": ADA }),
    )
    .await;
    let sent = outbox(&mailer, 2).await;

    let res = post(
        &mut ada,
        "/api/auth/password",
        json!({ "current_password": PASSWORD, "password": "changed meanwhile" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let res = post(
        &mut ada,
        "/api/auth/password-reset/confirm",
        json!({ "token": token(&sent[1]), "password": "a brand new one" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
}

#[sqlx::test]
async fn an_inbox_gets_at_most_five_mails_an_hour(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    confirmed_ada(&state, &mailer).await;
    let mut other = client(&state).await;
    for _ in 0..8 {
        let res = post(
            &mut other,
            "/api/auth/password-reset",
            json!({ "email": ADA }),
        )
        .await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    }
    // The confirmation counted too.
    outbox(&mailer, 5).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mailer.outbox().len(), 5);
}

#[sqlx::test]
async fn accounts_from_before_mail_must_confirm_once_mail_is_on(pool: PgPool) {
    // Registered while the instance had no mail: signed in at once.
    let mut before = Client::new(pool.clone());
    before.bootstrap().await;
    let res = before.register(ADA, PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let mail = before.send(Method::GET, "/api/auth/mail", None).await;
    assert_eq!(mail.body["enabled"], false);
    let res = post(
        &mut before,
        "/api/auth/password-reset",
        json!({ "email": ADA }),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    let (state, mailer) = mail_state(pool);
    let mut after = client(&state).await;
    let mail = after.send(Method::GET, "/api/auth/mail", None).await;
    assert_eq!(mail.body["enabled"], true);
    // That account may be in use: registering its address again does not
    // replace its password, it only mails its owner a confirmation link.
    let res = after.register(ADA, "stranger password", "Mallory").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    outbox(&mailer, 1).await;
    let res = after.login(ADA, "stranger password").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    let res = after.login(ADA, PASSWORD).await;
    assert_eq!(res.body["error"], "email_unverified", "{}", res.body);
    let sent = outbox(&mailer, 2).await;
    let res = post(
        &mut after,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[1]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["name"], "Ada");
}

#[sqlx::test]
async fn malformed_links_are_refused_without_touching_accounts(pool: PgPool) {
    let (state, _mailer) = mail_state(pool);
    let mut client = client(&state).await;
    for token in ["", "not-a-token", &"f".repeat(64)] {
        let res = post(
            &mut client,
            "/api/auth/verify-email",
            json!({ "token": token }),
        )
        .await;
        assert_eq!(res.status, StatusCode::GONE, "{token:?}: {}", res.body);
    }
    let res = post(
        &mut client,
        "/api/auth/password-reset/confirm",
        json!({ "token": "f".repeat(64), "password": "a brand new one" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
}
