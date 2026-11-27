//! A server upgrades from the previous release in one step (GEL-364): its
//! database, with rows in it, takes the migrations that came after that
//! release, the way a server runs them, and the current code reads the rows
//! back (GEL-413).
//!
//! The boundary is the highest migration of the newest tag of the previous
//! minor version (`.github/workflows/scripts/previous_release.sh`). CI and
//! `backend/build/run-native-ci.sh` pass it as `UPGRADE_FROM` and
//! `UPGRADE_FROM_MIGRATION`; elsewhere the test asks the script. The rows
//! are written as that release's schema has them ([`ROWS`]), so they need
//! updating when the boundary moves past a migration that changes these
//! tables; the test then fails on its own inserts.

mod common;

use std::borrow::Cow;
use std::path::Path;
use std::process::Command;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::migrate::Migrator;

use common::{Client, SESSION};

const PASSWORD: &str = "correct horse";

// Ada owns the server; Bob is a member with a role; Cara signed up through
// a provider and has no password; Dora asked to join through the directory;
// Eve is banned and blocked; Finn's account is being deleted.
const ADA: &str = "00000000-0000-4000-8000-00000000000a";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const CARA: &str = "00000000-0000-4000-8000-00000000000c";
const EVE: &str = "00000000-0000-4000-8000-00000000000e";
const SERVER: &str = "00000000-0000-4000-8000-000000000101";
const CATEGORY: &str = "00000000-0000-4000-8000-000000000102";
const TEXT: &str = "00000000-0000-4000-8000-000000000103";
const VOICE: &str = "00000000-0000-4000-8000-000000000104";
const LIMITED: &str = "00000000-0000-4000-8000-000000000105";
const ROLE: &str = "00000000-0000-4000-8000-000000000106";
const DM: &str = "00000000-0000-4000-8000-000000000107";
const REQUEST_DM: &str = "00000000-0000-4000-8000-000000000108";
const GREETING: &str = "00000000-0000-4000-8000-000000000201";
const PLAN: &str = "00000000-0000-4000-8000-000000000202";
const GIF: &str = "00000000-0000-4000-8000-000000000203";
const LIVE: &str = "00000000-0000-4000-8000-000000000204";
const ORPHAN_REPLY: &str = "00000000-0000-4000-8000-000000000205";
const BOBS_OWN: &str = "00000000-0000-4000-8000-000000000206";
const FILE: &str = "00000000-0000-4000-8000-000000000301";

/// The rows, in the previous release's schema. `{hash}` is the accounts'
/// password hash, `{session}` the hash of Ada's session token. Every table
/// of that schema gets a row (`every_table_of_the_previous_release_gets_a_row`).
///
/// The rows of these tables are only written, not read back: a migration
/// that cannot run on them fails the test (a `NOT NULL` column without a
/// default, say), one that rewrites them lossily does not. A sign-in through
/// `user_identities`, the operator's view of `reports`, `operator_audit` and
/// `feedback`, and the workers behind the rest are more than this test runs.
/// They are listed in [`ONLY_WRITTEN`].
const ROWS: &str = r#"
INSERT INTO users (id, email, name, password_hash, email_verified_at, locale, bio, pronouns,
                   status_text, accent_color, banner_color, avatar_decoration, friend_code,
                   presence_mode, presence_mode_until, notifications_hide_content)
VALUES ('00000000-0000-4000-8000-00000000000a', 'ada@example.test', 'Ada', '{hash}', now(), 'en',
        'Baut Möbel', 'sie/ihr', 'In der Werkstatt', '#336699', '#996633', 'ring', 'ABCD-EFGH',
        'dnd', now() + interval '1 hour', true);
