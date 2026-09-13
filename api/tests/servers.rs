//! Servers, categories, channels, invites and the coarse permission model
//! (issue #4), end to end against a real Postgres.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

/// Two browsers on one database: the owner and a second account.
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

async fn create_invite(client: &mut Client, server_id: &str, body: Value) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/invites"),
            Some(body),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

/// Owner creates, member joins through a link. Returns the server detail as
/// the owner sees it.
async fn server_with_member(owner: &mut Client, member: &mut Client) -> Value {
    let server = create_server(owner, "Team").await;
    let id = server["id"].as_str().unwrap();
    let invite = create_invite(owner, id, json!({})).await;
    let res = member
        .send(
            Method::POST,
            &format!("/api/invites/{}/join", invite["code"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    server
}

fn id_of(value: &Value) -> String {
    value["id"].as_str().expect("id").to_owned()
}

#[sqlx::test]
async fn server_routes_require_a_session(pool: PgPool) {
    let mut client = Client::new(pool);
    client.bootstrap().await;

    let res = client.send(Method::GET, "/api/servers", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"], "unauthenticated");

    let res = client
        .send(Method::POST, "/api/servers", Some(json!({ "name": "X" })))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let res = client
        .send(Method::GET, "/api/invites/abcdefghjk", None)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn creating_a_server_makes_the_caller_owner_with_a_first_channel(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;

    let empty = owner.send(Method::GET, "/api/servers", None).await;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(empty.body, json!([]));

    let server = create_server(&mut owner, "  Team Ada  ").await;
    assert_eq!(server["name"], "Team Ada");
    assert_eq!(server["role"], "owner");
    assert_eq!(server["owner_id"], owner_id(&mut owner).await);
    let permissions = server["permissions"].as_array().unwrap();
    assert_eq!(
        permissions.len(),
        6,
        "owner holds every flag: {permissions:?}"
    );
    assert_eq!(
        server["member_permissions"],
        json!(["send_messages", "send_files", "join_voice", "go_live"]),
        "default member mask"
    );
    assert_eq!(server["categories"].as_array().unwrap().len(), 1);
    assert_eq!(server["categories"][0]["name"], "Textkanäle");
    let channels = server["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0]["name"], "allgemein");
    assert_eq!(channels[0]["kind"], "text");
    assert_eq!(channels[0]["category_id"], server["categories"][0]["id"]);
    let members = server["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["name"], "Ada");
    assert_eq!(members[0]["role"], "owner");

    let list = owner.send(Method::GET, "/api/servers", None).await;
    assert_eq!(list.body.as_array().unwrap().len(), 1);
    assert_eq!(list.body[0]["id"], server["id"]);
    assert!(list.body[0].get("channels").is_none(), "list stays lean");

    let detail = owner
        .send(
            Method::GET,
            &format!("/api/servers/{}", id_of(&server)),
            None,
        )
        .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.body, server);
}

async fn owner_id(client: &mut Client) -> Value {
    client.send(Method::GET, "/api/me", None).await.body["id"].clone()
}

#[sqlx::test]
async fn server_name_is_validated(pool: PgPool) {
    let (mut owner, _) = two_users(pool).await;

    let res = owner
        .send(Method::POST, "/api/servers", Some(json!({ "name": "   " })))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["name"], "required");

    let res = owner
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "x".repeat(101) })),
        )
        .await;
    assert_eq!(res.body["fields"]["name"], "too_long");

    let res = owner
        .send(Method::POST, "/api/servers", Some(json!({})))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["name"], "required");
}

