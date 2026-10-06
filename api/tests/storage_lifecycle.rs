//! Real DB quotas/cascades, plus a local S3 failure proxy backed by real MinIO.
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode};
use gelabber_api::attachments::cleanup;
use gelabber_api::storage::{ObjectStore, StoreError};
use gelabber_api::{AppState, Config};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

fn state(pool: PgPool, quota: u64) -> AppState {
    let mut state = common::state(pool);
    state.limits.upload_bytes_per_day = quota;
    state.limits.upload_per_hour = 0;
    state.limits.api_per_min = 0;
    state
}

async fn owner(state: &AppState) -> (Client, Value, String) {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    assert_eq!(
        client
            .register("ada@example.com", "password123", "Ada")
            .await
            .status,
        StatusCode::CREATED
    );
    let server = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({"name":"Storage"})),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED);
    let channel = server.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (client, server.body, channel)
}

async fn presign(client: &mut Client, channel: &str) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({"filename":"x.png","content_type":"image/png","size":4})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    res.body
}

async fn message(client: &mut Client, channel: &str, id: &str) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({"content":"file","attachment_ids":[id]})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    res.body
}

async fn usage(state: &AppState, user: &str) -> (i64, i64) {
    sqlx::query_as("SELECT reserved, consumed FROM upload_daily_usage WHERE uploader_id=$1 AND day=(now() AT TIME ZONE 'UTC')::date")
        .bind(Uuid::parse_str(user).unwrap()).fetch_one(&state.db).await.unwrap()
}

async fn due(state: &AppState) {
    sqlx::query("UPDATE storage_cleanup SET next_attempt=now(), retain_until=now()-interval '1 second', claimed_until=now()-interval '1 second'")
        .execute(&state.db).await.unwrap();
}

#[sqlx::test]
async fn concurrent_reservations_are_atomic_and_deleted_messages_keep_daily_consumption(
    pool: PgPool,
) {
    let state = state(pool, 8);
    let (client, _, channel) = owner(&state).await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let mut seat = Client::from_state(state.clone());
        seat.jar = client.jar.clone();
        seat.csrf = client.csrf.clone();
        let channel = channel.clone();
        tasks.spawn(async move {
            seat.send(
                Method::POST,
                &format!("/api/channels/{channel}/attachments"),
                Some(json!({"filename":"x.png","content_type":"image/png","size":4})),
            )
            .await
        });
    }
    let mut accepted = Vec::new();
    while let Some(res) = tasks.join_next().await {
        let res = res.unwrap();
        if res.status == StatusCode::CREATED {
            accepted.push(res.body);
        } else {
            assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(res.body["error"], "quota_exceeded");
        }
    }
    assert_eq!(accepted.len(), 2);
    assert_eq!(usage(&state, client.user_id()).await, (8, 0));
    let mut client = client;
    for signed in accepted {
        let id = signed["id"].as_str().unwrap();
        state
            .store
            .put(&format!("att/{id}"), "image/png", vec![1, 2, 3, 4])
            .await
            .unwrap();
        let msg = message(&mut client, &channel, id).await;
        assert_eq!(
            client
                .send(
                    Method::DELETE,
                    &format!("/api/messages/{}", msg["id"].as_str().unwrap()),
                    None
                )
                .await
                .status,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(usage(&state, client.user_id()).await, (0, 8));
    let rejected = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({"filename":"x.png","content_type":"image/png","size":1})),
        )
        .await;
    assert_eq!(rejected.status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test]
async fn pending_expiry_refunds_missing_uploads_but_charges_uploaded_bytes(pool: PgPool) {
    let state = state(pool, 8);
    let (mut client, _, channel) = owner(&state).await;
    let absent = presign(&mut client, &channel).await;
    let present = presign(&mut client, &channel).await;
    let present_id = present["id"].as_str().unwrap();
    let key = format!("att/{present_id}");
    state
        .store
        .put(&key, "image/png", vec![1, 2, 3, 4])
        .await
        .unwrap();
    sqlx::query("UPDATE attachments SET expires_at=now()-interval '16 minutes'")
        .execute(&state.db)
        .await
        .unwrap();
    let invalid = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({"attachment_ids":[present_id]})),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    cleanup::expire_pending(&state, 32).await.unwrap();
    cleanup::delete_pending(&state, 32).await.unwrap();
    assert_eq!(usage(&state, client.user_id()).await, (0, 4));
    assert!(matches!(
        state.store.head(&key).await,
        Err(StoreError::NotFound)
    ));
    for signed in [absent, present] {
        let res = client
            .send(
                Method::GET,
                &format!("/api/attachments/{}", signed["id"].as_str().unwrap()),
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM attachments")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        0
    );
    presign(&mut client, &channel).await; // released missing-object reservation is reusable
}