INSERT INTO users (id, email, name, password_hash, email_verified_at, locale)
VALUES ('00000000-0000-4000-8000-00000000000b', 'bob@example.test', 'Bob', '{hash}', now(), 'de');
INSERT INTO users (id, email, name, email_verified_at, locale)
VALUES ('00000000-0000-4000-8000-00000000000c', 'cara@example.test', 'Cara', now(), 'de');
INSERT INTO users (id, email, name, password_hash)
VALUES ('00000000-0000-4000-8000-00000000000d', 'dora@example.test', 'Dora', '{hash}');
INSERT INTO users (id, email, name, password_hash, deleted_at)
VALUES ('00000000-0000-4000-8000-00000000000f', 'finn@example.test', 'Finn', '{hash}', now());
INSERT INTO users (id, email, name, password_hash)
VALUES ('00000000-0000-4000-8000-00000000000e', 'eve@example.test', 'Eve', '{hash}');
INSERT INTO sessions (token_hash, user_id, expires_at, device_client, device_os)
VALUES ('\x{session}', '00000000-0000-4000-8000-00000000000a', now() + interval '1 hour', 'firefox', 'linux');
INSERT INTO account_tokens (token_hash, user_id, purpose, expires_at)
VALUES ('\x01', '00000000-0000-4000-8000-00000000000b', 'reset_password', now() + interval '1 hour');
INSERT INTO user_identities (provider, subject, user_id, email)
VALUES ('google', 'cara-sub', '00000000-0000-4000-8000-00000000000c', 'cara@example.test');
INSERT INTO account_themes (user_id, revision, document)
VALUES ('00000000-0000-4000-8000-00000000000a', 3,
        '{"version":1,"revision":3,"active":"light","customThemes":[]}');
INSERT INTO friendships (user_low, user_high)
VALUES ('00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-00000000000b');
INSERT INTO friend_requests (sender_id, recipient_id)
VALUES ('00000000-0000-4000-8000-00000000000c', '00000000-0000-4000-8000-00000000000a');
INSERT INTO user_blocks (blocker_id, blocked_id)
VALUES ('00000000-0000-4000-8000-00000000000a', '00000000-0000-4000-8000-00000000000e');

INSERT INTO servers (id, name, owner_id, member_permissions)
VALUES ('00000000-0000-4000-8000-000000000101', 'Werkstatt', '00000000-0000-4000-8000-00000000000a', 60);
INSERT INTO server_members (server_id, user_id, joined_at) VALUES
  ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-00000000000a', now() - interval '1 day'),
  ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-00000000000b', now() - interval '1 day');
INSERT INTO categories (id, server_id, name)
VALUES ('00000000-0000-4000-8000-000000000102', '00000000-0000-4000-8000-000000000101', 'Drinnen');
INSERT INTO channels (id, server_id, category_id, name, kind, limited) VALUES
  ('00000000-0000-4000-8000-000000000103', '00000000-0000-4000-8000-000000000101',
   '00000000-0000-4000-8000-000000000102', 'allgemein', 'text', false),
  ('00000000-0000-4000-8000-000000000104', '00000000-0000-4000-8000-000000000101',
   '00000000-0000-4000-8000-000000000102', 'Wohnzimmer', 'voice', false),
  ('00000000-0000-4000-8000-000000000105', '00000000-0000-4000-8000-000000000101',
   NULL, 'Stammtisch', 'text', true);
INSERT INTO server_roles (id, server_id, name, color, permissions, position)
VALUES ('00000000-0000-4000-8000-000000000106', '00000000-0000-4000-8000-000000000101',
        'Stammgäste', 'green', 5, 1);
INSERT INTO server_member_roles (server_id, user_id, role_id)
VALUES ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-00000000000b',
        '00000000-0000-4000-8000-000000000106');
INSERT INTO channel_roles (server_id, channel_id, role_id)
VALUES ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-000000000105',
        '00000000-0000-4000-8000-000000000106');
INSERT INTO invites (code, server_id, created_by, max_uses, uses)
VALUES ('werkstatt', '00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-00000000000a', 5, 1);
INSERT INTO server_bans (server_id, user_id, banned_by)
VALUES ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-00000000000e',
        '00000000-0000-4000-8000-00000000000a');
INSERT INTO server_listings (server_id, listed, description, language, join_mode)
VALUES ('00000000-0000-4000-8000-000000000101', true, 'Holz und Metall', 'de', 'request');
INSERT INTO server_join_requests (server_id, user_id, message)
VALUES ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-00000000000d', 'Darf ich?');
INSERT INTO reports (server_id, reporter_id, reason, details)
VALUES ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-00000000000e', 'spam', 'Werbung');
INSERT INTO notification_settings (user_id, scope_kind, scope_id, level)
VALUES ('00000000-0000-4000-8000-00000000000a', 'server', '00000000-0000-4000-8000-000000000101', 'mentions');

