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

/// `auth::oidc` against the mock issuer: discovery, keys and the token
/// exchange, before any route uses them.
mod provider {
    use gelabber_api::auth::oidc::{Oidc, pkce};
    use gelabber_api::error::ApiError;
    use serde_json::json;

    use crate::common::mock_issuer::{Alg, MockIssuer, MockUser};

    const PUBLIC: &str = "https://chat.example.test";
    const NONCE: &str = "nonce-1";
    const CALLBACK: &str = "https://chat.example.test/api/auth/oidc/google/callback";

    fn lena() -> MockUser {
        MockUser::verified("sub-lena", "Lena@Example.TEST", "Lena Example")
    }

    async fn exchange(
        issuer: &MockIssuer,
        oidc: &Oidc,
        user: MockUser,
    ) -> Result<gelabber_api::auth::oidc::Claims, ApiError> {
        let verifier = pkce::verifier();
        let code = issuer.grant(&verifier, CALLBACK, NONCE, user);
        let google = oidc.provider("google").unwrap();
        oidc.exchange(google, &code, &verifier, NONCE).await
    }

    fn assert_unavailable(result: Result<impl std::fmt::Debug, ApiError>, reason: &str) {
        match result {
            Err(ApiError::OidcUnavailable(detail)) => {
                assert!(detail.contains(reason), "{detail}")
            }
            other => panic!("expected OidcUnavailable({reason}), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exchange_returns_verified_claims_for_es256_and_rs256() {
        for alg in [Alg::Es256, Alg::Rs256] {
            let issuer = MockIssuer::start_with(alg).await;
            let oidc = issuer.oidc(PUBLIC);
            let claims = exchange(&issuer, &oidc, lena()).await.unwrap();
            assert_eq!(claims.subject, "sub-lena");
            assert_eq!(claims.email.as_deref(), Some("lena@example.test"));
            assert!(claims.email_verified);
            assert_eq!(claims.name.as_deref(), Some("Lena Example"));
        }
    }

    #[tokio::test]
    async fn discovery_and_keys_are_fetched_once_and_cached() {
        let issuer = MockIssuer::start().await;
        let oidc = issuer.oidc(PUBLIC);
        exchange(&issuer, &oidc, lena()).await.unwrap();
        exchange(&issuer, &oidc, lena()).await.unwrap();
        assert_eq!(issuer.discovery_hits(), 1);
        assert_eq!(issuer.jwks_hits(), 1);
        assert_eq!(issuer.token_hits(), 2);
        let google = oidc.provider("google").unwrap();
        let doc = oidc.discovery(google).await.unwrap();
        assert_eq!(doc.issuer, issuer.issuer());
        assert_eq!(
            doc.authorization_endpoint.as_str(),
            format!("{}/auth", issuer.issuer())
        );
    }

    #[tokio::test]
    async fn an_unknown_kid_refetches_at_most_once_a_minute() {
        let issuer = MockIssuer::start().await;
        let oidc = issuer.oidc(PUBLIC);
        exchange(&issuer, &oidc, lena()).await.unwrap();
        issuer.rotate();
        // The key set was fetched moments ago: no second fetch yet.
        assert_unavailable(exchange(&issuer, &oidc, lena()).await, "kid");
        assert_eq!(issuer.jwks_hits(), 1);

        // A fresh cache picks the rotated key up.
        let fresh = issuer.oidc(PUBLIC);
        exchange(&issuer, &fresh, lena()).await.unwrap();
        assert_eq!(issuer.jwks_hits(), 2);
    }

    #[tokio::test]
    async fn a_discovery_document_for_another_issuer_is_refused_and_not_cached() {
        let issuer = MockIssuer::start().await;
        issuer.claim_issuer("http://127.0.0.1:1");
        let oidc = issuer.oidc(PUBLIC);
        let google = oidc.provider("google").unwrap();
        assert_unavailable(oidc.discovery(google).await, "issuer");
        issuer.claim_issuer(issuer.issuer());
        oidc.discovery(google).await.unwrap();
        assert_eq!(issuer.discovery_hits(), 2);
    }

    #[tokio::test]
    async fn an_unreachable_issuer_is_unavailable() {
        let issuer = MockIssuer::start().await;
        let mut config = issuer.config(PUBLIC);
        config.providers[0].issuer = url::Url::parse("http://127.0.0.1:1").unwrap();
        let oidc = Oidc::new(&config).unwrap();
        let google = oidc.provider("google").unwrap();
        assert_unavailable(oidc.discovery(google).await, "google discovery");
    }

    #[tokio::test]
    async fn the_token_endpoint_refuses_a_wrong_verifier_or_a_reused_code() {
        let issuer = MockIssuer::start().await;
        let oidc = issuer.oidc(PUBLIC);
        let google = oidc.provider("google").unwrap();
        let verifier = pkce::verifier();
        let code = issuer.grant(&verifier, CALLBACK, NONCE, lena());
        assert_unavailable(
            oidc.exchange(google, &code, &pkce::verifier(), NONCE).await,
            "invalid_grant",
        );
        let code = issuer.grant(&verifier, CALLBACK, NONCE, lena());
        oidc.exchange(google, &code, &verifier, NONCE)
            .await
            .unwrap();
        assert_unavailable(
            oidc.exchange(google, &code, &verifier, NONCE).await,
            "invalid_grant",
        );
    }

    #[tokio::test]
    async fn a_token_with_a_wrong_nonce_audience_or_issuer_is_refused() {
        let issuer = MockIssuer::start().await;
        let oidc = issuer.oidc(PUBLIC);
        for (claim, value, reason) in [
            ("nonce", json!("other"), "nonce"),
            ("aud", json!("someone-else"), "aud"),
            ("iss", json!("http://127.0.0.1:1"), "iss"),
            ("exp", json!(1_000_000_000), "expired"),
        ] {
            let user = lena().with_claim(claim, value);
            assert_unavailable(exchange(&issuer, &oidc, user).await, reason);
        }
    }

    #[tokio::test]
    async fn the_detail_names_no_secret_code_or_address() {
        let issuer = MockIssuer::start().await;
        let oidc = issuer.oidc(PUBLIC);
        let user = lena().with_claim("nonce", json!("other"));
        let verifier = pkce::verifier();
        let code = issuer.grant(&verifier, CALLBACK, NONCE, user);
        let google = oidc.provider("google").unwrap();
        let Err(ApiError::OidcUnavailable(detail)) =
            oidc.exchange(google, &code, &verifier, NONCE).await
        else {
            panic!("expected a refusal");
        };
        for secret in [
            code.as_str(),
            verifier.as_str(),
            NONCE,
            "lena",
            "mock-secret",
        ] {
            assert!(!detail.contains(secret), "{detail} contains {secret}");
        }
    }

    #[tokio::test]
    async fn the_mock_authorizes_the_url_it_is_given() {
        let issuer = MockIssuer::start().await;
        let challenge = pkce::challenge(&pkce::verifier());
        let auth = format!(
            "{}/auth?response_type=code&client_id={}&redirect_uri={}&state=s1&nonce=n1\
             &code_challenge={challenge}&code_challenge_method=S256",
            issuer.issuer(),
            crate::common::mock_issuer::CLIENT_ID,
            CALLBACK,
        );
        let back = url::Url::parse(&issuer.authorize(&auth, lena())).unwrap();
        assert_eq!(back.path(), "/api/auth/oidc/google/callback");
        let query: Vec<_> = back.query_pairs().map(|(k, _)| k.into_owned()).collect();
        assert_eq!(query, ["code", "state"]);
        let denied = issuer.deny(&auth);
        assert!(
            denied.ends_with("?error=access_denied&state=s1"),
            "{denied}"
        );
    }
}

/// The routes of sign-in through a provider against the mock issuer: the
/// browser and the desktop app, account resolution and its attack cases.
mod flows {
    use axum::http::request::Builder;
    use axum::http::{Method, StatusCode};
    use gelabber_api::AppState;
    use gelabber_api::auth::oidc::pkce;
    use gelabber_api::config::RegistrationMode;
    use gelabber_api::mail::Mailer;
    use serde_json::{Value, json};
    use sqlx::PgPool;
    use url::Url;

    use crate::common::mock_issuer::{MockIssuer, MockUser};
    use crate::common::{self, Client, Response};
    use crate::{ADA, PASSWORD, outbox};

    const PUBLIC: &str = "http://localhost:5180";
    const HOST: &str = "localhost:5180";
    const START: &str = "/api/auth/oidc/google/start";

    async fn setup(pool: PgPool) -> (MockIssuer, AppState) {
        let issuer = MockIssuer::start().await;
        let mut state = common::state(pool);
        state.oidc = Some(issuer.oidc(PUBLIC));
        // Every step is an auth request; the flood test sets its own.
        state.limits.auth_per_min = 1000;
        (issuer, state)
    }

    fn with_mail(state: &mut AppState) -> Mailer {
        let mailer = Mailer::memory(&Url::parse(PUBLIC).unwrap());
        state.mail = Some(mailer.clone());
        mailer
    }

    async fn browser(state: &AppState) -> Client {
        let mut client = Client::from_state(state.clone());
        client.bootstrap().await;
        client
    }

    fn at_public_url(builder: &mut Builder) {
        builder
            .headers_mut()
            .unwrap()
            .insert("host", HOST.parse().unwrap());
    }

    async fn start(client: &mut Client, body: Value) -> Response {
        client
            .send_with(Method::POST, START, Some(body), at_public_url)
            .await
    }

    /// The provider's redirect back, followed by this browser.
    async fn follow(client: &mut Client, callback: &str) -> Response {
        let url = Url::parse(callback).unwrap();
        let path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        client
            .send_with(Method::GET, &path, None, at_public_url)
            .await
    }

    fn location(res: &Response) -> String {
        res.headers
            .get("location")
            .unwrap_or_else(|| panic!("no location: {} {}", res.status, res.body))
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// Starts a web sign-in, signs `user` in at the mock and follows the
    /// redirect back. Returns the callback's answer.
    async fn web_sign_in(
        issuer: &MockIssuer,
        client: &mut Client,
        body: Value,
        user: MockUser,
    ) -> Response {
        let res = start(client, body).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        let callback = issuer.authorize(res.body["url"].as_str().unwrap(), user);
        let res = follow(client, &callback).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.body);
        res
    }

    fn sign_in() -> Value {
        json!({ "purpose": "sign_in" })
    }

    fn ada() -> MockUser {
        MockUser::verified("sub-ada", "Ada@Example.com", "Ada Lovelace")
    }

    fn token_of(location: &str) -> String {
        location
            .strip_prefix("/sign-in/name#token=")
            .unwrap_or_else(|| panic!("not the name step: {location}"))
            .to_owned()
    }

    async fn count(pool: &PgPool, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    /// Ada registered with a password while mail was off and is signed in.
    async fn registered(state: &AppState, confirmed: bool) -> Client {
        let mut client = browser(state).await;
        let res = client.register(ADA, PASSWORD, "Ada").await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        if confirmed {
            sqlx::query("UPDATE users SET email_verified_at = now() WHERE email = $1")
                .bind(ADA)
                .execute(&state.db)
                .await
                .unwrap();
        }
        client
    }

    #[sqlx::test]
    async fn providers_are_listed_only_when_configured(pool: PgPool) {
        let mut off = Client::new(pool.clone());
        off.bootstrap().await;
        let res = off.send(Method::GET, "/api/auth/providers", None).await;
        assert_eq!(res.body, json!({ "providers": [] }));
        let res = start(&mut off, sign_in()).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

        let (_issuer, state) = setup(pool).await;
        let mut on = browser(&state).await;
        let res = on.send(Method::GET, "/api/auth/providers", None).await;
        assert_eq!(
            res.body,
            json!({ "providers": [{ "id": "google", "name": "Google" }] })
        );
        let res = on
            .send_with(
                Method::POST,
                "/api/auth/oidc/github/start",
                Some(sign_in()),
                at_public_url,
            )
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    }

    #[sqlx::test]
    async fn start_answers_the_authorization_url_and_binds_the_browser(pool: PgPool) {
        let (issuer, state) = setup(pool).await;
        let mut client = browser(&state).await;
        let res = start(
            &mut client,
            json!({ "purpose": "sign_in", "redirect": "//evil.example" }),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        let url = Url::parse(res.body["url"].as_str().unwrap()).unwrap();
        assert!(
            url.as_str()
                .starts_with(&format!("{}/auth?", issuer.issuer()))
        );
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["response_type"], "code");
        assert_eq!(
            query["redirect_uri"],
            "http://localhost:5180/api/auth/oidc/google/callback"
        );
        assert_eq!(query["scope"], "openid email profile");
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["prompt"], "select_account");
        assert_eq!(query["state"].len(), 64);
        assert!(!query.contains_key("response_mode"));
        let cookie = res.set_cookie("gelabber_oidc").expect("binding cookie");
        assert!(
            cookie.contains("; Path=/api/auth/oidc; Max-Age=1800; HttpOnly; SameSite=Lax"),
            "{cookie}"
        );

        // Not at the public address: the cookie would land on the wrong host.
        for host in [None, Some("127.0.0.1:5180"), Some("localhost:5181")] {
            let res = client
                .send_with(Method::POST, START, Some(sign_in()), |builder| {
                    if let Some(host) = host {
                        builder
                            .headers_mut()
                            .unwrap()
                            .insert("host", host.parse().unwrap());
                    }
                })
                .await;
            assert_eq!(res.status, StatusCode::CONFLICT, "{host:?} {}", res.body);
            assert_eq!(res.body["error"], "oidc_wrong_origin");
        }
        for body in [
            json!({}),
            json!({ "purpose": "other" }),
            json!({ "purpose": "sign_in", "invitation": "not a code" }),
        ] {
            let res = start(&mut client, body.clone()).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        }
        // A link needs a session.
        let res = start(&mut client, json!({ "purpose": "link" })).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    }

    #[sqlx::test]
    async fn a_verified_address_joins_a_confirmed_account(pool: PgPool) {
        let (issuer, mut state) = setup(pool.clone()).await;
        registered(&state, true).await;
        let mailer = with_mail(&mut state);

        let mut client = browser(&state).await;
        let res = web_sign_in(
            &issuer,
            &mut client,
            json!({ "purpose": "sign_in", "redirect": "/invite/abc123" }),
            ada(),
        )
        .await;
        assert_eq!(location(&res), "/invite/abc123");
        assert_eq!(res.headers["cache-control"], "no-store");
        assert_eq!(res.headers["referrer-policy"], "no-referrer");
        assert!(res.set_cookie(common::SESSION).is_some());
        assert!(res.set_cookie(common::CSRF).is_some());
        let cleared = res.set_cookie("gelabber_oidc").unwrap();
        assert!(
            cleared.contains("Path=/api/auth/oidc; Max-Age=0"),
            "{cleared}"
        );
        let session = client.bootstrap().await;
        assert_eq!(session.body["user"]["email"], ADA);

        let (subject, email): (String, String) =
            sqlx::query_as("SELECT subject, email FROM user_identities")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((subject.as_str(), email.as_str()), ("sub-ada", ADA));
        let sent = outbox(&mailer, 1).await;
        assert_eq!(sent[0].to, ADA);
        assert!(sent[0].subject.contains("Google"), "{}", sent[0].subject);
        // The account keeps its password.
        let mut again = browser(&state).await;
        assert_eq!(again.login(ADA, PASSWORD).await.status, StatusCode::OK);

        // The identity signs in from now on, without another notice.
        let mut other = browser(&state).await;
        let res = web_sign_in(&issuer, &mut other, sign_in(), ada()).await;
        assert_eq!(location(&res), "/");
        assert_eq!(other.bootstrap().await.body["user"]["email"], ADA);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(mailer.outbox().len(), 1);
    }

    #[sqlx::test]
    async fn a_state_works_once(pool: PgPool) {
        let (issuer, state) = setup(pool).await;
        registered(&state, true).await;
        let mut client = browser(&state).await;
        let res = start(&mut client, sign_in()).await;
        let binding = client.jar["gelabber_oidc"].clone();
        let callback = issuer.authorize(res.body["url"].as_str().unwrap(), ada());
        assert_eq!(location(&follow(&mut client, &callback).await), "/");
        // The callback cleared the binding; with it back, the used state
        // alone still fails.
        client.jar.insert("gelabber_oidc".to_owned(), binding);
        let res = follow(&mut client, &callback).await;
        assert_eq!(location(&res), "/login?oidc_error=expired");

        let res = follow(
            &mut client,
            "http://localhost:5180/api/auth/oidc/google/callback?code=x&state=unknown",
        )
        .await;
        assert_eq!(location(&res), "/login?oidc_error=expired");
    }

    #[sqlx::test]
    async fn a_callback_without_the_starting_browsers_cookie_expires(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        registered(&state, true).await;

        // Login CSRF: the attacker's own flow, completed in the victim's
        // browser, which has no binding or another one.
        let mut attacker = browser(&state).await;
        let res = start(&mut attacker, sign_in()).await;
        let callback = issuer.authorize(res.body["url"].as_str().unwrap(), ada());
        let mut victim = browser(&state).await;
        let res = follow(&mut victim, &callback).await;
        assert_eq!(location(&res), "/login?oidc_error=expired");
        assert!(res.set_cookie(common::SESSION).is_none());

        let res = start(&mut attacker, sign_in()).await;
        let callback = issuer.authorize(res.body["url"].as_str().unwrap(), ada());
        start(&mut victim, sign_in()).await;
        let res = follow(&mut victim, &callback).await;
        assert_eq!(location(&res), "/login?oidc_error=expired");
        assert!(res.set_cookie(common::SESSION).is_none());
        assert_eq!(
            count(&pool, "SELECT count(*) FROM user_identities").await,
            0
        );
    }

    #[sqlx::test]
    async fn cancelling_at_the_provider_is_reported(pool: PgPool) {
        let (issuer, state) = setup(pool).await;
        let mut client = browser(&state).await;
        let res = start(&mut client, sign_in()).await;
        let callback = issuer.deny(res.body["url"].as_str().unwrap());
        let res = follow(&mut client, &callback).await;
        assert_eq!(location(&res), "/login?oidc_error=cancelled");
        assert_eq!(issuer.token_hits(), 0);
    }

    #[sqlx::test]
    async fn an_unverified_address_creates_and_joins_nothing(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        let mut client = browser(&state).await;
        let user = MockUser::unverified("sub-x", "lena@example.com", "Lena");
        let res = web_sign_in(&issuer, &mut client, sign_in(), user).await;
        assert_eq!(location(&res), "/login?oidc_error=email_unverified");

        registered(&state, true).await;
        let user = MockUser::unverified("sub-x", ADA, "Ada");
        let res = web_sign_in(&issuer, &mut client, sign_in(), user).await;
        assert_eq!(location(&res), "/login?oidc_error=email_unverified");
        assert_eq!(count(&pool, "SELECT count(*) FROM users").await, 1);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM user_identities").await,
            0
        );
        assert_eq!(count(&pool, "SELECT count(*) FROM oidc_results").await, 0);
    }

    #[sqlx::test]
    async fn an_account_never_confirmed_is_refused(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        // Registered while mail was off: no confirmation, not pending.
        registered(&state, false).await;
        let mut client = browser(&state).await;
        let res = web_sign_in(&issuer, &mut client, sign_in(), ada()).await;
        assert_eq!(location(&res), "/login?oidc_error=account_unverified");
        assert!(res.set_cookie(common::SESSION).is_none());
        assert_eq!(
            count(&pool, "SELECT count(*) FROM user_identities").await,
            0
        );
    }

    #[sqlx::test]
    async fn a_disabled_account_is_refused(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        registered(&state, true).await;
        let mut client = browser(&state).await;
        web_sign_in(&issuer, &mut client, sign_in(), ada()).await;
        sqlx::query("UPDATE users SET disabled_at = now()")
            .execute(&pool)
            .await
            .unwrap();
        // Through its identity and through its address alike.
        for user in [ada(), MockUser::verified("sub-other", ADA, "Ada")] {
            let mut client = browser(&state).await;
            let res = web_sign_in(&issuer, &mut client, sign_in(), user).await;
            assert_eq!(location(&res), "/login?oidc_error=account_disabled");
            assert!(res.set_cookie(common::SESSION).is_none());
        }
    }

    #[sqlx::test]
    async fn a_new_account_picks_its_name_and_has_no_password(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        let mut client = browser(&state).await;
        let lena = MockUser::verified("sub-lena", "Lena@Example.com", "  Lena Example ");
        let res = web_sign_in(
            &issuer,
            &mut client,
            json!({ "purpose": "sign_in", "redirect": "/invite/abc123" }),
            lena,
        )
        .await;
        let token = token_of(&location(&res));
        // The binding stays for the name step.
        assert!(res.set_cookie("gelabber_oidc").is_none());
        assert!(res.set_cookie(common::SESSION).is_none());

        let mut stranger = browser(&state).await;
        let res = stranger
            .send(
                Method::POST,
                "/api/auth/oidc/pending",
                Some(json!({ "token": token })),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(
            res.body,
            json!({
                "provider": { "id": "google", "name": "Google" },
                "email": "lena@example.com",
                "name": "Lena Example",
            })
        );
        // Another browser cannot finish it.
        let res = stranger
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(json!({ "token": token, "name": "Mallory" })),
            )
            .await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
        assert_eq!(res.body["error"], "token_invalid");

        let res = client
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(json!({ "token": token, "name": " " })),
            )
            .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["fields"]["name"], "required");

