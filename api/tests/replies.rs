//! Replies (issue #251): same-channel target, compact preview of the
//! original in every payload, and a marker once the original is gone.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

async fn user(pool: &PgPool, email: &str, name: &str) -> Client {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let res = client.register(email, "password123", name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn text_channel(client: &mut Client, name: &str) -> String {
    let res = client
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .expect("text channel")["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn post(client: &mut Client, channel: &str, body: Value) -> (StatusCode, Value) {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(body),
        )
        .await;
    (res.status, res.body)
}

async fn history(client: &mut Client, channel: &str) -> Vec<Value> {
    let res = client
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["messages"].as_array().unwrap().clone()
}

#[sqlx::test]
async fn reply_carries_a_preview_of_the_original(pool: PgPool) {
    let mut ada = user(&pool, "ada@example.com", "Ada").await;
    let channel = text_channel(&mut ada, "Team").await;
    let long = format!("erste Zeile\nzweite {}", "x".repeat(150));
    let (_, original) = post(&mut ada, &channel, json!({ "content": long })).await;
    let (status, reply) = post(
        &mut ada,
        &channel,
        json!({ "content": "stimmt", "reply_to": original["id"] }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{reply}");
    let preview = &reply["reply"];
    assert_eq!(preview["id"], original["id"]);
    assert_eq!(preview["deleted"], false);
    assert_eq!(preview["author"]["name"], "Ada");
    let excerpt = preview["excerpt"].as_str().unwrap();
    assert!(excerpt.starts_with("erste Zeile zweite x"), "{excerpt}");
    assert_eq!(excerpt.chars().count(), 101);
    assert!(preview.get("has_attachment").is_none());

    let rows = history(&mut ada, &channel).await;
    assert!(rows[0].get("reply").is_none(), "plain message has no reply");
    assert_eq!(rows[1]["reply"], *preview);
}

#[sqlx::test]
async fn reply_to_another_channel_is_rejected_and_a_gone_original_shows_deleted(pool: PgPool) {
    let mut ada = user(&pool, "ada@example.com", "Ada").await;
    let here = text_channel(&mut ada, "Hier").await;
    let there = text_channel(&mut ada, "Dort").await;
    let (_, elsewhere) = post(&mut ada, &there, json!({ "content": "dort" })).await;

    let (status, body) = post(
        &mut ada,
        &here,
        json!({ "content": "x", "reply_to": elsewhere["id"] }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["fields"]["reply_to"], "invalid");
    assert!(history(&mut ada, &here).await.is_empty(), "nothing stored");

    // The original was deleted while the reply was written (or before a
    // retry): the reply is kept and quotes it as deleted.
    let gone = uuid::Uuid::new_v4();
    let (status, body) = post(
        &mut ada,
        &here,
        json!({ "content": "zu spät", "reply_to": gone }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["reply"], json!({ "id": gone, "deleted": true }));
}

#[sqlx::test]
async fn original_edit_and_delete_show_in_the_reply(pool: PgPool) {
    let mut ada = user(&pool, "ada@example.com", "Ada").await;
    let channel = text_channel(&mut ada, "Team").await;
    let (_, original) = post(&mut ada, &channel, json!({ "content": "alt" })).await;
    let original_id = original["id"].as_str().unwrap().to_owned();
    let (_, reply) = post(
        &mut ada,
        &channel,
        json!({ "content": "antwort", "reply_to": original_id }),
    )
    .await;

    let res = ada
        .send(
            Method::PATCH,
            &format!("/api/messages/{original_id}"),
            Some(json!({ "content": "neu" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        history(&mut ada, &channel).await[1]["reply"]["excerpt"],
        "neu"
    );

    // An edit of the reply itself keeps its preview.
    let res = ada
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", reply["id"].as_str().unwrap()),
            Some(json!({ "content": "antwort 2" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["reply"]["excerpt"], "neu");

    let res = ada
        .send(
            Method::DELETE,
            &format!("/api/messages/{original_id}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let rows = history(&mut ada, &channel).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["content"], "antwort 2");
    assert_eq!(
        rows[0]["reply"],
        json!({ "id": original_id, "deleted": true })
    );

    // Search and jump-to context use the same payload.
    let res = ada
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/search?q=antwort"),
            None,
        )
        .await;
    assert_eq!(res.body["messages"][0]["reply"]["deleted"], true);
    let res = ada
        .send(
            Method::GET,
            &format!(
                "/api/channels/{channel}/messages/{}/context",
                reply["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(res.body["messages"][0]["reply"]["deleted"], true);
}

#[sqlx::test]
async fn reaction_event_keeps_the_reply(pool: PgPool) {
    let mut ada = user(&pool, "ada@example.com", "Ada").await;
    let channel = text_channel(&mut ada, "Team").await;
    let (_, original) = post(&mut ada, &channel, json!({ "content": "frage" })).await;
    let (_, reply) = post(
        &mut ada,
        &channel,
        json!({ "content": "antwort", "reply_to": original["id"] }),
    )
    .await;
    let id = reply["id"].as_str().unwrap();
    let res = ada
        .send(
            Method::PUT,
            &format!("/api/messages/{id}/reactions/%F0%9F%91%8D"),
            None,
        )
        .await;
    assert!(res.status.is_success(), "{} {}", res.status, res.body);
    assert_eq!(res.body["reply"]["excerpt"], "frage", "{}", res.body);
}
