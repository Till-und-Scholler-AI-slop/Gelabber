//! Feedback (#542), the public form: posting it, the confirmation link, the
//! limits around both and the sweeper of pending entries.

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::header::{ACCEPT_LANGUAGE, CONTENT_TYPE, LOCATION};
use axum::http::{Method, Request, StatusCode};
use gelabber_api::feedback::public;
use gelabber_api::mail::{Mailer, Sent};
use gelabber_api::{AppState, Config, app};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "password123";

fn config() -> Config {
    Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "API_LINK_PREVIEWS" => Some("false".to_owned()),
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(common::redis_url()),
        "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
        _ => None,
    })
    .expect("test config")
}

/// A state with `lena@example.test` as the operator and account mail into
/// memory, either of which can be left out.
fn state_with(pool: PgPool, operators: bool, mail: bool) -> (AppState, Mailer) {
    let mut state = AppState::with_pool(&config(), pool).expect("state");
    if operators {
        state.operators = Arc::new(HashSet::from(["lena@example.test".to_owned()]));
    }
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    if mail {
        state.mail = Some(mailer.clone());
    }
    (state, mailer)
}

fn form_state(pool: PgPool) -> (AppState, Mailer) {
    state_with(pool, true, true)
}

/// The form's body, as a browser encodes it.
fn form(fields: &[(&str, &str)]) -> String {
    let mut out = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in fields {
        out.append_pair(name, value);
    }
    out.finish()
}

fn valid(lang: &str, email: &str) -> String {
    form(&[
        ("lang", lang),
        ("kind", "problem"),
        ("text", "Der Knopf ist kaputt.\nGeheimes Detail"),
        ("email", email),
        ("website", ""),
    ])
}

/// The answer's status, `Location` and body.
async fn request(state: &AppState, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = app(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(LOCATION)
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, location, body)
}

/// Posts `body` from `ip` with `Sec-Fetch-Site` set to `site`, if any.
async fn post_from(
    state: &AppState,
    body: String,
    ip: &str,
    site: Option<&str>,
) -> (StatusCode, Option<String>, Value) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/api/public/feedback")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("x-forwarded-for", ip);
    if let Some(site) = site {
        builder = builder.header("sec-fetch-site", site);
    }
    request(state, builder.body(Body::from(body)).unwrap()).await
}

async fn post(state: &AppState, body: String) -> (StatusCode, Option<String>, Value) {
    post_from(state, body, "198.51.100.7", Some("same-origin")).await
}

async fn open(
    state: &AppState,
    query: &str,
    accept_language: Option<&str>,
) -> (StatusCode, Option<String>, Value) {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/public/feedback/confirm{query}"))
        .header("x-forwarded-for", "198.51.100.7");
    if let Some(language) = accept_language {
        builder = builder.header(ACCEPT_LANGUAGE, language);
    }
    request(state, builder.body(Body::empty()).unwrap()).await
}

async fn see_other(answer: (StatusCode, Option<String>, Value), location: &str) {
    assert_eq!(answer.0, StatusCode::SEE_OTHER, "{:?}", answer.2);
    assert_eq!(answer.1.as_deref(), Some(location));
}