        let res = client
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(json!({ "token": token, "name": "Lena", "locale": "en" })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        assert_eq!(res.body["user"]["email"], "lena@example.com");
        assert_eq!(res.body["user"]["name"], "Lena");
        assert_eq!(res.body["user"]["locale"], "en");
        assert_eq!(res.body["redirect"], "/invite/abc123");
        assert!(res.body["csrf_token"].is_string());
        assert!(res.set_cookie(common::SESSION).is_some());
        assert!(
            res.set_cookie("gelabber_oidc")
                .unwrap()
                .contains("Max-Age=0")
        );
        assert_eq!(
            client.bootstrap().await.body["user"]["email"],
            "lena@example.com"
        );

        let (hash, verified, pending): (Option<String>, bool, bool) = sqlx::query_as(
            "SELECT password_hash, email_verified_at IS NOT NULL, registration_pending \
             FROM users WHERE email = 'lena@example.com'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((hash, verified, pending), (None, true, false));
        let mut login = browser(&state).await;
        let res = login.login("lena@example.com", PASSWORD).await;
        assert_eq!(res.body["error"], "invalid_credentials");

        // The result is used up.
        let res = client
            .send(
                Method::POST,
                "/api/auth/oidc/pending",
                Some(json!({ "token": token })),
            )
            .await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);

        // The next sign-in finds the identity.
        let mut later = browser(&state).await;
        let lena = MockUser::verified("sub-lena", "lena@example.com", "Lena");
        let res = web_sign_in(&issuer, &mut later, sign_in(), lena).await;
        assert_eq!(location(&res), "/");
    }

