//! Issue #518: sign-in through OpenID Connect. An account may have no
//! password; it signs in through a provider until its owner sets one.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use gelabber_api::AppState;
use gelabber_api::auth::tokens::{self, Purpose};
use gelabber_api::auth::user;
use gelabber_api::error::ApiError;
use gelabber_api::i18n::Locale;
use gelabber_api::mail::{Mailer, Sent};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

const ADA: &str = "ada@example.com";
const PASSWORD: &str = "correct horse";

/// Ada registered with a password and is signed in; `password_less` then
/// drops the password, as for an account a provider created.
async fn signed_up(state: AppState) -> Client {
    let mut client = Client::from_state(state);
    client.bootstrap().await;
    let res = client.register(ADA, PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn password_less(pool: &PgPool) {
    sqlx::query("UPDATE users SET password_hash = NULL WHERE email = $1")
        .bind(ADA)
        .execute(pool)
        .await
        .unwrap();
}

async fn stored_hash(pool: &PgPool) -> Option<String> {
    sqlx::query_scalar("SELECT password_hash FROM users WHERE email = $1")
        .bind(ADA)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn set_password(client: &mut Client, body: serde_json::Value) -> common::Response {
    client
        .send(Method::POST, "/api/auth/password", Some(body))
        .await
}

#[sqlx::test]
async fn an_account_without_a_password_cannot_log_in_with_one(pool: PgPool) {
    signed_up(common::state(pool.clone())).await;
    password_less(&pool).await;

    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    for typed in [PASSWORD, "anything at all"] {
        let res = client.login(ADA, typed).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
        assert_eq!(res.body["error"], "invalid_credentials");
        assert!(res.set_cookie(common::SESSION).is_none());
    }
}

#[sqlx::test]
async fn a_fresh_session_sets_the_first_password(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(state.clone()).await;
    let mut other = Client::from_state(state);
    other.bootstrap().await;
    assert_eq!(other.login(ADA, PASSWORD).await.status, StatusCode::OK);
    password_less(&pool).await;

    // No current password to give; the new one still follows the rules.
    let res = set_password(&mut ada, json!({ "password": "short" })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["password"], "too_short");
    assert!(res.body["fields"].get("current_password").is_none());

    let res = set_password(&mut ada, json!({ "password": "a brand new one" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["email"], ADA);
    assert!(res.set_cookie(common::SESSION).is_some());
    assert!(stored_hash(&pool).await.is_some());

    // Every other session ended; this browser has a fresh one.
    let session = other.bootstrap().await;
    assert!(session.body["user"].is_null(), "{}", session.body);
    let session = ada.bootstrap().await;
    assert_eq!(session.body["user"]["email"], ADA);

    let mut again = Client::new(pool.clone());
    again.bootstrap().await;
    assert_eq!(
        again.login(ADA, "a brand new one").await.status,
        StatusCode::OK
    );

    // From now on the current password is asked for, as for everyone.
    let res = set_password(&mut ada, json!({ "password": "yet another one" })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["current_password"], "required");
}

#[sqlx::test]
async fn an_older_session_must_sign_in_again_before_the_first_password(pool: PgPool) {
    let mut ada = signed_up(common::state(pool.clone())).await;
    password_less(&pool).await;
    sqlx::query("UPDATE sessions SET created_at = now() - interval '11 minutes'")
        .execute(&pool)
        .await
        .unwrap();

    let res = set_password(&mut ada, json!({ "password": "a brand new one" })).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "reauth_required");
    assert!(stored_hash(&pool).await.is_none());
    // The session itself stays.
    let session = ada.bootstrap().await;
    assert_eq!(session.body["user"]["email"], ADA);
}

#[sqlx::test]
async fn an_account_with_a_password_still_needs_it(pool: PgPool) {
    let mut ada = signed_up(common::state(pool.clone())).await;
    let res = set_password(&mut ada, json!({ "password": "a brand new one" })).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["current_password"], "required");
}

fn mail_state(pool: PgPool) -> (AppState, Mailer) {
    let mut state = common::state(pool);
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    (state, mailer)
}

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

fn token(mail: &Sent) -> String {
    let (_, rest) = mail.body.split_once("#token=").expect("link in mail");
    rest.chars().take(64).collect()
}

async fn oidc_account(pool: &PgPool) -> Uuid {
    let mut conn = pool.acquire().await.unwrap();
    user::insert_oidc(&mut conn, ADA, "Ada", Locale::En)
        .await
        .unwrap()
        .id
}

#[sqlx::test]
async fn insert_oidc_creates_a_confirmed_account_without_a_password(pool: PgPool) {
    let id = oidc_account(&pool).await;
    let (hash, verified, pending, locale): (Option<String>, bool, bool, String) = sqlx::query_as(
        "SELECT password_hash, email_verified_at IS NOT NULL, registration_pending, locale \
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hash, None);
    assert!(verified);
    assert!(!pending);
    assert_eq!(locale, "en");
    assert_eq!(user::password_hash_by_id(&pool, id).await.unwrap(), None);

    let mut conn = pool.acquire().await.unwrap();
    let again = user::insert_oidc(&mut conn, "ADA@example.com", "Ada", Locale::De).await;
    assert!(matches!(again, Err(ApiError::EmailTaken)), "{again:?}");
}

#[sqlx::test]
async fn a_reset_link_sets_a_first_password(pool: PgPool) {
    let (state, mailer) = mail_state(pool.clone());
    oidc_account(&pool).await;

    let mut client = Client::from_state(state);
    client.bootstrap().await;
    let res = client
        .send(
            Method::POST,
            "/api/auth/password-reset",
            Some(json!({ "email": ADA })),
        )
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 1).await;
    let res = client
        .send(
            Method::POST,
            "/api/auth/password-reset/confirm",
            Some(json!({ "token": token(&sent[0]), "password": PASSWORD })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(stored_hash(&pool).await.is_some());

    let mut again = Client::new(pool.clone());
    again.bootstrap().await;
    assert_eq!(again.login(ADA, PASSWORD).await.status, StatusCode::OK);
}

#[sqlx::test]
async fn a_confirmation_link_without_a_password_is_refused(pool: PgPool) {
    // Cannot arise through the API (a provider's takeover clears the
    // pending flag), but must not confirm or fail with an internal error.
    let (state, _mailer) = mail_state(pool.clone());
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, name, password_hash, registration_pending) \
         VALUES ($1, 'Ada', NULL, true) RETURNING id",
    )
    .bind(ADA)
    .fetch_one(&pool)
    .await
    .unwrap();
    let raw = tokens::issue(&pool, id, Purpose::VerifyEmail)
        .await
        .unwrap();

    let mut client = Client::from_state(state);
    client.bootstrap().await;
    let res = client
        .send(
            Method::POST,
            "/api/auth/verify-email",
            Some(json!({ "token": raw, "password": PASSWORD })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["password"], "wrong");
    let verified: bool =
        sqlx::query_scalar("SELECT email_verified_at IS NOT NULL FROM users WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!verified);
}

#[sqlx::test]
async fn the_sign_in_tables_hold_their_rules(pool: PgPool) {
    let id = oidc_account(&pool).await;
    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id, email) \
         VALUES ('google', 'sub-1', $1, $2)",
    )
    .bind(id)
    .bind(ADA)
    .execute(&pool)
    .await
    .unwrap();
    // One identity per provider and account, and one account per identity.
    for (subject, user_id) in [("sub-2", id), ("sub-1", Uuid::nil())] {
        let err = sqlx::query(
            "INSERT INTO user_identities (provider, subject, user_id, email) \
             VALUES ('google', $1, $2, 'x@example.com')",
        )
        .bind(subject)
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap_err();
        assert!(matches!(err, sqlx::Error::Database(_)), "{err}");
    }

    // A flow is bound to a browser or to the desktop app, never both or
    // neither; a link flow names its account.
    let flow = |binding: Option<&'static [u8]>,
                challenge: Option<&'static str>,
                port: Option<i32>,
                purpose: &'static str,
                link: Option<Uuid>| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO oidc_flows (state_hash, provider, purpose, link_user_id, nonce, \
                     pkce_verifier, binding_hash, desktop_port, desktop_challenge, invitation, \
                     expires_at) \
                 VALUES (uuid_send(gen_random_uuid()), 'google', $1, $2, 'n', 'v', $3, $4, $5, 'code', \
                     now() + interval '10 minutes')",
            )
            .bind(purpose)
            .bind(link)
            .bind(binding)
            .bind(port)
            .bind(challenge)
            .execute(&pool)
            .await
        }
    };
    flow(Some(b"b"), None, None, "sign_in", None).await.unwrap();
    flow(None, Some("c"), Some(49152), "link", Some(id))
        .await
        .unwrap();
    assert!(flow(None, None, None, "sign_in", None).await.is_err());
    assert!(
        flow(Some(b"b"), Some("c"), Some(49152), "sign_in", None)
            .await
            .is_err()
    );
    assert!(flow(None, Some("c"), None, "sign_in", None).await.is_err());
    assert!(
        flow(None, Some("c"), Some(80), "sign_in", None)
            .await
            .is_err()
    );
    assert!(flow(Some(b"b"), None, None, "link", None).await.is_err());

    // Deleting the account takes its identities along.
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM user_identities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}
