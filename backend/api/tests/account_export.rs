//! Data export (#519): `/api/me/export`, its worker and the archive.

mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::time::Duration;

use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{Method, StatusCode};
use gelabber_api::AppState;
use gelabber_api::account::export;
use gelabber_api::mail::Mailer;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "password123";
/// Columns of an export the app's own worker may have built meanwhile, as
/// they are before it is built (the trigger queues a replaced archive).
const UNBUILT: &str = "claim = NULL, claimed_until = NULL, finished_at = NULL, \
                       expires_at = NULL, object_key = NULL, size_bytes = NULL";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x40\0\0\0\x40\x08\x06\0\0\0\0\0\0\0";

async fn signed_up(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

fn id_of(value: &Value) -> String {
    value["id"].as_str().unwrap().to_owned()
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}

async fn request(client: &mut Client) -> common::Response {
    client.send(Method::POST, "/api/me/export", None).await
}

async fn status(client: &mut Client) -> Value {
    let res = client.send(Method::GET, "/api/me/export", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

async fn post(client: &mut Client, channel_id: &str, body: Value) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/messages"),
            Some(body),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

/// Runs the worker until the account's export has left `pending` and
/// `running` (the app's own worker may get there first).
async fn built(state: &AppState, user_id: Uuid) -> String {
    for _ in 0..500 {
        export::run_once(state).await.unwrap();
        let current: String =
            sqlx::query_scalar("SELECT state FROM account_exports WHERE user_id = $1")
                .bind(user_id)
                .fetch_one(&state.db)
                .await
                .unwrap();
        if current != "pending" && current != "running" {
            return current;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the export of {user_id} was not built");
}

/// The archive through the download route, entry by entry.
async fn download(client: &mut Client) -> BTreeMap<String, Vec<u8>> {
    let (status, headers, bytes) = client
        .send_raw(Method::GET, "/api/me/export/download", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[CONTENT_TYPE], "application/zip");
    let disposition = headers[CONTENT_DISPOSITION].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment; filename=\"gelabber-export-")
            && disposition.ends_with(".zip\""),
        "{disposition}"
    );
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip archive");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        entries.insert(entry.name().unwrap().into_owned(), content);
    }
    entries
}

fn json_entry(entries: &BTreeMap<String, Vec<u8>>, name: &str) -> Value {
    serde_json::from_slice(
        entries
            .get(name)
            .unwrap_or_else(|| panic!("{name} missing")),
    )
    .unwrap()
}

#[sqlx::test]
async fn a_request_is_queued_once_and_needs_a_session(pool: PgPool) {
    let state = common::state(pool);
    let mut anon = Client::from_state(state.clone());
    anon.bootstrap().await;
    let res = anon.send(Method::GET, "/api/me/export", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    assert_eq!(
        status(&mut ada).await,
        json!({ "export": null, "mail": false })
    );
    let res = ada.send(Method::GET, "/api/me/export/download", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // Like every mutation, it needs the CSRF token.
    let token = ada.csrf.take();
    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    ada.csrf = token;

    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(res.body["mail"], false);
    let queued = &res.body["export"];
    assert_eq!(queued["state"], "pending", "{queued}");
    assert!(queued["requested_at"].is_string());
    for field in ["finished_at", "expires_at", "size_bytes"] {
        assert!(queued[field].is_null(), "{field}");
    }

    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "export_pending");
    // While it is built, too.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE account_exports SET state = 'running', {UNBUILT}"
    )))
    .execute(&state.db)
    .await
    .unwrap();
    let res = request(&mut ada).await;
    assert_eq!(res.body["error"], "export_pending");
}

