//! Mentions in the unread rows (#241) and notification settings (#242).
mod common;

use axum::http::{Method, StatusCode};
use common::Client;
use serde_json::{Value, json};
use sqlx::PgPool;

struct World {
    a: Client,
    b: Client,
    server: String,
    channel: String,
}

async fn user(pool: &PgPool, email: &str, name: &str) -> Client {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let r = client.register(email, "password123", name).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    client
}

async fn setup(pool: &PgPool) -> World {
    let mut a = user(pool, "a@test.example", "Ada").await;
    let mut b = user(pool, "b@test.example", "Bob").await;
    let server = a
        .send(Method::POST, "/api/servers", Some(json!({"name":"Chat"})))
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
    World {
        a,
        b,
        server: sid,
        channel: cid,
    }
}

async fn post(client: &mut Client, channel: &str, text: &str) -> Value {
    let r = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({"content":text})),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body
}

async fn row(client: &mut Client, channel: &str) -> Value {
    let r = client.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["channel_id"] == channel)
        .unwrap()
        .clone()
}

async fn set(client: &mut Client, path: &str, body: Value) -> (StatusCode, Value) {
    let r = client.send(Method::PUT, path, Some(body)).await;
    (r.status, r.body)
}

#[sqlx::test]
async fn a_mention_counts_for_the_one_mentioned_until_read(pool: PgPool) {
    let World {
        mut a,
        mut b,
        channel,
        ..
    } = setup(&pool).await;
    let bob = b.user_id().to_owned();
    let ada = a.user_id().to_owned();
    post(&mut a, &channel, "plain").await;
    let mention = post(&mut a, &channel, &format!("hey <@{bob}>")).await;
    assert_eq!(mention["mentions"], json!([bob]));
    assert_eq!(mention["mention_everyone"], false);
    let state = row(&mut b, &channel).await;
    assert_eq!(state["unread_count"], 2);
    assert_eq!(state["mention_count"], 1);
    // Nobody mentioned Ada, and her own messages never count.
    assert_eq!(row(&mut a, &channel).await["mention_count"], 0);
    // The author is never kept, nor someone outside the conversation.
    let stranger = user(&pool, "c@test.example", "Cem").await;
    let own = post(
        &mut a,
        &channel,
        &format!("<@{ada}> <@{}>", stranger.user_id()),
    )
    .await;
    assert_eq!(own["mentions"], json!([]));
    // In code it is no mention.
    let code = post(&mut a, &channel, &format!("`<@{bob}>`")).await;
    assert_eq!(code["mentions"], json!([]));

    let read = b
        .send(
            Method::PUT,
            &format!("/api/channels/{channel}/read"),
            Some(json!({"message_id": code["id"]})),
        )
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_eq!(read.body["mention_count"], 0);
    assert_eq!(read.body["unread_count"], 0);
}

#[sqlx::test]
async fn everyone_needs_the_right_and_stays_rare(pool: PgPool) {
    let World {
        mut a,
        mut b,
        server,
        channel,
    } = setup(&pool).await;
    // Without the right the message goes out as plain text.
    let plain = post(&mut b, &channel, "@everyone Essen!").await;
    assert_eq!(plain["mention_everyone"], false);
    assert_eq!(row(&mut a, &channel).await["mention_count"], 0);
    let granted = a
        .send(
            Method::PATCH,
            &format!("/api/servers/{server}"),
            Some(json!({"member_permissions":["send_messages","send_files","join_voice","go_live","mention_everyone"]})),
        )
        .await;
    assert_eq!(granted.status, StatusCode::OK, "{}", granted.body);
    let message = post(&mut b, &channel, "@everyone Essen!").await;
    assert_eq!(message["mention_everyone"], true);
    assert_eq!(row(&mut a, &channel).await["mention_count"], 1);
    // Not for the author.
    assert_eq!(row(&mut b, &channel).await["mention_count"], 0);
    // Escaped it is text.
    let escaped = post(&mut b, &channel, "\\@everyone").await;
    assert_eq!(escaped["mention_everyone"], false);
    // A few per window; beyond that the ping is dropped, not the message.
    post(&mut b, &channel, "@here eins").await;
    post(&mut b, &channel, "@everyone zwei").await;
    let limited = post(&mut b, &channel, "@everyone drei").await;
    assert_eq!(limited["mention_everyone"], false);
    assert_eq!(row(&mut a, &channel).await["mention_count"], 2);
    // With the limit spent, fixing a typo keeps a ping the message had,
    // and an edit cannot add one the limit dropped.
    let edit = |id: &str| format!("/api/messages/{id}");
    let kept = b
        .send(
            Method::PATCH,
            &edit(message["id"].as_str().unwrap()),
            Some(json!({"content": "@everyone Essen jetzt!"})),
        )
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.body);
    assert_eq!(kept.body["mention_everyone"], true);
    let still = b
        .send(
            Method::PATCH,
            &edit(limited["id"].as_str().unwrap()),
            Some(json!({"content": "@everyone drei!"})),
        )
        .await;
    assert_eq!(still.body["mention_everyone"], false);
    assert_eq!(row(&mut a, &channel).await["mention_count"], 2);
}

