//! Server roles (v0.10, issue #516): the schema, the union of the base role
//! and custom roles, channels limited to roles, and the role routes.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use gelabber_api::servers::access;
use gelabber_api::servers::permissions::Permission;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

use common::{Client, SESSION};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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

/// A role row put in directly, for the schema tests and bulk set-ups.
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

async fn create_role(client: &mut Client, server: Uuid, body: Value) -> common::Response {
    client
        .send(
            Method::POST,
            &format!("/api/servers/{server}/roles"),
            Some(body),
        )
        .await
}

async fn set_roles(
    client: &mut Client,
    server: Uuid,
    user: Uuid,
    roles: Value,
) -> common::Response {
    client
        .send(
            Method::PUT,
            &format!("/api/servers/{server}/members/{user}/roles"),
            Some(json!({ "role_ids": roles })),
        )
        .await
}

fn field(res: &common::Response, name: &str) -> Value {
    assert_eq!(
        res.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{name}: {}",
        res.body
    );
    assert_eq!(res.body["error"], "validation_failed");
    res.body["fields"][name].clone()
}

#[sqlx::test]
async fn roles_are_created_edited_and_deleted(pool: PgPool) {
    let (mut owner, mut member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    assert_eq!(server["roles"], json!([]), "a new server has no roles");

    let res = create_role(
        &mut owner,
        sid,
        json!({
            "name": "  Moderation ",
            "color": "blue",
            "permissions": ["mention_everyone", "moderate_members", "manage_messages"]
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let moderation = res.body.clone();
    assert_eq!(moderation["name"], "Moderation");
    assert_eq!(moderation["server_id"], server["id"]);
    assert_eq!(moderation["color"], "blue");
    assert_eq!(
        moderation["permissions"],
        json!(["moderate_members", "manage_messages", "mention_everyone"]),
        "canonical order"
    );
    assert_eq!(moderation["position"], 0);
    assert!(moderation["created_at"].is_string());

    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Stammgäste", "color": "green" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let regulars = res.body.clone();
    assert_eq!(
        regulars["permissions"],
        json!([]),
        "permissions default to none"
    );
    assert_eq!(regulars["position"], 1);

    // Every member sees the roles, by position.
    let detail = member
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    assert_eq!(detail.body["roles"], json!([moderation, regulars]));

    // Rename, recolour, replace the flags.
    let mid = moderation["id"].as_str().unwrap();
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/roles/{mid}"),
            Some(json!({ "name": "Mods", "color": "violet", "permissions": ["manage_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "Mods");
    assert_eq!(res.body["color"], "violet");
    assert_eq!(res.body["permissions"], json!(["manage_messages"]));
    assert_eq!(res.body["position"], 0);

    // A role may change the case of its own name; an empty body changes nothing.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/roles/{mid}"),
            Some(json!({ "name": "MODS" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "MODS");
    let unchanged = owner
        .send(Method::PATCH, &format!("/api/roles/{mid}"), Some(json!({})))
        .await;
    assert_eq!(unchanged.status, StatusCode::OK);
    assert_eq!(unchanged.body, res.body);

    let res = owner
        .send(Method::DELETE, &format!("/api/roles/{mid}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let res = owner
        .send(Method::DELETE, &format!("/api/roles/{mid}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/roles/{mid}"),
            Some(json!({ "name": "Zurück" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    let detail = owner
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    assert_eq!(detail.body["roles"], json!([regulars]));

    // A new role goes after the last one.
    let res = create_role(&mut owner, sid, json!({ "name": "Neu", "color": "amber" })).await;
    assert_eq!(res.body["position"], 2);
}

#[sqlx::test]
async fn role_bodies_are_validated(pool: PgPool) {
    let (mut owner, _member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Moderation", "color": "blue" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let rid = res.body["id"].as_str().unwrap().to_owned();
    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Stammgäste", "color": "green" }),
    )
    .await;
    let other = res.body["id"].as_str().unwrap().to_owned();

    for (body, name, code) in [
        (json!({ "color": "blue" }), "name", "required"),
        (json!({ "name": "  ", "color": "blue" }), "name", "required"),
        (
            json!({ "name": "x".repeat(33), "color": "blue" }),
            "name",
            "too_long",
        ),
        (
            json!({ "name": "a\u{1}b", "color": "blue" }),
            "name",
            "invalid",
        ),
        (
            json!({ "name": "moderation", "color": "blue" }),
            "name",
            "taken",
        ),
        (
            json!({ "name": "Alle Mitglieder", "color": "blue" }),
            "name",
            "taken",
        ),
        (
            json!({ "name": "all members", "color": "blue" }),
            "name",
            "taken",
        ),
        (
            json!({ "name": "Inhaber", "color": "blue" }),
            "name",
            "taken",
        ),
        (json!({ "name": "OWNER", "color": "blue" }), "name", "taken"),
        (json!({ "name": "Neu" }), "color", "invalid"),
        (json!({ "name": "Neu", "color": "red" }), "color", "invalid"),
        (
            json!({ "name": "Neu", "color": "blue", "permissions": ["admin"] }),
            "permissions",
            "invalid",
        ),
    ] {
        let res = create_role(&mut owner, sid, body.clone()).await;
        assert_eq!(field(&res, name), code, "{body}");
    }
    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "x".repeat(32), "color": "coral" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "32 characters fit");

    for (body, name, code) in [
        (json!({ "name": "" }), "name", "required"),
        (json!({ "name": "STAMMGÄSTE" }), "name", "taken"),
        (json!({ "name": "Owner" }), "name", "taken"),
        (json!({ "color": "black" }), "color", "invalid"),
        (
            json!({ "permissions": ["send_messages", "nuke"] }),
            "permissions",
            "invalid",
        ),
    ] {
        let res = owner
            .send(
                Method::PATCH,
                &format!("/api/roles/{rid}"),
                Some(body.clone()),
            )
            .await;
        assert_eq!(field(&res, name), code, "{body}");
    }
    let res = owner
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    assert_eq!(
        res.body["roles"][0]["name"], "Moderation",
        "nothing changed"
    );
    assert_eq!(res.body["roles"][1]["id"], other.as_str());
}

#[sqlx::test]
async fn a_server_holds_at_most_fifty_roles(pool: PgPool) {
    let (mut owner, _member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    for n in 0..49 {
        insert_role(&pool, sid, &format!("Rolle {n}"), 0, n).await;
    }
    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Fünfzig", "color": "pink" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["position"], 49);
    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Zu viel", "color": "pink" }),
    )
    .await;
    assert_eq!(field(&res, "roles"), "too_many");
}

#[sqlx::test]
async fn role_routes_need_manage_server_and_membership(pool: PgPool) {
    let (mut owner, mut member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    let ada: Uuid = owner.user_id().parse().unwrap();
    let bob: Uuid = member.user_id().parse().unwrap();
    let mut outsider = user(&pool, "outsider@example.com", "Cara").await;
    let cara: Uuid = outsider.user_id().parse().unwrap();
    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Moderation", "color": "blue" }),
    )
    .await;
    let rid = res.body["id"].as_str().unwrap().to_owned();

    // A member without manage_server: 403 everywhere.
    let res = create_role(
        &mut member,
        sid,
        json!({ "name": "Meins", "color": "blue" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "forbidden");
    let res = member
        .send(
            Method::PATCH,
            &format!("/api/roles/{rid}"),
            Some(json!({ "permissions": ["manage_server"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = member
        .send(Method::DELETE, &format!("/api/roles/{rid}"), None)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = set_roles(&mut member, sid, bob, json!([rid])).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // Someone outside the server: 404, as for a missing id.
    let res = create_role(
        &mut outsider,
        sid,
        json!({ "name": "Fremd", "color": "blue" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = outsider
        .send(
            Method::PATCH,
            &format!("/api/roles/{rid}"),
            Some(json!({ "name": "X" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = outsider
        .send(Method::DELETE, &format!("/api/roles/{rid}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = set_roles(&mut outsider, sid, bob, json!([])).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = outsider
        .send(
            Method::PATCH,
            &format!("/api/roles/{}", Uuid::new_v4()),
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // The owner gets no roles; a non-member target is a 404.
    let res = set_roles(&mut owner, sid, ada, json!([rid])).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(
        res.body["message"],
        "The owner always has every permission."
    );
    let res = set_roles(&mut owner, sid, cara, json!([rid])).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // manage_server through a role is enough.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/roles/{rid}"),
            Some(json!({ "permissions": ["manage_server"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = set_roles(&mut owner, sid, bob, json!([rid])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = create_role(
        &mut member,
        sid,
        json!({ "name": "Meins", "color": "green" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

#[sqlx::test]
async fn member_roles_are_replaced_and_checked(pool: PgPool) {
    let (mut owner, mut member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    let bob: Uuid = member.user_id().parse().unwrap();
    let first = create_role(
        &mut owner,
        sid,
        json!({ "name": "Moderation", "color": "blue", "permissions": ["moderate_members"] }),
    )
    .await
    .body;
    let second = create_role(
        &mut owner,
        sid,
        json!({ "name": "Stammgäste", "color": "green" }),
    )
    .await
    .body;
    let (first, second) = (first["id"].clone(), second["id"].clone());

    // Another server's role is no role of this one.
    let res = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Anderer" })),
        )
        .await;
    let foreign_server = uuid(&res.body["id"]);
    let foreign = create_role(
        &mut owner,
        foreign_server,
        json!({ "name": "Fremd", "color": "pink" }),
    )
    .await
    .body;

    for roles in [
        json!([foreign["id"]]),
        json!([Uuid::new_v4()]),
        json!(["nope"]),
        json!([first, Uuid::new_v4()]),
    ] {
        let res = set_roles(&mut owner, sid, bob, roles.clone()).await;
        assert_eq!(field(&res, "role_ids"), "invalid", "{roles}");
    }
    let res = owner
        .send(
            Method::PUT,
            &format!("/api/servers/{sid}/members/{bob}/roles"),
            Some(json!({})),
        )
        .await;
    assert_eq!(field(&res, "role_ids"), "invalid");

    // Duplicates collapse; the answer is in position order.
    let res = set_roles(&mut owner, sid, bob, json!([second, first, second])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body,
        json!({ "user_id": bob, "role_ids": [first, second] })
    );

    let detail = member
        .send(Method::GET, &format!("/api/servers/{sid}"), None)
        .await;
    let members = detail.body["members"].as_array().unwrap();
    let row = |id: &str| members.iter().find(|m| m["user_id"] == id).unwrap().clone();
    assert_eq!(row(member.user_id())["role_ids"], json!([first, second]));
    assert_eq!(row(member.user_id())["role"], "member");
    assert_eq!(row(owner.user_id())["role_ids"], json!([]));
    assert_eq!(row(owner.user_id())["role"], "owner");
    assert_eq!(
        detail.body["permissions"],
        json!([
            "moderate_members",
            "send_messages",
            "send_files",
            "join_voice",
            "go_live"
        ])
    );

    // moderate_members through a role may kick (a third member).
    let mut cara = user(&pool, "cara@example.com", "Cara").await;
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let res = cara
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = member
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/kick"),
            Some(json!({ "user_id": cara.user_id() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);

    // An empty list takes every role away.
    let res = set_roles(&mut owner, sid, bob, json!([])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["role_ids"], json!([]));
    let mine = member.send(Method::GET, "/api/servers", None).await;
    assert_eq!(
        mine.body[0]["permissions"],
        json!(["send_messages", "send_files", "join_voice", "go_live"])
    );
}

#[sqlx::test]
async fn holders_are_the_owner_and_members_granted_the_flag(pool: PgPool) {
    let (mut owner, member, server) = owner_and_member(&pool).await;
    let sid = uuid(&server["id"]);
    let ada: Uuid = owner.user_id().parse().unwrap();
    let bob: Uuid = member.user_id().parse().unwrap();

    let held = access::holders(&pool, sid, Permission::ModerateMembers)
        .await
        .unwrap();
    assert_eq!(held, vec![ada], "the owner holds everything");
    let held = access::holders(&pool, sid, Permission::SendMessages)
        .await
        .unwrap();
    assert_eq!(held, vec![ada, bob], "through the base role");

    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Moderation", "color": "blue", "permissions": ["moderate_members"] }),
    )
    .await;
    let rid = res.body["id"].clone();
    let res = set_roles(&mut owner, sid, bob, json!([rid])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let held = access::holders(&pool, sid, Permission::ModerateMembers)
        .await
        .unwrap();
    assert_eq!(held, vec![ada, bob], "through a custom role");
    let held = access::holders(&pool, sid, Permission::ManageServer)
        .await
        .unwrap();
    assert_eq!(held, vec![ada]);
}

async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    req.headers_mut().insert(
        "cookie",
        format!("{SESSION}={}", client.jar[SESSION])
            .parse()
            .unwrap(),
    );
    tokio_tungstenite::connect_async(req)
        .await
        .expect("connect")
        .0
}

async fn send(ws: &mut Ws, frame: Value) {
    ws.send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
}

/// The next `{"k": "a"}` server event, answering heartbeats on the way.
async fn access_changed(ws: &mut Ws, server: Uuid) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await.expect("open socket").expect("frame") {
                Message::Text(raw) => {
                    let frame: Value = serde_json::from_str(&raw).unwrap();
                    if frame["op"] == "h" {
                        send(ws, json!({ "op": "h" })).await;
                    }
                    if frame["op"] == "e" && frame["d"]["k"] == "a" {
                        assert_eq!(frame["t"], "e", "{frame}");
                        assert_eq!(frame["s"], server.to_string(), "{frame}");
                        assert!(frame.get("c").is_none_or(Value::is_null), "{frame}");
                        return frame;
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    })
    .await
    .expect("access change event")
}

#[sqlx::test]
async fn role_and_assignment_changes_announce_an_access_change(pool: PgPool) {
    let (addr, state) = common::serve_ws(pool).await;
    let mut owner = Client::with_state(state.clone());
    owner.bootstrap().await;
    let res = owner
        .register("owner@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    let mut member = Client::with_state(state);
    member.bootstrap().await;
    let res = member
        .register("member@example.com", "password123", "Bob")
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    let res = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    let sid = uuid(&res.body["id"]);
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{sid}/invites"),
            Some(json!({})),
        )
        .await;
    let code = invite.body["code"].as_str().unwrap();
    let res = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let bob: Uuid = member.user_id().parse().unwrap();

    let mut ws = connect(addr, &member).await;
    send(&mut ws, json!({ "op": "s", "s": sid })).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Message::Text(raw) = ws.next().await.unwrap().unwrap() {
                let frame: Value = serde_json::from_str(&raw).unwrap();
                if frame["op"] == "ok" {
                    break;
                }
            }
        }
    })
    .await
    .expect("subscribed");

    let res = create_role(
        &mut owner,
        sid,
        json!({ "name": "Moderation", "color": "blue" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED);
    access_changed(&mut ws, sid).await;
    let rid = res.body["id"].as_str().unwrap().to_owned();

    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/roles/{rid}"),
            Some(json!({ "color": "pink" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    access_changed(&mut ws, sid).await;

    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/roles/{rid}"),
            Some(json!({ "permissions": ["manage_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    access_changed(&mut ws, sid).await;

    let res = set_roles(&mut owner, sid, bob, json!([rid])).await;
    assert_eq!(res.status, StatusCode::OK);
    access_changed(&mut ws, sid).await;

    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{sid}"),
            Some(json!({ "member_permissions": ["send_messages"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    access_changed(&mut ws, sid).await;

    let res = owner
        .send(Method::DELETE, &format!("/api/roles/{rid}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    access_changed(&mut ws, sid).await;
}
