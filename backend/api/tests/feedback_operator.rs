//! Feedback (#542), the operators' side in Betrieb: the routes under
//! `/api/operator/feedback`, their audit rows, and the open count of the
//! area bootstrap.

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use gelabber_api::AppState;
use gelabber_api::image_size::fixtures;
use gelabber_api::mail::{Mailer, Sent};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "correct horse";
const LENA: &str = "lena@example.test";

fn state_with(pool: PgPool, operators: bool) -> AppState {
    let mut state = common::state(pool);
    if operators {
        state.operators = Arc::new(HashSet::from([LENA.to_owned()]));
    }
    state
}

async fn registered(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

fn uid(client: &Client) -> Uuid {
    client.user_id().parse().unwrap()
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

/// An entry of `user` written straight to the table (no rate limit), with
/// `last_activity_at` `minutes` ago.
async fn account_entry(
    pool: &PgPool,
    user: Option<Uuid>,
    kind: &str,
    status: &str,
    minutes: i64,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO feedback (kind, user_id, client, os, app_version, locale, page, body, \
             status, done_at, last_activity_at) \
         VALUES ($1, $2, 'web', 'linux', '0.10.0', 'de', '/', 'Text', $3, \
             CASE WHEN $3 = 'done' THEN now() END, now() - $4 * interval '1 minute') \
         RETURNING id",
    )
    .bind(kind)
    .bind(user)
    .bind(status)
    .bind(minutes as f64)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A confirmed visitor entry from the public form.
async fn visitor_entry(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO feedback (kind, visitor_email, visitor_locale, client, locale, body) \
         VALUES ('praise', 'visitor@example.test', 'en', 'form', 'en', 'Nice') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A visitor's entry from the public form, not confirmed yet.
async fn pending_entry(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO feedback (kind, visitor_email, visitor_locale, client, locale, body, \
         status, confirm_hash, confirm_expires_at) \
         VALUES ('idea', 'pending@example.test', 'de', 'form', 'de', 'hallo', 'pending', \
         sha256(gen_random_uuid()::text::bytea), now() + interval '1 day') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Rows of `operator_audit` as (action, target_feedback, target_user, detail).
type AuditRow = (String, Option<Uuid>, Option<Uuid>, Value);

async fn audit_rows(pool: &PgPool) -> Vec<AuditRow> {
    sqlx::query_as(
        "SELECT action, target_feedback, target_user, detail FROM operator_audit ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Every operator route on `id`, with a body where it takes one. The image
/// route comes last.
fn routes(id: Uuid) -> Vec<(Method, String, Option<Value>)> {
    vec![
        (Method::GET, "/api/operator/feedback".to_owned(), None),
        (Method::GET, format!("/api/operator/feedback/{id}"), None),
        (
            Method::POST,
            format!("/api/operator/feedback/{id}/messages"),
            Some(json!({ "text": "Danke" })),
        ),
        (
            Method::POST,
            format!("/api/operator/feedback/{id}/done"),
            None,
        ),
        (
            Method::POST,
            format!("/api/operator/feedback/{id}/reopen"),
            None,
        ),
        (Method::DELETE, format!("/api/operator/feedback/{id}"), None),
        (
            Method::GET,
            format!("/api/operator/feedback/{id}/image"),
            None,
        ),
    ]
}

#[sqlx::test]
async fn the_routes_answer_401_and_404_like_the_area(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let id = account_entry(&pool, None, "idea", "open", 0).await;
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    for (method, path, body) in routes(id) {
        let res = anonymous.send(method.clone(), &path, body.clone()).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{method} {path}");
        let res = tom.send(method.clone(), &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(res.body["error"], "not_found");
    }

    // Off (no operators), Lena's address makes nobody an operator, and
    // every route is an unknown one.
    let off = state_with(pool.clone(), false);
    let mut lena = registered(&off, LENA, "Lena").await;
    for (method, path, body) in routes(id) {
        let res = lena.send(method.clone(), &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    assert!(audit_rows(&pool).await.is_empty());
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM feedback WHERE id = $1)")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(exists);
}

#[sqlx::test]
async fn the_area_bootstrap_counts_open_feedback(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    account_entry(&pool, None, "idea", "open", 0).await;
    account_entry(&pool, None, "problem", "open", 0).await;
    account_entry(&pool, None, "idea", "done", 0).await;
    visitor_entry(&pool).await;
    pending_entry(&pool).await;
    let res = lena.send(Method::GET, "/api/operator", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["open_feedback"], 3);
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test]
async fn the_list_filters_pages_and_skips_pending_entries(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let tom = registered(&state, "tom@example.test", "Tom").await;
    // 52 open ideas, newest first by activity; two open problems further
    // back; one done praise; one pending visitor entry.
    let mut ideas = Vec::new();
    for minute in 0..52 {
        ideas.push(account_entry(&pool, Some(uid(&tom)), "idea", "open", minute + 10).await);
    }
    let problem = account_entry(&pool, Some(uid(&tom)), "problem", "open", 100).await;
    let older_problem = account_entry(&pool, Some(uid(&tom)), "problem", "open", 200).await;
    let praise = account_entry(&pool, Some(uid(&tom)), "praise", "done", 0).await;
    let pending = pending_entry(&pool).await;

    let ids = |body: &Value| -> Vec<Uuid> {
        body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().parse().unwrap())
            .collect()
    };

    let res = lena.send(Method::GET, "/api/operator/feedback", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["total"], 54);
    assert_eq!(ids(&res.body), ideas[..50].to_vec());
    let res = lena
        .send(Method::GET, "/api/operator/feedback?offset=50", None)
        .await;
    assert_eq!(
        ids(&res.body),
        vec![ideas[50], ideas[51], problem, older_problem]
    );
    let res = lena
        .send(
            Method::GET,
            "/api/operator/feedback?status=open&kind=problem",
            None,
        )
        .await;
    assert_eq!(res.body["total"], 2);
    assert_eq!(ids(&res.body), vec![problem, older_problem]);
    let res = lena
        .send(Method::GET, "/api/operator/feedback?status=done", None)
        .await;
    assert_eq!(res.body["total"], 1);
    assert_eq!(ids(&res.body), vec![praise]);
    assert_eq!(res.body["entries"][0]["status"], "done");
    let res = lena
        .send(
            Method::GET,
            "/api/operator/feedback?status=done&kind=idea",
            None,
        )
        .await;
    assert_eq!(res.body, json!({ "entries": [], "total": 0 }));
    // Pending entries are in no list, not even under `status=pending`.
    let res = lena
        .send(Method::GET, "/api/operator/feedback?status=pending", None)
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["status"], "invalid");
    for path in [
        "/api/operator/feedback?kind=bug",
        "/api/operator/feedback?offset=-1",
        "/api/operator/feedback?offset=x",
    ] {
        let res = lena.send(Method::GET, path, None).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
    }
    let res = lena
        .send(Method::GET, "/api/operator/feedback?kind=bug", None)
        .await;
    assert_eq!(res.body["fields"]["kind"], "invalid");
    let res = lena
        .send(Method::GET, "/api/operator/feedback?offset=x", None)
        .await;
    assert_eq!(res.body["fields"]["offset"], "invalid");

    let rows = audit_rows(&pool).await;
    let details: Vec<Value> = rows.iter().map(|row| row.3.clone()).collect();
    assert_eq!(
        details,
        vec![
            json!({ "status": "open", "kind": null, "offset": 0 }),
            json!({ "status": "open", "kind": null, "offset": 50 }),
            json!({ "status": "open", "kind": "problem", "offset": 0 }),
            json!({ "status": "done", "kind": null, "offset": 0 }),
            json!({ "status": "done", "kind": "idea", "offset": 0 }),
        ],
        "one row per successful list, none for refused ones"
    );
    assert!(
        rows.iter()
            .all(|row| row.0 == "feedback.list" && row.1.is_none() && row.2.is_none())
    );
    let still: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feedback WHERE id = $1")
        .bind(pending)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(still, 1);
}

#[sqlx::test]
async fn an_entry_shows_its_sender_context_and_image(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let png = fixtures::png(40, 30);
    let (status, staged) = tom
        .send_bytes(
            Method::POST,
            "/api/feedback/images",
            "application/octet-stream",
            png.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{staged}");
    let res = tom
        .send(
            Method::POST,
            "/api/feedback",
            Some(json!({
                "kind": "problem",
                "text": "Der Ton bricht ab",
                "image": staged["image"],
                "context": context(),
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id: Uuid = res.body["id"].as_str().unwrap().parse().unwrap();

    let res = lena
        .send(Method::GET, &format!("/api/operator/feedback/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let entry = res.body;
    assert_eq!(entry["kind"], "problem");
    assert_eq!(entry["status"], "open");
    assert_eq!(entry["text"], "Der Ton bricht ab");
    assert_eq!(
        entry["sender"],
        json!({
            "type": "account",
            "id": tom.user_id(),
            "name": "Tom",
            "email": "tom@example.test",
            "avatar_url": null,
        })
    );
    assert_eq!(
        entry["image"],
        json!({
            "url": format!("/api/operator/feedback/{id}/image"),
            "content_type": "image/png",
            "size": png.len(),
        })
    );
    assert_eq!(entry["context"], context());
    assert_eq!(entry["done_at"], Value::Null);
    assert_eq!(entry["done_by"], Value::Null);
    assert_eq!(entry["awaiting_reply"], true);
    assert_eq!(entry["messages"], json!([]));
    assert_eq!(
        audit_rows(&pool).await,
        vec![(
            "feedback.read".to_owned(),
            Some(id),
            Some(uid(&tom)),
            json!({})
        )]
    );

    // The image, for operators only, and without an audit row.
    let (status, headers, bytes) = lena
        .send_raw(
            Method::GET,
            &format!("/api/operator/feedback/{id}/image"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[CONTENT_TYPE], "image/png");
    assert_eq!(bytes, png);
    let (status, _, _) = tom
        .send_raw(
            Method::GET,
            &format!("/api/operator/feedback/{id}/image"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(audit_rows(&pool).await.len(), 1);

    // A visitor, and an account that is gone.
    let visitor = visitor_entry(&pool).await;
    let res = lena
        .send(
            Method::GET,
            &format!("/api/operator/feedback/{visitor}"),
            None,
        )
        .await;
    assert_eq!(
        res.body["sender"],
        json!({ "type": "visitor", "email": "visitor@example.test", "locale": "en" })
    );
    assert_eq!(
        res.body["context"],
        json!({ "client": "form", "os": null, "app_version": null, "locale": "en", "page": null })
    );
    assert_eq!(res.body["image"], Value::Null);
    let orphan = account_entry(&pool, None, "idea", "open", 0).await;
    let res = lena
        .send(
            Method::GET,
            &format!("/api/operator/feedback/{orphan}"),
            None,
        )
        .await;
    assert_eq!(res.body["sender"], json!({ "type": "deleted" }));
    let rows = audit_rows(&pool).await;
    assert_eq!(rows[1].1, Some(visitor));
    assert_eq!(rows[1].2, None);
    assert_eq!(rows[2].1, Some(orphan));
    assert_eq!(rows[2].2, None);

    // An unknown entry, or one without an image.
    let unknown = Uuid::new_v4();
    let res = lena
        .send(
            Method::GET,
            &format!("/api/operator/feedback/{unknown}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let (status, _, _) = lena
        .send_raw(
            Method::GET,
            &format!("/api/operator/feedback/{orphan}/image"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(audit_rows(&pool).await.len(), 3);
}

async fn unread_since(pool: &PgPool, id: Uuid) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT person_unread_since FROM feedback WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn operator_reply(op: &mut Client, id: Uuid, text: &str) -> common::Response {
    op.send(
        Method::POST,
        &format!("/api/operator/feedback/{id}/messages"),
        Some(json!({ "text": text })),
    )
    .await
}

#[sqlx::test]
async fn replies_mark_the_entry_unread_once_and_track_who_spoke_last(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let res = tom
        .send(
            Method::POST,
            "/api/feedback",
            Some(json!({ "kind": "idea", "text": "Dunkles Thema", "context": context() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id: Uuid = res.body["id"].as_str().unwrap().parse().unwrap();
    let activity = |pool: PgPool| async move {
        sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT last_activity_at FROM feedback WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let before = activity(pool.clone()).await;

    let res = operator_reply(&mut lena, id, "  Gute Idee!  ").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["from"], "operator");
    assert_eq!(res.body["text"], "Gute Idee!");
    assert_eq!(
        res.body["author"],
        json!({ "id": lena.user_id(), "name": "Lena" })
    );
    assert_eq!(res.body["mail"], Value::Null);
    let first = unread_since(&pool, id).await.expect("marked unread");
    assert!(activity(pool.clone()).await > before);

    let res = operator_reply(&mut lena, id, "Noch etwas").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(
        unread_since(&pool, id).await,
        Some(first),
        "a second reply keeps the mark"
    );
    let res = tom.send(Method::GET, "/api/feedback/status", None).await;
    assert_eq!(res.body["unread"], 1);

    let res = lena
        .send(Method::GET, &format!("/api/operator/feedback/{id}"), None)
        .await;
    assert_eq!(res.body["awaiting_reply"], false);
    assert_eq!(res.body["messages"].as_array().unwrap().len(), 2);

    // The person reads and answers: the mark is gone, and the last word is
    // theirs again.
    let res = tom
        .send(Method::POST, &format!("/api/feedback/{id}/read"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = tom
        .send(
            Method::POST,
            &format!("/api/feedback/{id}/messages"),
            Some(json!({ "text": "Danke" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = lena
        .send(Method::GET, &format!("/api/operator/feedback/{id}"), None)
        .await;
    assert_eq!(res.body["awaiting_reply"], true);
    let thread = res.body["messages"].as_array().unwrap();
    assert_eq!(thread[2]["from"], "person");
    assert_eq!(
        thread[2]["author"],
        json!({ "id": tom.user_id(), "name": "Tom" })
    );
    assert_eq!(unread_since(&pool, id).await, None);
    let res = operator_reply(&mut lena, id, "Gern").await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert!(unread_since(&pool, id).await.is_some(), "a new stretch");

    // A reply on a done entry leaves it done.
    let res = lena
        .send(
            Method::POST,
            &format!("/api/operator/feedback/{id}/done"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let res = operator_reply(&mut lena, id, "Erledigt").await;
    assert_eq!(res.status, StatusCode::CREATED);
    let status: String = sqlx::query_scalar("SELECT status FROM feedback WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "done");

    let replies: Vec<AuditRow> = audit_rows(&pool)
        .await
        .into_iter()
        .filter(|row| row.0 == "feedback.reply")
        .collect();
    assert_eq!(replies.len(), 4);
    assert_eq!(
        replies[0],
        (
            "feedback.reply".to_owned(),
            Some(id),
            Some(uid(&tom)),
            json!({ "chars": 10, "mail": false })
        )
    );
}

#[sqlx::test]
async fn replies_to_visitors_wait_for_their_mail_and_refusals_write_nothing(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let visitor = visitor_entry(&pool).await;
    let res = operator_reply(&mut lena, visitor, "Thanks!").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["mail"], "pending");
    assert_eq!(unread_since(&pool, visitor).await, None);
    let rows = audit_rows(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, Some(visitor));
    assert_eq!(rows[0].2, None);

    // Refused replies: no message and no audit row.
    for text in [
        json!(""),
        json!("   "),
        json!("a\u{0}b"),
        json!("x".repeat(4001)),
    ] {
        let res = lena
            .send(
                Method::POST,
                &format!("/api/operator/feedback/{visitor}/messages"),
                Some(json!({ "text": text })),
            )
            .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    }
    let res = operator_reply(&mut lena, Uuid::new_v4(), "Hallo").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // A full thread takes no more.
    sqlx::query(
        "INSERT INTO feedback_messages (feedback_id, author_kind, body) \
         SELECT $1, 'person', 'x' FROM generate_series(1, 199)",
    )
    .bind(visitor)
    .execute(&pool)
    .await
    .unwrap();
    let res = operator_reply(&mut lena, visitor, "Eine zu viel").await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["text"], "too_many");
    let messages: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM feedback_messages WHERE feedback_id = $1")
            .bind(visitor)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(messages, 200);
    assert_eq!(audit_rows(&pool).await.len(), 1);
}

#[sqlx::test]
async fn done_and_reopen_say_whether_they_changed_anything(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let tom = registered(&state, "tom@example.test", "Tom").await;
    let id = account_entry(&pool, Some(uid(&tom)), "idea", "open", 30).await;
    let activity: DateTime<Utc> =
        sqlx::query_scalar("SELECT last_activity_at FROM feedback WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let post = |path: &'static str| format!("/api/operator/feedback/{id}/{path}");

    let res = lena.send(Method::POST, &post("done"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["status"], "done");
    assert!(res.body["done_at"].is_string());
    assert_eq!(
        res.body["done_by"],
        json!({ "id": lena.user_id(), "name": "Lena" })
    );
    let res = lena.send(Method::POST, &post("done"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["status"], "done");
    let res = lena.send(Method::POST, &post("reopen"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["status"], "open");
    assert_eq!(res.body["done_at"], Value::Null);
    assert_eq!(res.body["done_by"], Value::Null);
    let res = lena.send(Method::POST, &post("reopen"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let after: DateTime<Utc> =
        sqlx::query_scalar("SELECT last_activity_at FROM feedback WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, activity, "closing is no activity");

    let rows = audit_rows(&pool).await;
    let expect = |action: &str, changed: bool| {
        (
            action.to_owned(),
            Some(id),
            Some(uid(&tom)),
            json!({ "changed": changed }),
        )
    };
    assert_eq!(
        rows,
        vec![
            expect("feedback.done", true),
            expect("feedback.done", false),
            expect("feedback.reopen", true),
            expect("feedback.reopen", false),
        ]
    );
    for path in ["done", "reopen"] {
        let res = lena
            .send(
                Method::POST,
                &format!("/api/operator/feedback/{}/{path}", Uuid::new_v4()),
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(audit_rows(&pool).await.len(), 4);
}

#[sqlx::test]
async fn deleting_takes_the_thread_and_queues_the_image(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let (status, staged) = tom
        .send_bytes(
            Method::POST,
            "/api/feedback/images",
            "application/octet-stream",
            fixtures::webp(20, 20),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{staged}");
    let res = tom
        .send(
            Method::POST,
            "/api/feedback",
            Some(json!({
                "kind": "problem",
                "text": "Spam",
                "image": staged["image"],
                "context": context(),
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id: Uuid = res.body["id"].as_str().unwrap().parse().unwrap();
    let key = format!(
        "feedback/{}/{}",
        tom.user_id(),
        staged["image"].as_str().unwrap()
    );
    let res = operator_reply(&mut lena, id, "Hallo").await;
    assert_eq!(res.status, StatusCode::CREATED);

    let res = lena
        .send(
            Method::DELETE,
            &format!("/api/operator/feedback/{id}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let left: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM feedback WHERE id = $1), \
                (SELECT COUNT(*) FROM feedback_messages WHERE feedback_id = $1)",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, (0, 0));
    let queued: bool = sqlx::query_scalar(
        "SELECT next_attempt <= now() FROM storage_cleanup WHERE object_key = $1",
    )
    .bind(&key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(queued, "the trigger queued the image");

    let res = lena
        .send(
            Method::DELETE,
            &format!("/api/operator/feedback/{id}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let rows = audit_rows(&pool).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[1],
        (
            "feedback.delete".to_owned(),
            Some(id),
            Some(uid(&tom)),
            json!({ "kind": "problem", "client": "web", "had_image": true, "messages": 1 })
        )
    );
}

#[sqlx::test]
async fn pending_entries_are_invisible(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let pending = pending_entry(&pool).await;
    for (method, path, body) in routes(pending).into_iter().skip(1) {
        let res = lena.send(method.clone(), &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    let res = lena.send(Method::GET, "/api/operator/feedback", None).await;
    assert_eq!(res.body, json!({ "entries": [], "total": 0 }));
    let res = lena
        .send(Method::GET, "/api/operator/feedback?status=done", None)
        .await;
    assert_eq!(res.body["total"], 0);
    let (status, messages): (String, i64) = sqlx::query_as(
        "SELECT status, (SELECT COUNT(*) FROM feedback_messages WHERE feedback_id = f.id) \
         FROM feedback f WHERE id = $1",
    )
    .bind(pending)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((status.as_str(), messages), ("pending", 0));
    let actions: Vec<String> = audit_rows(&pool)
        .await
        .into_iter()
        .map(|row| row.0)
        .collect();
    assert_eq!(actions, vec!["feedback.list", "feedback.list"]);
}

/// `client`'s session, against another state of the same database.
fn moved(client: &Client, state: &AppState) -> Client {
    let mut moved = Client::from_state(state.clone());
    moved.jar = client.jar.clone();
    moved.csrf = client.csrf.clone();
    moved.user_id = client.user_id.clone();
    moved
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

/// The `mail_state` of a message once it left `pending`.
async fn settled_mail_state(pool: &PgPool, message: &str) -> String {
    let id: Uuid = message.parse().unwrap();
    for _ in 0..200 {
        let state: Option<String> =
            sqlx::query_scalar("SELECT mail_state FROM feedback_messages WHERE id = $1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap();
        match state.as_deref() {
            Some("pending") => tokio::time::sleep(Duration::from_millis(10)).await,
            _ => return state.unwrap_or_default(),
        }
    }
    panic!("mail_state stayed pending");
}

async fn reply_as(lena: &mut Client, id: Uuid, text: &str) -> Value {
    let res = lena
        .send(
            Method::POST,
            &format!("/api/operator/feedback/{id}/messages"),
            Some(json!({ "text": text })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

/// The operators' state with account mail on, and Lena's session there.
/// Lena's address is confirmed, as an operator's must be while mail is on.
async fn with_mail(pool: &PgPool, lena: &Client) -> (AppState, Mailer, Client) {
    sqlx::query("UPDATE users SET email_verified_at = now() WHERE email = $1")
        .bind(LENA)
        .execute(pool)
        .await
        .unwrap();
    let mut state = state_with(pool.clone(), true);
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    let lena = moved(lena, &state);
    (state, mailer, lena)
}

#[sqlx::test]
async fn the_answer_mail_goes_once_per_unread_stretch(pool: PgPool) {
    let plain = state_with(pool.clone(), true);
    let lena = registered(&plain, LENA, "Lena").await;
    let tom = registered(&plain, "tom@example.test", "Tom").await;
    let ada = registered(&plain, "ada@example.test", "Ada").await;
    let kai = registered(&plain, "kai@example.test", "Kai").await;
    sqlx::query(
        "UPDATE users SET email_verified_at = now() \
         WHERE email IN ('tom@example.test', 'kai@example.test')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE users SET locale = 'en' WHERE email = 'tom@example.test'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET disabled_at = now() WHERE email = 'kai@example.test'")
        .execute(&pool)
        .await
        .unwrap();
    let (state, mailer, mut lena) = with_mail(&pool, &lena).await;

    let toms = account_entry(&pool, Some(uid(&tom)), "idea", "open", 0).await;
    reply_as(&mut lena, toms, "Danke").await;
    reply_as(&mut lena, toms, "Und noch etwas").await;
    let sent = mails(&mailer, 1).await;
    assert_eq!(sent[0].to, "tom@example.test");
    assert_eq!(sent[0].subject, "Reply to your feedback");
    assert!(
        sent[0]
            .body
            .contains("\n\nhttps://chat.example.org/settings/feedback\n\n"),
        "{}",
        sent[0].body
    );
    assert!(!sent[0].body.contains("Danke"), "{}", sent[0].body);

    // Once Tom has read it, the next reply mails again.
    let mut tom = moved(&tom, &state);
    let res = tom
        .send(Method::POST, &format!("/api/feedback/{toms}/read"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    reply_as(&mut lena, toms, "Erledigt").await;
    mails(&mailer, 2).await;

    // Unconfirmed (Ada) and disabled (Kai) accounts get nothing.
    for user in [uid(&ada), uid(&kai)] {
        let id = account_entry(&pool, Some(user), "problem", "open", 0).await;
        reply_as(&mut lena, id, "Danke").await;
    }
    mails(&mailer, 2).await;
}

#[sqlx::test]
async fn a_reply_to_a_visitor_goes_out_by_mail(pool: PgPool) {
    let plain = state_with(pool.clone(), true);
    let lena = registered(&plain, LENA, "Lena").await;
    let (_state, mailer, mut lena) = with_mail(&pool, &lena).await;
    let id = visitor_entry(&pool).await;

    let message = reply_as(&mut lena, id, "Thanks!\nWe fixed it.").await;
    assert_eq!(message["mail"], "pending");
    let sent = mails(&mailer, 1).await;
    assert_eq!(sent[0].to, "visitor@example.test");
    assert_eq!(sent[0].subject, "Reply to your feedback to Gelabber");
    assert!(
        sent[0]
            .body
            .contains("(Praise):\n\n> Thanks!\n> We fixed it.\n\n"),
        "{}",
        sent[0].body
    );
    assert!(
        sent[0]
            .body
            .ends_with("\n\nhttps://chat.example.org/en/feedback\n"),
        "{}",
        sent[0].body
    );
    let message = message["id"].as_str().unwrap();
    assert_eq!(settled_mail_state(&pool, message).await, "sent");
    let res = lena
        .send(Method::GET, &format!("/api/operator/feedback/{id}"), None)
        .await;
    assert_eq!(res.body["messages"][0]["mail"], "sent", "{}", res.body);
}

#[sqlx::test]
async fn without_mail_a_reply_to_a_visitor_fails_at_once(pool: PgPool) {
    let state = state_with(pool.clone(), true);
    let mut lena = registered(&state, LENA, "Lena").await;
    let id = visitor_entry(&pool).await;
    let message = reply_as(&mut lena, id, "Thanks").await;
    let message = message["id"].as_str().unwrap();
    assert_eq!(settled_mail_state(&pool, message).await, "failed");
}
