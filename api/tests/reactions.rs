//! Idempotency, permission loss and reaction/edit/delete serialization.
mod common;
use axum::http::{Method, StatusCode};
use common::Client;
use gelabber_api::{AppState, Config};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

fn offline_client(pool: PgPool) -> Client {
    let config = Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".into()),
        "DATABASE_URL" => Some("postgres://unused@127.0.0.1:1/unused".into()),
        "REDIS_URL" => Some("redis://127.0.0.1:1".into()),
        _ => None,
    })
    .unwrap();
    Client::with_state(AppState::with_pool(&config, pool).unwrap())
}

async fn users(pool: &PgPool, offline: bool) -> (Client, Client) {
    let mut a = if offline {
        offline_client(pool.clone())
    } else {
        Client::new(pool.clone())
    };
    a.bootstrap().await;
    assert_eq!(
        a.register("a@test.invalid", "password123", "Ada")
            .await
            .status,
        StatusCode::CREATED
    );
    let mut b = if offline {
        offline_client(pool.clone())
    } else {
        Client::new(pool.clone())
    };
    b.bootstrap().await;
    assert_eq!(
        b.register("b@test.invalid", "password123", "Bob")
            .await
            .status,
        StatusCode::CREATED
    );
    (a, b)
}
async fn channel(a: &mut Client, b: &mut Client) -> (String, String) {
    let server = a
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({"name":"Reactions"})),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED);
    let id = server.body["id"].as_str().unwrap().to_owned();
    let text = server.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let invite = a
        .send(
            Method::POST,
            &format!("/api/servers/{id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(
        b.send(
            Method::POST,
            &format!(
                "/api/invites/{}/join",
                invite.body["code"].as_str().unwrap()
            ),
            None
        )
        .await
        .status,
        StatusCode::OK
    );
    (id, text)
}
async fn message(a: &mut Client, channel: &str) -> Value {
    let res = a
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({"content":"original"})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    res.body
}
fn path(m: &Value, emoji: &str) -> String {
    // Percent-encode UTF-8, including selectors and ZWJ, as the web client does.
    let encoded = emoji
        .as_bytes()
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    format!(
        "/api/messages/{}/reactions/{encoded}",
        m["id"].as_str().unwrap()
    )
}
fn same_session(pool: &PgPool, original: &Client) -> Client {
    let mut client = Client::new(pool.clone());
    client.jar = original.jar.clone();
    client.csrf = original.csrf.clone();
    client.user_id = original.user_id.clone();
    client
}

#[sqlx::test]
async fn idempotent_canonical_votes_preserve_content_and_edit_time(pool: PgPool) {
    let (mut a, mut b) = users(&pool, true).await;
    let (_, ch) = channel(&mut a, &mut b).await;
    let m = message(&mut a, &ch).await;
    let read = b
        .send(
            Method::PUT,
            &format!("/api/channels/{ch}/read"),
            Some(json!({"message_id": m["id"]})),
        )
        .await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["unread_count"], 0);
    let first = b.send(Method::PUT, &path(&m, "❤"), None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(
        first.body["reactions"],
        json!([{"emoji":"❤️","user_ids":[b.user_id()]}])
    );
    assert_eq!(first.body["content"], m["content"]);
    assert_eq!(first.body["edited_at"], m["edited_at"]);
    let again = b.send(Method::PUT, &path(&m, "❤️"), None).await;
    assert_eq!(again.body, first.body);
    let second = b.send(Method::PUT, &path(&m, "👍🏽"), None).await;
    assert_eq!(second.body["reactions"].as_array().unwrap().len(), 2);
    let edited = a
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", m["id"].as_str().unwrap()),
            Some(json!({"content":"edited"})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.body["reactions"], second.body["reactions"]);
    let removed = b.send(Method::DELETE, &path(&m, "❤"), None).await;
    assert_eq!(removed.body["edited_at"], edited.body["edited_at"]);
    let repeat = b.send(Method::DELETE, &path(&m, "❤️"), None).await;
    assert_eq!(repeat.body, removed.body);
    let history = a
        .send(Method::GET, &format!("/api/channels/{ch}/messages"), None)
        .await;
    assert_eq!(history.body["messages"][0], removed.body);
    let search = a
        .send(
            Method::GET,
            &format!("/api/channels/{ch}/messages/search?q=edited"),
            None,
        )
        .await;
    assert_eq!(search.status, StatusCode::OK, "{}", search.body);
    assert_eq!(search.body["messages"][0], removed.body);
    assert_eq!(removed.body["created_order"], m["created_order"]);
    let unread = b.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(unread.status, StatusCode::OK);
    let cursor = unread
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["channel_id"] == ch)
        .unwrap();
    assert_eq!(
        cursor["unread_count"], 0,
        "reactions and edits must not create unread messages"
    );
    let id: Uuid = m["id"].as_str().unwrap().parse().unwrap();
    let changes: Vec<(String, Value)> =
        sqlx::query_as("SELECT kind,delta FROM gateway_outbox WHERE entity_id=$1 ORDER BY id")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(changes.len(), 5); // create, two adds, edit, one remove.
    assert_eq!(
        changes.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
        ["c", "e", "e", "e", "e"]
    );
    assert_eq!(changes.last().unwrap().1, removed.body);
}