    #[sqlx::test]
    async fn complete_resolves_again_for_an_account_that_appeared_meanwhile(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        let mut client = browser(&state).await;
        let res = web_sign_in(&issuer, &mut client, sign_in(), ada()).await;
        let token = token_of(&location(&res));
        let complete = json!({ "token": token, "name": "Someone" });

        // Registered while mail was off, between callback and name step.
        registered(&state, false).await;
        let res = client
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(complete.clone()),
            )
            .await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
        assert_eq!(res.body["error"], "oidc_account_unverified");

        // Once confirmed, the same result joins it instead of creating one.
        sqlx::query("UPDATE users SET email_verified_at = now()")
            .execute(&pool)
            .await
            .unwrap();
        let res = client
            .send(Method::POST, "/api/auth/oidc/complete", Some(complete))
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(res.body["user"]["name"], "Ada");
        assert_eq!(count(&pool, "SELECT count(*) FROM users").await, 1);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM user_identities").await,
            1
        );
    }

    #[sqlx::test]
    async fn a_pending_registration_is_taken_over(pool: PgPool) {
        let (issuer, mut state) = setup(pool.clone()).await;
        let mailer = with_mail(&mut state);
        // A stranger registered Ada's address and never confirmed it.
        let mut stranger = browser(&state).await;
        let res = stranger.register(ADA, PASSWORD, "Mallory").await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
        let link_token = crate::token(&outbox(&mailer, 1).await[0]);

        let mut client = browser(&state).await;
        let res = web_sign_in(&issuer, &mut client, sign_in(), ada()).await;
        let token = token_of(&location(&res));
        let res = client
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(json!({ "token": token, "name": "Ada" })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        assert_eq!(res.body["user"]["name"], "Ada");

        let (hash, verified, pending): (Option<String>, bool, bool) = sqlx::query_as(
            "SELECT password_hash, email_verified_at IS NOT NULL, registration_pending \
             FROM users WHERE email = $1",
        )
        .bind(ADA)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((hash, verified, pending), (None, true, false));
        assert_eq!(count(&pool, "SELECT count(*) FROM users").await, 1);
        assert_eq!(count(&pool, "SELECT count(*) FROM account_tokens").await, 0);

        // The stranger's password and confirmation link are gone.
        let res = stranger.login(ADA, PASSWORD).await;
        assert_eq!(res.body["error"], "invalid_credentials");
        let res = stranger
            .send(
                Method::POST,
                "/api/auth/verify-email",
                Some(json!({ "token": link_token, "password": PASSWORD })),
            )
            .await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    }

    #[sqlx::test]
    async fn a_new_account_passes_the_admission_gate_twice(pool: PgPool) {
        let (issuer, mut state) = setup(pool.clone()).await;
        state.registration = RegistrationMode::Invitation;
        let lena = || MockUser::verified("sub-lena", "lena@example.com", "Lena");

        let mut client = browser(&state).await;
        let res = web_sign_in(
            &issuer,
            &mut client,
            json!({ "purpose": "sign_in", "redirect": "/invite/abc123" }),
            lena(),
        )
        .await;
        // The redirect's server invite does not exist: it is a code, but
        // not a valid one.
        assert_eq!(
            location(&res),
            "/register?oidc_error=invitation_invalid&redirect=%2Finvite%2Fabc123"
        );
        let res = web_sign_in(&issuer, &mut client, sign_in(), lena()).await;
        assert_eq!(location(&res), "/register?oidc_error=invitation_required");
        let res = web_sign_in(
            &issuer,
            &mut client,
            json!({ "purpose": "sign_in", "invitation": "k7m2p9qrs4tx" }),
            lena(),
        )
        .await;
        assert_eq!(
            location(&res),
            "/register?oidc_error=invitation_invalid&invitation=k7m2p9qrs4tx"
        );
        assert_eq!(count(&pool, "SELECT count(*) FROM oidc_results").await, 0);

        sqlx::query("INSERT INTO beta_invitations (code) VALUES ('k7m2p9qrs4tx')")
            .execute(&pool)
            .await
            .unwrap();
        let res = web_sign_in(
            &issuer,
            &mut client,
            json!({ "purpose": "sign_in", "invitation": "K7M2P9QRS4TX" }),
            lena(),
        )
        .await;
        let token = token_of(&location(&res));

        // Revoked before the name step: refused again at complete, and the
        // result stays for another try.
        sqlx::query("UPDATE beta_invitations SET revoked_at = now()")
            .execute(&pool)
            .await
            .unwrap();
        let complete = json!({ "token": token, "name": "Lena" });
        let res = client
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(complete.clone()),
            )
            .await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
        assert_eq!(res.body["error"], "invitation_invalid");
        assert_eq!(count(&pool, "SELECT count(*) FROM users").await, 0);

        sqlx::query("UPDATE beta_invitations SET revoked_at = NULL")
            .execute(&pool)
            .await
            .unwrap();
        let res = client
            .send(Method::POST, "/api/auth/oidc/complete", Some(complete))
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        let used: bool =
            sqlx::query_scalar("SELECT used_by = (SELECT id FROM users) FROM beta_invitations")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(used);

        // Joining an existing account is never gated.
        let mut later = browser(&state).await;
        let res = web_sign_in(&issuer, &mut later, sign_in(), lena()).await;
        assert_eq!(location(&res), "/");
    }

    #[sqlx::test]
    async fn linking_needs_the_password_and_the_same_session(pool: PgPool) {
        let (issuer, mut state) = setup(pool.clone()).await;
        let mut ada_client = registered(&state, false).await;
        let mailer = with_mail(&mut state);
        let mut ada_client = {
            let mut moved = Client::from_state(state.clone());
            moved.jar = std::mem::take(&mut ada_client.jar);
            moved.csrf = ada_client.csrf.take();
            moved
        };
        let link = |password: Option<&str>| match password {
            Some(password) => json!({ "purpose": "link", "current_password": password }),
            None => json!({ "purpose": "link" }),
        };

        let res = start(&mut ada_client, link(None)).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["fields"]["current_password"], "required");
        let res = start(&mut ada_client, link(Some("wrong password"))).await;
        assert_eq!(res.body["fields"]["current_password"], "wrong");

        // The provider's address equals the account's: it counts as
        // confirmed now.
        let res = web_sign_in(&issuer, &mut ada_client, link(Some(PASSWORD)), ada()).await;
        assert_eq!(location(&res), "/settings/konto?connected=google");
        assert!(res.set_cookie(common::SESSION).is_none());
        let verified: bool =
            sqlx::query_scalar("SELECT email_verified_at IS NOT NULL FROM users WHERE email = $1")
                .bind(ADA)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(verified);
        assert!(outbox(&mailer, 1).await[0].subject.contains("Google"));
        let res = start(&mut ada_client, link(Some(PASSWORD))).await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
        assert_eq!(res.body["error"], "identity_taken");

        // Bob cannot attach Ada's Google account.
        let mut bob = browser(&state).await;
        let res = bob.register("bob@example.com", PASSWORD, "Bob").await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
        sqlx::query("UPDATE users SET email_verified_at = now(), registration_pending = false")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            bob.login("bob@example.com", PASSWORD).await.status,
            StatusCode::OK
        );
        let res = web_sign_in(&issuer, &mut bob, link(Some(PASSWORD)), ada()).await;
        assert_eq!(location(&res), "/settings/konto?oidc_error=identity_taken");

        // The browser signed in as someone else before coming back.
        let res = start(&mut bob, link(Some(PASSWORD))).await;
        let callback = issuer.authorize(
            res.body["url"].as_str().unwrap(),
            MockUser::verified("sub-bob", "bob@example.com", "Bob"),
        );
        assert_eq!(bob.login(ADA, PASSWORD).await.status, StatusCode::OK);
        let res = follow(&mut bob, &callback).await;
        assert_eq!(location(&res), "/settings/konto?oidc_error=session_changed");
        assert_eq!(
            count(&pool, "SELECT count(*) FROM user_identities").await,
            1
        );
    }

    fn desktop(verifier: &str) -> Value {
        json!({
            "port": 49152,
            "state": "app-state-0123456789-0123456789-0123456789~x",
            "challenge": pkce::challenge(verifier),
        })
    }

    /// Starts a desktop flow, signs `user` in at the mock and returns where
    /// the system browser is sent: the app's loopback receiver.
    async fn desktop_flow(
        issuer: &MockIssuer,
        state: &AppState,
        app: &mut Client,
        purpose: Value,
        verifier: &str,
        user: MockUser,
    ) -> Url {
        let mut body = purpose;
        body["desktop"] = desktop(verifier);
        // The app may talk to any address of the instance.
        let res = app.send(Method::POST, START, Some(body)).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert!(res.set_cookie("gelabber_oidc").is_none());
        let callback = match user.sub.as_str() {
            "deny" => issuer.deny(res.body["url"].as_str().unwrap()),
            _ => issuer.authorize(res.body["url"].as_str().unwrap(), user),
        };
        // The system browser has none of the app's cookies.
        let mut system_browser = Client::from_state(state.clone());
        let res = follow(&mut system_browser, &callback).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.body);
        assert!(res.set_cookie(common::SESSION).is_none());
        Url::parse(&location(&res)).unwrap()
    }

    /// The code the loopback receiver got, after checking the URL's shape.
    fn loopback_code(url: &Url) -> String {
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.port(), Some(49152));
        assert_eq!(url.path(), "/callback");
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(query[0].0, "state");
        assert_eq!(query[0].1, "app-state-0123456789-0123456789-0123456789~x");
        assert_eq!(query[1].0, "code", "{url}");
        query[1].1.clone()
    }

    async fn exchange(app: &mut Client, code: &str, verifier: &str) -> Response {
        app.send(
            Method::POST,
            "/api/auth/oidc/desktop/exchange",
            Some(json!({ "code": code, "verifier": verifier })),
        )
        .await
    }

    #[sqlx::test]
    async fn the_desktop_app_names_a_new_account_then_signs_in(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        let mut app = browser(&state).await;
        let verifier = pkce::verifier();
        let lena = || MockUser::verified("sub-lena", "lena@example.com", "Lena");
        let url = desktop_flow(&issuer, &state, &mut app, sign_in(), &verifier, lena()).await;
        let code = loopback_code(&url);

        let res = exchange(&mut app, &code, &pkce::verifier()).await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
        assert_eq!(res.body["error"], "token_invalid");

        let res = exchange(&mut app, &code, &verifier).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(
            res.body,
            json!({
                "result": "needs_name",
                "provider": { "id": "google", "name": "Google" },
                "email": "lena@example.com",
                "name": "Lena",
            })
        );
        assert!(res.set_cookie(common::SESSION).is_none());

        // complete with the same code; the verifier is the binding.
        let complete =
            |verifier: &str| json!({ "token": code, "name": "Lena", "verifier": verifier });
        let res = app
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(complete(&pkce::verifier())),
            )
            .await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
        let res = app
            .send(
                Method::POST,
                "/api/auth/oidc/complete",
                Some(complete(&verifier)),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        assert!(res.set_cookie(common::SESSION).is_some());
        assert_eq!(
            app.bootstrap().await.body["user"]["email"],
            "lena@example.com"
        );

        // The next desktop sign-in is one exchange.
        let mut app = browser(&state).await;
        let verifier = pkce::verifier();
        let url = desktop_flow(&issuer, &state, &mut app, sign_in(), &verifier, lena()).await;
        let code = loopback_code(&url);
        let res = exchange(&mut app, &code, &verifier).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(res.body["result"], "signed_in");
        assert_eq!(res.body["user"]["email"], "lena@example.com");
        assert!(res.body["csrf_token"].is_string());
        assert!(res.set_cookie(common::SESSION).is_some());
        // Used up.
        let res = exchange(&mut app, &code, &verifier).await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    }

    #[sqlx::test]
    async fn the_desktop_app_gets_refusals_as_json_and_errors_at_its_port(pool: PgPool) {
        let (issuer, mut state) = setup(pool.clone()).await;
        let mut app = browser(&state).await;
        let verifier = pkce::verifier();
        let deny = MockUser::verified("deny", "x@example.com", "X");
        let url = desktop_flow(&issuer, &state, &mut app, sign_in(), &verifier, deny).await;
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:49152/callback?state=app-state-0123456789-0123456789-0123456789%7Ex&error=cancelled"
        );

        let user = MockUser::unverified("sub-x", "x@example.com", "X");
        let url = desktop_flow(&issuer, &state, &mut app, sign_in(), &verifier, user).await;
        let res = exchange(&mut app, &loopback_code(&url), &verifier).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
        assert_eq!(res.body["error"], "oidc_email_unverified");

        registered(&state, false).await;
        let url = desktop_flow(&issuer, &state, &mut app, sign_in(), &verifier, ada()).await;
        let res = exchange(&mut app, &loopback_code(&url), &verifier).await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
        assert_eq!(res.body["error"], "oidc_account_unverified");

        state.registration = RegistrationMode::Invitation;
        let mut app = browser(&state).await;
        let lena = MockUser::verified("sub-lena", "lena@example.com", "Lena");
        let url = desktop_flow(&issuer, &state, &mut app, sign_in(), &verifier, lena).await;
        let res = exchange(&mut app, &loopback_code(&url), &verifier).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
        assert_eq!(res.body["error"], "invitation_required");
        assert_eq!(
            count(&pool, "SELECT count(*) FROM user_identities").await,
            0
        );
    }

    #[sqlx::test]
    async fn the_desktop_app_links_through_its_own_session(pool: PgPool) {
        let (issuer, state) = setup(pool.clone()).await;
        let mut app = registered(&state, false).await;
        let verifier = pkce::verifier();
        let link = json!({ "purpose": "link", "current_password": PASSWORD });
        let url = desktop_flow(&issuer, &state, &mut app, link.clone(), &verifier, ada()).await;
        let code = loopback_code(&url);

        // Another session (or none) cannot take it.
        let mut other = browser(&state).await;
        let res = exchange(&mut other, &code, &verifier).await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
        assert_eq!(res.body["error"], "session_changed");

        let res = exchange(&mut app, &code, &verifier).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(res.body, json!({ "result": "linked" }));
        let owner: String = sqlx::query_scalar(
            "SELECT u.email FROM user_identities i JOIN users u ON u.id = i.user_id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(owner, ADA);
    }

    #[sqlx::test]
    async fn a_callback_flood_is_rate_limited(pool: PgPool) {
        let (_issuer, mut state) = setup(pool).await;
        state.limits.auth_per_min = 20;
        let mut client = browser(&state).await;
        let mut statuses = Vec::new();
        for _ in 0..25 {
            let res = follow(
                &mut client,
                "http://localhost:5180/api/auth/oidc/google/callback?code=x&state=y",
            )
            .await;
            statuses.push(res.status);
        }
        assert_eq!(statuses[0], StatusCode::SEE_OTHER);
        assert_eq!(statuses[24], StatusCode::TOO_MANY_REQUESTS, "{statuses:?}");
    }
}