#[sqlx::test]
async fn cascades_enqueue_objects_and_rollback_never_deletes_bytes(pool: PgPool) {
    let state = state(pool.clone(), 0);
    let (mut client, server, channel) = owner(&state).await;
    let signed = presign(&mut client, &channel).await;
    let id = signed["id"].as_str().unwrap();
    let key = format!("att/{id}");
    state
        .store
        .put(&key, "image/png", vec![1, 2, 3, 4])
        .await
        .unwrap();
    let msg = message(&mut client, &channel, id).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM messages WHERE id=$1")
        .bind(Uuid::parse_str(msg["id"].as_str().unwrap()).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    cleanup::delete_pending(&state, 32).await.unwrap();
    assert_eq!(state.store.head(&key).await.unwrap().size, 4);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM storage_cleanup")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        client
            .send(Method::DELETE, &format!("/api/channels/{channel}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    due(&state).await;
    cleanup::delete_pending(&state, 32).await.unwrap();
    assert!(matches!(
        state.store.head(&key).await,
        Err(StoreError::NotFound)
    ));
    // A still-pending upload is also covered by the server cascade.
    let ch = client
        .send(
            Method::POST,
            &format!("/api/servers/{}/channels", server["id"].as_str().unwrap()),
            Some(json!({"name":"files","kind":"text"})),
        )
        .await;
    assert_eq!(ch.status, StatusCode::CREATED);
    let signed = presign(&mut client, ch.body["id"].as_str().unwrap()).await;
    let key = format!("att/{}", signed["id"].as_str().unwrap());
    state
        .store
        .put(&key, "image/png", vec![1, 2, 3, 4])
        .await
        .unwrap();
    assert_eq!(
        client
            .send(
                Method::DELETE,
                &format!("/api/servers/{}", server["id"].as_str().unwrap()),
                None
            )
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    due(&state).await;
    cleanup::delete_pending(&state, 32).await.unwrap();
    assert!(matches!(
        state.store.head(&key).await,
        Err(StoreError::NotFound)
    ));
}

#[sqlx::test]
async fn valid_and_foreign_pending_objects_survive_failed_attachment_bind(pool: PgPool) {
    let state = state(pool, 0);
    let (mut client, server, channel) = owner(&state).await;
    let good = presign(&mut client, &channel).await;
    let bad = presign(&mut client, &channel).await;
    let goodkey = format!("att/{}", good["id"].as_str().unwrap());
    let badkey = format!("att/{}", bad["id"].as_str().unwrap());
    state
        .store
        .put(&goodkey, "image/png", vec![1, 2, 3, 4])
        .await
        .unwrap();
    state
        .store
        .put(&badkey, "image/png", vec![1])
        .await
        .unwrap();
    let failed = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({"attachment_ids":[bad["id"]]})),
        )
        .await;
    assert_eq!(failed.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.store.head(&goodkey).await.unwrap().size, 4);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !matches!(state.store.head(&badkey).await, Err(StoreError::NotFound)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    message(&mut client, &channel, good["id"].as_str().unwrap()).await;
    let mut other = Client::from_state(state.clone());
    other.bootstrap().await;
    assert_eq!(
        other
            .register("bob@example.com", "password123", "Bob")
            .await
            .status,
        StatusCode::CREATED
    );
    let invite = client
        .send(
            Method::POST,
            &format!("/api/servers/{}/invites", server["id"].as_str().unwrap()),
            Some(json!({})),
        )
        .await;
    assert_eq!(
        other
            .send(
                Method::POST,
                &format!(
                    "/api/invites/{}/join",
                    invite.body["code"].as_str().unwrap()
                ),
                None
            )
            .await
            .status,
        StatusCode::OK
    );
    let foreign = presign(&mut other, &channel).await;
    let foreignkey = format!("att/{}", foreign["id"].as_str().unwrap());
    state
        .store
        .put(&foreignkey, "image/png", vec![1])
        .await
        .unwrap();
    let denied = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({"attachment_ids":[foreign["id"]]})),
        )
        .await;
    assert_eq!(denied.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.store.head(&foreignkey).await.unwrap().size, 1);
}

struct Proxy {
    endpoint: String,
    fail_delete: Arc<AtomicBool>,
    deletes: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn proxy(endpoint: String) -> Proxy {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fail_delete = Arc::new(AtomicBool::new(true));
    let deletes = Arc::new(AtomicUsize::new(0));
    let fail = fail_delete.clone();
    let calls = deletes.clone();
    let http = reqwest::Client::new();
    let router = axum::Router::new().fallback(move |request: Request<Body>| {
        let endpoint = endpoint.clone();
        let fail = fail.clone();
        let calls = calls.clone();
        let http = http.clone();
        async move {
            if request.method() == Method::DELETE {
                calls.fetch_add(1, Ordering::SeqCst);
                if fail.load(Ordering::SeqCst) {
                    return Response::builder().status(503).body(Body::empty()).unwrap();
                }
            }
            let (parts, body) = request.into_parts();
            let body = to_bytes(body, 8 << 20).await.unwrap();
            let target = format!(
                "{}{}",
                endpoint.trim_end_matches('/'),
                parts.uri.path_and_query().unwrap()
            );
            let res = http
                .request(parts.method, target)
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .unwrap();
            let status = res.status();
            let headers = res.headers().clone();
            // HEAD has no response body despite the object's Content-Length.
            let bytes = res.bytes().await.unwrap();
            let mut response = Response::builder()
                .status(status)
                .body(Body::from(bytes))
                .unwrap();
            *response.headers_mut() = headers;
            response
        }
    });
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Proxy {
        endpoint: format!("http://{addr}"),
        fail_delete,
        deletes,
        task,
    }
}

fn minio_config(endpoint: &str) -> gelabber_api::config::MinioConfig {
    let local = std::env::var("MINIO_ENDPOINT").expect("source local minio-test.env");
    assert!(
        local.starts_with("http://127.0.0.1:"),
        "only the local test service is allowed"
    );
    gelabber_api::config::MinioConfig {
        endpoint: endpoint.to_owned(),
        public_endpoint: local,
        access_key: std::env::var("MINIO_ROOT_USER").expect("local test username"),
        secret_key: std::env::var("MINIO_ROOT_PASSWORD").expect("local test password"),
        bucket: format!("gb-test-{}", Uuid::new_v4().simple()),
    }
}

#[sqlx::test]
async fn real_minio_failed_delete_retains_job_and_later_retry_removes_only_its_object(
    pool: PgPool,
) {
    let endpoint = std::env::var("MINIO_ENDPOINT").expect("source local minio-test.env");
    let proxy = proxy(endpoint).await;
    let config = minio_config(&proxy.endpoint);
    let mut state = state(pool, 0);
    state.store = ObjectStore::from_minio(Some(&config)).unwrap();
    state.store.ensure_ready().await.unwrap();
    let (mut client, _, channel) = owner(&state).await;
    let signed = presign(&mut client, &channel).await;
    let key = format!("att/{}", signed["id"].as_str().unwrap());
    let foreign = format!("att/{}", Uuid::new_v4());
    state
        .store
        .put(&key, "image/png", vec![1, 2, 3, 4])
        .await
        .unwrap();
    state
        .store
        .put(&foreign, "image/png", vec![5, 6, 7, 8])
        .await
        .unwrap();
    let msg = message(&mut client, &channel, signed["id"].as_str().unwrap()).await;
    let deleted = client
        .send(
            Method::DELETE,
            &format!("/api/messages/{}", msg["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    tokio::time::timeout(Duration::from_secs(3), async {
        while proxy.deletes.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM attachments")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM storage_cleanup")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(state.store.head(&key).await.unwrap().size, 4);
    assert_eq!(state.store.head(&foreign).await.unwrap().size, 4);
    // A fresh store handle can resume the durable job independently.
    let mut restarted = state.clone();
    restarted.store = ObjectStore::from_minio(Some(&config)).unwrap();
    proxy.fail_delete.store(false, Ordering::SeqCst);
    sqlx::query(
        "UPDATE storage_cleanup SET next_attempt=now(),claimed_until=now()-interval '1 second'",
    )
    .execute(&state.db)
    .await
    .unwrap();
    cleanup::delete_pending(&restarted, 32).await.unwrap();
    assert!(matches!(
        state.store.head(&key).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(state.store.head(&foreign).await.unwrap().size, 4);
    // Keep intent while the original PUT URL is valid. Replaying it after
    // message deletion must not leave a permanent orphan behind.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM storage_cleanup")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        1
    );
    let mut request = reqwest::Client::new().put(signed["upload_url"].as_str().unwrap());
    for (name, value) in signed["headers"].as_object().unwrap() {
        request = request.header(name, value.as_str().unwrap());
    }
    let response = request
        .body(vec![1, 2, 3, 4])
        .send()
        .await
        .unwrap_or_else(|_| panic!("local presigned PUT failed"));
    assert!(response.status().is_success());
    assert_eq!(state.store.head(&key).await.unwrap().size, 4);
    due(&restarted).await;
    cleanup::delete_pending(&restarted, 32).await.unwrap();
    assert!(matches!(
        state.store.head(&key).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM storage_cleanup")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        0
    );
    state.store.delete(&foreign).await.unwrap();
}

#[tokio::test]
async fn missing_production_storage_is_rejected_and_memory_requires_explicit_opt_in() {
    let config = Config::from_source(|key| match key {
        "DATABASE_URL" => Some("postgres://gelabber@127.0.0.1:55432/gelabber".to_owned()),
        "REDIS_URL" => Some("redis://127.0.0.1:56379".to_owned()),
        _ => None,
    })
    .unwrap();
    assert!(!config.allow_memory_store);
    assert!(matches!(
        ObjectStore::from_minio(None),
        Err(StoreError::Unconfigured)
    ));
    assert!(AppState::from_config(&config).is_err());
    let mut config = config;
    config.allow_memory_store = true;
    assert!(matches!(
        AppState::from_config(&config).unwrap().store,
        ObjectStore::Memory(_)
    ));
}

#[sqlx::test]
async fn stale_cleanup_ack_cannot_remove_a_new_claim_and_s3_does_not_hold_pool_connection(
    pool: PgPool,
) {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let seen = entered.clone();
    let resume = release.clone();
    let router = axum::Router::new().fallback(move || {
        let seen = seen.clone();
        let resume = resume.clone();
        async move {
            seen.notify_one();
            resume.notified().await;
            StatusCode::NO_CONTENT
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut state = state(pool, 0);
    state.store = ObjectStore::from_minio(Some(&gelabber_api::config::MinioConfig {
        endpoint: endpoint.clone(),
        public_endpoint: endpoint,
        access_key: "test".into(),
        secret_key: "test".into(),
        bucket: "test".into(),
    }))
    .unwrap();
    let key = format!("att/{}", Uuid::new_v4());
    sqlx::query("INSERT INTO storage_cleanup (object_key,retain_until) VALUES ($1,now()-interval '1 second')")
        .bind(&key).execute(&state.db).await.unwrap();
    let deleting = state.clone();
    let attempt = tokio::spawn(async move { cleanup::delete_pending(&deleting, 32).await });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    let replacement = Uuid::new_v4();
    // Pool=1: this query must complete while the storage request is stalled.
    tokio::time::timeout(Duration::from_secs(1),sqlx::query("UPDATE storage_cleanup SET claim=$2,claimed_until=now()+interval '6 minutes' WHERE object_key=$1")
        .bind(&key).bind(replacement).execute(&state.db)).await.unwrap().unwrap();
    release.notify_one();
    attempt.await.unwrap().unwrap();
    let claim: Uuid = sqlx::query_scalar("SELECT claim FROM storage_cleanup WHERE object_key=$1")
        .bind(&key)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(claim, replacement);
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn migration_backfills_existing_bytes_and_rollback_preserves_metadata_and_jobs(pool: PgPool) {
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 8)
    {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap();
    }
    let state = state(pool.clone(), 0);
    let (mut client, _, channel) = owner(&state).await;
    // Old-schema fixtures, before quota lifecycle exists.
    let user = Uuid::parse_str(client.user_id()).unwrap();
    let channel = Uuid::parse_str(&channel).unwrap();
    let message=sqlx::query_scalar::<_,Uuid>("INSERT INTO messages (channel_id,author_id,content) VALUES ($1,$2,'existing') RETURNING id")
        .bind(channel).bind(user).fetch_one(&pool).await.unwrap();
    for message_id in [Some(message), None] {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO attachments (id,message_id,channel_id,uploader_id,object_key,filename,content_type,size_bytes) VALUES ($1,$2,$3,$4,$5,'x.png','image/png',4)")
            .bind(id).bind(message_id).bind(channel).bind(user).bind(format!("att/{id}")).execute(&pool).await.unwrap();
    }
    let migration = include_str!("../migrations/0008_storage_lifecycle.sql");
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(migration).execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    assert!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('storage_cleanup')::text")
            .fetch_one(&pool)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    assert_eq!(usage(&state, client.user_id()).await, (4, 4));
    sqlx::query("DELETE FROM messages WHERE id=$1")
        .bind(message)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(usage(&state, client.user_id()).await, (4, 4));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM storage_cleanup")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // The old reader's selected columns still work on the upgraded storage schema.
    sqlx::query("SELECT id,channel_id,author_id,content,created_at,edited_at FROM messages WHERE channel_id=$1")
        .bind(channel).fetch_all(&pool).await.unwrap();
    // The current binary also needs the current schema (including later chat DTO fields).
    for later in sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version > 8)
    {
        sqlx::raw_sql(later.sql.clone())
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(
        client
            .send(
                Method::GET,
                &format!("/api/channels/{channel}/messages"),
                None
            )
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn cleanup_batches_are_bounded_and_never_delete_a_referenced_object(pool: PgPool) {
    let state = state(pool, 0); // no router/worker: deterministic batch observation
    let user:Uuid=sqlx::query_scalar("INSERT INTO users (email,name,password_hash) VALUES ('test@example.com','Test','unused') RETURNING id")
        .fetch_one(&state.db).await.unwrap();
    let server: Uuid =
        sqlx::query_scalar("INSERT INTO servers (name,owner_id) VALUES ('Test',$1) RETURNING id")
            .bind(user)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let channel: Uuid = sqlx::query_scalar(
        "INSERT INTO channels (name,kind,server_id) VALUES ('text','text',$1) RETURNING id",
    )
    .bind(server)
    .fetch_one(&state.db)
    .await
    .unwrap();
    let referenced = format!("att/{}", Uuid::new_v4());
    sqlx::query("INSERT INTO attachments (channel_id,uploader_id,object_key,filename,content_type,size_bytes) VALUES ($1,$2,$3,'x.png','image/png',4)")
        .bind(channel).bind(user).bind(&referenced).execute(&state.db).await.unwrap();
    state
        .store
        .put(&referenced, "image/png", vec![1, 2, 3, 4])
        .await
        .unwrap();
    let mut keys = vec![referenced.clone()];
    for _ in 0..100 {
        let key = format!("att/{}", Uuid::new_v4());
        state
            .store
            .put(&key, "image/png", vec![1, 2, 3, 4])
            .await
            .unwrap();
        keys.push(key);
    }
    sqlx::query("INSERT INTO storage_cleanup (object_key,retain_until) SELECT k,now()-interval '1 second' FROM unnest($1::text[]) k")
        .bind(&keys).execute(&state.db).await.unwrap();
    cleanup::delete_pending(&state, i64::MAX).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM storage_cleanup")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        69
    );
    assert_eq!(state.store.head(&referenced).await.unwrap().size, 4);
    for _ in 0..3 {
        cleanup::delete_pending(&state, 32).await.unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM storage_cleanup")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(state.store.head(&referenced).await.unwrap().size, 4);
}

#[sqlx::test]
async fn presign_parent_lock_precedes_quota_and_cannot_deadlock_a_channel_cascade(pool: PgPool) {
    let state = state(pool.clone(), 0);
    let (mut client, _, channel) = owner(&state).await;
    presign(&mut client, &channel).await; // existing daily ledger row
    let user = Uuid::parse_str(client.user_id()).unwrap();
    let id = Uuid::parse_str(&channel).unwrap();
    let mut deleting = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM channels WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *deleting)
        .await
        .unwrap();
    let mut seat = Client::from_state(state.clone());
    seat.jar = client.jar.clone();
    seat.csrf = client.csrf.clone();
    let request = tokio::spawn(async move {
        seat.send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({"filename":"x.png","content_type":"image/png","size":4})),
        )
        .await
    });
    // Observe the actual request waiting on the held channel parent row.
    tokio::time::timeout(Duration::from_secs(2),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND (query LIKE 'INSERT INTO attachments%' OR query LIKE 'SELECT id FROM channels%'))")
                .fetch_one(&pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    // A cascade holds the parent before its quota transition. Presign must
    // not already own the ledger while it waits for that parent.
    sqlx::query("SELECT reserved FROM upload_daily_usage WHERE uploader_id=$1 FOR UPDATE NOWAIT")
        .bind(user)
        .execute(&mut *deleting)
        .await
        .expect("parent-before-ledger prevents the lock cycle");
    deleting.commit().await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap()
            .status,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn pending_channel_cascade_commits_without_waiting_for_another_quota_owner(pool: PgPool) {
    let state = state(pool.clone(), 0);
    let (mut client, _, channel) = owner(&state).await;
    presign(&mut client, &channel).await;
    let user = Uuid::parse_str(client.user_id()).unwrap();
    let mut quota_owner = pool.begin().await.unwrap();
    sqlx::query("SELECT reserved FROM upload_daily_usage WHERE uploader_id=$1 FOR UPDATE")
        .bind(user)
        .execute(&mut *quota_owner)
        .await
        .unwrap();
    // Cascades preserve the charge in the intent, with no cross-uploader
    // ledger locks while parent/attachment rows are held.
    let deleted = tokio::time::timeout(
        Duration::from_secs(2),
        client.send(Method::DELETE, &format!("/api/channels/{channel}"), None),
    )
    .await
    .unwrap();
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(usage(&state, client.user_id()).await, (4, 0));
    quota_owner.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            cleanup::delete_pending(&state, 32).await.unwrap();
            if usage(&state, client.user_id()).await == (0, 4) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[sqlx::test]
async fn pending_upload_expiring_during_storage_head_cannot_bind(pool: PgPool) {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let first = Arc::new(AtomicBool::new(true));
    let seen = entered.clone();
    let resume = release.clone();
    let router = axum::Router::new().fallback(move |request: Request<Body>| {
        let seen = seen.clone();
        let resume = resume.clone();
        let first = first.clone();
        async move {
            if request.method() == Method::HEAD {
                if first.swap(false, Ordering::SeqCst) {
                    seen.notify_one();
                    resume.notified().await;
                }
                Response::builder()
                    .status(200)
                    .header("content-length", "4")
                    .header("content-type", "image/png")
                    .body(Body::empty())
                    .unwrap()
            } else {
                Response::builder().status(200).body(Body::empty()).unwrap()
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut state = state(pool.clone(), 0);
    state.store = ObjectStore::from_minio(Some(&gelabber_api::config::MinioConfig {
        endpoint: endpoint.clone(),
        public_endpoint: endpoint,
        access_key: "test".into(),
        secret_key: "test".into(),
        bucket: "test".into(),
    }))
    .unwrap();
    let (mut client, _, channel) = owner(&state).await;
    let signed = presign(&mut client, &channel).await;
    let id = Uuid::parse_str(signed["id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE attachments SET expires_at=clock_timestamp()+interval '500 milliseconds' WHERE id=$1")
        .bind(id).execute(&pool).await.unwrap();
    let request = tokio::spawn(async move {
        client
            .send(
                Method::POST,
                &format!("/api/channels/{channel}/messages"),
                Some(json!({"attachment_ids":[id]})),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    // Observe DB wall time past expiry while the binding transaction still
    // has its old now() snapshot and is awaiting an actual storage HEAD.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let expired: bool = sqlx::query_scalar(
                "SELECT clock_timestamp()>expires_at FROM attachments WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
            if expired {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    release.notify_one();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap()
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[sqlx::test]
async fn inflight_put_after_expiry_is_not_lost_from_daily_usage(pool: PgPool) {
    use rusty_s3::actions::S3Action;
    use rusty_s3::{Bucket, Credentials, UrlStyle};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let endpoint = std::env::var("MINIO_ENDPOINT").unwrap();
    let config = minio_config(&endpoint);
    let mut state = state(pool.clone(), 4);
    state.store = ObjectStore::from_minio(Some(&config)).unwrap();
    state.store.ensure_ready().await.unwrap();
    let (mut client, _, channel) = owner(&state).await;
    let first = presign(&mut client, &channel).await;
    let id = Uuid::parse_str(first["id"].as_str().unwrap()).unwrap();
    let key = format!("att/{id}");
    // Compress the otherwise ten-minute boundary to three seconds; the actual
    // valid signed request and DB pending deadline agree, and it starts before both.
    let bucket = Bucket::new(
        url::Url::parse(&endpoint).unwrap(),
        UrlStyle::Path,
        config.bucket.clone(),
        "us-east-1".to_owned(),
    )
    .unwrap();
    let credentials = Credentials::new(config.access_key.clone(), config.secret_key.clone());
    let mut action = bucket.put_object(Some(&credentials), &key);
    action
        .headers_mut()
        .insert("content-type", "image/png".to_owned());
    action
        .headers_mut()
        .insert("content-length", "4".to_owned());
    let signed = action.sign(Duration::from_secs(3));
    let date = signed
        .query_pairs()
        .find(|(k, _)| k == "X-Amz-Date")
        .unwrap()
        .1
        .to_string();
    let expiry = chrono::NaiveDateTime::parse_from_str(&date, "%Y%m%dT%H%M%SZ")
        .unwrap()
        .and_utc()
        + chrono::Duration::seconds(3);
    sqlx::query("UPDATE attachments SET expires_at=$2 WHERE id=$1")
        .bind(id)
        .bind(expiry)
        .execute(&pool)
        .await
        .unwrap();
    let host = signed.host_str().unwrap();
    let port = signed.port_or_known_default().unwrap();
    let mut socket = tokio::net::TcpStream::connect((host, port)).await.unwrap();
    let target = format!("{}?{}", signed.path(), signed.query().unwrap());
    let request = format!(
        "PUT {target} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: image/png\r\nContent-Length: 4\r\nConnection: close\r\n\r\n"
    );
    socket.write_all(request.as_bytes()).await.unwrap();
    socket.write_all(&[1, 2]).await.unwrap();
    let initial = tokio::time::timeout(Duration::from_secs(1), state.store.head(&key))
        .await
        .expect("local MinIO HEAD must finish while the PUT is incomplete");
    assert!(matches!(initial, Err(StoreError::NotFound)));
    while chrono::Utc::now() <= expiry {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cleanup::expire_pending(&state, 32).await.unwrap();
    println!(
        "After expiry HEAD404: ledger={:?}",
        usage(&state, client.user_id()).await
    );
    assert_eq!(usage(&state, client.user_id()).await, (4, 0));
    socket.write_all(&[3, 4]).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(
        response.starts_with(b"HTTP/1.1 200"),
        "request started before signed expiry must complete successfully"
    );
    assert_eq!(state.store.head(&key).await.unwrap().size, 4);
    let second = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({"filename":"second.png","content_type":"image/png","size":4})),
        )
        .await;
    if second.status == StatusCode::CREATED {
        let mut put = reqwest::Client::new().put(second.body["upload_url"].as_str().unwrap());
        for (name, value) in second.body["headers"].as_object().unwrap() {
            put = put.header(name, value.as_str().unwrap());
        }
        let uploaded = put
            .body(vec![5, 6, 7, 8])
            .send()
            .await
            .unwrap_or_else(|_| panic!("own second local PUT transport failed"));
        assert!(uploaded.status().is_success());
        let second_key = format!("att/{}", second.body["id"].as_str().unwrap());
        assert_eq!(state.store.head(&second_key).await.unwrap().size, 4);
        println!(
            "Two real 4-byte MinIO uploads succeeded under 4-byte daily cap; ledger={:?}; second reservation HTTP{}",
            usage(&state, client.user_id()).await,
            second.status.as_u16()
        );
        state.store.delete(&second_key).await.unwrap();
        cleanup::delete_pending(&state, 32).await.unwrap();
        state.store.delete(&key).await.unwrap();
    }
    println!(
        "Completed first 4-byte upload after expiry; ledger={:?}; second reservation HTTP{}",
        usage(&state, client.user_id()).await,
        second.status.as_u16()
    );
    assert_eq!(
        second.status,
        StatusCode::TOO_MANY_REQUESTS,
        "completed first upload must still consume the 4-byte daily cap"
    );
    // The real background worker can observe the completed PUT after its
    // earlier in-flight HEAD. Both ledger states keep the entire daily cap;
    // demanding only the reserved intermediate state races valid settlement.
    let ledger = usage(&state, client.user_id()).await;
    assert!(
        matches!(ledger, (4, 0) | (0, 4)),
        "quota was lost: {ledger:?}"
    );
    // Advance only the test's DB deadline; settle the actual late-completed PUT
    // once and verify final object cleanup without refunding its consumed bytes.
    sqlx::query("UPDATE attachments SET expires_at=now()-interval '16 minutes',expiry_retry_at=now() WHERE id=$1")
  .bind(id).execute(&pool).await.unwrap();
    cleanup::expire_pending(&state, 32).await.unwrap();
    cleanup::delete_pending(&state, 32).await.unwrap();
    assert_eq!(usage(&state, client.user_id()).await, (0, 4));
    assert!(matches!(
        state.store.head(&key).await,
        Err(StoreError::NotFound)
    ));
}

#[sqlx::test]
async fn absent_upload_keeps_quota_through_grace_and_concurrent_expiry_refunds_once(pool: PgPool) {
    let state = state(pool.clone(), 4);
    let (mut client, _, channel) = owner(&state).await;
    let signed = presign(&mut client, &channel).await;
    let id = Uuid::parse_str(signed["id"].as_str().unwrap()).unwrap();
    sqlx::query(
        "UPDATE attachments SET expires_at=clock_timestamp()-interval '1 minute' WHERE id=$1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    cleanup::expire_pending(&state, 32).await.unwrap();
    assert_eq!(usage(&state, client.user_id()).await, (4, 0));
    let deferred: bool = sqlx::query_scalar(
        "SELECT expiry_retry_at>=expires_at+interval '15 minutes' FROM attachments WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(deferred);
    assert_eq!(
        client
            .send(Method::GET, &format!("/api/attachments/{id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .send(
                Method::POST,
                &format!("/api/channels/{channel}/messages"),
                Some(json!({"attachment_ids":[id]}))
            )
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let denied = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/attachments"),
            Some(json!({"filename":"y.png","content_type":"image/png","size":4})),
        )
        .await;
    assert_eq!(denied.status, StatusCode::TOO_MANY_REQUESTS);
    sqlx::query("UPDATE attachments SET expires_at=clock_timestamp()-interval '16 minutes',expiry_retry_at=now() WHERE id=$1")
        .bind(id).execute(&pool).await.unwrap();
    let (first, second) = tokio::join!(
        cleanup::expire_pending(&state, 32),
        cleanup::expire_pending(&state, 32)
    );
    first.unwrap();
    second.unwrap();
    cleanup::delete_pending(&state, 32).await.unwrap();
    assert_eq!(usage(&state, client.user_id()).await, (0, 0));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM attachments")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    presign(&mut client, &channel).await;
    assert_eq!(usage(&state, client.user_id()).await, (4, 0));
}