#[sqlx::test]
async fn foreign_servers_are_invisible(pool: PgPool) {
    let (mut owner, mut stranger) = two_users(pool).await;
    let server = create_server(&mut owner, "Private").await;
    let id = id_of(&server);

    let list = stranger.send(Method::GET, "/api/servers", None).await;
    assert_eq!(list.body, json!([]));

    // Every read and write answers 404, never 403: the id is not confirmed.
    for (method, path, body) in [
        (Method::GET, format!("/api/servers/{id}"), None),
        (
            Method::PATCH,
            format!("/api/servers/{id}"),
            Some(json!({ "name": "Hijacked" })),
        ),
        (Method::DELETE, format!("/api/servers/{id}"), None),
        (
            Method::POST,
            format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "x" })),
        ),
        (
            Method::POST,
            format!("/api/servers/{id}/categories"),
            Some(json!({ "name": "x" })),
        ),
        (Method::GET, format!("/api/servers/{id}/invites"), None),
        (
            Method::POST,
            format!("/api/servers/{id}/invites"),
            Some(json!({})),
        ),
        (Method::POST, format!("/api/servers/{id}/leave"), None),
    ] {
        let res = stranger.send(method.clone(), &path, body).await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            res.body
        );
        assert_eq!(res.body["error"], "not_found");
    }

    let channel_id = server["channels"][0]["id"].as_str().unwrap();
    let res = stranger
        .send(
            Method::PATCH,
            &format!("/api/channels/{channel_id}"),
            Some(json!({ "name": "pwned" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = stranger
        .send(Method::DELETE, &format!("/api/channels/{channel_id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // Malformed ids are "not found" too, as JSON, never a plain-text 400.
    let res = stranger
        .send(Method::GET, "/api/servers/not-a-uuid", None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"], "not_found");

    // Nothing changed.
    let detail = owner
        .send(Method::GET, &format!("/api/servers/{id}"), None)
        .await;
    assert_eq!(detail.body["name"], "Private");
    assert_eq!(detail.body["channels"][0]["name"], "allgemein");
}

#[sqlx::test]
async fn invite_link_brings_a_signed_in_user_into_the_server(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = create_server(&mut owner, "Team").await;
    let id = id_of(&server);

    let invite = create_invite(&mut owner, &id, json!({})).await;
    let code = invite["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 10);
    assert_eq!(invite["server_id"], server["id"]);
    assert_eq!(invite["uses"], 0);
    assert_eq!(invite["max_uses"], Value::Null);
    assert_eq!(invite["expires_at"], Value::Null);

    // Preview: name and size only, plus "am I already in".
    let preview = member
        .send(Method::GET, &format!("/api/invites/{code}"), None)
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.body);
    assert_eq!(preview.body["server"]["name"], "Team");
    assert_eq!(preview.body["server"]["member_count"], 1);
    assert_eq!(preview.body["member"], false);
    assert!(preview.body["server"].get("channels").is_none());

    let joined = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    assert_eq!(joined.body["id"], server["id"]);
    assert_eq!(joined.body["role"], "member");
    assert_eq!(
        joined.body["permissions"],
        json!(["send_messages", "send_files", "join_voice", "go_live"])
    );

    let list = member.send(Method::GET, "/api/servers", None).await;
    assert_eq!(list.body.as_array().unwrap().len(), 1);

    let detail = member
        .send(Method::GET, &format!("/api/servers/{id}"), None)
        .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.body["members"].as_array().unwrap().len(), 2);
    assert_eq!(detail.body["members"][1]["name"], "Bob");
    assert_eq!(detail.body["members"][1]["role"], "member");

    // Joining again is idempotent and does not burn a use.
    let again = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(again.status, StatusCode::OK);
    let preview = member
        .send(Method::GET, &format!("/api/invites/{code}"), None)
        .await;
    assert_eq!(preview.body["member"], true);

    let invites = owner
        .send(Method::GET, &format!("/api/servers/{id}/invites"), None)
        .await;
    assert_eq!(invites.status, StatusCode::OK);
    assert_eq!(invites.body[0]["uses"], 1);
}

#[sqlx::test]
async fn invites_expire_and_run_out(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = create_server(&mut owner, "Team").await;
    let id = id_of(&server);

    // Limited to one use: the owner is already in, the member takes it, a
    // third account is turned away.
    let single = create_invite(&mut owner, &id, json!({ "max_uses": 1 })).await;
    assert_eq!(single["max_uses"], 1);
    let code = single["code"].as_str().unwrap().to_owned();
    let res = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let mut third = Client::new(pool.clone());
    third.bootstrap().await;
    third
        .register("third@example.com", "password123", "Cy")
        .await;
    let res = third
        .send(Method::GET, &format!("/api/invites/{code}"), None)
        .await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    assert_eq!(res.body["error"], "invite_invalid");
    let res = third
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::GONE);
    let list = third.send(Method::GET, "/api/servers", None).await;
    assert_eq!(list.body, json!([]));

    // Expiry: create with a TTL, then age it in the database.
    let timed = create_invite(&mut owner, &id, json!({ "expires_in_hours": 1 })).await;
    assert!(timed["expires_at"].is_string());
    let code = timed["code"].as_str().unwrap().to_owned();
    sqlx::query("UPDATE invites SET expires_at = now() - interval '1 minute' WHERE code = $1")
        .bind(&code)
        .execute(&pool)
        .await
        .unwrap();
    let res = third
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::GONE);
    assert_eq!(res.body["error"], "invite_invalid");
    let invites = owner
        .send(Method::GET, &format!("/api/servers/{id}/invites"), None)
        .await;
    assert!(
        invites
            .body
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["code"] != code),
        "expired links drop out of the list"
    );

    // Unknown or malformed codes are 404.
    for code in ["abcdefghjk", "nope", "0000000000"] {
        let res = third
            .send(Method::POST, &format!("/api/invites/{code}/join"), None)
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{code}");
        assert_eq!(res.body["error"], "not_found");
    }

    // Validation of the limits.
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/invites"),
            Some(json!({ "max_uses": 0, "expires_in_hours": 0 })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["max_uses"], "invalid");
    assert_eq!(res.body["fields"]["expires_in_hours"], "invalid");
}

#[sqlx::test]
async fn members_can_create_invites_but_only_managers_list_or_revoke_others(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let id = id_of(&server);

    let own = create_invite(&mut member, &id, json!({})).await;
    let owners = create_invite(&mut owner, &id, json!({})).await;

    let res = member
        .send(Method::GET, &format!("/api/servers/{id}/invites"), None)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "forbidden");

    let res = member
        .send(
            Method::DELETE,
            &format!("/api/invites/{}", owners["code"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    let res = member
        .send(
            Method::DELETE,
            &format!("/api/invites/{}", own["code"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "creator revokes own");

    let res = owner
        .send(
            Method::DELETE,
            &format!("/api/invites/{}", owners["code"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let invites = owner
        .send(Method::GET, &format!("/api/servers/{id}/invites"), None)
        .await;
    let remaining: Vec<&Value> = invites
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|i| &i["code"])
        .collect();
    assert!(
        !remaining.contains(&&own["code"]) && !remaining.contains(&&owners["code"]),
        "revoked links are gone: {remaining:?}"
    );
    assert_eq!(remaining.len(), 1, "the link the member joined with stays");
}

#[sqlx::test]
async fn categories_and_channels_crud_with_manage_channels(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let id = id_of(&server);

    // Members cannot manage channels by default.
    let res = member
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "sneaky" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "forbidden");
    let res = member
        .send(
            Method::POST,
            &format!("/api/servers/{id}/categories"),
            Some(json!({ "name": "Sneaky" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // Owner: category, text channel (slugified), voice channel (as typed).
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/categories"),
            Some(json!({ "name": " Sprachkanäle " })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let voice_category = res.body;
    assert_eq!(voice_category["name"], "Sprachkanäle");

    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": " Off Topic ", "category_id": server["categories"][0]["id"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let text = res.body;
    assert_eq!(text["name"], "off-topic");
    assert_eq!(text["kind"], "text");
    assert_eq!(text["category_id"], server["categories"][0]["id"]);

    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "Lounge Eins", "kind": "voice", "category_id": voice_category["id"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let voice = res.body;
    assert_eq!(voice["name"], "Lounge Eins");
    assert_eq!(voice["kind"], "voice");

    // Uncategorised channel and validation.
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "loose" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["category_id"], Value::Null);
    let loose = res.body;

    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "###", "kind": "stage" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["kind"], "invalid");

    // A category from another server is an invalid field, not a hint.
    let other = create_server(&mut member, "Elsewhere").await;
    let res = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "x", "category_id": other["categories"][0]["id"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["category_id"], "invalid");

    // Rename + move, then rename the category.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/channels/{}", id_of(&loose)),
            Some(json!({ "name": "Neu Hier", "category_id": voice_category["id"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "neu-hier");
    assert_eq!(res.body["category_id"], voice_category["id"]);
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/channels/{}", id_of(&loose)),
            Some(json!({ "category_id": "" })),
        )
        .await;
    assert_eq!(
        res.body["category_id"],
        Value::Null,
        "empty string detaches"
    );
    assert_eq!(res.body["name"], "neu-hier");

    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/categories/{}", id_of(&voice_category)),
            Some(json!({ "name": "Voice" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["name"], "Voice");

    // Members see everything, in creation order.
    let detail = member
        .send(Method::GET, &format!("/api/servers/{id}"), None)
        .await;
    let names: Vec<&str> = detail.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["allgemein", "off-topic", "Lounge Eins", "neu-hier"]);
    assert_eq!(detail.body["categories"][1]["name"], "Voice");

    // Deleting a category keeps its channels, uncategorised.
    let res = owner
        .send(
            Method::DELETE,
            &format!("/api/categories/{}", id_of(&voice_category)),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let detail = owner
        .send(Method::GET, &format!("/api/servers/{id}"), None)
        .await;
    assert_eq!(detail.body["categories"].as_array().unwrap().len(), 1);
    let lounge = detail.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Lounge Eins")
        .unwrap();
    assert_eq!(lounge["category_id"], Value::Null);

    // Delete a channel; a second delete is 404.
    let res = owner
        .send(
            Method::DELETE,
            &format!("/api/channels/{}", id_of(&text)),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = owner
        .send(
            Method::DELETE,
            &format!("/api/channels/{}", id_of(&text)),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn member_permission_flags_are_enforced_and_editable(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let id = id_of(&server);

    // Member may not touch server settings.
    let res = member
        .send(
            Method::PATCH,
            &format!("/api/servers/{id}"),
            Some(json!({ "name": "Mine now" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "forbidden");

    // Owner grants manage_channels to members; the member's next call works.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{id}"),
            Some(json!({ "member_permissions": ["send_messages", "manage_channels"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body["member_permissions"],
        json!(["manage_channels", "send_messages"]),
        "canonical order, independent of input order"
    );
    assert_eq!(res.body["role"], "owner");
    assert_eq!(res.body["permissions"].as_array().unwrap().len(), 6);

    let res = member
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "member-made" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    let mine = member.send(Method::GET, "/api/servers", None).await;
    assert_eq!(
        mine.body[0]["permissions"],
        json!(["manage_channels", "send_messages"])
    );
    assert_eq!(mine.body[0]["role"], "member");

    // Still no manage_server: settings stay closed, invites list too.
    let res = member
        .send(
            Method::PATCH,
            &format!("/api/servers/{id}"),
            Some(json!({ "member_permissions": ["manage_server"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // Unknown flag names are a field error.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{id}"),
            Some(json!({ "member_permissions": ["send_messages", "administrator"] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["member_permissions"], "invalid");

    // Empty set is allowed (read-only members); rename in the same patch.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{id}"),
            Some(json!({ "name": "Team Ada", "member_permissions": [] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["name"], "Team Ada");
    assert_eq!(res.body["member_permissions"], json!([]));
    let res = member
        .send(
            Method::POST,
            &format!("/api/servers/{id}/channels"),
            Some(json!({ "name": "no-longer" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // A patch without recognised fields is a no-op that echoes the server.
    let res = owner
        .send(
            Method::PATCH,
            &format!("/api/servers/{id}"),
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["name"], "Team Ada");
}

#[sqlx::test]
async fn leaving_and_deleting(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool.clone()).await;
    let server = server_with_member(&mut owner, &mut member).await;
    let id = id_of(&server);

    // Owner cannot leave; member cannot delete.
    let res = owner
        .send(Method::POST, &format!("/api/servers/{id}/leave"), None)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    let res = member
        .send(Method::DELETE, &format!("/api/servers/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    // Member leaves and loses access.
    let res = member
        .send(Method::POST, &format!("/api/servers/{id}/leave"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = member
        .send(Method::GET, &format!("/api/servers/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let list = member.send(Method::GET, "/api/servers", None).await;
    assert_eq!(list.body, json!([]));

    // Owner deletes; everything under it is gone.
    create_invite(&mut owner, &id, json!({})).await;
    let res = owner
        .send(Method::DELETE, &format!("/api/servers/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = owner
        .send(Method::GET, &format!("/api/servers/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM servers), (SELECT count(*) FROM server_members), \
                (SELECT count(*) FROM categories), (SELECT count(*) FROM channels), \
                (SELECT count(*) FROM invites)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0, 0, 0), "cascade left rows behind");
}

#[sqlx::test]
async fn servers_are_listed_in_join_order(pool: PgPool) {
    let (mut owner, mut member) = two_users(pool).await;
    let first = create_server(&mut owner, "First").await;
    let second = create_server(&mut owner, "Second").await;
    let theirs = create_server(&mut member, "Theirs").await;

    let invite = create_invite(&mut member, &id_of(&theirs), json!({})).await;
    owner
        .send(
            Method::POST,
            &format!("/api/invites/{}/join", invite["code"].as_str().unwrap()),
            None,
        )
        .await;

    let list = owner.send(Method::GET, "/api/servers", None).await;
    let ids: Vec<&Value> = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["id"])
        .collect();
    assert_eq!(ids, [&first["id"], &second["id"], &theirs["id"]]);
    assert_eq!(list.body[2]["role"], "member");
}
