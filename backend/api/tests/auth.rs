//! End-to-end auth flow against a real Postgres (`#[sqlx::test]` creates one
//! throw-away database per test from `DATABASE_URL` and applies
//! `api/migrations`). Requests are driven in-process via `common::Client`.

mod common;

use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::{CSRF, Client, SESSION};

fn assert_session_cookie_attributes(raw: &str) {
    for attr in ["HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(raw.contains(attr), "{attr} missing in {raw}");
    }
    assert!(raw.contains("Max-Age=7200"), "ttl from config: {raw}");
    assert!(!raw.contains("Secure"), "Secure is opt-in via env: {raw}");
}

#[sqlx::test]
async fn session_bootstrap_issues_csrf_cookie_without_user(pool: PgPool) {
    let mut client = Client::new(pool);

    let first = client.bootstrap().await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.body["user"], Value::Null);
    let token = first.body["csrf_token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64);
    let cookie = first.set_cookie(CSRF).expect("csrf cookie issued");
    assert!(cookie.contains(&token));
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));

    // Second call reuses the cookie instead of rotating it.
    let second = client.bootstrap().await;
    assert_eq!(second.body["csrf_token"], token);
    assert!(second.set_cookie(CSRF).is_none());
}

#[sqlx::test]
async fn register_signs_in_and_sets_httponly_lax_cookies(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;

    let res = client
        .register("  Ada@Example.com ", "correct horse battery", " Ada ")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["user"]["email"], "ada@example.com");
    assert_eq!(res.body["user"]["name"], "Ada");
    assert_eq!(res.body["user"]["avatar_url"], Value::Null);
    assert!(res.body["user"]["id"].is_string());
    assert!(res.body["user"].get("password_hash").is_none());
    assert!(res.body["user"].get("password").is_none());

    let session_cookie = res.set_cookie(SESSION).expect("session cookie");
    assert_session_cookie_attributes(&session_cookie);
    let csrf_cookie = res.set_cookie(CSRF).expect("csrf rotated on register");
    assert!(csrf_cookie.contains(res.body["csrf_token"].as_str().unwrap()));

    // The browser is now "in": session resolves to the user.
    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["email"], "ada@example.com");

    let session = client.bootstrap().await;
    assert_eq!(session.body["user"]["email"], "ada@example.com");

    // Argon2id PHC string in the database, nothing resembling the password.
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(stored.starts_with("$argon2id$v=19$"), "{stored}");
    assert!(!stored.contains("correct horse"));

    // Only the hash of the session token is stored.
    let hashes: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(hashes.len(), 1);
    let raw_token = client.jar.get(SESSION).unwrap();
    assert_ne!(hashes[0], raw_token.as_bytes());
    assert_eq!(hashes[0].len(), 32);
}