#[sqlx::test]
async fn removal_requires_read_but_does_not_require_write(pool: PgPool) {
    let (mut a, mut b) = users(&pool, false).await;
    let (server, ch) = channel(&mut a, &mut b).await;
    let m = message(&mut a, &ch).await;
    assert_eq!(
        b.send(Method::PUT, &path(&m, "😀"), None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        a.send(
            Method::PATCH,
            &format!("/api/servers/{server}"),
            Some(json!({"member_permissions":["join_voice"]}))
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        b.send(Method::PUT, &path(&m, "👍"), None).await.status,
        StatusCode::FORBIDDEN
    );
    let removed = b.send(Method::DELETE, &path(&m, "😀"), None).await;
    assert_eq!(removed.status, StatusCode::OK);
    assert_eq!(removed.body["reactions"], json!([]));
    assert_eq!(
        b.send(Method::POST, &format!("/api/servers/{server}/leave"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        b.send(Method::DELETE, &path(&m, "😀"), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        b.send(Method::PUT, &path(&m, "😀"), None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn dm_votes_are_private_and_invalid_sequences_are_rejected(pool: PgPool) {
    let (mut a, mut b) = users(&pool, false).await;
    let dm = a
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({"user_id":b.user_id()})),
        )
        .await;
    assert_eq!(dm.status, StatusCode::CREATED);
    let ch = dm.body["id"].as_str().unwrap();
    let m = message(&mut a, ch).await;
    let reacted = b.send(Method::PUT, &path(&m, "👩‍💻"), None).await;
    assert_eq!(reacted.status, StatusCode::OK);
    let mut other = offline_client(pool.clone());
    other.bootstrap().await;
    other
        .register("other@test.invalid", "password123", "Other")
        .await;
    assert_eq!(
        other.send(Method::PUT, &path(&m, "😀"), None).await.status,
        StatusCode::NOT_FOUND
    );
    for invalid in ["text", "😀😀", "🧑‍unknown"] {
        let res = a.send(Method::PUT, &path(&m, invalid), None).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(res.body["fields"]["emoji"], "invalid");
    }
    let malformed = a
        .send(
            Method::PUT,
            "/api/messages/not-an-id/reactions/%F0%9F%98%80",
            None,
        )
        .await;
    assert_eq!(malformed.status, StatusCode::NOT_FOUND);
    assert_eq!(malformed.body["error"], "not_found");
    let deleted = a
        .send(
            Method::DELETE,
            &format!("/api/messages/{}", m["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(
        b.send(Method::PUT, &path(&m, "😀"), None).await.status,
        StatusCode::NOT_FOUND
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM message_reactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn simultaneous_votes_edits_and_delete_keep_canonical_revisions(pool: PgPool) {
    let (mut a, mut b) = users(&pool, false).await;
    let (_, ch) = channel(&mut a, &mut b).await;
    let m = message(&mut a, &ch).await;
    let mut duplicate = same_session(&pool, &b);
    let heart = path(&m, "❤️");
    let thumb = path(&m, "👍");
    let (one, two, three) = tokio::join!(
        b.send(Method::PUT, &heart, None),
        duplicate.send(Method::PUT, &heart, None),
        a.send(Method::PUT, &thumb, None)
    );
    assert_eq!(one.status, StatusCode::OK);
    assert_eq!(two.status, StatusCode::OK);
    assert_eq!(three.status, StatusCode::OK);
    let history = a
        .send(Method::GET, &format!("/api/channels/{ch}/messages"), None)
        .await;
    assert_eq!(
        history.body["messages"][0]["reactions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let before = history.body["messages"][0]["revision"].as_i64().unwrap();
    let edit_path = format!("/api/messages/{}", m["id"].as_str().unwrap());
    let (edited, removed) = tokio::join!(
        a.send(
            Method::PATCH,
            &edit_path,
            Some(json!({"content":"new text"}))
        ),
        b.send(Method::DELETE, &heart, None)
    );
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(removed.status, StatusCode::OK);
    let history = a
        .send(Method::GET, &format!("/api/channels/{ch}/messages"), None)
        .await;
    let latest = &history.body["messages"][0];
    assert_eq!(latest["content"], "new text");
    assert_eq!(
        latest["reactions"],
        json!([{"emoji":"👍","user_ids":[a.user_id()]}])
    );
    assert!(latest["revision"].as_i64().unwrap() > before);
    let (deleted, reaction) = tokio::join!(
        a.send(Method::DELETE, &edit_path, None),
        b.send(Method::PUT, &heart, None)
    );
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!([StatusCode::OK, StatusCode::NOT_FOUND].contains(&reaction.status));
    let history = a
        .send(Method::GET, &format!("/api/channels/{ch}/messages"), None)
        .await;
    assert_eq!(history.body["messages"], json!([]));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM message_reactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn in_flight_reaction_rechecks_membership_after_the_writer_lock(pool: PgPool) {
    let (mut a, mut b) = users(&pool, false).await;
    let (server, ch) = channel(&mut a, &mut b).await;
    let m = message(&mut a, &ch).await;
    let server: Uuid = server.parse().unwrap();
    let member: Uuid = b.user_id().parse().unwrap();
    let mut removal = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM servers WHERE id=$1 FOR UPDATE")
        .bind(server)
        .fetch_one(&mut *removal)
        .await
        .unwrap();
    let path = path(&m, "😀");
    let mut reaction = tokio::spawn(async move { b.send(Method::PUT, &path, None).await });
    // A permission writer can hold the row while a request has already read old access.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut reaction)
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM server_members WHERE server_id=$1 AND user_id=$2")
        .bind(server)
        .bind(member)
        .execute(&mut *removal)
        .await
        .unwrap();
    removal.commit().await.unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(2), reaction)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM message_reactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