INSERT INTO messages (id, channel_id, author_id, content, created_at, edited_at, mentions)
VALUES ('00000000-0000-4000-8000-000000000201', '00000000-0000-4000-8000-000000000103',
        '00000000-0000-4000-8000-00000000000a', 'Hallo <@00000000-0000-4000-8000-00000000000b>',
        now() - interval '5 minutes', now() - interval '4 minutes',
        '{00000000-0000-4000-8000-00000000000b}');
INSERT INTO messages (id, channel_id, author_id, content, created_at, reply_to)
VALUES ('00000000-0000-4000-8000-000000000202', '00000000-0000-4000-8000-000000000103',
        '00000000-0000-4000-8000-00000000000b', 'Der Plan', now() - interval '4 minutes',
        '00000000-0000-4000-8000-000000000201');
INSERT INTO messages (id, channel_id, author_id, content, created_at, gif)
VALUES ('00000000-0000-4000-8000-000000000203', '00000000-0000-4000-8000-000000000103',
        '00000000-0000-4000-8000-00000000000a', '', now() - interval '3 minutes',
        '{"provider":"klipy","slug":"hammer","title":"Hammer","url":"https://static.klipy.com/hammer.webp","width":200,"height":150}');
INSERT INTO messages (id, channel_id, author_id, content, created_at, system_kind, system_params)
VALUES ('00000000-0000-4000-8000-000000000204', '00000000-0000-4000-8000-000000000103',
        '00000000-0000-4000-8000-00000000000a', '', now() - interval '2 minutes', 'live_started',
        '{"voice_name":"Wohnzimmer"}');
INSERT INTO messages (id, channel_id, author_id, content, created_at, reply_to)
VALUES ('00000000-0000-4000-8000-000000000205', '00000000-0000-4000-8000-000000000103',
        '00000000-0000-4000-8000-00000000000b', 'Worauf?', now() - interval '1 minute',
        '00000000-0000-4000-8000-0000000002ff');
INSERT INTO messages (id, channel_id, author_id, content, created_at, sender_only)
VALUES ('00000000-0000-4000-8000-000000000206', '00000000-0000-4000-8000-000000000103',
        '00000000-0000-4000-8000-00000000000b', 'Nur für mich', now(), true);
INSERT INTO attachments (id, message_id, channel_id, uploader_id, object_key, filename,
                         content_type, size_bytes, quota_state, width, height, description)
VALUES ('00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000202',
        '00000000-0000-4000-8000-000000000103', '00000000-0000-4000-8000-00000000000b',
        'att/00000000-0000-4000-8000-000000000301', 'plan.png', 'image/png', 4096, 'consumed',
        640, 480, 'Der Grundriss');
INSERT INTO message_reactions (message_id, user_id, emoji) VALUES
  ('00000000-0000-4000-8000-000000000202', '00000000-0000-4000-8000-00000000000a', '👍'),
  ('00000000-0000-4000-8000-000000000202', '00000000-0000-4000-8000-00000000000b', '👍');
INSERT INTO link_previews (id, url_hash, url, status, title)
VALUES ('00000000-0000-4000-8000-000000000401', sha256('https://example.org/'),
        'https://example.org/', 'ok', 'Beispiel');
INSERT INTO message_previews (message_id, preview_id, position)
VALUES ('00000000-0000-4000-8000-000000000202', '00000000-0000-4000-8000-000000000401', 0);
INSERT INTO channel_read_state (user_id, channel_id, membership_at, message_at, created_order, message_id)
SELECT '00000000-0000-4000-8000-00000000000a', channel_id, now() - interval '1 day', created_at,
       created_order, id
FROM messages WHERE id = '00000000-0000-4000-8000-000000000202';

INSERT INTO channels (id, name, kind, pair_key)
VALUES ('00000000-0000-4000-8000-000000000107', '', 'dm',
        '00000000-0000-4000-8000-00000000000a:00000000-0000-4000-8000-00000000000b');
