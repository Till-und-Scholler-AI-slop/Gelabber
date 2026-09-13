//! Text-channel messages: write, own edit/delete, rights, and
//! channel-local cursor/time paging (issue #5).

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

async fn two_users(pool: PgPool) -> (Client, Client) {
    let mut owner = Client::new(pool.clone());
    owner.bootstrap().await;
    let res = owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let mut member = Client::new(pool);
    member.bootstrap().await;
    let res = member
        .register("member@example.com", "password123", "Bob")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    (owner, member)
}

async fn create_server(client: &mut Client, name: &str) -> Value {
    let res = client
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

async fn server_with_member(owner: &mut Client, member: &mut Client) -> Value {
    let server = create_server(owner, "Team").await;
    let id = server["id"].as_str().unwrap();
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let res = member
        .send(
            Method::POST,
            &format!(
                "/api/invites/{}/join",
                invite.body["code"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    server
}

fn text_channel_id(server: &Value) -> String {
    server["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .expect("text channel")["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn post_message(client: &mut Client, channel_id: &str, content: &str) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": content })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

#[sqlx::test]
async fn message_routes_require_a_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;
    let id = "00000000-0000-0000-0000-000000000001";

    let res = client
        .send(Method::GET, &format!("/api/channels/{id}/messages"), None)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "unauthenticated");

    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{id}/messages"),
            Some(json!({ "content": "hi" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn member_can_post_read_edit_and_delete_own_message(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);

    let posted = post_message(&mut member, &channel_id, "  hallo  ").await;
    assert_eq!(posted["content"], "hallo");
    assert_eq!(posted["channel_id"], channel_id);
    assert_eq!(posted["author"]["name"], "Bob");
    assert!(posted["edited_at"].is_null());

    let page = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page.body["messages"][0]["id"], posted["id"]);
    assert_eq!(page.body["has_more"], false);

    let edited = member
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", posted["id"].as_str().unwrap()),
            Some(json!({ "content": "hallo, edit" })),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.body["content"], "hallo, edit");
    assert!(edited.body["edited_at"].is_string());

    let deleted = member
        .send(
            Method::DELETE,
            &format!("/api/messages/{}", posted["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let empty = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(empty.body["messages"], json!([]));
}

#[sqlx::test]
async fn content_is_validated(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);

    let empty = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "   " })),
        )
        .await;
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(empty.body["error"], "validation_failed");
    assert_eq!(empty.body["fields"]["content"], "required");

    let too_long = owner
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "x".repeat(2001) })),
        )
        .await;
    assert_eq!(too_long.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(too_long.body["fields"]["content"], "too_long");
}