/// Decision: an edit names its mentions anew. Adding one to an unread
/// message counts it, removing it takes the count back.
#[sqlx::test]
async fn an_edit_changes_what_a_message_mentions(pool: PgPool) {
    let World {
        mut a,
        mut b,
        channel,
        ..
    } = setup(&pool).await;
    let bob = b.user_id().to_owned();
    let message = post(&mut a, &channel, "hallo").await;
    let id = message["id"].as_str().unwrap();
    assert_eq!(row(&mut b, &channel).await["mention_count"], 0);
    let edited = a
        .send(
            Method::PATCH,
            &format!("/api/messages/{id}"),
            Some(json!({"content": format!("hallo <@{bob}>")})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(edited.body["mentions"], json!([bob]));
    assert_eq!(row(&mut b, &channel).await["mention_count"], 1);
    let back = a
        .send(
            Method::PATCH,
            &format!("/api/messages/{id}"),
            Some(json!({"content": "hallo"})),
        )
        .await;
    assert_eq!(back.body["mentions"], json!([]));
    assert_eq!(row(&mut b, &channel).await["mention_count"], 0);
}

#[sqlx::test]
async fn a_dm_mentions_the_other_participant(pool: PgPool) {
    let World { mut a, mut b, .. } = setup(&pool).await;
    let dm = a
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({"user_id": b.user_id()})),
        )
        .await;
    assert!(dm.status.is_success(), "{}", dm.body);
    let channel = dm.body["id"].as_str().unwrap().to_owned();
    let message = post(&mut a, &channel, "@here kurz?").await;
    assert_eq!(message["mention_everyone"], true);
    let state = row(&mut b, &channel).await;
    assert_eq!(state["mention_count"], 1);
    assert_eq!(state["level"], "all");
    // DM settings are channel settings.
    let (status, body) = set(
        &mut b,
        &format!("/api/channels/{channel}/notification-settings"),
        json!({"level": "none"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(&mut b, &channel).await["level"], "none");
    // Not for the other participant.
    assert_eq!(row(&mut a, &channel).await["level"], "all");
}

#[sqlx::test]
async fn a_channel_setting_overrides_its_server_and_inherit_falls_back(pool: PgPool) {
    let World {
        mut b,
        server,
        channel,
        ..
    } = setup(&pool).await;
    let state = row(&mut b, &channel).await;
    assert_eq!(state["level"], "all");
    assert_eq!(state["muted"], false);
    assert_eq!(state["muted_until"], Value::Null);

    let server_path = format!("/api/servers/{server}/notification-settings");
    let channel_path = format!("/api/channels/{channel}/notification-settings");
    let (status, body) = set(&mut b, &server_path, json!({"level": "mentions"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"scope":"server","id":server,"level":"mentions","muted":false,"muted_until":null})
    );
    assert_eq!(row(&mut b, &channel).await["level"], "mentions");
    set(&mut b, &channel_path, json!({"level": "all"})).await;
    assert_eq!(row(&mut b, &channel).await["level"], "all");
    // Inherit again: back to the server's.
    let (_, body) = set(&mut b, &channel_path, json!({"level": null})).await;
    assert_eq!(body["level"], Value::Null);
    assert_eq!(row(&mut b, &channel).await["level"], "mentions");

    let listed = b
        .send(Method::GET, "/api/notification-settings", None)
        .await;
    // The inheriting, unmuted channel row is gone.
    assert_eq!(listed.body.as_array().unwrap().len(), 1, "{}", listed.body);
}

#[sqlx::test]
async fn a_mute_ends_on_the_server_clock(pool: PgPool) {
    let World {
        mut a,
        mut b,
        server,
        channel,
    } = setup(&pool).await;
    let channel_path = format!("/api/channels/{channel}/notification-settings");
    let (status, body) = set(
        &mut b,
        &channel_path,
        json!({"muted": true, "mute_minutes": 60}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let until = chrono::DateTime::parse_from_rfc3339(body["muted_until"].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc);
    let left = until - chrono::Utc::now();
    assert!(
        left > chrono::Duration::minutes(59) && left <= chrono::Duration::minutes(60),
        "{left}"
    );
    let state = row(&mut b, &channel).await;
    assert_eq!(state["muted"], true);
    assert_eq!(state["muted_until"], body["muted_until"]);
    // Only for Bob.
    assert_eq!(row(&mut a, &channel).await["muted"], false);

    // The hour has passed.
    sqlx::query("UPDATE notification_settings SET muted_until = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let state = row(&mut b, &channel).await;
    assert_eq!(state["muted"], false);
    assert_eq!(state["muted_until"], Value::Null);

    // A server muted until turned back on mutes its channels, without an end.
    let server_path = format!("/api/servers/{server}/notification-settings");
    let (_, body) = set(&mut b, &server_path, json!({"muted": true})).await;
    assert_eq!(body["muted_until"], Value::Null);
    let state = row(&mut b, &channel).await;
    assert_eq!(state["muted"], true);
    assert_eq!(state["muted_until"], Value::Null);
    set(&mut b, &server_path, json!({"muted": false})).await;
    assert_eq!(row(&mut b, &channel).await["muted"], false);
}

#[sqlx::test]
async fn settings_are_validated_and_scoped_to_members(pool: PgPool) {
    let World {
        mut b,
        server,
        channel,
        ..
    } = setup(&pool).await;
    let channel_path = format!("/api/channels/{channel}/notification-settings");
    for body in [
        json!({"muted": true, "mute_minutes": 0}),
        json!({"muted": true, "mute_minutes": 600_000}),
        json!({"muted": false, "mute_minutes": 15}),
    ] {
        let (status, _) = set(&mut b, &channel_path, body.clone()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    let (status, _) = set(&mut b, &channel_path, json!({"level": "loud"})).await;
    assert!(status.is_client_error());

    let mut stranger = user(&pool, "c@test.example", "Cem").await;
    for path in [
        format!("/api/servers/{server}/notification-settings"),
        channel_path.clone(),
    ] {
        let (status, _) = set(&mut stranger, &path, json!({"level": "none"})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

/// How many settings name one of `ids`, for anybody.
async fn stored(pool: &PgPool, ids: &[&str]) -> i64 {
    let ids: Vec<uuid::Uuid> = ids.iter().map(|id| id.parse().unwrap()).collect();
    sqlx::query_scalar("SELECT count(*) FROM notification_settings WHERE scope_id = ANY($1)")
        .bind(ids)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A deleted server or channel takes its settings along. The triggers doing
/// that find the rows by scope alone, which has its own index (#294).
#[sqlx::test]
async fn deleting_a_server_or_channel_forgets_its_settings(pool: PgPool) {
    let indexed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes \
         WHERE tablename = 'notification_settings' \
           AND indexname = 'notification_settings_scope_idx')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(indexed);

    let World {
        mut a,
        mut b,
        server,
        channel,
    } = setup(&pool).await;
    let mut channels = vec![channel];
    for name in ["zwei", "drei"] {
        let r = a
            .send(
                Method::POST,
                &format!("/api/servers/{server}/channels"),
                Some(json!({ "name": name })),
            )
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
        channels.push(r.body["id"].as_str().unwrap().to_owned());
    }
    for client in [&mut a, &mut b] {
        let (status, body) = set(
            client,
            &format!("/api/servers/{server}/notification-settings"),
            json!({"level": "mentions"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for channel in &channels {
            let (status, body) = set(
                client,
                &format!("/api/channels/{channel}/notification-settings"),
                json!({"muted": true}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
    }
    // Another server of Ada's keeps its settings throughout.
    let other = a
        .send(Method::POST, "/api/servers", Some(json!({"name":"Andere"})))
        .await;
    assert_eq!(other.status, StatusCode::CREATED, "{}", other.body);
    let other_server = other.body["id"].as_str().unwrap().to_owned();
    let other_channel = other.body["channels"][0]["id"].as_str().unwrap().to_owned();
    for path in [
        format!("/api/servers/{other_server}/notification-settings"),
        format!("/api/channels/{other_channel}/notification-settings"),
    ] {
        let (status, body) = set(&mut a, &path, json!({"level": "none"})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let mut scopes: Vec<&str> = channels.iter().map(String::as_str).collect();
    scopes.push(&server);
    assert_eq!(stored(&pool, &scopes).await, 8);

    let gone = a
        .send(
            Method::DELETE,
            &format!("/api/channels/{}", channels[2]),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    assert_eq!(stored(&pool, &[channels[2].as_str()]).await, 0);
    assert_eq!(stored(&pool, &scopes).await, 6);

    // The server's own rows and, by the channel cascade, its channels'.
    let gone = a
        .send(Method::DELETE, &format!("/api/servers/{server}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    assert_eq!(stored(&pool, &scopes).await, 0);
    assert_eq!(
        stored(&pool, &[other_server.as_str(), other_channel.as_str()]).await,
        2
    );
    let listed = b
        .send(Method::GET, "/api/notification-settings", None)
        .await;
    assert_eq!(listed.body, json!([]));
}