#[sqlx::test]
async fn register_validates_inline(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    let res = client.register("nope", "short", "   ").await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["error"], "validation_failed");
    assert_eq!(res.body["fields"]["email"], "invalid");
    assert_eq!(res.body["fields"]["password"], "too_short");
    assert_eq!(res.body["fields"]["name"], "required");
    assert!(res.set_cookie(SESSION).is_none());

    let res = client
        .send(
            Method::POST,
            "/api/auth/register",
            Some(json!({ "email": "a@b.co" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["password"], "required");
    assert_eq!(res.body["fields"]["name"], "required");
}

/// Display-name and quoted forms all reach one inbox; registration takes
/// only the bare address, sign-in still looks such a string up.
#[sqlx::test]
async fn register_takes_only_a_bare_address(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    for email in ["a<victim@example.org>", "\"victim\"@example.org"] {
        let res = client.register(email, "correct horse", "Ada").await;
        assert_eq!(
            res.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{email}: {}",
            res.body
        );
        assert_eq!(res.body["fields"]["email"], "invalid", "{email}");

        let res = client.login(email, "correct horse").await;
        assert_eq!(
            res.status,
            StatusCode::UNAUTHORIZED,
            "{email}: {}",
            res.body
        );
    }
}

/// Sign-in looks the address up through the `lower(email)` index, so a row
/// stored with capitals (before addresses were normalised) is found too.
#[sqlx::test]
async fn login_finds_the_address_through_the_lower_case_index(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client.send(Method::POST, "/api/auth/logout", None).await;
    sqlx::query("UPDATE users SET email = 'Ada@Example.com'")
        .execute(&pool)
        .await
        .unwrap();

    let res = client.login("ada@example.com", "password123").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

/// Two spellings of one domain are one account: it is stored with the
/// ASCII form of its domain, and either spelling signs in.
#[sqlx::test]
async fn a_domain_in_two_spellings_is_one_account(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let res = client
        .register("victim@bücher.de", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["user"]["email"], "victim@xn--bcher-kva.de");
    let res = client
        .register("victim@xn--bcher-kva.de", "otherpassword", "Ada 2")
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);

    for spelling in ["victim@bücher.de", "victim@xn--bcher-kva.de"] {
        client.send(Method::POST, "/api/auth/logout", None).await;
        let res = client.login(spelling, "password123").await;
        assert_eq!(res.status, StatusCode::OK, "{spelling}: {}", res.body);
    }
}

#[sqlx::test]
async fn fullwidth_letters_name_the_same_domain(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let res = client
        .register("victim@example.org", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = client
        .register("victim@ｅｘａｍｐｌｅ.org", "otherpassword", "Mallory")
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);

    client.send(Method::POST, "/api/auth/logout", None).await;
    let res = client
        .login("victim@ｅｘａｍｐｌｅ.org", "password123")
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test]
async fn register_rejects_duplicate_email_case_insensitively(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let first = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(first.status, StatusCode::CREATED);

    let dup = client
        .register("ADA@example.com", "otherpassword", "Ada 2")
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT, "{}", dup.body);
    assert_eq!(dup.body["error"], "email_taken");
    assert_eq!(dup.body["fields"]["email"], "taken");
}

#[sqlx::test]
async fn schema_enforces_email_uniqueness_case_insensitively(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    client
        .register("ada@example.com", "password123", "Ada")
        .await;

    // A write path that skips the API's normalisation still cannot create a
    // second account for the same address.
    let err = sqlx::query("INSERT INTO users (email, name, password_hash) VALUES ($1, 'x', 'y')")
        .bind("ADA@Example.com")
        .execute(&pool)
        .await
        .unwrap_err();
    match err {
        sqlx::Error::Database(db) => assert_eq!(db.code().as_deref(), Some("23505"), "{db}"),
        other => panic!("expected unique violation, got {other}"),
    }
}

#[sqlx::test]
async fn signing_in_again_revokes_the_previous_session(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    client
        .register("ada@example.com", "password123", "Ada")
        .await;
    let first_token = client.jar.get(SESSION).unwrap().clone();

    // Same browser logs in twice more without ever logging out.
    let res = client.login("ada@example.com", "password123").await;
    assert_eq!(res.status, StatusCode::OK);
    let res = client.login("ada@example.com", "password123").await;
    assert_eq!(res.status, StatusCode::OK);
    let current_token = client.jar.get(SESSION).unwrap().clone();
    assert_ne!(first_token, current_token);

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1, "one live row per browser, no orphans");

    client.jar.insert(SESSION.to_owned(), first_token);
    let replay = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED, "old token is dead");

    client.jar.insert(SESSION.to_owned(), current_token);
    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);

    // Expired rows from other browsers are swept by the next sign-in.
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, expires_at) \
         SELECT decode(repeat('ab', 32), 'hex'), id, now() - interval '1 minute' FROM users",
    )
    .execute(&pool)
    .await
    .unwrap();
    client.login("ada@example.com", "password123").await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[sqlx::test]
async fn login_rejects_overlong_passwords_as_invalid_credentials(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    client
        .register("ada@example.com", "password123", "Ada")
        .await;
    client.send(Method::POST, "/api/auth/logout", None).await;

    let res = client.login("ada@example.com", &"x".repeat(129)).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    assert_eq!(res.body["error"], "invalid_credentials");
    assert!(res.set_cookie(SESSION).is_none());

    // Exactly the maximum is still checked normally.
    let res = client.login("ada@example.com", &"x".repeat(128)).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "invalid_credentials");
}

