//! Uploaded avatars (#208): stored and served by the API, replaced objects
//! deleted, foreign upload paths refused on `PATCH /api/me`.

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;

use common::Client;
use gelabber_api::AppState;

/// A 64×64 PNG header (signature + IHDR): enough for the API's size check.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x40\0\0\0\x40\x08\x06\0\0\0\0\0\0\0";
/// A 64×64 lossy WebP header (`VP8 ` key frame).
const WEBP: &[u8] = b"RIFF\x16\0\0\0WEBPVP8 \x0a\0\0\0\x10\x02\x00\x9d\x01\x2a\x40\x00\x40\x00";

async fn signed_in(state: &AppState, email: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, "password123", "Ada").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_serves_bytes_and_replaces_the_old_object(pool: PgPool) {
    let mut ada = signed_in(&common::state(pool), "ada@example.com").await;

    let (status, first) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_url = first["avatar_url"].as_str().unwrap().to_owned();
    assert!(first_url.starts_with(&format!("/api/avatars/{}/", ada.user_id())));

    let (status, headers, bytes) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["content-type"], "image/png");
    assert!(
        headers["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");

    // Saving the profile form sends the uploaded path back unchanged.
    let res = ada
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "name": "Ada L", "avatar_url": first_url })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["avatar_url"], first_url.as_str());
    let (status, _, bytes) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::OK, "an echo must not delete the upload");
    assert_eq!(bytes, PNG);

    let (status, second) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/webp", WEBP.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let second_url = second["avatar_url"].as_str().unwrap().to_owned();
    assert_ne!(second_url, first_url);
    let (status, _, _) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "replaced avatar is deleted");

    // A stale form (other tab) echoing the earlier upload must neither
    // point at the deleted object nor delete the current one.
    let res = ada
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "name": "Ada S", "avatar_url": first_url })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["avatar_url"], "stale");
    let (status, _, bytes) = ada.send_raw(Method::GET, &second_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, WEBP);

    // Switching to an external URL deletes the upload as well.
    let res = ada
        .send(
            Method::PATCH,
            "/api/me",
            Some(json!({ "avatar_url": "https://cdn.example/a.png" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let (status, _, _) = ada.send_raw(Method::GET, &second_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn refuses_non_images_strangers_and_foreign_paths(pool: PgPool) {
    // One state: the in-memory store is per state.
    let state = common::state(pool);
    let mut ada = signed_in(&state, "ada@example.com").await;
    let (status, _) = ada
        .send_bytes(
            Method::PUT,
            "/api/me/avatar",
            "image/png",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = ada
        .send_bytes(
            Method::PUT,
            "/api/me/avatar",
            "image/png",
            [PNG, &vec![0; 512 * 1024]].concat(),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    let (_, uploaded) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    let url = uploaded["avatar_url"].as_str().unwrap().to_owned();

    let mut bob = signed_in(&state, "bob@example.com").await;
    let res = bob
        .send(Method::PATCH, "/api/me", Some(json!({ "avatar_url": url })))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    // Other signed-in users see it; signed-out requests do not.
    let (status, _, _) = bob.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::OK);
    let mut anonymous = Client::from_state(state);
    let (status, _, _) = anonymous.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = bob
        .send_raw(Method::GET, "/api/avatars/not-a-uuid/x", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn cleanup_rows(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT object_key FROM storage_cleanup ORDER BY object_key")
        .fetch_all(pool)
        .await
        .unwrap()
}

fn key_of(url: &str) -> String {
    format!("avatars/{}", url.strip_prefix("/api/avatars/").unwrap())
}

#[sqlx::test(migrations = "./migrations")]
async fn clearing_an_uploaded_avatar_deletes_it(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com").await;
    let (status, uploaded) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    let url = uploaded["avatar_url"].as_str().unwrap().to_owned();
    assert!(
        cleanup_rows(&pool).await.is_empty(),
        "saved upload has no pending cleanup"
    );

    let res = ada
        .send(Method::PATCH, "/api/me", Some(json!({ "avatar_url": "" })))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(res.body["avatar_url"].is_null());
    let (status, _, _) = ada.send_raw(Method::GET, &url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(cleanup_rows(&pool).await.is_empty());
}

/// Two overlapping writes by the same user both read avatar A at request
/// start. The first commits upload B; the second (here: clearing the
/// avatar) must delete B, which its UPDATE really replaced, and not A
/// again, else B stays in the store unreferenced.
#[sqlx::test(migrations = "./migrations")]
async fn overlapping_writes_delete_what_they_really_replaced(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com").await;
    let id = uuid::Uuid::parse_str(ada.user_id()).unwrap();
    let (_, first) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    let a = first["avatar_url"].as_str().unwrap().to_owned();

    // The other request's upload commits between our read and our UPDATE.
    let b = format!("/api/avatars/{id}/{}", uuid::Uuid::new_v4());
    state
        .store
        .put(&key_of(&b), "image/webp", WEBP.to_vec())
        .await
        .unwrap();
    sqlx::query("UPDATE users SET avatar_url=$2 WHERE id=$1")
        .bind(id)
        .bind(&b)
        .execute(&pool)
        .await
        .unwrap();

    let patch = gelabber_api::auth::user::ProfilePatch {
        name: None,
        avatar_url: Some(None),
        ..Default::default()
    };
    let updated = gelabber_api::avatars::save_profile(&state, id, &patch, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.avatar_url, None);
    let (status, _, _) = ada.send_raw(Method::GET, &b, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the replaced upload is gone");
    let (status, _, _) = ada.send_raw(Method::GET, &a, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "A is not this write's to delete (its own replacer handles it)"
    );
    assert!(cleanup_rows(&pool).await.is_empty());
}

/// A request that dies after the PUT (client abort, crash) leaves its
/// pending cleanup row; the worker deletes the orphan once it is due. A
/// late profile update for it is then refused instead of pointing at a
/// deleted object.
#[sqlx::test(migrations = "./migrations")]
async fn abandoned_upload_is_removed_by_the_cleanup_worker(pool: PgPool) {
    let state = common::state(pool.clone());
    let ada = signed_in(&state, "ada@example.com").await;
    let id = uuid::Uuid::parse_str(ada.user_id()).unwrap();
    let url = format!("/api/avatars/{id}/{}", uuid::Uuid::new_v4());
    let key = key_of(&url);
    // What `PUT /api/me/avatar` records before storing the bytes.
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, next_attempt, retain_until) \
         VALUES ($1, now() + interval '10 minutes', now() + interval '10 minutes')",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .unwrap();
    state
        .store
        .put(&key, "image/png", PNG.to_vec())
        .await
        .unwrap();

    gelabber_api::attachments::cleanup::delete_pending(&state, 32)
        .await
        .unwrap();
    assert!(state.store.head(&key).await.is_ok(), "not due yet");

    sqlx::query("UPDATE storage_cleanup SET next_attempt=now(), retain_until=now()")
        .execute(&pool)
        .await
        .unwrap();
    gelabber_api::attachments::cleanup::delete_pending(&state, 32)
        .await
        .unwrap();
    assert!(state.store.head(&key).await.is_err(), "orphan deleted");
    assert!(cleanup_rows(&pool).await.is_empty());

    let patch = gelabber_api::auth::user::ProfilePatch {
        name: None,
        avatar_url: Some(Some(url)),
        ..Default::default()
    };
    assert!(
        gelabber_api::avatars::save_profile(&state, id, &patch, Some(&key))
            .await
            .is_err()
    );
    let avatar: Option<String> = sqlx::query_scalar("SELECT avatar_url FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(avatar, None);
}

/// Waits until some other session of this test database blocks on a lock.
async fn until_a_session_waits_for_a_lock(pool: &PgPool) {
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the profile write never waited for the row lock");
}

/// The same overlap with real concurrency: the other write (upload B) holds
/// the row lock while our clear starts. Our statement must read the
/// predecessor under its own lock, i.e. after B commits, and delete B. An
/// unlocked read sees A from the statement snapshot and deletes A instead.
#[sqlx::test(migrations = "./migrations")]
async fn concurrent_write_reads_its_predecessor_under_the_row_lock(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com").await;
    let id = uuid::Uuid::parse_str(ada.user_id()).unwrap();
    let (_, first) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    let a = first["avatar_url"].as_str().unwrap().to_owned();
    let b = format!("/api/avatars/{id}/{}", uuid::Uuid::new_v4());
    state
        .store
        .put(&key_of(&b), "image/webp", WEBP.to_vec())
        .await
        .unwrap();

    let mut other = pool.begin().await.unwrap();
    sqlx::query("UPDATE users SET avatar_url=$2 WHERE id=$1")
        .bind(id)
        .bind(&b)
        .execute(&mut *other)
        .await
        .unwrap();
    let writer = state.clone();
    let clearing = tokio::spawn(async move {
        let patch = gelabber_api::auth::user::ProfilePatch {
            name: None,
            avatar_url: Some(None),
            ..Default::default()
        };
        gelabber_api::avatars::save_profile(&writer, id, &patch, None).await
    });
    until_a_session_waits_for_a_lock(&pool).await;
    other.commit().await.unwrap();
    let updated = clearing.await.unwrap().unwrap().unwrap();
    assert_eq!(updated.avatar_url, None);

    let (status, _, _) = ada.send_raw(Method::GET, &b, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "B was really replaced");
    let (status, _, _) = ada.send_raw(Method::GET, &a, None).await;
    assert_eq!(status, StatusCode::OK, "A was not this write's predecessor");
    assert!(cleanup_rows(&pool).await.is_empty());
}

/// The banner analogue: an upload holding the row lock commits banner B
/// while our clear waits; the clear must delete B, not the earlier A.
#[sqlx::test(migrations = "./migrations")]
async fn concurrent_banner_write_reads_its_predecessor_under_the_row_lock(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com").await;
    let id = uuid::Uuid::parse_str(ada.user_id()).unwrap();
    let (_, first) = ada
        .send_bytes(Method::PUT, "/api/me/banner", "image/png", PNG.to_vec())
        .await;
    let a = first["banner_url"].as_str().unwrap().to_owned();
    let file = uuid::Uuid::new_v4();
    let b = format!("/api/banners/{id}/{file}");
    state
        .store
        .put(&format!("banners/{id}/{file}"), "image/webp", WEBP.to_vec())
        .await
        .unwrap();

    let mut other = pool.begin().await.unwrap();
    sqlx::query("UPDATE users SET banner_url=$2 WHERE id=$1")
        .bind(id)
        .bind(&b)
        .execute(&mut *other)
        .await
        .unwrap();
    let writer = state.clone();
    let clearing = tokio::spawn(async move {
        let patch = gelabber_api::auth::user::ProfilePatch {
            banner_url: Some(None),
            ..Default::default()
        };
        gelabber_api::avatars::save_profile(&writer, id, &patch, None).await
    });
    until_a_session_waits_for_a_lock(&pool).await;
    other.commit().await.unwrap();
    let updated = clearing.await.unwrap().unwrap().unwrap();
    assert_eq!(updated.card.banner_url, None);

    let (status, _, _) = ada.send_raw(Method::GET, &b, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "B was really replaced");
    let (status, _, _) = ada.send_raw(Method::GET, &a, None).await;
    assert_eq!(status, StatusCode::OK, "A was not this write's predecessor");
    assert!(cleanup_rows(&pool).await.is_empty());
}

/// A foreign-key check (e.g. inserting a message by this user) holds a
/// key-share lock on the user row. A profile write must not wait for it:
/// the row lock is `FOR NO KEY UPDATE`, not `FOR UPDATE`.
#[sqlx::test(migrations = "./migrations")]
async fn profile_write_does_not_wait_for_foreign_key_share_locks(pool: PgPool) {
    let state = common::state(pool.clone());
    let ada = signed_in(&state, "ada@example.com").await;
    let id = uuid::Uuid::parse_str(ada.user_id()).unwrap();

    let mut fk_check = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM users WHERE id=$1 FOR KEY SHARE")
        .bind(id)
        .execute(&mut *fk_check)
        .await
        .unwrap();
    let patch = gelabber_api::auth::user::ProfilePatch {
        name: Some("Ada L".into()),
        avatar_url: None,
        ..Default::default()
    };
    let updated = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        gelabber_api::avatars::save_profile(&state, id, &patch, None),
    )
    .await
    .expect("profile write blocked behind a key-share lock")
    .unwrap()
    .unwrap();
    assert_eq!(updated.name, "Ada L");
    fk_check.rollback().await.unwrap();
}

/// The worker has claimed the abandoned upload's row (and may be deleting
/// the object) but has not removed the row yet. A late profile save must not
/// withdraw that row and point the profile at the object being deleted.
#[sqlx::test(migrations = "./migrations")]
async fn late_save_is_refused_once_the_worker_claimed_the_upload(pool: PgPool) {
    let state = common::state(pool.clone());
    let ada = signed_in(&state, "ada@example.com").await;
    let id = uuid::Uuid::parse_str(ada.user_id()).unwrap();
    let url = format!("/api/avatars/{id}/{}", uuid::Uuid::new_v4());
    let key = key_of(&url);
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, next_attempt, retain_until) \
         VALUES ($1, now(), now())",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .unwrap();
    state
        .store
        .put(&key, "image/png", PNG.to_vec())
        .await
        .unwrap();
    // What the worker's claim statement does before its S3 delete.
    sqlx::query(
        "UPDATE storage_cleanup SET claim=gen_random_uuid(), \
         claimed_until=now()+interval '6 minutes', attempts=attempts+1 \
         WHERE object_key=$1",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .unwrap();

    let patch = gelabber_api::auth::user::ProfilePatch {
        name: None,
        avatar_url: Some(Some(url)),
        ..Default::default()
    };
    assert!(
        gelabber_api::avatars::save_profile(&state, id, &patch, Some(&key))
            .await
            .is_err(),
        "a claimed upload is not the profile's to keep"
    );
    let avatar: Option<String> = sqlx::query_scalar("SELECT avatar_url FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(avatar, None);
    assert_eq!(
        cleanup_rows(&pool).await,
        vec![key],
        "the worker's claimed row stays"
    );
}

/// A PUT that fails (or only times out on our side) may still complete in
/// the store later. The pending cleanup row must stay so the worker removes
/// that object; an immediate delete that also withdrew the row would leak it.
#[sqlx::test(migrations = "./migrations")]
async fn failed_put_keeps_its_pending_cleanup(pool: PgPool) {
    use axum::body::Body;
    use axum::http::Request;
    // Fake S3: every PUT fails, every DELETE (of a not yet stored object)
    // succeeds.
    let router = axum::Router::new().fallback(|request: Request<Body>| async move {
        if request.method() == Method::PUT {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::NO_CONTENT
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut state = common::state(pool.clone());
    let mut ada = signed_in(&state, "ada@example.com").await;
    state.store =
        gelabber_api::storage::ObjectStore::from_minio(Some(&gelabber_api::config::MinioConfig {
            endpoint: endpoint.clone(),
            public_endpoint: endpoint,
            access_key: "test".into(),
            secret_key: "test".into(),
            bucket: "test".into(),
        }))
        .unwrap();
    // Same session cookies, now against the failing store.
    ada.app = gelabber_api::app(state.clone());
    ada.store = state.store.clone();

    let (status, body) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let rows = cleanup_rows(&pool).await;
    assert_eq!(rows.len(), 1, "pending cleanup kept: {rows:?}");
    assert!(rows[0].starts_with(&format!("avatars/{}/", ada.user_id())));
    let due: bool = sqlx::query_scalar(
        "SELECT next_attempt > now() + interval '9 minutes' FROM storage_cleanup",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(due, "deleted only after the PUT can no longer land");
    task.abort();
}

/// The same flow against the local MinIO test service (CI starts it). The
/// public endpoint is unreachable on purpose: avatars must only use the
/// internal one.
#[sqlx::test(migrations = "./migrations")]
async fn real_minio_avatar_upload_fetch_and_replace(pool: PgPool) {
    let endpoint = std::env::var("MINIO_ENDPOINT").expect("source local minio-test.env");
    assert!(
        endpoint.starts_with("http://127.0.0.1:"),
        "only the local test service is allowed"
    );
    let config = gelabber_api::config::MinioConfig {
        endpoint,
        public_endpoint: "http://127.0.0.1:1".into(),
        access_key: std::env::var("MINIO_ROOT_USER").expect("local test username"),
        secret_key: std::env::var("MINIO_ROOT_PASSWORD").expect("local test password"),
        bucket: format!("gb-test-{}", uuid::Uuid::new_v4().simple()),
    };
    let mut state = common::state(pool);
    state.store = gelabber_api::storage::ObjectStore::from_minio(Some(&config)).unwrap();
    state.store.ensure_ready().await.unwrap();
    let mut ada = signed_in(&state, "ada@example.com").await;

    let (status, first) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/png", PNG.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_url = first["avatar_url"].as_str().unwrap().to_owned();
    let (status, headers, bytes) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers["content-type"], "image/png");

    let (status, second) = ada
        .send_bytes(Method::PUT, "/api/me/avatar", "image/webp", WEBP.to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let (status, _, bytes) = ada
        .send_raw(Method::GET, second["avatar_url"].as_str().unwrap(), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, WEBP);
    let (status, _, _) = ada.send_raw(Method::GET, &first_url, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
