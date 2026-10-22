//! Marking whole servers and everything as read (#245).
mod common;

use axum::http::{Method, StatusCode};
use common::Client;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn user(pool: &PgPool, email: &str, name: &str) -> Client {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let r = client.register(email, "password123", name).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    client
}

async fn server(owner: &mut Client, member: &mut Client, name: &str) -> (String, String) {
    let server = owner
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(server.status, StatusCode::CREATED, "{}", server.body);
    let sid = server.body["id"].as_str().unwrap().to_owned();
    let cid = server.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let joined = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    (sid, cid)
}

async fn channel(owner: &mut Client, server: &str, name: &str) -> String {
    let r = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server}/channels"),
            Some(json!({ "name": name, "kind": "text" })),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body["id"].as_str().unwrap().to_owned()
}

async fn post(client: &mut Client, channel: &str, text: &str) -> Value {
    let r = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": text })),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body
}

async fn unread(client: &mut Client) -> Vec<Value> {
    let r = client.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body.as_array().unwrap().clone()
}

fn count(rows: &[Value], channel: &str) -> i64 {
    rows.iter()
        .find(|r| r["channel_id"] == channel)
        .map_or(-1, |r| r["unread_count"].as_i64().unwrap())
}

#[sqlx::test]
async fn a_server_is_read_in_one_request_and_nothing_else(pool: PgPool) {
    let mut a = user(&pool, "a@test.example", "Ada").await;
    let mut b = user(&pool, "b@test.example", "Bob").await;
    let (sid, general) = server(&mut a, &mut b, "Eins").await;
    let other = channel(&mut a, &sid, "zwei").await;
    let (_, elsewhere) = server(&mut a, &mut b, "Anders").await;
    for c in [&general, &other, &elsewhere] {
        post(&mut a, c, "hallo").await;
        post(&mut a, c, "noch was").await;
    }
    let r = b
        .send(Method::PUT, &format!("/api/servers/{sid}/read"), None)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    // The answer is the rows that moved, already read.
    let moved: Vec<&str> = r
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            assert_eq!(row["unread_count"], 0);
            row["channel_id"].as_str().unwrap()
        })
        .collect();
    assert_eq!(moved.len(), 2);
    assert!(moved.contains(&general.as_str()) && moved.contains(&other.as_str()));
    let rows = unread(&mut b).await;
    assert_eq!(count(&rows, &general), 0);
    assert_eq!(count(&rows, &other), 0);
    assert_eq!(count(&rows, &elsewhere), 2);
    // Nothing more to move: an empty answer.
    let again = b
        .send(Method::PUT, &format!("/api/servers/{sid}/read"), None)
        .await;
    assert_eq!(again.body, json!([]));
    // Not a member: as if the server did not exist.
    let mut c = user(&pool, "c@test.example", "Cem").await;
    let r = c
        .send(Method::PUT, &format!("/api/servers/{sid}/read"), None)
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn read_all_and_dms_only(pool: PgPool) {
    let mut a = user(&pool, "a@test.example", "Ada").await;
    let mut b = user(&pool, "b@test.example", "Bob").await;
    let (_, general) = server(&mut a, &mut b, "Eins").await;
    let dm = a
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({"user_id": b.user_id()})),
        )
        .await;
    assert!(dm.status.is_success(), "{}", dm.body);
    let dm = dm.body["id"].as_str().unwrap().to_owned();
    post(&mut a, &general, "hallo").await;
    post(&mut a, &dm, "psst").await;
    let r = b
        .send(
            Method::PUT,
            "/api/messages/read-all",
            Some(json!({"scope":"dms"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let rows = unread(&mut b).await;
    assert_eq!(count(&rows, &dm), 0);
    assert_eq!(count(&rows, &general), 1);
    let r = b
        .send(Method::PUT, "/api/messages/read-all", Some(json!({})))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(count(&unread(&mut b).await, &general), 0);
    // Only the conversations that moved, with the position clients order by.
    let moved = r.body.as_array().unwrap();
    assert_eq!(moved.len(), 1, "{}", r.body);
    assert_eq!(moved[0]["channel_id"], general.as_str());
    assert!(moved[0]["read_order"].is_i64(), "{}", r.body);
    assert_eq!(moved[0]["read_order"], moved[0]["latest_order"]);
    assert!(moved[0]["membership_at"].is_string(), "{}", r.body);
    // A loop is held back.
    let mut status = StatusCode::OK;
    for _ in 0..25 {
        status = b
            .send(Method::PUT, "/api/messages/read-all", Some(json!({})))
            .await
            .status;
    }
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test]
async fn a_bulk_read_never_moves_a_cursor_back(pool: PgPool) {
    let mut a = user(&pool, "a@test.example", "Ada").await;
    let mut b = user(&pool, "b@test.example", "Bob").await;
    let (sid, general) = server(&mut a, &mut b, "Eins").await;
    post(&mut a, &general, "eins").await;
    let newest = post(&mut a, &general, "zwei").await;
    // Read to the newest, then delete it: the newest left is older than
    // the cursor, and a bulk read must not pull the cursor back to it.
    let r = b
        .send(
            Method::PUT,
            &format!("/api/channels/{general}/read"),
            Some(json!({"message_id": newest["id"]})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let id = newest["id"].as_str().unwrap();
    let r = a
        .send(Method::DELETE, &format!("/api/messages/{id}"), None)
        .await;
    assert!(r.status.is_success(), "{}", r.body);
    let r = b
        .send(Method::PUT, &format!("/api/servers/{sid}/read"), None)
        .await;
    assert_eq!(r.body, json!([]));
    let rows = unread(&mut b).await;
    let row = rows.iter().find(|r| r["channel_id"] == general).unwrap();
    assert_eq!(row["read_message_id"], newest["id"]);
}

#[sqlx::test]
async fn a_channel_without_a_message_id_is_read_to_its_newest(pool: PgPool) {
    let mut a = user(&pool, "a@test.example", "Ada").await;
    let mut b = user(&pool, "b@test.example", "Bob").await;
    let (_, general) = server(&mut a, &mut b, "Eins").await;
    let empty = b
        .send(
            Method::PUT,
            &format!("/api/channels/{general}/read"),
            Some(json!({})),
        )
        .await;
    assert_eq!(empty.status, StatusCode::OK, "{}", empty.body);
    assert_eq!(empty.body["read_message_id"], Value::Null);
    post(&mut a, &general, "eins").await;
    let newest = post(&mut a, &general, "zwei").await;
    let r = b
        .send(
            Method::PUT,
            &format!("/api/channels/{general}/read"),
            Some(json!({})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["read_message_id"], newest["id"]);
    assert_eq!(r.body["unread_count"], 0);
}

/// Everything read holds no server row, so a channel deleted while its
/// upsert runs fails the foreign key. The request tries again and succeeds
/// without the deleted channel instead of answering 500.
#[sqlx::test]
async fn read_all_survives_a_channel_deleted_meanwhile(pool: PgPool) {
    let mut a = user(&pool, "a@test.example", "Ada").await;
    let mut b = user(&pool, "b@test.example", "Bob").await;
    let (sid, general) = server(&mut a, &mut b, "Eins").await;
    let doomed = channel(&mut a, &sid, "weg").await;
    post(&mut a, &general, "hallo").await;
    post(&mut a, &doomed, "gleich weg").await;

    // The delete has run but not committed: the read-all still sees the
    // channel and waits on its row in the foreign key check.
    let mut delete = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM channels WHERE id=$1::uuid")
        .bind(&doomed)
        .execute(&mut *delete)
        .await
        .unwrap();
    let reading = tokio::spawn(async move {
        let r = b
            .send(Method::PUT, "/api/messages/read-all", Some(json!({})))
            .await;
        (b, r)
    });
    until_a_session_waits_for_a_lock(&pool).await;
    delete.commit().await.unwrap();
    let (mut b, r) = reading.await.unwrap();
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let moved: Vec<&str> = r
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["channel_id"].as_str().unwrap())
        .collect();
    assert_eq!(moved, [general.as_str()]);
    assert_eq!(count(&unread(&mut b).await, &general), 0);
}

/// Waits until some other session of this test database blocks on a lock.
async fn until_a_session_waits_for_a_lock(pool: &PgPool) {
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the read-all never waited for the channel row");
}

/// History from before joining stays out of the counts and the bulk reads,
/// whether there is no read position yet or one on an old message (#277:
/// both are now found through the time index, not by walking the history).
#[sqlx::test]
async fn history_before_joining_stays_read(pool: PgPool) {
    let mut a = user(&pool, "a@test.example", "Ada").await;
    let mut c = user(&pool, "c@test.example", "Cem").await;
    let mut b = user(&pool, "b@test.example", "Bob").await;
    let (sid, general) = server(&mut a, &mut c, "Eins").await;
    let quiet = channel(&mut a, &sid, "ruhig").await;
    let old = post(&mut a, &general, "alt @everyone").await;
    post(&mut a, &quiet, "auch alt @everyone").await;
    let invite = a
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let joined = b
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);

    let rows = unread(&mut b).await;
    for id in [&general, &quiet] {
        let row = rows.iter().find(|r| r["channel_id"] == *id).unwrap();
        assert_eq!(row["unread_count"], 0, "{row}");
        assert_eq!(row["mention_count"], 0, "{row}");
    }
    // Read up to an old message: still nothing before joining counts.
    let r = b
        .send(
            Method::PUT,
            &format!("/api/channels/{general}/read"),
            Some(json!({ "message_id": old["id"] })),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["unread_count"], 0);
    post(&mut a, &general, "neu @everyone").await;
    let rows = unread(&mut b).await;
    let row = rows.iter().find(|r| r["channel_id"] == general).unwrap();
    assert_eq!(row["unread_count"], 1, "{row}");
    assert_eq!(row["mention_count"], 1, "{row}");

    // The quiet channel has nothing to read, so nothing to write.
    let r = b
        .send(Method::PUT, "/api/messages/read-all", Some(json!({})))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let moved: Vec<&Value> = r.body.as_array().unwrap().iter().collect();
    assert_eq!(moved.len(), 1, "{}", r.body);
    assert_eq!(moved[0]["channel_id"], general.as_str());
    assert_eq!(moved[0]["unread_count"], 0);
    let r = b
        .send(
            Method::PUT,
            &format!("/api/channels/{quiet}/read"),
            Some(json!({})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["read_message_id"], Value::Null);
}