#[sqlx::test]
async fn login_logout_round_trip(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    client
        .register("ada@example.com", "password123", "Ada")
        .await;
    client.send(Method::POST, "/api/auth/logout", None).await;
    assert!(
        !client.jar.contains_key(SESSION),
        "logout clears the cookie"
    );

    let wrong = client.login("ada@example.com", "password124").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.body["error"], "invalid_credentials");

    let unknown = client.login("nobody@example.com", "password123").await;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        unknown.body, wrong.body,
        "unknown user is indistinguishable"
    );

    let ok = client.login("Ada@Example.com", "password123").await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert_eq!(ok.body["user"]["email"], "ada@example.com");
    assert_session_cookie_attributes(&ok.set_cookie(SESSION).unwrap());
    let session_token = client.jar.get(SESSION).unwrap().clone();

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::OK);

    let out = client.send(Method::POST, "/api/auth/logout", None).await;
    assert_eq!(out.status, StatusCode::OK, "{}", out.body);
    let cleared = out.set_cookie(SESSION).unwrap();
    assert!(cleared.starts_with("gelabber_session=;"), "{cleared}");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(out.body["csrf_token"].is_string(), "csrf rotates on logout");
    assert!(!client.jar.contains_key(SESSION));

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    assert_eq!(me.body["error"], "unauthenticated");

    // The old token is dead server-side too, not just forgotten by the browser.
    client.jar.insert(SESSION.to_owned(), session_token);
    let replay = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn mutations_require_matching_csrf_header(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let cookie_token = client.jar.get(CSRF).unwrap().clone();

    // No header at all.
    client.csrf = None;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "csrf_invalid");

    // Header does not match the cookie.
    client.csrf = Some("0".repeat(64));
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body["error"], "csrf_invalid");

    // Header matches, but no cookie (token stolen without the cookie).
    client.csrf = Some(cookie_token.clone());
    client.jar.remove(CSRF);
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // Browser-flagged cross-site request with an otherwise valid pair.
    client.jar.insert(CSRF.to_owned(), cookie_token.clone());
    let res = client
        .send_with(
            Method::POST,
            "/api/auth/register",
            Some(json!({ "email": "ada@example.com", "password": "password123", "name": "Ada" })),
            |b| {
                let headers = b.headers_mut().unwrap();
                headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
            },
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body["error"], "csrf_invalid");

    // Matching pair: goes through.
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    // Authenticated mutations are covered as well.
    client.csrf = None;
    let res = client
        .send(Method::PATCH, "/api/me", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = client.send(Method::POST, "/api/auth/logout", None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // GET never needs the header.
    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[sqlx::test]
async fn profile_name_and_avatar_are_readable_and_editable(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    client
        .register("ada@example.com", "password123", "Ada")
        .await;

    let res = client
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "name": " Ada Lovelace ", "avatar_url": "https://cdn.example/ada.png" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "Ada Lovelace");
    assert_eq!(res.body["avatar_url"], "https://cdn.example/ada.png");

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(me.body["name"], "Ada Lovelace");
    assert_eq!(me.body["avatar_url"], "https://cdn.example/ada.png");

    // Partial patch leaves the other field alone; empty avatar clears it.
    let res = client
        .send(Method::PATCH, "/api/me", Some(json!({ "avatar_url": "" })))
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["name"], "Ada Lovelace");
    assert_eq!(res.body["avatar_url"], Value::Null);

    let res = client
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "name": "", "avatar_url": "javascript:alert(1)" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["name"], "required");
    assert_eq!(res.body["fields"]["avatar_url"], "invalid");

    // Plain http would be mixed content behind TLS.
    let res = client
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "avatar_url": "http://cdn.example/ada.png" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["avatar_url"], "invalid");

    let me = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(
        me.body["name"], "Ada Lovelace",
        "rejected patch changed nothing"
    );

    // Session shows the same profile the client would render after login.
    let session = client.bootstrap().await;
    assert_eq!(session.body["user"]["name"], "Ada Lovelace");
}

#[sqlx::test]
async fn profile_requires_a_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "unauthenticated");

    let res = client
        .send(Method::PATCH, "/api/me", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    // A forged cookie value is just "no session", never a 500.
    client
        .jar
        .insert(SESSION.to_owned(), "not-a-real-token".to_owned());
    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    client.jar.insert(SESSION.to_owned(), "f".repeat(64));
    let res = client.send(Method::GET, "/api/me", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn malformed_json_is_a_json_error(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    let res = client
        .send_with(Method::POST, "/api/auth/login", None, |b| {
            b.headers_mut()
                .unwrap()
                .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        })
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.body);
    assert_eq!(res.body["error"], "bad_request");

    let res = client
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!("not an object")),
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.body["error"], "bad_request");
}

// --- Admission on an invitation-only instance (#517) ---

const INVITED_PASSWORD: &str = "correct horse";

fn invitation_state(pool: PgPool) -> gelabber_api::AppState {
    let mut state = common::state(pool);
    state.registration = gelabber_api::config::RegistrationMode::Invitation;
    state
}

