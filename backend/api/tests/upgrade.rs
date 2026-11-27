//! A server upgrades from the previous release in one step (GEL-364): its
//! database, with rows in it, takes the migrations that came after that
//! release, and the current code reads the rows back (GEL-413).
//!
//! The boundary is the highest migration of the newest tag of the previous
//! minor version (`.github/workflows/scripts/previous_release.sh`). CI
//! passes it as `UPGRADE_FROM` and `UPGRADE_FROM_MIGRATION`; elsewhere the
//! test asks git the same way. The rows are written as that release's schema
//! has them, so they need updating when the boundary moves past a migration
//! that changes these tables; the test then fails on its own inserts.

mod common;

use std::path::Path;
use std::process::Command;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "correct horse";

/// The previous release's tag and highest migration.
fn boundary() -> (String, i64) {
    if let (Ok(tag), Ok(migration)) = (
        std::env::var("UPGRADE_FROM"),
        std::env::var("UPGRADE_FROM_MIGRATION"),
    ) {
        return (tag, migration.parse().expect("UPGRADE_FROM_MIGRATION"));
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/scripts/previous_release.sh");
    let out = Command::new("sh")
        .arg(&script)
        .output()
        .expect("sh for previous_release.sh");
    assert!(
        out.status.success(),
        "the previous release is unknown: set UPGRADE_FROM and UPGRADE_FROM_MIGRATION, \
         or fetch its tag ({})",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let line = String::from_utf8(out.stdout).unwrap();
    let (tag, migration) = line.trim().split_once(' ').expect("TAG NUMBER");
    (tag.to_owned(), migration.parse().expect("migration number"))
}

/// Rows as the previous release left them: two accounts, a server with a
/// category, a text and a voice channel and a role, messages with a file
/// and a reaction, a direct message, a notification setting and a theme.
struct Rows {
    ada: Uuid,
    server: Uuid,
    text: Uuid,
    dm: Uuid,
    with_file: Uuid,
    file: Uuid,
}

async fn write_rows(pool: &PgPool) -> Rows {
    let hash = gelabber_api::password::hash(PASSWORD.to_owned())
        .await
        .unwrap();
    let user = async |email: &str, name: &str| -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO users (email, name, password_hash, email_verified_at, locale) \
             VALUES ($1, $2, $3, now(), 'de') RETURNING id",
        )
        .bind(email)
        .bind(name)
        .bind(&hash)
        .fetch_one(pool)
        .await
        .unwrap()
    };
    let ada = user("ada@example.test", "Ada").await;
    let bob = user("bob@example.test", "Bob").await;

    let server: Uuid = sqlx::query_scalar(
        "INSERT INTO servers (name, owner_id) VALUES ('Werkstatt', $1) RETURNING id",
    )
    .bind(ada)
    .fetch_one(pool)
    .await
    .unwrap();
    for member in [ada, bob] {
        sqlx::query("INSERT INTO server_members (server_id, user_id) VALUES ($1, $2)")
            .bind(server)
            .bind(member)
            .execute(pool)
            .await
            .unwrap();
    }
    let category: Uuid = sqlx::query_scalar(
        "INSERT INTO categories (server_id, name) VALUES ($1, 'Drinnen') RETURNING id",
    )
    .bind(server)
    .fetch_one(pool)
    .await
    .unwrap();
    let channel = async |name: &str, kind: &str| -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO channels (server_id, category_id, name, kind) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(server)
        .bind(category)
        .bind(name)
        .bind(kind)
        .fetch_one(pool)
        .await
        .unwrap()
    };
    let text = channel("allgemein", "text").await;
    channel("Wohnzimmer", "voice").await;
    let role: Uuid = sqlx::query_scalar(
        "INSERT INTO server_roles (server_id, name, color, permissions, position) \
         VALUES ($1, 'Stammgäste', 'green', 1, 1) RETURNING id",
    )
    .bind(server)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO server_member_roles (server_id, user_id, role_id) VALUES ($1, $2, $3)",
    )
    .bind(server)
    .bind(bob)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();

    let message = async |channel: Uuid, author: Uuid, content: &str| -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO messages (channel_id, author_id, content) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(channel)
        .bind(author)
        .bind(content)
        .fetch_one(pool)
        .await
        .unwrap()
    };
    message(text, ada, "Hallo Werkstatt").await;
    let with_file = message(text, bob, "Der Plan").await;
    let file: Uuid = sqlx::query_scalar(
        "INSERT INTO attachments (message_id, channel_id, uploader_id, object_key, filename, \
                                  content_type, size_bytes, quota_state) \
         VALUES ($1, $2, $3, 'att/plan', 'plan.pdf', 'application/pdf', 4, 'consumed') \
         RETURNING id",
    )
    .bind(with_file)
    .bind(text)
    .bind(bob)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO message_reactions (message_id, user_id, emoji) VALUES ($1, $2, '👍')")
        .bind(with_file)
        .bind(ada)
        .execute(pool)
        .await
        .unwrap();

    let dm: Uuid = sqlx::query_scalar(
        "INSERT INTO channels (name, kind, pair_key) VALUES ('', 'dm', $1) RETURNING id",
    )
    .bind(gelabber_api::dms::pair_key(ada, bob))
    .fetch_one(pool)
    .await
    .unwrap();
    for member in [ada, bob] {
        sqlx::query("INSERT INTO channel_members (channel_id, user_id) VALUES ($1, $2)")
            .bind(dm)
            .bind(member)
            .execute(pool)
            .await
            .unwrap();
    }
    message(dm, bob, "Kommst du morgen?").await;

    sqlx::query(
        "INSERT INTO notification_settings (user_id, scope_kind, scope_id, level) \
         VALUES ($1, 'server', $2, 'mentions')",
    )
    .bind(ada)
    .bind(server)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO account_themes (user_id, revision, document) VALUES ($1, 3, $2)")
        .bind(ada)
        .bind(json!({ "version": 1, "revision": 3, "active": "light", "customThemes": [] }))
        .execute(pool)
        .await
        .unwrap();

    Rows {
        ada,
        server,
        text,
        dm,
        with_file,
        file,
    }
}