INSERT INTO channels (id, name, kind, pair_key, dm_request_by, dm_request_sent)
VALUES ('00000000-0000-4000-8000-000000000108', '', 'dm',
        '00000000-0000-4000-8000-00000000000a:00000000-0000-4000-8000-00000000000c',
        '00000000-0000-4000-8000-00000000000c', 1);
INSERT INTO channel_members (channel_id, user_id) VALUES
  ('00000000-0000-4000-8000-000000000107', '00000000-0000-4000-8000-00000000000a'),
  ('00000000-0000-4000-8000-000000000107', '00000000-0000-4000-8000-00000000000b'),
  ('00000000-0000-4000-8000-000000000108', '00000000-0000-4000-8000-00000000000a'),
  ('00000000-0000-4000-8000-000000000108', '00000000-0000-4000-8000-00000000000c');
INSERT INTO messages (channel_id, author_id, content) VALUES
  ('00000000-0000-4000-8000-000000000107', '00000000-0000-4000-8000-00000000000b', 'Kommst du morgen?'),
  ('00000000-0000-4000-8000-000000000108', '00000000-0000-4000-8000-00000000000c', 'Hallo Ada');

INSERT INTO push_subscriptions (endpoint, user_id, session_hash, p256dh, auth)
VALUES ('https://push.example.test/ada', '00000000-0000-4000-8000-00000000000a', '\x{session}',
        'BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I99e8QcYP7DkM',
        'tBHItJI5svbpez7KI4CCXg');
INSERT INTO push_receipts (message_id, endpoint)
VALUES ('00000000-0000-4000-8000-000000000201', 'https://push.example.test/ada');
INSERT INTO upload_daily_usage (uploader_id, day, reserved, consumed)
VALUES ('00000000-0000-4000-8000-00000000000e', CURRENT_DATE - 1, 0, 8192)
ON CONFLICT (uploader_id, day) DO NOTHING;
INSERT INTO storage_cleanup (object_key, retain_until)
VALUES ('att/left-over', now() + interval '1 day');
INSERT INTO account_exports (user_id, state)
VALUES ('00000000-0000-4000-8000-00000000000b', 'pending');
INSERT INTO gateway_outbox (id, channel_id, server_id, kind)
VALUES (1, '00000000-0000-4000-8000-000000000103', '00000000-0000-4000-8000-000000000101', 'c');
INSERT INTO oidc_flows (state_hash, provider, purpose, nonce, pkce_verifier, binding_hash, expires_at)
VALUES ('\x02', 'google', 'sign_in', 'nonce', 'verifier', '\x04', now() + interval '10 minutes');
INSERT INTO oidc_results (code_hash, provider, subject, email, email_verified, purpose, binding_hash,
                          expires_at)
VALUES ('\x03', 'google', 'cara-sub', 'cara@example.test', true, 'sign_in', '\x05',
        now() + interval '1 minute');
INSERT INTO operator_audit (operator_email, action, target_server, detail)
VALUES ('lena@example.test', 'servers.block', '00000000-0000-4000-8000-000000000101', '{"reason":"Test"}');
INSERT INTO feedback (id, kind, user_id, client, os, app_version, locale, page, body, status)
VALUES ('00000000-0000-4000-8000-000000000501', 'idea', '00000000-0000-4000-8000-00000000000a',
        'web', 'linux', '0.13.1', 'de', '/', 'Dunkler Modus für Holz', 'open');
INSERT INTO feedback_messages (feedback_id, author_kind, body, mail_state)
VALUES ('00000000-0000-4000-8000-000000000501', 'operator', 'Danke!', 'sent');
INSERT INTO beta_invitations (code, email)
VALUES ('BETA-0001', 'frank@example.test');
"#;

/// Tables whose rows [`ROWS`] writes but the test does not read back.
const ONLY_WRITTEN: &[&str] = &[
    "account_exports",
    "account_tokens",
    "beta_invitations",
    "feedback",
    "feedback_messages",
    "gateway_outbox",
    "oidc_flows",
    "oidc_results",
    "operator_audit",
    "push_receipts",
    "push_subscriptions",
    "reports",
    "storage_cleanup",
    "upload_daily_usage",
    "user_identities",
];