#[sqlx::test]
async fn send_messages_is_required_to_post_and_edit(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let server_id = server["id"].as_str().unwrap();
    let channel_id = text_channel_id(&server);

    let posted = post_message(&mut member, &channel_id, "vor dem Entzug").await;

    let stripped = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{server_id}"),
            Some(json!({ "member_permissions": ["send_files", "join_voice", "go_live"] })),
        )
        .await;
    assert_eq!(stripped.status, StatusCode::OK, "{}", stripped.body);

    let denied = member
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(json!({ "content": "nein" })),
        )
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.body["error"], "forbidden");

    let edit = member
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", posted["id"].as_str().unwrap()),
            Some(json!({ "content": "auch nicht" })),
        )
        .await;
    assert_eq!(edit.status, StatusCode::FORBIDDEN);

    // Delete stays the author's, even without send_messages.
    let deleted = member
        .send(
            Method::DELETE,
            &format!("/api/messages/{}", posted["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    // History is still readable.
    let page = member
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
}

#[sqlx::test]
async fn cannot_edit_or_delete_someone_elses_message(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let channel_id = text_channel_id(&server);
    let posted = post_message(&mut owner, &channel_id, "Ada only").await;

    let edit = member
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", posted["id"].as_str().unwrap()),
            Some(json!({ "content": "Bob steals" })),
        )
        .await;
    assert_eq!(edit.status, StatusCode::FORBIDDEN);

    let delete = member
        .send(
            Method::DELETE,
            &format!("/api/messages/{}", posted["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(delete.status, StatusCode::FORBIDDEN);

    let still = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages"),
            None,
        )
        .await;
    assert_eq!(still.body["messages"].as_array().unwrap().len(), 1);
}

#[sqlx::test]
async fn foreign_or_voice_channel_is_not_found(pool: PgPool) {
    let (mut owner, mut stranger) = two_users(pool).await;
    let server = create_server(&mut owner, "Secret").await;
    let text_id = text_channel_id(&server);

    let voice = owner
        .send(
            Method::POST,
            &format!("/api/servers/{}/channels", server["id"].as_str().unwrap()),
            Some(json!({ "name": "Lounge", "kind": "voice" })),
        )
        .await;
    assert_eq!(voice.status, StatusCode::CREATED, "{}", voice.body);
    let voice_id = voice.body["id"].as_str().unwrap();

    let res = stranger
        .send(
            Method::GET,
            &format!("/api/channels/{text_id}/messages"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"], "not_found");

    let res = owner
        .send(
            Method::POST,
            &format!("/api/channels/{voice_id}/messages"),
            Some(json!({ "content": "nein" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    let res = owner
        .send(
            Method::GET,
            &format!("/api/channels/{voice_id}/messages"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn history_pages_by_id_cursor_and_time(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);

    let a = post_message(&mut owner, &channel_id, "eins").await;
    let b = post_message(&mut owner, &channel_id, "zwei").await;
    let c = post_message(&mut owner, &channel_id, "drei").await;
    let ids = [
        a["id"].as_str().unwrap(),
        b["id"].as_str().unwrap(),
        c["id"].as_str().unwrap(),
    ];

    let latest = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages?limit=2"),
            None,
        )
        .await;
    assert_eq!(latest.status, StatusCode::OK);
    let latest_ids: Vec<&str> = latest.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        latest_ids,
        vec![ids[1], ids[2]],
        "oldest → newest, last two"
    );
    assert_eq!(latest.body["has_more"], true);

    let older = owner
        .send(
            Method::GET,
            &format!(
                "/api/channels/{channel_id}/messages?before={}&limit=2",
                ids[1]
            ),
            None,
        )
        .await;
    assert_eq!(older.status, StatusCode::OK);
    let older_ids: Vec<&str> = older.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(older_ids, vec![ids[0]]);
    assert_eq!(older.body["has_more"], false);

    let after = owner
        .send(
            Method::GET,
            &format!(
                "/api/channels/{channel_id}/messages?after={}&limit=2",
                ids[0]
            ),
            None,
        )
        .await;
    let after_ids: Vec<&str> = after.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(after_ids, vec![ids[1], ids[2]]);
    assert_eq!(after.body["has_more"], false);

    let after_epoch = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages?after=1970-01-01T00:00:00Z&limit=10"),
            None,
        )
        .await;
    assert_eq!(after_epoch.status, StatusCode::OK);
    assert_eq!(
        after_epoch.body["messages"].as_array().unwrap().len(),
        3,
        "RFC3339 after-cursor includes everything newer than the instant"
    );
    let before_future = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages?before=2099-01-01T00:00:00Z&limit=10"),
            None,
        )
        .await;
    assert_eq!(before_future.body["messages"].as_array().unwrap().len(), 3);

    let both = owner
        .send(
            Method::GET,
            &format!(
                "/api/channels/{channel_id}/messages?before={}&after={}",
                ids[2], ids[0]
            ),
            None,
        )
        .await;
    assert_eq!(both.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(both.body["fields"]["before"], "invalid");
}

#[sqlx::test]
async fn dead_id_cursor_still_pages_with_time_bound(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let channel_id = text_channel_id(&server);

    let a = post_message(&mut owner, &channel_id, "eins").await;
    let b = post_message(&mut owner, &channel_id, "zwei").await;
    let c = post_message(&mut owner, &channel_id, "drei").await;
    let b_id = b["id"].as_str().unwrap();
    let b_at = b["created_at"].as_str().unwrap();

    let deleted = owner
        .send(Method::DELETE, &format!("/api/messages/{b_id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    // Bare deleted id must not pretend the channel ends.
    let dead = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages?before={b_id}&limit=10"),
            None,
        )
        .await;
    assert_eq!(dead.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(dead.body["fields"]["before"], "invalid");

    // `{created_at}|{id}` still walks older than the deleted row.
    let older = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages?before={b_at}%7C{b_id}&limit=10"),
            None,
        )
        .await;
    assert_eq!(older.status, StatusCode::OK, "{}", older.body);
    let older_ids: Vec<&str> = older.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(older_ids, vec![a["id"].as_str().unwrap()]);
    assert_eq!(older.body["has_more"], false);

    let newer = owner
        .send(
            Method::GET,
            &format!("/api/channels/{channel_id}/messages?after={b_at}%7C{b_id}&limit=10"),
            None,
        )
        .await;
    let newer_ids: Vec<&str> = newer.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(newer_ids, vec![c["id"].as_str().unwrap()]);
}