async fn get(client: &mut Client, path: &str) -> Value {
    let res = client.send(Method::GET, path, None).await;
    assert_eq!(res.status, StatusCode::OK, "{path}: {}", res.body);
    res.body
}

#[sqlx::test(migrations = false)]
async fn a_server_upgrades_from_the_previous_release_with_its_rows(pool: PgPool) {
    let (tag, last) = boundary();
    let migrator = sqlx::migrate!("./migrations");
    let shipped = migrator
        .iter()
        .filter(|migration| migration.version <= last)
        .count();
    assert!(shipped > 0, "{tag} shipped no migration of this tree");
    for migration in migrator.iter().filter(|m| m.version <= last) {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap_or_else(|err| panic!("{tag}'s migration {}: {err}", migration.version));
    }
    let rows = write_rows(&pool).await;

    let after: Vec<_> = migrator.iter().filter(|m| m.version > last).collect();
    // While both releases end at the same migration there is nothing to
    // upgrade across; the test says so instead of passing quietly.
    eprintln!(
        "upgrade from {tag} (migration {last}): {} migration(s) after it",
        after.len()
    );
    for migration in after {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "migration {} ({}) on {tag}'s rows: {err}",
                    migration.version, migration.description
                )
            });
    }

    // The current code on the upgraded rows.
    let mut ada = Client::new(pool.clone());
    ada.bootstrap().await;
    let res = ada.login("ada@example.test", PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(ada.user_id(), rows.ada.to_string());

    let servers = get(&mut ada, "/api/servers").await;
    assert!(
        servers
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == rows.server.to_string() && s["name"] == "Werkstatt"),
        "{servers}"
    );
    let server = get(&mut ada, &format!("/api/servers/{}", rows.server)).await;
    let mut channels: Vec<&str> = server["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    channels.sort_unstable();
    assert_eq!(channels, ["Wohnzimmer", "allgemein"], "{server}");
    let role = server["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["name"] == "Stammgäste")
        .unwrap_or_else(|| panic!("{server}"));
    assert_eq!(role["color"], "green", "{role}");

    let page = get(&mut ada, &format!("/api/channels/{}/messages", rows.text)).await;
    let messages = page["messages"].as_array().unwrap();
    let texts: Vec<&str> = messages
        .iter()
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert!(texts.contains(&"Hallo Werkstatt"), "{page}");
    let with_file = messages
        .iter()
        .find(|m| m["id"] == rows.with_file.to_string())
        .unwrap_or_else(|| panic!("{page}"));
    assert_eq!(with_file["content"], "Der Plan");
    assert_eq!(
        with_file["attachments"][0]["id"],
        rows.file.to_string(),
        "{with_file}"
    );
    assert_eq!(with_file["attachments"][0]["filename"], "plan.pdf");
    assert_eq!(with_file["reactions"][0]["emoji"], "👍", "{with_file}");

    let dms = get(&mut ada, "/api/dms").await;
    assert!(
        dms.as_array()
            .unwrap()
            .iter()
            .any(|dm| dm["id"] == rows.dm.to_string()),
        "{dms}"
    );
    let page = get(&mut ada, &format!("/api/channels/{}/messages", rows.dm)).await;
    assert_eq!(
        page["messages"][0]["content"], "Kommst du morgen?",
        "{page}"
    );

    let settings = get(&mut ada, "/api/notification-settings").await;
    assert_eq!(
        settings,
        json!([{
            "scope": "server",
            "id": rows.server,
            "level": "mentions",
            "muted": false,
            "muted_until": null,
            "voice": false,
        }])
    );
    let theme = get(&mut ada, "/api/me/themes").await;
    assert_eq!(theme["active"], "light", "{theme}");
    assert_eq!(theme["revision"], 3, "{theme}");
}