/// A beta invitation row as the operator page would write it, expiring in
/// `expires_in` seconds (negative: expired), used or revoked.
async fn beta_invitation(
    pool: &PgPool,
    code: &str,
    email: Option<&str>,
    expires_in: Option<i64>,
    used: bool,
    revoked: bool,
) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO beta_invitations (code, email, expires_at, used_at, revoked_at) \
         VALUES ($1, $2, now() + make_interval(secs => $3::double precision), \
             CASE WHEN $4 THEN now() END, CASE WHEN $5 THEN now() END) \
         RETURNING id",
    )
    .bind(code)
    .bind(email)
    .bind(expires_in)
    .bind(used)
    .bind(revoked)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn server_invite(founder: &mut Client, server: &str, body: Value) -> String {
    let res = founder
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(body),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["code"].as_str().unwrap().to_owned()
}

async fn register_with(client: &mut Client, email: &str, codes: Value) -> common::Response {
    let mut body = json!({ "email": email, "password": INVITED_PASSWORD, "name": "Neu" });
    for (key, value) in codes.as_object().unwrap() {
        body[key] = value.clone();
    }
    client
        .send(Method::POST, "/api/auth/register", Some(body))
        .await
}

async fn fresh(state: &gelabber_api::AppState) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    client
}

#[sqlx::test]
async fn registration_mode_is_public(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    let res = client
        .send(Method::GET, "/api/auth/registration", None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!({ "mode": "open" }));

    let mut client = fresh(&invitation_state(pool)).await;
    let res = client
        .send(Method::GET, "/api/auth/registration", None)
        .await;
    assert_eq!(res.body, json!({ "mode": "invitation" }));
}