/// Tables of the previous release that [`ROWS`] leaves empty, and why.
const NOT_COVERED: &[(&str, &str)] = &[];

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
    let out = Command::new("sh").arg(&script).output();
    let out = match out {
        Ok(out) if out.status.success() => out,
        other => panic!(
            "set UPGRADE_FROM and UPGRADE_FROM_MIGRATION (the tag and the highest migration of \
             the previous release; {} prints them where git and the tags are): {other:?}",
            script.display()
        ),
    };
    let line = String::from_utf8(out.stdout).unwrap();
    let (tag, migration) = line.trim().split_once(' ').expect("TAG NUMBER");
    (tag.to_owned(), migration.parse().expect("migration number"))
}

/// The migrations up to `last`, run as a server runs them.
fn up_to(last: i64) -> Migrator {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|migration| migration.version <= last)
            .cloned()
            .collect(),
    );
    migrator
}

/// The previous release's schema with [`ROWS`] in it: its tag, its highest
/// migration and Ada's session token.
async fn previous_release_with_rows(pool: &PgPool) -> (String, i64, String) {
    let (tag, last) = boundary();
    let shipped = up_to(last);
    assert!(
        !shipped.migrations.is_empty(),
        "{tag} shipped no migration of this tree"
    );
    shipped
        .run(pool)
        .await
        .unwrap_or_else(|err| panic!("{tag}'s migrations: {err}"));
    let hash = gelabber_api::password::hash(PASSWORD.to_owned())
        .await
        .unwrap();
    let token = gelabber_api::token::generate();
    let session: String = gelabber_api::token::hash(&token)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let rows = ROWS.replace("{hash}", &hash).replace("{session}", &session);
    sqlx::raw_sql(sqlx::AssertSqlSafe(rows))
        .execute(pool)
        .await
        .unwrap_or_else(|err| panic!("rows in {tag}'s schema: {err}"));
    (tag, last, token)
}

async fn get(client: &mut Client, path: &str) -> Value {
    let res = client.send(Method::GET, path, None).await;
    assert_eq!(res.status, StatusCode::OK, "{path}: {}", res.body);
    res.body
}

fn find<'a>(list: &'a Value, key: &str, value: &str) -> &'a Value {
    list.as_array()
        .unwrap_or_else(|| panic!("not a list: {list}"))
        .iter()
        .find(|item| item[key] == value)
        .unwrap_or_else(|| panic!("no {key} = {value} in {list}"))
}

