//! Server roles (v0.10, issue #516): the schema, the union of the base role
//! and custom roles, and channels limited to roles.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

async fn user(pool: &PgPool, email: &str, name: &str) -> Client {
    let mut client = Client::new(pool.clone());
    client.bootstrap().await;
    let res = client.register(email, "password123", name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

fn uuid(value: &Value) -> Uuid {
    value.as_str().unwrap().parse().unwrap()
}

/// Ada owns a server, Bob joined it through an invite.
async fn owner_and_member(pool: &PgPool) -> (Client, Client, Value) {
    let mut owner = user(pool, "owner@example.com", "Ada").await;
    let mut member = user(pool, "member@example.com", "Bob").await;
    let res = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = res.body;
    let id = server["id"].as_str().unwrap();
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let code = invite.body["code"].as_str().unwrap();
    let res = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    (owner, member, server)
}

/// Until the role routes exist, rows go in directly.
async fn insert_role(
    pool: &PgPool,
    server: Uuid,
    name: &str,
    permissions: i32,
    position: i32,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO server_roles (server_id, name, color, permissions, position) \
         VALUES ($1, $2, 'blue', $3, $4) RETURNING id",
    )
    .bind(server)
    .bind(name)
    .bind(permissions)
    .bind(position)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn assign(pool: &PgPool, server: Uuid, user: Uuid, role: Uuid) {
    sqlx::query(
        "INSERT INTO server_member_roles (server_id, user_id, role_id) VALUES ($1, $2, $3)",
    )
    .bind(server)
    .bind(user)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

async fn limit(pool: &PgPool, server: Uuid, channel: Uuid, roles: &[Uuid]) {
    sqlx::query("UPDATE channels SET limited = true WHERE id = $1")
        .bind(channel)
        .execute(pool)
        .await
        .unwrap();
    for role in roles {
        sqlx::query(
            "INSERT INTO channel_roles (server_id, channel_id, role_id) VALUES ($1, $2, $3)",
        )
        .bind(server)
        .bind(channel)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn visible(pool: &PgPool, channel: Uuid, user: Uuid) -> bool {
    sqlx::query_scalar("SELECT channel_visible_to($1, $2)")
        .bind(channel)
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn member_bits(pool: &PgPool, server: Uuid, user: Uuid) -> i32 {
    sqlx::query_scalar("SELECT server_member_bits($1, $2)")
        .bind(server)
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn a_member_holds_the_union_of_the_base_role_and_their_roles(pool: PgPool) {
    let (_owner, mut member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    let bob: Uuid = member.user_id().parse().unwrap();

    let mine = member.send(Method::GET, "/api/servers", None).await;
    assert_eq!(
        mine.body[0]["permissions"],
        json!(["send_messages", "send_files", "join_voice", "go_live"])
    );

    // Bit 256 is moderate_members, 64 manage_messages.
    let moderation = insert_role(&pool, sid, "Moderation", 256 | 64, 1).await;
    let regulars = insert_role(&pool, sid, "Stammgäste", 0, 0).await;
    assign(&pool, sid, bob, moderation).await;
    assign(&pool, sid, bob, regulars).await;
    assert_eq!(member_bits(&pool, sid, bob).await, 60 | 64 | 256);

    let union = json!([
        "moderate_members",
        "manage_messages",
        "send_messages",
        "send_files",
        "join_voice",
        "go_live"
    ]);
    let mine = member.send(Method::GET, "/api/servers", None).await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.body);
    assert_eq!(mine.body[0]["permissions"], union);
    assert_eq!(
        mine.body[0]["member_permissions"],
        json!(["send_messages", "send_files", "join_voice", "go_live"]),
        "the base role is unchanged"
    );
    assert_eq!(mine.body[0]["role"], "member");

    let detail = member
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    assert_eq!(detail.body["permissions"], union);

    // Deleting a role takes its rights with it.
    sqlx::query("DELETE FROM server_roles WHERE id = $1")
        .bind(moderation)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(member_bits(&pool, sid, bob).await, 60);
    let mine = member.send(Method::GET, "/api/servers", None).await;
    assert_eq!(
        mine.body[0]["permissions"],
        json!(["send_messages", "send_files", "join_voice", "go_live"])
    );
}

#[sqlx::test]
async fn channels_carry_their_access_and_the_visibility_rule_holds(pool: PgPool) {
    let (mut owner, member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    let ada: Uuid = owner.user_id().parse().unwrap();
    let bob: Uuid = member.user_id().parse().unwrap();
    let channel = server["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap();
    assert_eq!(channel["limited"], false);
    assert_eq!(channel["role_ids"], json!([]));
    let cid = uuid(&channel["id"]);

    assert!(visible(&pool, cid, bob).await, "not limited: everyone");
    let role = insert_role(&pool, sid, "Stammgäste", 0, 0).await;
    limit(&pool, sid, cid, &[role]).await;
    assert!(visible(&pool, cid, ada).await, "the owner sees everything");
    assert!(!visible(&pool, cid, bob).await);
    assert_eq!(
        member_bits(&pool, sid, bob).await,
        60,
        "visibility is no permission"
    );

    let detail = owner
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    let limited = detail.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == channel["id"])
        .unwrap()
        .clone();
    assert_eq!(limited["limited"], true);
    assert_eq!(limited["role_ids"], json!([role]));

    assign(&pool, sid, bob, role).await;
    assert!(visible(&pool, cid, bob).await, "holds one of its roles");

    // The last role goes: the channel stays limited, the owner's alone.
    sqlx::query("DELETE FROM server_roles WHERE id = $1")
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!visible(&pool, cid, bob).await);
    assert!(visible(&pool, cid, ada).await);
    let detail = owner
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    let limited = detail.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == channel["id"])
        .unwrap()
        .clone();
    assert_eq!(limited["limited"], true);
    assert_eq!(limited["role_ids"], json!([]));
}

#[sqlx::test]
async fn leaving_drops_the_role_assignments(pool: PgPool) {
    let (_owner, mut member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    let bob: Uuid = member.user_id().parse().unwrap();
    let role = insert_role(&pool, sid, "Stammgäste", 0, 0).await;
    assign(&pool, sid, bob, role).await;

    let res = member
        .send(Method::POST, &format!("/api/servers/{sid}/leave"), None)
        .await;
    assert!(res.status.is_success(), "{} {}", res.status, res.body);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM server_member_roles")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrations = false)]
async fn the_migration_moves_kick_and_ban_to_moderate_members(pool: PgPool) {
    let migrator = sqlx::migrate!("./migrations");
    for migration in migrator.iter().filter(|m| m.version < 31) {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap();
    }
    let owner: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, name, password_hash) \
         VALUES ('upgrade@example.test', 'Upgrade', 'unused') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut servers = Vec::new();
    for bits in [1 | 4, 60] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO servers (name, owner_id, member_permissions) \
             VALUES ('Upgrade', $1, $2) RETURNING id",
        )
        .bind(owner)
        .bind(bits)
        .fetch_one(&pool)
        .await
        .unwrap();
        servers.push(id);
    }
    for migration in migrator.iter().filter(|m| m.version >= 31) {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap();
    }
    let bits: Vec<i32> = sqlx::query_scalar(
        "SELECT member_permissions FROM servers WHERE id = ANY($1) ORDER BY member_permissions",
    )
    .bind(&servers)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(bits, vec![60, 1 | 4 | 256]);
}