#[sqlx::test]
async fn the_archive_holds_what_the_person_wrote_and_nothing_of_others(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let mut bob = signed_up(&state, "bob@example.com", "Bob").await;
    let ada_id = uuid(ada.user_id());

    // A server of Bob's that Ada joins and writes in.
    let res = bob
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let server = res.body;
    let server_id = id_of(&server);
    let channel_id = server["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .map(id_of)
        .unwrap();
    let res = bob
        .send(
            Method::POST,
            &format!("/api/servers/{server_id}/invites"),
            Some(json!({})),
        )
        .await;
    let code = res.body["code"].as_str().unwrap().to_owned();
    let res = ada
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let (uploaded, _) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(uploaded, StatusCode::OK);

    let signed = ada
        .send(
            Method::POST,
            &format!("/api/channels/{channel_id}/attachments"),
            Some(json!({ "filename": "cat.png", "content_type": "image/png", "size": PNG.len() })),
        )
        .await;
    assert_eq!(signed.status, StatusCode::CREATED, "{}", signed.body);
    let file_id = id_of(&signed.body);
    ada.store
        .put(&format!("att/{file_id}"), "image/png", PNG.to_vec())
        .await
        .unwrap();
    // A name that would leave its folder, as an older client may have stored.
    sqlx::query("UPDATE attachments SET filename = '../sub\\cat.png' WHERE id = $1")
        .bind(uuid(&file_id))
        .execute(&pool)
        .await
        .unwrap();
    let first = post(&mut ada, &channel_id, json!({ "content": "erste" })).await;
    let bobs = post(
        &mut bob,
        &channel_id,
        json!({ "content": "geheim von bob" }),
    )
    .await;
    let second = post(
        &mut ada,
        &channel_id,
        json!({ "content": "mit bild", "attachment_ids": [file_id], "reply_to": id_of(&bobs) }),
    )
    .await;
    let res = ada
        .send(
            Method::PUT,
            &format!("/api/messages/{}/reactions/%F0%9F%91%8D", id_of(&bobs)),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let res = ada
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": bob.user_id() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let dm = id_of(&res.body);
    let in_dm = post(&mut ada, &dm, json!({ "content": "hallo bob" })).await;
    post(&mut bob, &dm, json!({ "content": "bobs antwort" })).await;

    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(built(&state, ada_id).await, "ready");

    let view = status(&mut ada).await;
    let export = &view["export"];
    assert_eq!(export["state"], "ready", "{view}");
    assert!(export["finished_at"].is_string() && export["expires_at"].is_string());

    let entries = download(&mut ada).await;
    assert_eq!(
        export["size_bytes"].as_u64().unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT size_bytes FROM account_exports")
            .fetch_one(&pool)
            .await
            .unwrap() as u64
    );
    let server_file = format!("messages/servers/{server_id}/{channel_id}.json");
    let dm_file = format!("messages/dms/{dm}.json");
    let attachment_path = format!("files/{file_id}/_sub_cat.png");
    let names: Vec<&str> = entries.keys().map(String::as_str).collect();
    let mut expected = vec![
        "README.txt",
        "account.json",
        "profile/avatar.png",
        server_file.as_str(),
        attachment_path.as_str(),
        dm_file.as_str(),
        "reactions.json",
        "feedback.json",
    ];
    expected.sort();
    assert_eq!(names, expected);

    // Other people's messages appear nowhere.
    for (name, content) in &entries {
        let text = String::from_utf8_lossy(content);
        assert!(!text.contains("geheim von bob"), "{name}");
        assert!(!text.contains("bobs antwort"), "{name}");
        assert!(!text.contains("password"), "{name}");
    }

    assert!(String::from_utf8_lossy(&entries["README.txt"]).starts_with("Gelabber-Datenexport"));
    assert_eq!(entries["profile/avatar.png"], PNG);
    assert_eq!(entries[&attachment_path], PNG);

    let account = json_entry(&entries, "account.json");
    assert_eq!(account["id"], ada.user_id());
    assert_eq!(account["email"], "ada@example.com");
    assert_eq!(account["name"], "Ada");
    assert_eq!(account["locale"], "de");
    assert_eq!(account["servers"].as_array().unwrap().len(), 1);
    assert_eq!(account["servers"][0]["id"], server_id.as_str());
    assert_eq!(account["servers"][0]["owner"], false);
    assert!(account["sessions"][0]["client"].is_string());
    assert!(account["sessions"][0].get("token_hash").is_none());
    assert!(account.get("password_hash").is_none());
    assert_eq!(account["invites"], json!([]));

    let channel = json_entry(&entries, &server_file);
    assert_eq!(channel["channel"]["id"], channel_id.as_str());
    assert_eq!(channel["channel"]["kind"], "text");
    assert_eq!(channel["channel"]["server"]["id"], server_id.as_str());
    assert_eq!(channel["channel"]["server"]["name"], "Team");
    assert!(channel["channel"]["peer"].is_null());
    let messages = channel["messages"].as_array().unwrap();
    let ids: Vec<&str> = messages.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [id_of(&first), id_of(&second)]);
    assert_eq!(messages[0]["content"], "erste");
    assert_eq!(messages[0]["attachments"], json!([]));
    assert_eq!(messages[1]["reply_to"], id_of(&bobs).as_str());
    assert_eq!(
        messages[1]["attachments"],
        json!([{
            "id": file_id,
            "filename": "../sub\\cat.png",
            "content_type": "image/png",
            "size_bytes": PNG.len(),
            "path": attachment_path,
        }])
    );

    let dm_channel = json_entry(&entries, &dm_file);
    assert_eq!(dm_channel["channel"]["kind"], "dm");
    assert!(dm_channel["channel"]["server"].is_null());
    assert_eq!(dm_channel["channel"]["peer"]["id"], bob.user_id());
    assert_eq!(dm_channel["channel"]["peer"]["name"], "Bob");
    let dm_messages = dm_channel["messages"].as_array().unwrap();
    assert_eq!(dm_messages.len(), 1);
    assert_eq!(dm_messages[0]["id"], id_of(&in_dm).as_str());

    assert_eq!(
        json_entry(&entries, "reactions.json"),
        json!([{ "message_id": id_of(&bobs), "channel_id": channel_id, "emoji": "👍" }])
    );
}

#[sqlx::test]
async fn a_ready_export_waits_a_day_and_a_failed_one_does_not(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = uuid(ada.user_id());
    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(built(&state, ada_id).await, "ready");

    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
    assert_eq!(res.body["error"], "rate_limited");
    let wait: u64 = res.headers[RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!((24 * 3600 - 60..=24 * 3600).contains(&wait), "{wait}");

    // A day later the next request replaces it, and its archive goes.
    let key: String = sqlx::query_scalar(
        "UPDATE account_exports SET requested_at = now() - interval '25 hours' \
         RETURNING object_key",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let queued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM storage_cleanup WHERE object_key = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(queued, 1);

    // A failed export can be requested again at once.
    sqlx::query(
        "UPDATE account_exports SET state = 'failed', claim = NULL, claimed_until = NULL, \
         finished_at = now()",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(status(&mut ada).await["export"]["state"], "failed");
    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(res.body["export"]["state"], "pending");
}

#[sqlx::test]
async fn an_expired_export_is_swept_and_its_archive_queued(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = uuid(ada.user_id());
    request(&mut ada).await;
    assert_eq!(built(&state, ada_id).await, "ready");
    let key: String = sqlx::query_scalar(
        "UPDATE account_exports SET expires_at = now() - interval '1 second' \
         RETURNING object_key",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(key.starts_with(&format!("exports/{ada_id}/")), "{key}");
    assert!(key.ends_with(".zip"), "{key}");

    // Past its time it is neither shown nor served, also before the sweep.
    assert_eq!(status(&mut ada).await["export"], Value::Null);
    let res = ada.send(Method::GET, "/api/me/export/download", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    export::sweep(&state).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM account_exports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    let queued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM storage_cleanup WHERE object_key = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(queued, 1);
    // A new one can be requested at once.
    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
}

#[sqlx::test]
async fn the_retention_setting_decides_how_long_it_stays(pool: PgPool) {
    let mut state = common::state(pool.clone());
    state.export_retention = Duration::from_secs(3 * 3600);
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = uuid(ada.user_id());
    request(&mut ada).await;
    assert_eq!(built(&state, ada_id).await, "ready");
    let hours: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM expires_at - finished_at)::float8 / 3600 FROM account_exports",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((hours - 3.0).abs() < 0.01, "{hours}");
}

/// Ada with a server of her own and one message in it; returns the
/// message's id and channel.
async fn wrote_once(state: &AppState) -> (Client, Uuid, Uuid, Uuid) {
    let mut ada = signed_up(state, "ada@example.com", "Ada").await;
    let ada_id = uuid(ada.user_id());
    let res = ada
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let channel_id = res.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .map(id_of)
        .unwrap();
    let message = post(
        &mut ada,
        &channel_id,
        json!({ "content": "x".repeat(2000) }),
    )
    .await;
    (ada, ada_id, uuid(&id_of(&message)), uuid(&channel_id))
}

async fn attempts(pool: &PgPool) -> i32 {
    sqlx::query_scalar("SELECT attempts FROM account_exports")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// An archive that would pass `API_EXPORT_MAX_BYTES` fails at once rather
/// than being tried again: the files' sizes are summed before the build.
#[sqlx::test]
async fn files_past_the_size_limit_fail_the_export_at_once(pool: PgPool) {
    let mut state = common::state(pool.clone());
    state.export_max_bytes = 100_000;
    let (mut ada, ada_id, message_id, channel_id) = wrote_once(&state).await;
    sqlx::query(
        "INSERT INTO attachments (message_id, channel_id, uploader_id, object_key, filename, \
             content_type, size_bytes) \
         VALUES ($1, $2, $3, 'att/big', 'big.bin', 'application/octet-stream', 100001)",
    )
    .bind(message_id)
    .bind(channel_id)
    .bind(ada_id)
    .execute(&pool)
    .await
    .unwrap();
    request(&mut ada).await;
    assert_eq!(built(&state, ada_id).await, "failed");
    assert_eq!(attempts(&pool).await, 1);
    assert_eq!(status(&mut ada).await["export"]["state"], "failed");
}

/// The archive file itself refuses to grow past the limit, for messages
/// as much as for files.
#[sqlx::test]
async fn an_archive_growing_past_the_size_limit_fails_at_once(pool: PgPool) {
    let mut state = common::state(pool.clone());
    state.export_max_bytes = 2000;
    let (mut ada, ada_id, _, _) = wrote_once(&state).await;
    request(&mut ada).await;
    assert_eq!(built(&state, ada_id).await, "failed");
    assert_eq!(attempts(&pool).await, 1);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_cleanup")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "nothing was uploaded");
}

#[sqlx::test]
async fn the_ready_mail_goes_out_only_with_mail_on(pool: PgPool) {
    // Mail off: nothing to send, and the page polls instead.
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    request(&mut ada).await;
    assert_eq!(built(&state, uuid(ada.user_id())).await, "ready");
    assert!(state.mail.is_none());

    let mut state = common::state(pool.clone());
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    let mut bob = Client::from_state(state.clone());
    bob.bootstrap().await;
    let res = bob.register("bob@example.com", PASSWORD, "Bob").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    sqlx::query(
        "UPDATE users SET email_verified_at = now(), registration_pending = false, locale = 'en' \
         WHERE email = 'bob@example.com'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let res = bob.login("bob@example.com", PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let before = mailer.outbox().len();
    let res = request(&mut bob).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    assert_eq!(res.body["mail"], true);
    assert_eq!(built(&state, uuid(bob.user_id())).await, "ready");
    for _ in 0..200 {
        if mailer.outbox().len() > before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let sent = mailer.outbox();
    assert_eq!(sent.len(), before + 1, "{sent:?}");
    let last = sent.last().unwrap();
    assert_eq!(last.to, "bob@example.com");
    assert_eq!(last.subject, "Your Gelabber data export is ready");
    assert!(
        last.body
            .contains("\n\nhttps://chat.example.org/settings/konto\n\n"),
        "{}",
        last.body
    );
}

#[sqlx::test]
async fn an_account_under_deletion_is_not_built(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = uuid(ada.user_id());
    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    // Hold the purge's lock on the account, so the row stays while the
    // test looks at it; the account is marked as phase one marks it, with
    // the export row kept.
    let mut purge_lock = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(hashtext('account-purge'), hashtext($1::text))")
        .bind(ada_id)
        .execute(&mut *purge_lock)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(ada_id)
        .execute(&pool)
        .await
        .unwrap();
    // The app's own worker may have claimed it before the mark; reset that.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE account_exports SET state = 'pending', {UNBUILT}"
    )))
    .execute(&pool)
    .await
    .unwrap();

    assert!(!export::run_once(&state).await.unwrap());
    let current: String =
        sqlx::query_scalar("SELECT state FROM account_exports WHERE user_id = $1")
            .bind(ada_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(current, "pending");
    sqlx::query("SELECT pg_advisory_unlock(hashtext('account-purge'), hashtext($1::text))")
        .bind(ada_id)
        .execute(&mut *purge_lock)
        .await
        .unwrap();
}

#[sqlx::test]
async fn an_export_claimed_too_often_fails(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = uuid(ada.user_id());
    request(&mut ada).await;
    // Three claims ran out without a result (a process died each time).
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE account_exports SET state = 'running', attempts = $1, {UNBUILT}"
    )))
    .bind(export::ATTEMPTS)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE account_exports SET claimed_until = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(built(&state, ada_id).await, "failed");
    let view = status(&mut ada).await;
    assert_eq!(view["export"]["state"], "failed", "{view}");
    assert!(view["export"]["finished_at"].is_string());
    let res = request(&mut ada).await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
}

/// A feedback entry of `user` written straight to the table, with an image
/// under `key` when given.
async fn feedback_entry(pool: &PgPool, user: Uuid, text: &str, image: Option<(&str, i32)>) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO feedback (kind, user_id, client, os, app_version, locale, page, body, \
             image_key, image_type, image_bytes, created_at) \
         VALUES ('problem', $1, 'desktop', 'windows', '0.10.0', 'en', '/settings/darstellung', \
             $2, $3, CASE WHEN $3 IS NOT NULL THEN 'image/png' END, $4, clock_timestamp()) \
         RETURNING id",
    )
    .bind(user)
    .bind(text)
    .bind(image.map(|(key, _)| key))
    .bind(image.map(|(_, size)| size))
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn the_archive_holds_the_persons_feedback_with_its_replies(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let lena = signed_up(&state, "lena@example.com", "Lena").await;
    let ada_id = uuid(ada.user_id());
    let lena_id = uuid(lena.user_id());

    let key = format!("feedback/{ada_id}/{}", Uuid::new_v4());
    state
        .store
        .put(&key, "image/png", PNG.to_vec())
        .await
        .unwrap();
    let with_image = feedback_entry(&pool, ada_id, "Es hakt", Some((&key, PNG.len() as i32))).await;
    let plain = feedback_entry(&pool, ada_id, "Toll", None).await;
    feedback_entry(&pool, lena_id, "Not Ada's", None).await;
    for (author_kind, author, text) in [
        ("operator", lena_id, "Danke, schauen wir uns an"),
        ("person", ada_id, "Gerne"),
    ] {
        sqlx::query(
            "INSERT INTO feedback_messages (feedback_id, author_kind, author_id, body, created_at) \
             VALUES ($1, $2, $3, $4, clock_timestamp())",
        )
        .bind(with_image)
        .bind(author_kind)
        .bind(author)
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
    }

    request(&mut ada).await;
    assert_eq!(built(&state, ada_id).await, "ready");
    let entries = download(&mut ada).await;
    assert_eq!(entries[&format!("feedback/{with_image}.png")], PNG);
    let readme = String::from_utf8(entries["README.txt"].clone()).unwrap();
    assert!(readme.contains("feedback.json / feedback/"), "{readme}");
    let feedback = json_entry(&entries, "feedback.json");
    assert_eq!(
        feedback,
        json!([
            {
                "id": with_image,
                "kind": "problem",
                "status": "open",
                "created_at": feedback[0]["created_at"],
                "text": "Es hakt",
                "context": {
                    "client": "desktop",
                    "os": "windows",
                    "app_version": "0.10.0",
                    "locale": "en",
                    "page": "/settings/darstellung",
                },
                "image": format!("feedback/{with_image}.png"),
                "messages": [
                    {
                        "from": "operators",
                        "text": "Danke, schauen wir uns an",
                        "created_at": feedback[0]["messages"][0]["created_at"],
                    },
                    {
                        "from": "you",
                        "text": "Gerne",
                        "created_at": feedback[0]["messages"][1]["created_at"],
                    },
                ],
            },
            {
                "id": plain,
                "kind": "problem",
                "status": "open",
                "created_at": feedback[1]["created_at"],
                "text": "Toll",
                "context": {
                    "client": "desktop",
                    "os": "windows",
                    "app_version": "0.10.0",
                    "locale": "en",
                    "page": "/settings/darstellung",
                },
                "image": null,
                "messages": [],
            },
        ])
    );
    assert!(!feedback.to_string().contains("Lena"));
}

/// Feedback images count toward the size check before the build.
#[sqlx::test]
async fn feedback_images_count_toward_the_size_limit(pool: PgPool) {
    let mut state = common::state(pool.clone());
    state.export_max_bytes = 100_000;
    let mut ada = signed_up(&state, "ada@example.com", "Ada").await;
    let ada_id = uuid(ada.user_id());
    let key = format!("feedback/{ada_id}/{}", Uuid::new_v4());
    feedback_entry(&pool, ada_id, "Groß", Some((&key, 100_001))).await;
    request(&mut ada).await;
    assert_eq!(built(&state, ada_id).await, "failed");
    assert_eq!(attempts(&pool).await, 1);
}