#[sqlx::test(migrations = false)]
async fn a_server_upgrades_from_the_previous_release_with_its_rows(pool: PgPool) {
    let (tag, last, token) = previous_release_with_rows(&pool).await;

    // The rest as a server starts: the whole chain, the applied part checked.
    let after = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version > last)
        .count();
    // CI says so in its summary when this is 0 (ci.yml).
    eprintln!("upgrade from {tag} (migration {last}): {after} migration(s) after it");
    common::state(pool.clone())
        .migrate()
        .await
        .unwrap_or_else(|err| panic!("upgrading {tag}'s rows: {err}"));

    // Ada's session from before the upgrade still holds.
    let mut with_directory = common::state(pool.clone());
    with_directory.directory = true;
    let mut ada = Client::from_state(with_directory);
    ada.jar.insert(SESSION.to_owned(), token);
    let me = get(&mut ada, "/api/me").await;
    assert_eq!(me["id"], ADA, "{me}");
    assert_eq!(me["locale"], "en", "{me}");
    assert_eq!(me["pronouns"], "sie/ihr", "{me}");
    assert_eq!(me["accent_color"], "#336699", "{me}");
    assert_eq!(me["bio"], "Baut Möbel", "{me}");
    assert_eq!(me["status_text"], "In der Werkstatt", "{me}");
    assert_eq!(me["banner_color"], "#996633", "{me}");
    assert_eq!(me["avatar_decoration"], "ring", "{me}");
    let presence = get(&mut ada, "/api/me/presence").await;
    assert_eq!(presence["mode"], "dnd", "{presence}");
    assert!(presence["until"].is_string(), "{presence}");
    let code = get(&mut ada, "/api/me/friend-code").await;
    assert_eq!(code["code"], "ABCD-EFGH", "{code}");
    let sessions = get(&mut ada, "/api/auth/sessions").await;
    assert_eq!(
        sessions["sessions"].as_array().map(Vec::len),
        Some(1),
        "{sessions}"
    );

    // Bob signs in with his password.
    let mut bob = Client::new(pool.clone());
    bob.bootstrap().await;
    let res = bob.login("bob@example.test", PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let server = get(&mut ada, &format!("/api/servers/{SERVER}")).await;
    let channels = &server["channels"];
    for (id, name, kind, category) in [
        (TEXT, "allgemein", "text", Value::from(CATEGORY)),
        (VOICE, "Wohnzimmer", "voice", Value::from(CATEGORY)),
        (LIMITED, "Stammtisch", "text", Value::Null),
    ] {
        let channel = find(channels, "id", id);
        assert_eq!(channel["name"], name, "{channel}");
        assert_eq!(channel["kind"], kind, "{channel}");
        assert_eq!(channel["category_id"], category, "{channel}");
    }
    let role = find(&server["roles"], "id", ROLE);
    assert_eq!(role["name"], "Stammgäste", "{role}");
    assert_eq!(role["color"], "green", "{role}");
    assert_eq!(
        role["permissions"],
        json!(["manage_server", "send_messages"]),
        "{role}"
    );
    assert_eq!(
        find(&server["members"], "user_id", BOB)["role_ids"],
        json!([ROLE]),
        "{server}"
    );
    assert_eq!(find(channels, "id", LIMITED)["role_ids"], json!([ROLE]));
    assert_eq!(find(channels, "id", LIMITED)["limited"], true);
    assert_eq!(
        server["member_permissions"],
        json!(["send_messages", "send_files", "join_voice", "go_live"]),
        "{server}"
    );
    let listing = get(&mut ada, &format!("/api/servers/{SERVER}/listing")).await;
    assert_eq!(listing["listed"], true, "{listing}");
    assert_eq!(listing["description"], "Holz und Metall", "{listing}");
    // The limited channel is the role's: Bob, holding it, sees it.
    let bobs_view = get(&mut bob, &format!("/api/servers/{SERVER}")).await;
    find(&bobs_view["channels"], "id", LIMITED);

    let page = get(&mut ada, &format!("/api/channels/{TEXT}/messages")).await;
    let ids: Vec<&str> = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    // In order; Bob's message only for him stays his.
    assert_eq!(ids, [GREETING, PLAN, GIF, LIVE, ORPHAN_REPLY], "{page}");
    let messages = &page["messages"];
    let greeting = find(messages, "id", GREETING);
    assert!(greeting["edited_at"].is_string(), "{greeting}");
    assert_eq!(greeting["mentions"], json!([BOB]), "{greeting}");
    let plan = find(messages, "id", PLAN);
    assert_eq!(plan["reply"]["id"], GREETING, "{plan}");
    let file = &plan["attachments"][0];
    assert_eq!(file["id"], FILE, "{plan}");
    assert_eq!(file["filename"], "plan.png", "{file}");
    assert_eq!(file["content_type"], "image/png", "{file}");
    assert_eq!(file["size"], 4096, "{file}");
    assert_eq!(file["width"], 640, "{file}");
    assert_eq!(file["height"], 480, "{file}");
    assert_eq!(file["description"], "Der Grundriss", "{file}");
    let reaction = &plan["reactions"][0];
    assert_eq!(reaction["emoji"], "👍", "{plan}");
    assert_eq!(reaction["user_ids"], json!([ADA, BOB]), "{plan}");
    assert_eq!(plan["previews"][0]["title"], "Beispiel", "{plan}");
    assert_eq!(find(messages, "id", GIF)["gif"]["slug"], "hammer");
    let live = find(messages, "id", LIVE);
    assert_eq!(live["system"]["kind"], "live_started", "{live}");
    assert_eq!(
        live["system"]["params"]["voice_name"], "Wohnzimmer",
        "{live}"
    );
    let orphan = find(messages, "id", ORPHAN_REPLY);
    // Its original is gone, as migration 0017 left such replies.
    assert_eq!(orphan["reply"]["deleted"], true, "{orphan}");
    let bobs_page = get(&mut bob, &format!("/api/channels/{TEXT}/messages")).await;
    find(&bobs_page["messages"], "id", BOBS_OWN);

    // Ada read up to Bob's plan: of what came after, Bob's reply is unread
    // (her own GIF and Live are not, his message only for him is not hers).
    let unread = get(&mut ada, "/api/messages/unread").await;
    let text = find(&unread, "channel_id", TEXT);
    assert_eq!(text["read_message_id"], PLAN, "{unread}");
    assert_eq!(text["unread_count"], 1, "{unread}");

    let dms = get(&mut ada, "/api/dms").await;
    find(&dms, "id", DM);
    let page = get(&mut ada, &format!("/api/channels/{DM}/messages")).await;
    assert_eq!(
        page["messages"][0]["content"], "Kommst du morgen?",
        "{page}"
    );
    let requests = get(&mut ada, "/api/dms/requests").await;
    let request = &find(&requests, "id", REQUEST_DM)["request"];
    assert_eq!(request["incoming"], true, "{requests}");
    let request = get(&mut ada, &format!("/api/channels/{REQUEST_DM}/messages")).await;
    assert_eq!(request["messages"][0]["content"], "Hallo Ada", "{request}");

    let friends = get(&mut ada, "/api/friends").await;
    assert_eq!(friends["friends"][0]["user"]["id"], BOB, "{friends}");
    assert_eq!(friends["incoming"][0]["user"]["id"], CARA, "{friends}");
    let blocks = get(&mut ada, "/api/users/me/blocks").await;
    assert!(blocks.to_string().contains(EVE), "{blocks}");
    let invites = get(&mut ada, &format!("/api/servers/{SERVER}/invites")).await;
    let invite = find(&invites, "code", "werkstatt");
    assert_eq!(
        (invite["uses"].clone(), invite["max_uses"].clone()),
        (json!(1), json!(5)),
        "{invite}"
    );
    let bans = get(&mut ada, &format!("/api/servers/{SERVER}/bans")).await;
    assert!(bans.to_string().contains(EVE), "{bans}");
    let joining = get(&mut ada, &format!("/api/servers/{SERVER}/join-requests")).await;
    assert!(joining.to_string().contains("Darf ich?"), "{joining}");

    let settings = get(&mut ada, "/api/notification-settings").await;
    assert_eq!(
        find(&settings, "id", SERVER)["level"],
        "mentions",
        "{settings}"
    );
    let theme = get(&mut ada, "/api/me/themes").await;
    assert_eq!(
        (theme["active"].clone(), theme["revision"].clone()),
        (json!("light"), json!(3)),
        "{theme}"
    );
}

