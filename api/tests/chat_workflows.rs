mod common;

use axum::http::{Method, StatusCode};
use common::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn setup(pool: PgPool) -> (Client, Client, String, String) {
    let mut a = Client::new(pool.clone());
    a.bootstrap().await;
    assert_eq!(
        a.register("a@test.example", "password123", "Ada")
            .await
            .status,
        StatusCode::CREATED
    );
    let mut b = Client::new(pool);
    b.bootstrap().await;
    assert_eq!(
        b.register("b@test.example", "password123", "Bob")
            .await
            .status,
        StatusCode::CREATED
    );
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
    let joined = b
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
    (a, b, sid, cid)
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
async fn read(client: &mut Client, channel: &str, message: &Value) -> Value {
    let r = client
        .send(
            Method::PUT,
            &format!("/api/channels/{channel}/read"),
            Some(json!({"message_id":message["id"]})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}
async fn summary(client: &mut Client, channel: &str) -> Value {
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

#[sqlx::test]
async fn cursors_are_monotonic_cross_device_and_preserve_later_messages(pool: PgPool) {
    let (mut a, mut b, _, channel) = setup(pool.clone()).await;
    let first = post(&mut a, &channel, "first").await;
    post(&mut b, &channel, "mine").await;
    let second = post(&mut a, &channel, "second").await;
    assert_eq!(summary(&mut b, &channel).await["unread_count"], 2);
    let through_first = read(&mut b, &channel, &first).await;
    assert_eq!(through_first["unread_count"], 1);
    // Another device shares the account, not the frontend state.
    let mut device = Client::new(pool);
    device.jar = b.jar.clone();
    device.csrf = b.csrf.clone();
    device.user_id = b.user_id.clone();
    assert_eq!(
        read(&mut device, &channel, &second).await["unread_count"],
        0
    );
    let third = post(&mut a, &channel, "arrived later").await;
    let stale = read(&mut b, &channel, &first).await;
    assert_eq!(stale["read_message_id"], second["id"]);
    assert_eq!(stale["unread_count"], 1);
    // A deleted boundary must not erase the read floor or include old history again.
    assert_eq!(
        a.send(
            Method::DELETE,
            &format!("/api/messages/{}", second["id"].as_str().unwrap()),
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(summary(&mut b, &channel).await["unread_count"], 1);
    read(&mut b, &channel, &third).await;
    assert_eq!(summary(&mut device, &channel).await["unread_count"], 0);
}

#[sqlx::test]
async fn concurrent_devices_cannot_move_a_read_cursor_backwards(pool: PgPool) {
    let (mut a, mut b, _, channel) = setup(pool.clone()).await;
    let first = post(&mut a, &channel, "one").await;
    let second = post(&mut a, &channel, "two").await;
    let mut other = Client::new(pool);
    other.jar = b.jar.clone();
    other.csrf = b.csrf.clone();
    other.user_id = b.user_id.clone();
    let (_, _) = tokio::join!(
        read(&mut b, &channel, &second),
        read(&mut other, &channel, &first)
    );
    let state = summary(&mut b, &channel).await;
    assert_eq!(state["read_message_id"], second["id"]);
    assert_eq!(state["unread_count"], 0);
}

#[sqlx::test]
async fn workflows_are_private_and_rejoin_starts_a_new_scope(pool: PgPool) {
    let (mut a, mut b, sid, channel) = setup(pool.clone()).await;
    let message = post(&mut a, &channel, "secret word").await;
    let private = a
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({"name":"Private"})),
        )
        .await;
    let private_channel = private.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let other = post(&mut a, private_channel, "secret word").await;
    for path in [
        format!("/api/channels/{private_channel}/messages/search?q=secret"),
        format!("/api/channels/{sid}/messages/search?q=secret"),
    ] {
        assert_eq!(
            b.send(Method::GET, &path, None).await.status,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        b.send(
            Method::PUT,
            &format!("/api/channels/{channel}/read"),
            Some(json!({"message_id":other["id"]}))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    read(&mut b, &channel, &message).await;
    let user: Uuid = b.user_id().parse().unwrap();
    let server: Uuid = sid.parse().unwrap();
    sqlx::query("DELETE FROM server_members WHERE server_id=$1 AND user_id=$2")
        .bind(server)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        b.send(
            Method::PUT,
            &format!("/api/channels/{channel}/read"),
            Some(json!({"message_id":message["id"]}))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let states = b.send(Method::GET, "/api/messages/unread", None).await;
    assert!(
        !states
            .body
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["channel_id"] == channel)
    );
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,joined_at) VALUES($1,$2,clock_timestamp())",
    )
    .bind(server)
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let reset = summary(&mut b, &channel).await;
    assert!(reset["read_message_id"].is_null());
    assert_eq!(reset["unread_count"], 0);
    post(&mut a, &channel, "after rejoin").await;
    assert_eq!(summary(&mut b, &channel).await["unread_count"], 1);
}

#[sqlx::test]
async fn postgres_search_tracks_edits_deletes_and_pages_stably(pool: PgPool) {
    let (mut a, mut b, sid, channel) = setup(pool.clone()).await;
    let first = post(&mut a, &channel, "red fox").await;
    let second = post(&mut a, &channel, "red bird").await;
    post(&mut a, &channel, "a red fox jumps").await;
    let route = format!("/api/channels/{channel}/messages/search?q=red&limit=1");
    let page = b.send(Method::GET, &route, None).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert_eq!(page.body["has_more"], true);
    let row = &page.body["messages"][0];
    let before = format!(
        "{}|{}",
        row["created_at"].as_str().unwrap(),
        row["id"].as_str().unwrap()
    );
    let older = b
        .send(Method::GET, &format!("{route}&before={before}"), None)
        .await;
    assert_eq!(older.body["messages"][0]["id"], second["id"]);
    a.send(
        Method::PATCH,
        &format!("/api/messages/{}", first["id"].as_str().unwrap()),
        Some(json!({"content":"blue fox"})),
    )
    .await;
    a.send(
        Method::DELETE,
        &format!("/api/messages/{}", second["id"].as_str().unwrap()),
        None,
    )
    .await;
    // Read/search rights remain even when posting is revoked.
    assert_eq!(
        a.send(
            Method::PATCH,
            &format!("/api/servers/{sid}"),
            Some(json!({"member_permissions":[]}))
        )
        .await
        .status,
        StatusCode::OK
    );
    let result = b
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/search?q=red"),
            None,
        )
        .await;
    assert_eq!(result.status, StatusCode::OK, "{}", result.body);
    assert_eq!(result.body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        b.send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/search?q=%22blue%20fox%22"),
            None
        )
        .await
        .body["messages"][0]["id"],
        first["id"]
    );
    assert_eq!(
        b.send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/search?q=%20"),
            None
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test]
async fn dm_unread_and_search_are_participant_scoped(pool: PgPool) {
    let (mut a, mut b, _, _) = setup(pool.clone()).await;
    let dm = a
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({"user_id":b.user_id()})),
        )
        .await;
    assert_eq!(dm.status, StatusCode::CREATED, "{}", dm.body);
    let channel = dm.body["id"].as_str().unwrap();
    let message = post(&mut a, channel, "private fox").await;
    assert_eq!(summary(&mut a, channel).await["unread_count"], 0);
    assert_eq!(summary(&mut b, channel).await["unread_count"], 1);
    assert!(summary(&mut b, channel).await["server_id"].is_null());
    assert_eq!(
        b.send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/search?q=fox"),
            None
        )
        .await
        .body["messages"][0]["id"],
        message["id"]
    );
    read(&mut b, channel, &message).await;
    let mut foreign = Client::new(pool);
    foreign.bootstrap().await;
    foreign
        .register("c@test.example", "password123", "Cara")
        .await;
    assert_eq!(
        foreign
            .send(
                Method::GET,
                &format!("/api/channels/{channel}/messages/search?q=fox"),
                None
            )
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        foreign
            .send(
                Method::PUT,
                &format!("/api/channels/{channel}/read"),
                Some(json!({"message_id":message["id"]}))
            )
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn a_transaction_started_before_read_still_creates_an_unread_later_message(pool: PgPool) {
    let (mut a, mut b, _, channel) = setup(pool.clone()).await;
    let mut delayed = pool.begin().await.unwrap();
    // Freeze the older transaction's now() before the later message is read.
    let started_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut *delayed)
        .await
        .unwrap();
    let first = post(&mut a, &channel, "already visible").await;
    read(&mut b, &channel, &first).await;
    let cid: Uuid = channel.parse().unwrap();
    let uid: Uuid = a.user_id().parse().unwrap();
    gelabber_api::gateway::delivery::lock_channel(&mut delayed, cid)
        .await
        .unwrap();
    let (id,at,order):(Uuid,chrono::DateTime<chrono::Utc>,i64)=sqlx::query_as("INSERT INTO messages(channel_id,author_id,content) VALUES($1,$2,'delayed transaction') RETURNING id,created_at,created_order").bind(cid).bind(uid).fetch_one(&mut *delayed).await.unwrap();
    delayed.commit().await.unwrap();
    let original: chrono::DateTime<chrono::Utc> =
        first["created_at"].as_str().unwrap().parse().unwrap();
    assert!(started_at < original);
    assert!(
        at > original,
        "INSERT time, rather than transaction start, preserves catch-up"
    );
    let catchup = b
        .send(
            Method::GET,
            &format!(
                "/api/channels/{channel}/messages?after={}",
                first["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(catchup.status, StatusCode::OK, "{}", catchup.body);
    assert!(
        catchup.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["id"] == id.to_string())
    );
    assert!(order > first["created_order"].as_i64().unwrap());
    assert_eq!(summary(&mut b, &channel).await["unread_count"], 1);
    read(&mut b, &channel, &json!({"id":id})).await;
    let edited = a
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", first["id"].as_str().unwrap()),
            Some(json!({"content":"edited old message"})),
        )
        .await;
    assert_eq!(edited.body["created_order"], first["created_order"]);
    assert_eq!(summary(&mut b, &channel).await["unread_count"], 0);
}

#[sqlx::test(migrations = false)]
async fn migration_backfills_creation_order_and_continues_without_duplicates(pool: PgPool) {
    // A real pre-feature database with deliberately reversed physical row order.
    for source in [
        include_str!("../migrations/0001_users_sessions.sql"),
        include_str!("../migrations/0002_servers_channels_invites.sql"),
        include_str!("../migrations/0003_messages.sql"),
        include_str!("../migrations/0004_direct_messages.sql"),
        include_str!("../migrations/0005_moderation.sql"),
        include_str!("../migrations/0006_attachments.sql"),
        include_str!("../migrations/0007_realtime_delivery.sql"),
        include_str!("../migrations/0008_storage_lifecycle.sql"),
        include_str!("../migrations/0009_account_themes.sql"),
    ] {
        sqlx::raw_sql(source).execute(&pool).await.unwrap();
    }
    let user:Uuid=sqlx::query_scalar("INSERT INTO users(email,name,password_hash) VALUES('upgrade@example.test','Upgrade','unused') RETURNING id").fetch_one(&pool).await.unwrap();
    let server: Uuid =
        sqlx::query_scalar("INSERT INTO servers(name,owner_id) VALUES('Upgrade',$1) RETURNING id")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
    let channel: Uuid = sqlx::query_scalar(
        "INSERT INTO channels(server_id,name,kind) VALUES($1,'Upgrade','text') RETURNING id",
    )
    .bind(server)
    .fetch_one(&pool)
    .await
    .unwrap();
    let recent:Uuid=sqlx::query_scalar("INSERT INTO messages(channel_id,author_id,content,created_at) VALUES($1,$2,'newer','2026-10-05T01:00:00Z') RETURNING id").bind(channel).bind(user).fetch_one(&pool).await.unwrap();
    let older:Uuid=sqlx::query_scalar("INSERT INTO messages(channel_id,author_id,content,created_at) VALUES($1,$2,'older','2026-10-05T00:00:00Z') RETURNING id").bind(channel).bind(user).fetch_one(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/0010_chat_read_state.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let ordering: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT id,created_order FROM messages ORDER BY created_order")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(ordering, vec![(older, 1), (recent, 2)]);
    let next:i64=sqlx::query_scalar("INSERT INTO messages(channel_id,author_id,content) VALUES($1,$2,'after upgrade') RETURNING created_order").bind(channel).bind(user).fetch_one(&pool).await.unwrap();
    assert_eq!(next, 3);
}

#[sqlx::test]
async fn equal_timestamp_and_lower_uuid_cannot_hide_a_later_unread_insert(pool: PgPool) {
    let (mut a, mut b, _, channel) = setup(pool.clone()).await;
    let first = post(&mut a, &channel, "visible boundary").await;
    read(&mut b, &channel, &first).await;
    let at: chrono::DateTime<chrono::Utc> = first["created_at"].as_str().unwrap().parse().unwrap();
    let channel_id: Uuid = channel.parse().unwrap();
    let user_id: Uuid = a.user_id().parse().unwrap();
    let mut tx = pool.begin().await.unwrap();
    gelabber_api::gateway::delivery::lock_channel(&mut tx, channel_id)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES($1,$2,$3,'same timestamp',$4)")
        .bind(Uuid::nil()).bind(channel_id).bind(user_id).bind(at).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(summary(&mut b, &channel).await["unread_count"], 1);
    let latest = read(&mut b, &channel, &json!({"id":Uuid::nil()})).await;
    assert_eq!(latest["unread_count"], 0);
    let stale = read(&mut b, &channel, &first).await;
    assert_eq!(stale["read_message_id"], Uuid::nil().to_string());
}

#[sqlx::test]
async fn search_context_is_bounded_scoped_and_keeps_the_read_cursor(pool: PgPool) {
    let (mut a, mut b, _, channel) = setup(pool.clone()).await;
    let channel_uuid = Uuid::parse_str(&channel).unwrap();
    let author_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email='a@test.example'")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Deep history is independent of the current latest page and HTTP paging cost.
    let ids: Vec<Uuid> = sqlx::query_scalar("INSERT INTO messages(channel_id,author_id,content,created_at)
        SELECT $1,$2,'context-'||n,clock_timestamp()+n*interval '1 second' FROM generate_series(1,150) n
        RETURNING id")
        .bind(channel_uuid).bind(author_id).fetch_all(&pool).await.unwrap();
    let target = ids[69];
    let before = summary(&mut b, &channel).await;
    let path = format!("/api/channels/{channel}/messages/{target}/context");
    let context = b.send(Method::GET, &path, None).await;
    assert_eq!(context.status, StatusCode::OK, "{}", context.body);
    let messages = context.body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 61);
    assert_eq!(context.body["target_id"], target.to_string());
    assert_eq!(messages[30]["id"], target.to_string());
    assert_eq!(messages[0]["content"], "context-40");
    assert_eq!(messages[60]["content"], "context-100");
    assert!(context.body["before"].as_str().unwrap().contains('|'));
    assert!(context.body["after"].as_str().unwrap().contains('|'));
    assert_eq!(summary(&mut b, &channel).await, before);
    // The target sits outside the standard latest page but its snapshot updates.
    let edited = a
        .send(
            Method::PATCH,
            &format!("/api/messages/{target}"),
            Some(json!({"content":"edited context"})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    let updated = b.send(Method::GET, &path, None).await;
    assert_eq!(updated.body["messages"][30]["content"], "edited context");
    assert_eq!(
        a.send(Method::DELETE, &format!("/api/messages/{target}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        b.send(Method::GET, &path, None).await.status,
        StatusCode::NOT_FOUND
    );
    // Deleted target IDs in another channel never reveal neighbouring content.
    assert_eq!(
        b.send(
            Method::GET,
            &format!("/api/channels/{}/messages/{target}/context", Uuid::new_v4()),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn message_context_requires_channel_or_dm_membership(pool: PgPool) {
    let (mut a, mut b, _, channel) = setup(pool.clone()).await;
    let message = post(&mut a, &channel, "private context").await;
    let target = message["id"].as_str().unwrap();
    let mut outsider = Client::new(pool);
    outsider.bootstrap().await;
    assert_eq!(
        outsider
            .register("outsider@test.example", "password123", "Outsider")
            .await
            .status,
        StatusCode::CREATED
    );
    let path = format!("/api/channels/{channel}/messages/{target}/context");
    assert_eq!(
        outsider.send(Method::GET, &path, None).await.status,
        StatusCode::NOT_FOUND
    );
    let peer = b.send(Method::GET, "/api/auth/session", None).await.body["user"]["id"].clone();
    let malformed = a
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages/not-a-uuid/context"),
            None,
        )
        .await;
    assert_eq!(malformed.status, StatusCode::NOT_FOUND);
    assert_eq!(malformed.body["error"], "not_found");
    let dm = a
        .send(Method::POST, "/api/dms", Some(json!({"user_id":peer})))
        .await;
    assert_eq!(dm.status, StatusCode::CREATED);
    let dm_id = dm.body["id"].as_str().unwrap();
    let dm_message = post(&mut a, dm_id, "private DM context").await;
    let dm_path = format!(
        "/api/channels/{dm_id}/messages/{}/context",
        dm_message["id"].as_str().unwrap()
    );
    assert_eq!(
        b.send(Method::GET, &dm_path, None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        outsider.send(Method::GET, &dm_path, None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        b.send(
            Method::GET,
            &format!(
                "/api/channels/{channel}/messages/{}/context",
                dm_message["id"].as_str().unwrap()
            ),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}
