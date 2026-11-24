//! The server overview's "Geteilt" (GEL-69): the newest files of a
//! server's text channels, as far as each channel's history shows them.

mod common;

use axum::http::{Method, StatusCode};
use common::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn member_pair(pool: PgPool) -> (Client, Client, String, String) {
    let mut ada = Client::new(pool.clone());
    ada.bootstrap().await;
    let res = ada.register("ada@test.example", "password123", "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let mut bob = Client::new(pool);
    bob.bootstrap().await;
    let res = bob.register("bob@test.example", "password123", "Bob").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let (server, text) = new_server(&mut ada, "Team").await;
    let invite = ada
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let joined = bob
        .send(
            Method::POST,
            &format!(
                "/api/invites/{}/join",
                invite.body["code"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    (ada, bob, server, text)
}

async fn new_server(client: &mut Client, name: &str) -> (String, String) {
    let res = client
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let text = res.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (res.body["id"].as_str().unwrap().to_owned(), text)
}

async fn channel(client: &mut Client, server: &str, name: &str) -> String {
    let res = client
        .send(
            Method::POST,
            &format!("/api/servers/{server}/channels"),
            Some(json!({ "name": name, "kind": "text" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().to_owned()
}

/// A message with one file per name. The rows go straight into the table:
/// what the endpoint reads, without a store behind it.
async fn send_files(
    pool: &PgPool,
    client: &mut Client,
    channel: &str,
    text: &str,
    files: &[&str],
) -> String {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": if text.is_empty() { "-" } else { text } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let message: Uuid = res.body["id"].as_str().unwrap().parse().unwrap();
    let channel: Uuid = channel.parse().unwrap();
    let uploader: Uuid = client.user_id().parse().unwrap();
    for name in files {
        let kind = if name.ends_with(".png") {
            "image/png"
        } else {
            "application/pdf"
        };
        sqlx::query(
            "INSERT INTO attachments (message_id, channel_id, uploader_id, object_key, \
                 filename, content_type, size_bytes) \
             VALUES ($1, $2, $3, $4, $5, $6, 2048)",
        )
        .bind(message)
        .bind(channel)
        .bind(uploader)
        .bind(format!("att/{}", Uuid::new_v4()))
        .bind(name)
        .bind(kind)
        .execute(pool)
        .await
        .unwrap();
    }
    message.to_string()
}

async fn shared(client: &mut Client, server: &str, query: &str) -> (StatusCode, Value) {
    let res = client
        .send(
            Method::GET,
            &format!("/api/servers/{server}/attachments{query}"),
            None,
        )
        .await;
    (res.status, res.body)
}

fn names(body: &Value) -> Vec<&str> {
    body.as_array()
        .unwrap()
        .iter()
        .map(|f| f["filename"].as_str().unwrap())
        .collect()
}

#[sqlx::test]
async fn newest_message_first_with_sender_and_channel(pool: PgPool) {
    let (mut ada, mut bob, server, text) = member_pair(pool.clone()).await;
    let homework = channel(&mut ada, &server, "hausaufgaben").await;
    let first = send_files(&pool, &mut ada, &text, "folien", &["a.pdf"]).await;
    let second = send_files(&pool, &mut bob, &homework, "", &["b.png", "c.pdf"]).await;

    let (status, body) = shared(&mut ada, &server, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(names(&body), ["b.png", "c.pdf", "a.pdf"]);
    let b = &body[0];
    assert_eq!(b["message_id"], second.as_str());
    assert_eq!(b["channel_id"], homework.as_str());
    assert_eq!(b["author"]["name"], "Bob");
    assert_eq!(b["author"]["id"], bob.user_id());
    assert_eq!(b["content_type"], "image/png");
    assert_eq!(b["size"], 2048);
    assert!(b["created_at"].is_string());
    assert_eq!(body[2]["message_id"], first.as_str());

    let (status, body) = shared(&mut ada, &server, "?limit=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(names(&body), ["b.png"]);
}

#[sqlx::test]
async fn only_what_the_history_shows(pool: PgPool) {
    let (mut ada, mut bob, server, text) = member_pair(pool.clone()).await;
    let secret = channel(&mut ada, &server, "geheim").await;
    send_files(&pool, &mut ada, &text, "", &["open.pdf"]).await;
    send_files(&pool, &mut ada, &secret, "", &["secret.pdf"]).await;
    // A role-limited channel without Bob's role: only its owner sees it.
    sqlx::query("UPDATE channels SET limited = true WHERE id = $1")
        .bind(secret.parse::<Uuid>().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    // Another server's files stay there.
    let (other, other_text) = new_server(&mut ada, "Elsewhere").await;
    send_files(&pool, &mut ada, &other_text, "", &["other.pdf"]).await;
    // Someone else's sender-only message is not in Bob's history.
    let hidden = send_files(&pool, &mut ada, &text, "", &["hidden.pdf"]).await;
    sqlx::query("UPDATE messages SET sender_only = true WHERE id = $1")
        .bind(hidden.parse::<Uuid>().unwrap())
        .execute(&pool)
        .await
        .unwrap();

    let (_, body) = shared(&mut bob, &server, "").await;
    assert_eq!(names(&body), ["open.pdf"]);
    let (_, body) = shared(&mut ada, &server, "").await;
    assert_eq!(names(&body), ["hidden.pdf", "secret.pdf", "open.pdf"]);
    let (status, _) = shared(&mut bob, &other, "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn limit_is_checked(pool: PgPool) {
    let (mut ada, _, server, _) = member_pair(pool).await;
    for query in ["?limit=0", "?limit=51"] {
        let (status, body) = shared(&mut ada, &server, query).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
    }
    let (status, body) = shared(&mut ada, &server, "?limit=50").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
