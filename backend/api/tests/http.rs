use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use gelabber_api::{AppState, Config, app};
use serde_json::Value;
use tokio::net::TcpListener;
use tower::ServiceExt;

fn config(postgres: SocketAddr, redis: SocketAddr, ready_timeout_ms: u64) -> Config {
    Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "DATABASE_URL" => Some(format!("postgres://u:p@{postgres}/db")),
        "REDIS_URL" => Some(format!("redis://{redis}")),
        "API_READY_TIMEOUT_MS" => Some(ready_timeout_ms.to_string()),
        _ => None,
    })
    .expect("test config is valid")
}

async fn get(state: AppState, path: &str) -> (StatusCode, Value) {
    let response = app(state)
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 16).await.unwrap();
    let json = serde_json::from_slice(&bytes).expect("response body is JSON");
    (status, json)
}

/// Binds and immediately releases a loopback port so that connecting to it is
/// refused instead of hanging.
async fn closed_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

/// A TCP server that accepts connections and never answers, simulating a
/// dependency that is reachable but stalled.
async fn silent_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    (addr, handle)
}

#[tokio::test]
async fn health_answers_without_dependencies() {
    let unreachable = closed_port().await;
    let state = AppState::from_config(&config(unreachable, unreachable, 500)).unwrap();

    let (status, body) = get(state, "/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({ "status": "ok" }));
}

#[tokio::test]
async fn ready_reports_each_refused_dependency() {
    let postgres = closed_port().await;
    let redis = closed_port().await;
    let state = AppState::from_config(&config(postgres, redis, 2000)).unwrap();

    let started = Instant::now();
    let (status, body) = get(state, "/ready").await;
    let elapsed = started.elapsed();

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "not_ready");
    for check in ["postgres", "redis"] {
        assert_eq!(body["checks"][check]["status"], "error", "{check}: {body}");
        assert_eq!(
            body["checks"][check]["error"], "connection_refused",
            "{check}: {body}"
        );
        assert!(body["checks"][check]["latency_ms"].is_u64(), "{body}");
    }
    assert_no_driver_detail(&body);
    assert!(
        elapsed < Duration::from_secs(1),
        "refused connections must fail fast, took {elapsed:?}"
    );
}

/// The public body must only carry the coarse class, never the raw driver
/// message (which can include OS error text, user or database names).
fn assert_no_driver_detail(body: &Value) {
    let text = body.to_string().to_lowercase();
    for needle in [
        "os error",
        "connection refused",
        "error returned",
        "error communicating",
        "timed out after",
        "user \"",
        "database \"",
    ] {
        assert!(
            !text.contains(needle),
            "public body leaks driver detail ({needle:?}): {text}"
        );
    }
}

#[tokio::test]
async fn ready_does_not_hang_on_stalled_dependencies() {
    let (postgres, pg_task) = silent_server().await;
    let (redis, redis_task) = silent_server().await;
    let state = AppState::from_config(&config(postgres, redis, 300)).unwrap();

    let started = Instant::now();
    let (status, body) = get(state, "/ready").await;
    let elapsed = started.elapsed();

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "not_ready");
    for check in ["postgres", "redis"] {
        assert_eq!(body["checks"][check]["status"], "error", "{check}: {body}");
        assert_eq!(
            body["checks"][check]["error"], "timed_out",
            "{check}: {body}"
        );
    }
    assert_no_driver_detail(&body);
    assert!(
        elapsed < Duration::from_secs(2),
        "stalled dependencies must hit the deadline, took {elapsed:?}"
    );

    pg_task.abort();
    redis_task.abort();
}

/// Like `silent_server`, but counts the connections it accepted.
async fn counting_server() -> (
    SocketAddr,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = accepted.clone();
    let handle = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            held.push(socket);
        }
    });
    (addr, accepted, handle)
}

/// `/ready` is public: a burst of requests must not open a database and a
/// Redis connection each.
#[tokio::test]
async fn ready_probes_once_for_a_burst() {
    let (postgres, pg_accepted, pg_task) = counting_server().await;
    let (redis, redis_accepted, redis_task) = counting_server().await;
    let state = AppState::from_config(&config(postgres, redis, 300)).unwrap();
    let app = app(state);

    let burst = (0..20).map(|_| {
        app.clone()
            .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
    });
    for response in futures_util::future::join_all(burst).await {
        assert_eq!(response.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    let again = app
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::SERVICE_UNAVAILABLE);

    // The app's background workers also try the pool and Redis, so allow a
    // few more than the one probe; a probe per request would be 21 each.
    let ordering = std::sync::atomic::Ordering::SeqCst;
    let (pg, redis) = (pg_accepted.load(ordering), redis_accepted.load(ordering));
    assert!(
        pg < 10 && redis < 10,
        "postgres {pg}, redis {redis} connections"
    );
    pg_task.abort();
    redis_task.abort();
}

#[tokio::test]
async fn metrics_is_prometheus_text_and_health_stays_json() {
    let unreachable = closed_port().await;
    let state = AppState::from_config(&config(unreachable, unreachable, 500)).unwrap();
    let app = app(state);

    let health = app
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let health_bytes = to_bytes(health.into_body(), 1 << 16).await.unwrap();
    let health_json: Value = serde_json::from_slice(&health_bytes).unwrap();
    assert_eq!(health_json["status"], "ok");

    let metrics = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let content_type = metrics
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("text/plain"),
        "metrics content-type: {content_type}"
    );
    let body = to_bytes(metrics.into_body(), 1 << 16).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("# TYPE gelabber_http_requests_total counter"));
    assert!(text.contains("gelabber_http_requests_total{method=\"GET\",status=\"200\"}"));
    assert!(!text.contains("\"status\":\"ok\""));
}
