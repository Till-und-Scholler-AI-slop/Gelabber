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
async fn past_the_mail_limit_a_registration_changes_nothing(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    let mut stranger = client(&state).await;
    for _ in 0..4 {
        stranger.register(ADA, "stranger password", "Mallory").await;
    }
    let mut ada = client(&state).await;
    ada.register(ADA, PASSWORD, "Ada").await;
    let sent = outbox(&mailer, 5).await;

    // The address had its five mails: no new link, so no takeover either.
    let res = stranger.register(ADA, "another password", "Mallory").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(res.body["verification_sent"], true);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mailer.outbox().len(), 5);

    let res = post(
        &mut ada,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[4]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["name"], "Ada");
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

/// Registration mails and reset mails have five an hour each, so an inbox
/// gets at most ten, however many requests name it.
#[sqlx::test]
async fn an_inbox_gets_at_most_five_mails_an_hour_per_budget(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    confirmed_ada(&state, &mailer).await;
    let mut other = client(&state).await;
    for _ in 0..8 {
        let res = other.register(ADA, "stranger password", "Mallory").await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    }
    for _ in 0..8 {
        let res = post(
            &mut other,
            "/api/auth/password-reset",
            json!({ "email": ADA }),
        )
        .await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    }
    let sent = outbox(&mailer, 10).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mailer.outbox().len(), 10);
    let resets = sent
        .iter()
        .filter(|mail| mail.body.contains("/reset-password#token="))
        .count();
    assert_eq!(resets, 5, "{sent:?}");
}

/// A reset asked for with the Unicode spelling of the domain reaches the
/// account stored with its ASCII form.
#[sqlx::test]
async fn a_reset_finds_the_account_by_either_spelling(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    let mut client = client(&state).await;
    let res = client.register("ada@bücher.de", PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 1).await;
    assert_eq!(sent[0].to, "ada@xn--bcher-kva.de");
    let res = post(
        &mut client,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[0]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let res = post(
        &mut client,
        "/api/auth/password-reset",
        json!({ "email": "ada@bücher.de" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 2).await;
    assert_eq!(sent[1].to, "ada@xn--bcher-kva.de");
    assert!(sent[1].body.contains("/reset-password#token="));
}

/// Registrations by strangers spend the address's registration mails, not
/// the ones its owner needs for a reset.
#[sqlx::test]
async fn strangers_registering_an_address_leave_its_reset_mails(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    confirmed_ada(&state, &mailer).await;
    let mut stranger = client(&state).await;
    for _ in 0..6 {
        let res = stranger.register(ADA, "stranger password", "Mallory").await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    }
    outbox(&mailer, 5).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mailer.outbox().len(), 5);

    let res = post(
        &mut stranger,
        "/api/auth/password-reset",
        json!({ "email": ADA }),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 6).await;
    assert!(
        sent[5].body.contains("/reset-password#token="),
        "{}",
        sent[5].body
    );
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

/// Issue #515: every account mail is in the account's language, whatever
/// language the request that caused it carried.
#[sqlx::test]
async fn mails_follow_the_account_language(pool: PgPool) {
    let (state, mailer) = mail_state(pool);
    let mut ada = client(&state).await;
    let register = |locale: &'static str, password: &'static str| json!({ "email": ADA, "password": password, "name": "Ada", "locale": locale });

    let res = post(&mut ada, "/api/auth/register", register("en", PASSWORD)).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 1).await;
    assert_eq!(sent[0].subject, "Confirm your email address for Gelabber");
    assert!(sent[0].body.starts_with("Hi,\n\n"), "{}", sent[0].body);

    // Taking over the pending registration takes its language too.
    let res = post(&mut ada, "/api/auth/register", register("de", PASSWORD)).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 2).await;
    assert_eq!(
        sent[1].subject,
        "Bestätige deine E-Mail-Adresse für Gelabber"
    );
    let res = post(&mut ada, "/api/auth/register", register("en", PASSWORD)).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 3).await;
    assert_eq!(sent[2].subject, "Confirm your email address for Gelabber");
    let res = post(
        &mut ada,
        "/api/auth/verify-email",
        json!({ "token": token(&sent[2]), "password": PASSWORD }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["user"]["locale"], "en");

    // A German registration with her address: the notice is in her language,
    // and her account keeps it.
    let mut stranger = client(&state).await;
    let res = post(
        &mut stranger,
        "/api/auth/register",
        register("de", "another password"),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 4).await;
    assert_eq!(sent[3].subject, "Your Gelabber account");
    assert!(
        sent[3]
            .body
            .contains("Forgot password: https://chat.example.org/forgot-password"),
        "{}",
        sent[3].body
    );

    let res = post(
        &mut stranger,
        "/api/auth/password-reset",
        json!({ "email": ADA }),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 5).await;
    assert_eq!(sent[4].subject, "New password for Gelabber");
    assert!(
        sent[4]
            .body
            .contains("https://chat.example.org/reset-password#token="),
        "{}",
        sent[4].body
    );

    // A switch to German applies to the next mail.
    let res = ada
        .send(
            Method::PUT,
            "/api/me/locale",
            Some(json!({ "locale": "de" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = post(
        &mut stranger,
        "/api/auth/password-reset",
        json!({ "email": ADA }),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let sent = outbox(&mailer, 6).await;
    assert_eq!(sent[5].subject, "Neues Passwort für Gelabber");
}

/// Registration with mail on, on an invitation-only instance (#517): the
/// answer stays `202` either way, and only a new or pending account uses
/// up the invitation.
#[sqlx::test]
async fn with_mail_only_a_new_or_pending_account_uses_an_invitation(pool: PgPool) {
    let (open, mailer) = mail_state(pool.clone());
    confirmed_ada(&open, &mailer).await;
    let mut state = open.clone();
    state.registration = gelabber_api::config::RegistrationMode::Invitation;
    let invitation = |code: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, uuid::Uuid>(
                "INSERT INTO beta_invitations (code) VALUES ($1) RETURNING id",
            )
            .bind(code)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let used_by = |id: uuid::Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<uuid::Uuid>>(
                "SELECT used_by FROM beta_invitations WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let register = |email: &'static str, code: Option<&'static str>| {
        let state = state.clone();
        async move {
            let mut client = client(&state).await;
            let mut body = json!({ "email": email, "password": PASSWORD, "name": "Neu" });
            if let Some(code) = code {
                body["invitation"] = json!(code);
            }
            post(&mut client, "/api/auth/register", body).await
        }
    };

    let res = register("bob@example.com", None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "invitation_required");

    // A confirmed address keeps its account and the invitation stays open.
    let first = invitation("aaaaaaaaaaaa").await;
    let res = register(ADA, Some("aaaaaaaaaaaa")).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(used_by(first).await, None);
    let sent = outbox(&mailer, 2).await;
    assert_eq!(sent[1].to, ADA);

    // A new address uses it up.
    let res = register("bob@example.com", Some("aaaaaaaaaaaa")).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let bob: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE email = 'bob@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(used_by(first).await, Some(bob));
    outbox(&mailer, 3).await;

    // Taking over the pending row uses the next one.
    let second = invitation("bbbbbbbbbbbb").await;
    let res = register("bob@example.com", Some("aaaaaaaaaaaa")).await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    let res = register("bob@example.com", Some("bbbbbbbbbbbb")).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(used_by(second).await, Some(bob));
}