/// Every table of the previous release's schema has a row of [`ROWS`], or
/// [`NOT_COVERED`] says why not: a new table cannot slip past the upgrade
/// test unnoticed.
#[sqlx::test(migrations = false)]
async fn every_table_of_the_previous_release_gets_a_row(pool: PgPool) {
    let (tag, _, _) = previous_release_with_rows(&pool).await;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' \
         AND tablename <> '_sqlx_migrations' ORDER BY tablename",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut empty = Vec::new();
    for table in &tables {
        let has_rows: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS (SELECT 1 FROM {table})"
        )))
        .fetch_one(&pool)
        .await
        .unwrap();
        let excused = NOT_COVERED.iter().any(|(name, _)| name == table);
        match (has_rows, excused) {
            (false, false) => empty.push(table.clone()),
            (true, true) => panic!("{table} has rows now; take it off NOT_COVERED"),
            _ => {}
        }
    }
    assert!(
        empty.is_empty(),
        "{tag}'s tables without a row in ROWS: {empty:?}; add one, or say in NOT_COVERED why not"
    );
    for table in ONLY_WRITTEN {
        assert!(
            tables.contains(&(*table).to_owned()),
            "ONLY_WRITTEN names {table}"
        );
    }
    eprintln!(
        "{} tables of {tag}, all with a row; {} of them only written, not read back",
        tables.len(),
        ONLY_WRITTEN.len()
    );
}