async fn rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM feedback")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Mails go out in the background; wait until `count` have been taken,
/// then a little longer so that one too many would show.
async fn mails(mailer: &Mailer, count: usize) -> Vec<Sent> {
    for _ in 0..200 {
        if mailer.outbox().len() >= count {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let sent = mailer.outbox();
    assert_eq!(sent.len(), count, "{sent:?}");
    sent
}

/// The token of a confirmation mail's link.
fn token_of(mail: &Sent) -> String {
    let (_, rest) = mail
        .body
        .split_once("/api/public/feedback/confirm?token=")
        .expect("confirmation link");
    rest[..64].to_owned()
}

/// Lena, the operator, with a confirmed address, made while mail was off.
async fn operator(pool: &PgPool) {
    let (plain, _) = state_with(pool.clone(), true, false);
    let mut lena = Client::from_state(plain);
    lena.bootstrap().await;
    let res = lena.register("lena@example.test", PASSWORD, "Lena").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    sqlx::query("UPDATE users SET email_verified_at = now(), locale = 'en'")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn off_without_mail_or_without_operators(pool: PgPool) {
    for (operators, mail) in [(false, true), (true, false)] {
        let (state, mailer) = state_with(pool.clone(), operators, mail);
        let (status, location, body) = post(&state, valid("de", "ada@example.org")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(location, None);
        assert_eq!(body["error"], "not_found", "{body}");
        let (status, _, body) = open(&state, &format!("?token={}", "a".repeat(64)), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not_found", "{body}");
        assert_eq!(rows(&pool).await, 0);
        assert!(mailer.outbox().is_empty());
    }
}

#[sqlx::test]
async fn answers_go_to_the_pages_of_the_form_language(pool: PgPool) {
    let (state, _) = form_state(pool.clone());
    see_other(
        post(&state, valid("de", "ada@example.org")).await,
        "/de/feedback/gesendet",
    )
    .await;
    see_other(
        post(&state, valid("en", "ben@example.org")).await,
        "/en/feedback/sent",
    )
    .await;
    // Anything but de and en is English.
    see_other(
        post(&state, valid("fr", "cem@example.org")).await,
        "/en/feedback/sent",
    )
    .await;
    // Invalid input: the error page, nothing stored.
    let before = rows(&pool).await;
    assert_eq!(before, 3);
    let bad = [
        form(&[
            ("lang", "de"),
            ("kind", "rant"),
            ("text", "x"),
            ("email", "d@example.org"),
        ]),
        form(&[
            ("lang", "de"),
            ("kind", "idea"),
            ("text", "  "),
            ("email", "d@example.org"),
        ]),
        form(&[
            ("lang", "de"),
            ("kind", "idea"),
            ("text", &"x".repeat(4001)),
            ("email", "d@example.org"),
        ]),
        form(&[
            ("lang", "de"),
            ("kind", "idea"),
            ("text", "x"),
            ("email", "keine Adresse"),
        ]),
        form(&[("lang", "de"), ("kind", "idea"), ("text", "x")]),
    ];
    for body in bad {
        let (status, location, _) = post_from(&state, body, "203.0.113.9", None).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location.as_deref(), Some("/de/feedback/fehler"));
    }
    assert_eq!(rows(&pool).await, before);

    let stored: (String, String, String, String, String, String, bool) = sqlx::query_as(
        "SELECT kind, client, status, visitor_email, visitor_locale, locale, \
                confirm_expires_at BETWEEN now() + interval '23 hours 59 minutes' \
                                       AND now() + interval '24 hours' \
         FROM feedback WHERE visitor_email = 'ada@example.org'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        stored,
        (
            "problem".to_owned(),
            "form".to_owned(),
            "pending".to_owned(),
            "ada@example.org".to_owned(),
            "de".to_owned(),
            "de".to_owned(),
            true
        )
    );
}

#[sqlx::test]
async fn a_full_text_with_line_breaks_or_of_any_script_is_taken(pool: PgPool) {
    let (state, _) = form_state(pool.clone());
    // `maxlength` counts a line break once; the form sends it as CRLF.
    let line = "x".repeat(99);
    let lines = vec![line.as_str(); 40].join("\r\n") + "y";
    assert_eq!(lines.replace("\r\n", "\n").chars().count(), 4000);
    // A three-byte character is nine bytes on the wire.
    let euros = "€".repeat(4000);
    for (text, email) in [(&lines, "ada@example.org"), (&euros, "ben@example.org")] {
        let body = form(&[
            ("lang", "de"),
            ("kind", "idea"),
            ("text", text),
            ("email", email),
            ("website", ""),
        ]);
        see_other(post(&state, body).await, "/de/feedback/gesendet").await;
    }
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT body FROM feedback ORDER BY visitor_email")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(stored, [lines.replace("\r\n", "\n"), euros]);
}

#[sqlx::test]
async fn the_honeypot_stores_and_sends_nothing(pool: PgPool) {
    let (state, mailer) = form_state(pool.clone());
    let body = form(&[
        ("lang", "de"),
        ("kind", "idea"),
        ("text", "Kauf jetzt"),
        ("email", "ada@example.org"),
        ("website", "https://spam.example"),
    ]);
    see_other(post(&state, body).await, "/de/feedback/gesendet").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rows(&pool).await, 0);
    assert!(mailer.outbox().is_empty());
}

#[sqlx::test]
async fn cross_site_and_same_site_posts_are_refused(pool: PgPool) {
    let (state, mailer) = form_state(pool.clone());
    for site in ["cross-site", "same-site"] {
        let (status, location, body) = post_from(
            &state,
            valid("de", "ada@example.org"),
            "198.51.100.7",
            Some(site),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(location, None);
        assert_eq!(body["error"], "csrf_invalid", "{body}");
    }
    assert_eq!(rows(&pool).await, 0);
    // `none` (typed or bookmarked) and no header at all pass.
    see_other(
        post_from(
            &state,
            valid("de", "ada@example.org"),
            "198.51.100.7",
            Some("none"),
        )
        .await,
        "/de/feedback/gesendet",
    )
    .await;
    see_other(
        post_from(&state, valid("de", "ben@example.org"), "198.51.100.7", None).await,
        "/de/feedback/gesendet",
    )
    .await;
    assert_eq!(rows(&pool).await, 2);
    mails(&mailer, 2).await;
}

#[sqlx::test]
async fn the_sixth_post_from_one_address_is_refused(pool: PgPool) {
    let (state, _) = form_state(pool.clone());
    for n in 0..5 {
        see_other(
            post(&state, valid("en", &format!("p{n}@example.org"))).await,
            "/en/feedback/sent",
        )
        .await;
    }
    see_other(
        post(&state, valid("en", "p5@example.org")).await,
        "/en/feedback/error",
    )
    .await;
    // Past it, every post is refused, whatever it holds.
    see_other(
        post(&state, form(&[("lang", "de")])).await,
        "/de/feedback/fehler",
    )
    .await;
    assert_eq!(rows(&pool).await, 5);
    // Another address has its own hour.
    see_other(
        post_from(&state, valid("en", "p6@example.org"), "203.0.113.9", None).await,
        "/en/feedback/sent",
    )
    .await;
    assert_eq!(rows(&pool).await, 6);
}

#[sqlx::test]
async fn an_address_out_of_mails_gets_the_sent_page_and_nothing_is_stored(pool: PgPool) {
    let (state, mailer) = form_state(pool.clone());
    for n in 0..5 {
        let ip = format!("198.51.100.{n}");
        see_other(
            post_from(&state, valid("de", "ada@example.org"), &ip, None).await,
            "/de/feedback/gesendet",
        )
        .await;
    }
    mails(&mailer, 5).await;
    see_other(
        post_from(&state, valid("de", "ada@example.org"), "198.51.100.9", None).await,
        "/de/feedback/gesendet",
    )
    .await;
    assert_eq!(rows(&pool).await, 5);
    mails(&mailer, 5).await;
}

#[sqlx::test]
async fn the_confirmation_mail_has_the_link_and_not_the_text(pool: PgPool) {
    let (state, mailer) = form_state(pool.clone());
    see_other(
        post(&state, valid("de", "ada@example.org")).await,
        "/de/feedback/gesendet",
    )
    .await;
    let sent = mails(&mailer, 1).await;
    assert_eq!(sent[0].to, "ada@example.org");
    assert_eq!(sent[0].subject, "Bestätige dein Feedback an Gelabber");
    let token = token_of(&sent[0]);
    assert!(
        sent[0].body.contains(&format!(
            "https://chat.example.org/api/public/feedback/confirm?token={token}\n"
        )),
        "{}",
        sent[0].body
    );
    assert!(!sent[0].body.contains("Knopf"), "{}", sent[0].body);
    assert!(!sent[0].body.contains("Geheimes"), "{}", sent[0].body);
    // Only the hash is stored.
    let hash: Vec<u8> = sqlx::query_scalar("SELECT confirm_hash FROM feedback")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(hash, gelabber_api::token::hash(&token));

    // English for an English form.
    see_other(
        post(&state, valid("en", "ben@example.org")).await,
        "/en/feedback/sent",
    )
    .await;
    let sent = mails(&mailer, 2).await;
    assert_eq!(sent[1].subject, "Confirm your feedback to Gelabber");
}

#[sqlx::test]
async fn confirming_opens_the_entry_once_and_tells_the_operators_once(pool: PgPool) {
    operator(&pool).await;
    let (state, mailer) = form_state(pool.clone());
    see_other(
        post(&state, valid("de", "ada@example.org")).await,
        "/de/feedback/gesendet",
    )
    .await;
    let sent = mails(&mailer, 1).await;
    let token = token_of(&sent[0]);
    let id: Uuid = sqlx::query_scalar("SELECT id FROM feedback")
        .fetch_one(&pool)
        .await
        .unwrap();
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_activity_at FROM feedback")
            .fetch_one(&pool)
            .await
            .unwrap();

    // The row's language wins over the browser's.
    see_other(
        open(&state, &format!("?token={token}"), Some("en")).await,
        "/de/feedback/danke",
    )
    .await;
    let (status, hash_kept, moved): (String, bool, bool) = sqlx::query_as(
        "SELECT status, confirm_hash IS NOT NULL, last_activity_at > $1 FROM feedback",
    )
    .bind(before)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((status.as_str(), hash_kept, moved), ("open", true, true));
    let sent = mails(&mailer, 2).await;
    assert_eq!(sent[1].to, "lena@example.test");
    assert_eq!(sent[1].subject, "New feedback: Problem");
    assert!(
        sent[1].body.contains("From: ada@example.org\n"),
        "{}",
        sent[1].body
    );
    assert!(
        sent[1].body.contains(&format!(
            "https://chat.example.org/betrieb/feedback?entry={id}\n"
        )),
        "{}",
        sent[1].body
    );

    // A second open (a scanner, a second click) lands on thanks again and
    // changes nothing.
    let activity: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_activity_at FROM feedback")
            .fetch_one(&pool)
            .await
            .unwrap();
    see_other(
        open(&state, &format!("?token={token}"), None).await,
        "/de/feedback/danke",
    )
    .await;
    let again: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_activity_at FROM feedback")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(again, activity);
    mails(&mailer, 2).await;

    // The operators see it now.
    let mut lena = Client::from_state(state.clone());
    lena.bootstrap().await;
    let res = lena.login("lena@example.test", PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = lena.send(Method::GET, "/api/operator/feedback", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["total"], 1, "{}", res.body);
    assert_eq!(res.body["entries"][0]["sender"]["type"], "visitor");
}

#[sqlx::test]
async fn wrong_malformed_or_expired_tokens_land_on_expired(pool: PgPool) {
    let (state, mailer) = form_state(pool.clone());
    // No token, a malformed one, one that matches nothing: the browser's
    // language, else English.
    see_other(
        open(&state, "", Some("de-DE,de;q=0.9")).await,
        "/de/feedback/abgelaufen",
    )
    .await;
    see_other(
        open(&state, "?token=xyz", None).await,
        "/en/feedback/expired",
    )
    .await;
    see_other(
        open(&state, &format!("?token={}", "a".repeat(64)), Some("de")).await,
        "/de/feedback/abgelaufen",
    )
    .await;

    // One that ran out: the row's language.
    see_other(
        post(&state, valid("en", "ada@example.org")).await,
        "/en/feedback/sent",
    )
    .await;
    let token = token_of(&mails(&mailer, 1).await[0]);
    sqlx::query("UPDATE feedback SET confirm_expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    see_other(
        open(&state, &format!("?token={token}"), Some("de")).await,
        "/en/feedback/expired",
    )
    .await;
    let status: String = sqlx::query_scalar("SELECT status FROM feedback")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
}

#[sqlx::test]
async fn the_sweeper_deletes_run_out_pending_entries_and_clears_run_out_hashes(pool: PgPool) {
    let (state, mailer) = form_state(pool.clone());
    for email in ["old@example.org", "done@example.org", "new@example.org"] {
        see_other(
            post(&state, valid("de", email)).await,
            "/de/feedback/gesendet",
        )
        .await;
    }
    mails(&mailer, 3).await;
    sqlx::query("UPDATE feedback SET status = 'open' WHERE visitor_email = 'done@example.org'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE feedback SET confirm_expires_at = now() - interval '1 minute' \
         WHERE visitor_email IN ('old@example.org', 'done@example.org')",
    )
    .execute(&pool)
    .await
    .unwrap();

    public::sweep(&state).await.unwrap();
    let left: Vec<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT visitor_email, status, confirm_hash IS NULL, confirm_expires_at IS NULL \
         FROM feedback ORDER BY visitor_email",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        left,
        vec![
            ("done@example.org".to_owned(), "open".to_owned(), true, true),
            (
                "new@example.org".to_owned(),
                "pending".to_owned(),
                false,
                false
            ),
        ]
    );
    // Nothing more to do.
    assert_eq!(public::sweep(&state).await.unwrap(), (0, 0));
}

#[sqlx::test]
async fn too_many_pending_entries_refuse_the_form(pool: PgPool) {
    let (state, mailer) = form_state(pool.clone());
    sqlx::query(
        "INSERT INTO feedback (kind, visitor_email, visitor_locale, client, locale, body, \
                               status, confirm_hash, confirm_expires_at) \
         SELECT 'idea', 'v' || n || '@example.org', 'de', 'form', 'de', 'x', 'pending', \
                sha256(n::text::bytea), now() + interval '1 hour' \
         FROM generate_series(1, $1) AS n",
    )
    .bind(gelabber_api::limits::FEEDBACK_PENDING_MAX)
    .execute(&pool)
    .await
    .unwrap();
    see_other(
        post(&state, valid("de", "ada@example.org")).await,
        "/de/feedback/fehler",
    )
    .await;
    assert_eq!(
        rows(&pool).await,
        gelabber_api::limits::FEEDBACK_PENDING_MAX
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(mailer.outbox().is_empty());

    // The refused post gave its mail back: five more fit once there is room.
    sqlx::query("DELETE FROM feedback")
        .execute(&pool)
        .await
        .unwrap();
    for n in 0..5 {
        let ip = format!("203.0.113.{n}");
        see_other(
            post_from(&state, valid("de", "ada@example.org"), &ip, None).await,
            "/de/feedback/gesendet",
        )
        .await;
    }
    assert_eq!(rows(&pool).await, 5);
    mails(&mailer, 5).await;
}
