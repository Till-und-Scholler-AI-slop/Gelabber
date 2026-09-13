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
