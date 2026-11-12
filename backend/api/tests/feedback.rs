//! Feedback (#542), the routes of signed-in people: the status, staging an
//! image, creating entries, the list, replies, read marks and the image.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use gelabber_api::image_size::fixtures;
use gelabber_api::mail::{Mailer, Sent};
use gelabber_api::{AppState, Config};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "password123";

fn state_with(pool: PgPool, operators: bool, extra: &[(&str, &str)]) -> AppState {
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
            "API_LINK_PREVIEWS" => Some("false".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
            "REDIS_URL" => Some(common::redis_url()),
            "API_SESSION_TTL_HOURS" => Some("2".to_owned()),
            _ => None,
        }
    })
    .expect("test config");
    let mut state = AppState::with_pool(&config, pool).expect("state");
    if operators {
        state.operators = Arc::new(HashSet::from(["lena@example.test".to_owned()]));
    }
    state
}

fn feedback_state(pool: PgPool) -> AppState {
    state_with(pool, true, &[])
}

async fn signed_in(state: &AppState, email: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

/// `client`'s session, against another state of the same database.
fn moved(client: &Client, state: &AppState) -> Client {
    let mut moved = Client::from_state(state.clone());
    moved.jar = client.jar.clone();
    moved.csrf = client.csrf.clone();
    moved.user_id = client.user_id.clone();
    moved
}

fn context() -> Value {
    json!({
        "client": "web",
        "os": "linux",
        "app_version": "0.10.0",
        "locale": "de",
        "page": "/settings/darstellung",
    })
}

fn entry(text: &str) -> Value {
    json!({ "kind": "idea", "text": text, "context": context() })
}

async fn upload(client: &mut Client, bytes: Vec<u8>) -> (StatusCode, Value) {
    client
        .send_bytes(
            Method::POST,
            "/api/feedback/images",
            "application/octet-stream",
            bytes,
        )
        .await
}

async fn staged(client: &mut Client, bytes: Vec<u8>) -> String {
    let (status, body) = upload(client, bytes).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["image"].as_str().expect("image id").to_owned()
}

async fn create(client: &mut Client, body: Value) -> Value {
    let res = client.send(Method::POST, "/api/feedback", Some(body)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

/// An operator reply as the operator routes will write it.
async fn operator_reply(pool: &PgPool, id: &str, text: &str) {
    let id = Uuid::parse_str(id).unwrap();
    // One statement, so the message and the unread mark share `now()`.
    sqlx::query(
        "WITH message AS (INSERT INTO feedback_messages (feedback_id, author_kind, body) \
         VALUES ($1, 'operator', $2)) \
         UPDATE feedback SET last_activity_at = now(), \
         person_unread_since = COALESCE(person_unread_since, now()) WHERE id = $1",
    )
    .bind(id)
    .bind(text)
    .execute(pool)
    .await
    .unwrap();
}

/// A visitor's entry from the public form, not confirmed yet.
async fn pending_entry(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO feedback (kind, visitor_email, visitor_locale, client, locale, body, \
         status, confirm_hash, confirm_expires_at) \
         VALUES ('idea', 'visitor@example.test', 'de', 'form', 'de', 'hallo', 'pending', \
         sha256(gen_random_uuid()::text::bytea), now() + interval '1 day') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn off_without_operators(pool: PgPool) {
    let state = state_with(pool, false, &[]);
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    let res = anonymous
        .send(Method::GET, "/api/feedback/status", None)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);

    let mut ada = signed_in(&state, "ada@example.test").await;
    let res = ada.send(Method::GET, "/api/feedback/status", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!({ "enabled": false, "unread": 0 }));

    let id = Uuid::new_v4();
    for (method, path, body) in [
        (Method::GET, "/api/feedback".to_owned(), None),
        (Method::POST, "/api/feedback".to_owned(), Some(entry("x"))),
        (
            Method::POST,
            format!("/api/feedback/{id}/messages"),
            Some(json!({ "text": "x" })),
        ),
        (Method::POST, format!("/api/feedback/{id}/read"), None),
        (Method::GET, format!("/api/feedback/{id}/image"), None),
    ] {
        let res = ada.send(method.clone(), &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(res.body["error"], "not_found");
    }
    let (status, body) = upload(&mut ada, fixtures::png(10, 10)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[sqlx::test]
async fn on_with_operators_and_signed_in_only(pool: PgPool) {
    let state = feedback_state(pool);
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    for path in ["/api/feedback/status", "/api/feedback"] {
        let res = anonymous.send(Method::GET, path, None).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{path}");
    }
    let res = anonymous
        .send(Method::POST, "/api/feedback", Some(entry("x")))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let mut ada = signed_in(&state, "ada@example.test").await;
    let res = ada.send(Method::GET, "/api/feedback/status", None).await;
    assert_eq!(res.body, json!({ "enabled": true, "unread": 0 }));
}

#[sqlx::test]
async fn images_are_staged_by_their_bytes(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;
    for bytes in [
        fixtures::png(800, 600),
        fixtures::jpeg(800, 600),
        fixtures::webp(800, 600),
    ] {
        let id = staged(&mut ada, bytes).await;
        let key = format!("feedback/{}/{id}", ada.user_id());
        assert!(state.store.head(&key).await.is_ok(), "{key}");
        let pending: bool = sqlx::query_scalar(
            "SELECT next_attempt > now() + interval '9 minutes' FROM storage_cleanup \
             WHERE object_key = $1 AND attempts = 0",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(pending, "a pending cleanup row guards the object");
    }

    for bytes in [
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
        b"just text".to_vec(),
        Vec::new(),
        fixtures::png(8193, 10),
        fixtures::png(10, 8193),
    ] {
        let (status, body) = upload(&mut ada, bytes).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["fields"]["image"], "invalid");
    }
    let (status, _) = upload(&mut ada, fixtures::png(8192, 8192)).await;
    assert_eq!(status, StatusCode::CREATED);

    let mut big = fixtures::png(100, 100);
    big.resize(5 * 1024 * 1024, 0);
    let (status, body) = upload(&mut ada, big.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    big.push(0);
    let (status, _) = upload(&mut ada, big).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[sqlx::test]
async fn uploads_count_in_the_up_bucket_and_the_quota(pool: PgPool) {
    let png = fixtures::png(64, 64);
    let quota = (2 * png.len() + 10).to_string();
    let state = state_with(
        pool.clone(),
        true,
        &[
            ("API_RATE_UPLOAD_PER_HOUR", "3"),
            ("API_UPLOAD_QUOTA_BYTES_PER_DAY", quota.as_str()),
        ],
    );
    let mut ada = signed_in(&state, "ada@example.test").await;
    staged(&mut ada, png.clone()).await;
    staged(&mut ada, png.clone()).await;
    let consumed: i64 = sqlx::query_scalar("SELECT consumed FROM upload_daily_usage")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(consumed, 2 * png.len() as i64);
    let (status, body) = upload(&mut ada, png.clone()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "quota_exceeded");
    let (status, body) = upload(&mut ada, png).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "rate_limited");
}

#[sqlx::test]
async fn entries_are_created_with_and_without_an_image(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;

    let plain = create(&mut ada, entry("  Eine Idee\nmit zwei Zeilen  ")).await;
    assert_eq!(plain["kind"], "idea");
    assert_eq!(plain["status"], "open");
    assert_eq!(plain["text"], "Eine Idee\nmit zwei Zeilen");
    assert_eq!(plain["image"], Value::Null);
    assert_eq!(plain["context"], context());
    assert_eq!(plain["unread"], false);
    assert_eq!(plain["messages"], json!([]));

    let png = fixtures::png(320, 200);
    let image = staged(&mut ada, png.clone()).await;
    let mut body = entry("Mit Bild");
    body["kind"] = json!("problem");
    body["image"] = json!(image);
    body["context"]["client"] = json!("desktop");
    body["context"]["os"] = json!("windows");
    let with_image = create(&mut ada, body).await;
    let id = with_image["id"].as_str().unwrap();
    assert_eq!(with_image["kind"], "problem");
    assert_eq!(
        with_image["image"],
        json!({
            "url": format!("/api/feedback/{id}/image"),
            "content_type": "image/png",
            "size": png.len(),
        })
    );
    assert_eq!(with_image["context"]["client"], "desktop");
    let key = format!("feedback/{}/{image}", ada.user_id());
    let pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM storage_cleanup WHERE object_key = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pending, 0, "the entry claimed the staged image");
    let stored: String = sqlx::query_scalar("SELECT image_key FROM feedback WHERE id = $1")
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, key);

    // Used once, the image is gone for a second entry.
    let mut again = entry("Noch einmal");
    again["image"] = json!(image);
    let res = ada.send(Method::POST, "/api/feedback", Some(again)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["image"], "invalid");
}

#[sqlx::test]
async fn only_the_callers_live_staged_image_is_taken(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;
    let mut tom = signed_in(&state, "tom@example.test").await;

    let adas = staged(&mut ada, fixtures::png(10, 10)).await;
    let mut body = entry("Fremdes Bild");
    body["image"] = json!(adas);
    let res = tom
        .send(Method::POST, "/api/feedback", Some(body.clone()))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["image"], "invalid");

    for image in [json!(Uuid::new_v4()), json!("not-a-uuid")] {
        let mut body = entry("Kein Bild");
        body["image"] = image;
        let res = ada.send(Method::POST, "/api/feedback", Some(body)).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["fields"]["image"], "invalid");
    }

    // The staging row ran out: the cleanup worker may delete it any time.
    sqlx::query("UPDATE storage_cleanup SET next_attempt = now(), retain_until = now()")
        .execute(&pool)
        .await
        .unwrap();
    let res = ada.send(Method::POST, "/api/feedback", Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["image"], "invalid");

    // None of the refusals cost an entry of the hour.
    for i in 0..5 {
        create(&mut ada, entry(&format!("Eintrag {i}"))).await;
    }
}

#[sqlx::test]
async fn bad_fields_are_named(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;
    let cases: Vec<(Value, &str, &str)> = vec![
        (json!({ "kind": "bug" }), "kind", "invalid"),
        (json!({ "text": "" }), "text", "required"),
        (json!({ "text": "   " }), "text", "required"),
        (json!({ "text": "x".repeat(4001) }), "text", "too_long"),
        (json!({ "text": "a\u{0}b" }), "text", "invalid"),
        (
            json!({ "context": { "client": "form" } }),
            "client",
            "invalid",
        ),
        (json!({ "context": { "os": "beos" } }), "os", "invalid"),
        (
            json!({ "context": { "app_version": "0.10 beta" } }),
            "app_version",
            "invalid",
        ),
        (
            json!({ "context": { "locale": "fr" } }),
            "locale",
            "invalid",
        ),
        (
            json!({ "context": { "page": "/s/x?token=1" } }),
            "page",
            "invalid",
        ),
        (
            json!({ "context": { "page": "/s/x#frag" } }),
            "page",
            "invalid",
        ),
        (
            json!({ "context": { "page": "//evil.example" } }),
            "page",
            "invalid",
        ),
    ];
    for (patch, field, code) in cases {
        let mut body = entry("Gut");
        for (key, value) in patch.as_object().unwrap() {
            if key == "context" {
                for (name, value) in value.as_object().unwrap() {
                    body["context"][name] = value.clone();
                }
            } else {
                body[key] = value.clone();
            }
        }
        let res = ada.send(Method::POST, "/api/feedback", Some(body)).await;
        assert_eq!(
            res.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{field}: {}",
            res.body
        );
        assert_eq!(res.body["fields"][field], code, "{}", res.body);
    }
    let res = ada
        .send(
            Method::POST,
            "/api/feedback",
            Some(json!({ "kind": "idea", "text": "x" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    for field in ["client", "os", "app_version", "locale", "page"] {
        assert_eq!(res.body["fields"][field], "invalid", "{field}");
    }
    // A text of exactly 4,000 characters fits.
    create(&mut ada, entry(&"ä".repeat(4000))).await;
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feedback")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1, "refused entries store nothing");
}

#[sqlx::test]
async fn the_sixth_entry_of_the_hour_is_refused(pool: PgPool) {
    let state = feedback_state(pool);
    let mut ada = signed_in(&state, "ada@example.test").await;
    for i in 0..5 {
        create(&mut ada, entry(&format!("Eintrag {i}"))).await;
    }
    let res = ada
        .send(Method::POST, "/api/feedback", Some(entry("Zu viel")))
        .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
    assert_eq!(res.body["error"], "rate_limited");
    assert!(res.headers.contains_key("retry-after"));
    // A doomed entry stages no image either.
    let (status, body) = upload(&mut ada, fixtures::png(10, 10)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "rate_limited");
    // Someone else still can.
    let mut tom = signed_in(&state, "tom@example.test").await;
    create(&mut tom, entry("Tom")).await;
}

#[sqlx::test]
async fn the_list_holds_own_entries_newest_activity_first(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;
    let mut tom = signed_in(&state, "tom@example.test").await;
    let ada_id = Uuid::parse_str(ada.user_id()).unwrap();
    // 23 entries, an hour apart, the newest last.
    for i in 0..23 {
        sqlx::query(
            "INSERT INTO feedback (kind, user_id, client, os, app_version, locale, page, body, \
             created_at, last_activity_at) \
             VALUES ('idea', $1, 'web', 'linux', '0.10.0', 'de', '/', $2, \
             now() - $3 * interval '1 hour', now() - $3 * interval '1 hour')",
        )
        .bind(ada_id)
        .bind(format!("Eintrag {i}"))
        .bind(f64::from(23 - i))
        .execute(&pool)
        .await
        .unwrap();
    }
    create(&mut tom, entry("Toms")).await;
    pending_entry(&pool).await;

    let res = ada.send(Method::GET, "/api/feedback", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["total"], 23);
    let texts: Vec<&str> = res.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts.len(), 20);
    assert_eq!(texts[0], "Eintrag 22");
    assert_eq!(texts[19], "Eintrag 3");

    // A reply moves an old entry to the top.
    let oldest: Uuid = sqlx::query_scalar("SELECT id FROM feedback WHERE body = 'Eintrag 0'")
        .fetch_one(&pool)
        .await
        .unwrap();
    operator_reply(&pool, &oldest.to_string(), "Danke!").await;
    let res = ada.send(Method::GET, "/api/feedback?offset=20", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let texts: Vec<&str> = res.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["Eintrag 3", "Eintrag 2", "Eintrag 1"]);
    let res = ada.send(Method::GET, "/api/feedback", None).await;
    let first = &res.body["entries"][0];
    assert_eq!(first["text"], "Eintrag 0");
    assert_eq!(first["unread"], true);
    assert_eq!(first["messages"][0]["from"], "operators");
    assert_eq!(first["messages"][0]["new"], true);

    let res = tom.send(Method::GET, "/api/feedback", None).await;
    assert_eq!(res.body["total"], 1);
    assert_eq!(res.body["entries"][0]["text"], "Toms");

    for offset in ["-1", "x"] {
        let res = ada
            .send(Method::GET, &format!("/api/feedback?offset={offset}"), None)
            .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(res.body["fields"]["offset"], "invalid");
    }
}

#[sqlx::test]
async fn replies_build_the_thread_and_reopen_a_done_entry(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;
    let mut tom = signed_in(&state, "tom@example.test").await;
    let created = create(&mut ada, entry("Erste Worte")).await;
    let id = created["id"].as_str().unwrap().to_owned();
    let uuid = Uuid::parse_str(&id).unwrap();

    operator_reply(&pool, &id, "Was genau?").await;
    sqlx::query("UPDATE feedback SET status = 'done', done_at = now() WHERE id = $1")
        .bind(uuid)
        .execute(&pool)
        .await
        .unwrap();
    let path = format!("/api/feedback/{id}/messages");
    let res = ada
        .send(Method::POST, &path, Some(json!({ "text": " Genau das. " })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["from"], "you");
    assert_eq!(res.body["text"], "Genau das.");
    assert_eq!(res.body["new"], false);

    let res = ada.send(Method::GET, "/api/feedback", None).await;
    let entry = &res.body["entries"][0];
    assert_eq!(entry["status"], "open", "a reply opens it again");
    let from: Vec<&str> = entry["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["from"].as_str().unwrap())
        .collect();
    assert_eq!(from, ["operators", "you"]);
    let done_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT done_at FROM feedback WHERE id = $1")
            .bind(uuid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(done_at, None);

    // Nobody else answers in it, and bad text is refused.
    let res = tom
        .send(Method::POST, &path, Some(json!({ "text": "Ich auch" })))
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = ada
        .send(Method::POST, &path, Some(json!({ "text": "" })))
        .await;
    assert_eq!(res.body["fields"]["text"], "required");
    let res = ada
        .send(
            Method::POST,
            &format!("/api/feedback/{}/messages", Uuid::new_v4()),
            Some(json!({ "text": "x" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // A pending visitor entry belongs to nobody.
    let pending = pending_entry(&pool).await;
    let res = ada
        .send(
            Method::POST,
            &format!("/api/feedback/{pending}/messages"),
            Some(json!({ "text": "x" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn a_thread_holds_at_most_200_messages(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;
    let id = create(&mut ada, entry("Lang")).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    sqlx::query(
        "INSERT INTO feedback_messages (feedback_id, author_kind, body) \
         SELECT $1, 'operator', 'x' FROM generate_series(1, 199)",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    let path = format!("/api/feedback/{id}/messages");
    let res = ada
        .send(Method::POST, &path, Some(json!({ "text": "200." })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = ada
        .send(Method::POST, &path, Some(json!({ "text": "201." })))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["text"], "too_many");
}

#[sqlx::test]
async fn the_31st_reply_of_the_hour_is_refused(pool: PgPool) {
    let state = feedback_state(pool);
    let mut ada = signed_in(&state, "ada@example.test").await;
    let id = create(&mut ada, entry("Viel zu sagen")).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/api/feedback/{id}/messages");
    for i in 0..30 {
        let res = ada
            .send(Method::POST, &path, Some(json!({ "text": format!("{i}") })))
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{i}: {}", res.body);
    }
    let res = ada
        .send(Method::POST, &path, Some(json!({ "text": "31" })))
        .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
    assert_eq!(res.body["error"], "rate_limited");
}

#[sqlx::test]
async fn reading_clears_unread(pool: PgPool) {
    let state = feedback_state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.test").await;
    let mut tom = signed_in(&state, "tom@example.test").await;
    let id = create(&mut ada, entry("Hallo")).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    operator_reply(&pool, &id, "Eins").await;
    operator_reply(&pool, &id, "Zwei").await;
    let res = ada.send(Method::GET, "/api/feedback/status", None).await;
    assert_eq!(res.body, json!({ "enabled": true, "unread": 1 }));

    let path = format!("/api/feedback/{id}/read");
    let res = tom.send(Method::POST, &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = ada.send(Method::POST, &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = ada.send(Method::GET, "/api/feedback/status", None).await;
    assert_eq!(res.body["unread"], 0);
    let res = ada.send(Method::GET, "/api/feedback", None).await;
    let entry = &res.body["entries"][0];
    assert_eq!(entry["unread"], false);
    assert!(
        entry["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|message| message["new"] == false)
    );
    let res = ada
        .send(
            Method::POST,
            &format!("/api/feedback/{}/read", Uuid::new_v4()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn the_image_is_served_to_its_owner_only(pool: PgPool) {
    let state = feedback_state(pool);
    let mut ada = signed_in(&state, "ada@example.test").await;
    let mut tom = signed_in(&state, "tom@example.test").await;
    let jpeg = fixtures::jpeg(640, 480);
    let image = staged(&mut ada, jpeg.clone()).await;
    let mut body = entry("Siehe Bild");
    body["image"] = json!(image);
    let created = create(&mut ada, body).await;
    let url = created["image"]["url"].as_str().unwrap().to_owned();

    let (status, headers, bytes) = ada.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, jpeg);
    assert_eq!(headers["content-type"], "image/jpeg");
    assert_eq!(headers["content-disposition"], "inline");
    assert_eq!(
        headers["cache-control"],
        "private, max-age=31536000, immutable"
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");

    let (status, _, _) = tom.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let plain = create(&mut ada, entry("Ohne Bild")).await;
    let (status, _, _) = ada
        .send_raw(
            Method::GET,
            &format!("/api/feedback/{}/image", plain["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
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

#[sqlx::test]
async fn operator_notices_go_to_confirmed_enabled_operators_in_their_language(pool: PgPool) {
    // Accounts made while mail was off, then confirmed or not by hand.
    let plain = feedback_state(pool.clone());
    let mut tom = Client::from_state(plain.clone());
    tom.bootstrap().await;
    let res = tom.register("tom@example.test", PASSWORD, "Tom").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    for email in [
        "lena@example.test",
        "max@example.test",
        "ute@example.test",
        "kai@example.test",
    ] {
        signed_in(&plain, email).await;
    }
    sqlx::query(
        "UPDATE users SET email_verified_at = now() \
         WHERE email IN ('tom@example.test', 'lena@example.test', 'max@example.test', \
                         'kai@example.test')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE users SET locale = 'en' WHERE email = 'lena@example.test'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET disabled_at = now() WHERE email = 'max@example.test'")
        .execute(&pool)
        .await
        .unwrap();

    let mut state = feedback_state(pool.clone());
    // Kai is confirmed but no operator; Ute is an operator, unconfirmed.
    state.operators = Arc::new(HashSet::from([
        "lena@example.test".to_owned(),
        "max@example.test".to_owned(),
        "ute@example.test".to_owned(),
    ]));
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    let mut tom = moved(&tom, &state);

    let created = create(
        &mut tom,
        json!({
            "kind": "problem",
            "text": "Eins\nZwei\nDrei\nVier",
            "context": context(),
        }),
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let sent = mails(&mailer, 1).await;
    assert_eq!(sent[0].to, "lena@example.test");
    assert_eq!(sent[0].subject, "New feedback: Problem");
    assert!(
        sent[0]
            .body
            .contains("From: Tom (tom@example.test)\n\n> Eins\n> Zwei\n> Drei…\n\n"),
        "{}",
        sent[0].body
    );
    assert!(
        sent[0].body.contains(&format!(
            "https://chat.example.org/betrieb/feedback?entry={id}\n"
        )),
        "{}",
        sent[0].body
    );

    let res = tom
        .send(
            Method::POST,
            &format!("/api/feedback/{id}/messages"),
            Some(json!({ "text": "Noch etwas" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let sent = mails(&mailer, 2).await;
    assert_eq!(sent[1].to, "lena@example.test");
    assert_eq!(sent[1].subject, "New reply to feedback: Problem");
    assert!(
        sent[1].body.contains("\n\n> Noch etwas\n\n"),
        "{}",
        sent[1].body
    );
}