#[sqlx::test]
async fn open_registration_ignores_invitation_codes(pool: PgPool) {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let res = register_with(
        &mut client,
        "ada@example.com",
        json!({ "invitation": "kein-code", "server_invite": "kkkkkkkkkkkk" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    // A valid invitation is not used up either.
    let id = beta_invitation(&pool, "k7m2p9qrs4tx", None, None, false, false).await;
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let res = register_with(
        &mut client,
        "bob@example.com",
        json!({ "invitation": "k7m2p9qrs4tx" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let used: bool =
        sqlx::query_scalar("SELECT used_at IS NOT NULL FROM beta_invitations WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!used);
}

#[sqlx::test]
async fn invitation_only_registration_needs_a_code(pool: PgPool) {
    let state = invitation_state(pool.clone());
    // An address that has an account answers alike: the code is checked
    // before any account lookup.
    sqlx::query(
        "INSERT INTO users (email, name, password_hash) VALUES ('ada@example.com', 'Ada', 'x')",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (email, codes) in [
        ("new@example.com", json!({})),
        ("ada@example.com", json!({})),
        (
            "new@example.com",
            json!({ "invitation": "  ", "server_invite": "" }),
        ),
        ("new@example.com", json!({ "invitation": null })),
    ] {
        let mut client = fresh(&state).await;
        let res = register_with(&mut client, email, codes.clone()).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{codes} {}", res.body);
        assert_eq!(res.body["error"], "invitation_required");
        assert_eq!(res.body["fields"], json!({ "invitation": "required" }));
        assert!(res.set_cookie(SESSION).is_none());
    }
    // Field errors still come first.
    let mut client = fresh(&state).await;
    let res = register_with(&mut client, "not-an-address", json!({})).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
}

#[sqlx::test]
async fn invalid_invitations_answer_alike(pool: PgPool) {
    let state = invitation_state(pool.clone());
    beta_invitation(&pool, "aaaaaaaaaaaa", None, Some(-60), false, false).await;
    beta_invitation(&pool, "bbbbbbbbbbbb", None, None, false, true).await;
    beta_invitation(&pool, "cccccccccccc", None, None, true, false).await;
    beta_invitation(
        &pool,
        "dddddddddddd",
        Some("bob@example.com"),
        None,
        false,
        false,
    )
    .await;
    for code in [
        "eeeeeeeeeeee", // unknown
        "zu-kurz",      // not a code at all
        "aaaaaaaaaaaa", // expired
        "bbbbbbbbbbbb", // revoked
        "cccccccccccc", // used
        "dddddddddddd", // for another address
    ] {
        let mut client = fresh(&state).await;
        let res = register_with(
            &mut client,
            "ada@example.com",
            json!({ "invitation": code }),
        )
        .await;
        assert_eq!(res.status, StatusCode::GONE, "{code}: {}", res.body);
        assert_eq!(res.body["error"], "invitation_invalid");
        assert_eq!(res.body["fields"], json!({ "invitation": "invalid" }));
    }
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
}

#[sqlx::test]
async fn a_valid_invitation_admits_once(pool: PgPool) {
    let state = invitation_state(pool.clone());
    let id = beta_invitation(
        &pool,
        "k7m2p9qrs4tx",
        Some("ada@example.com"),
        Some(86_400),
        false,
        false,
    )
    .await;

    // Retyped in capitals, and the address in another spelling.
    let mut ada = fresh(&state).await;
    let res = register_with(
        &mut ada,
        "Ada@Example.com",
        json!({ "invitation": " K7M2P9QRS4TX " }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert!(res.set_cookie(SESSION).is_some());
    let used_by: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT used_by FROM beta_invitations WHERE id = $1 AND used_at IS NOT NULL",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        used_by.map(|id| id.to_string()).as_deref(),
        Some(ada.user_id())
    );

    // Nobody else gets in with it, not even the same address again.
    let mut again = fresh(&state).await;
    let res = register_with(
        &mut again,
        "ada2@example.com",
        json!({ "invitation": "k7m2p9qrs4tx" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
}

#[sqlx::test]
async fn two_registrations_racing_for_one_invitation_admit_one(pool: PgPool) {
    let state = invitation_state(pool.clone());
    let id = beta_invitation(&pool, "k7m2p9qrs4tx", None, None, false, false).await;
    let mut a = fresh(&state).await;
    let mut b = fresh(&state).await;
    let code = json!({ "invitation": "k7m2p9qrs4tx" });
    let (ra, rb) = tokio::join!(
        register_with(&mut a, "a@example.com", code.clone()),
        register_with(&mut b, "b@example.com", code.clone()),
    );
    let mut statuses = [ra.status, rb.status];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::GONE],
        "{} {}",
        ra.body,
        rb.body
    );
    let users: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM users")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(users.len(), 1, "the loser's account rolled back");
    let used_by: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT used_by FROM beta_invitations WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(used_by, Some(users[0]));
}

#[sqlx::test]
async fn a_server_invite_admits_unless_its_server_is_blocked(pool: PgPool) {
    let state = invitation_state(pool.clone());
    beta_invitation(&pool, "k7m2p9qrs4tx", None, None, false, false).await;
    let mut founder = fresh(&state).await;
    let res = register_with(
        &mut founder,
        "founder@example.com",
        json!({ "invitation": "k7m2p9qrs4tx" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = founder
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Runde" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = res.body["id"].as_str().unwrap().to_owned();
    let open = server_invite(&mut founder, &server, json!({})).await;
    let single = server_invite(&mut founder, &server, json!({ "max_uses": 1 })).await;
    sqlx::query("UPDATE invites SET uses = 1 WHERE code = $1")
        .bind(&single)
        .execute(&pool)
        .await
        .unwrap();

    let mut member = fresh(&state).await;
    let res = register_with(
        &mut member,
        "member@example.com",
        json!({ "server_invite": open }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    // Not used up by the registration; joining counts it.
    let uses: i32 = sqlx::query_scalar("SELECT uses FROM invites WHERE code = $1")
        .bind(&open)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uses, 0);

    // A stale beta code next to a good server invite still admits.
    let mut other = fresh(&state).await;
    let res = register_with(
        &mut other,
        "other@example.com",
        json!({ "invitation": "k7m2p9qrs4tx", "server_invite": open }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let mut late = fresh(&state).await;
    let res = register_with(
        &mut late,
        "late@example.com",
        json!({ "server_invite": single }),
    )
    .await;
    assert_eq!(res.status, StatusCode::GONE, "used up: {}", res.body);

    sqlx::query("UPDATE servers SET blocked_at = now() WHERE id = $1::uuid")
        .bind(&server)
        .execute(&pool)
        .await
        .unwrap();
    let res = register_with(
        &mut late,
        "late@example.com",
        json!({ "server_invite": open }),
    )
    .await;
    assert_eq!(res.status, StatusCode::GONE, "blocked: {}", res.body);
    assert_eq!(res.body["error"], "invitation_invalid");
}

#[test]
fn register_bodies_print_their_codes_redacted() {
    let body = gelabber_api::auth::RegisterBody {
        email: "ada@example.com".into(),
        password: "geheim".into(),
        name: "Ada".into(),
        locale: None,
        invitation: Some("k7m2p9qrs4tx".into()),
        server_invite: Some("kkkkkkkkkkkk".into()),
    };
    let printed = format!("{body:?}");
    assert!(
        !printed.contains("k7m2p9qrs4tx") && !printed.contains("kkkkkkkkkkkk"),
        "{printed}"
    );
    assert!(!printed.contains("geheim"));
}
