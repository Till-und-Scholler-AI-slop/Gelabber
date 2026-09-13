//! `GET /health` (process is up) and `GET /ready` (Redis answers).
//! Caddy proxies `/media` and `/media/*`, so both prefixes are registered.

use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use serde::Serialize;
use tokio::time::timeout;
use tracing::{info, warn};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/media/health", get(health))
        .route("/ready", get(ready))
        .route("/media/ready", get(ready))
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub service: &'static str,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub checks: Checks,
}

#[derive(Debug, Serialize)]
pub struct Checks {
    pub redis: CheckResult,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckResult {
    Ok {
        latency_ms: u64,
    },
    Error {
        error: &'static str,
        latency_ms: u64,
    },
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        service: "media",
        status: "ok",
    })
}

async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadyResponse>) {
    let started = Instant::now();
    let outcome = timeout(state.ready_timeout, ping_redis(&state)).await;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    let redis = match outcome {
        Ok(Ok(())) => {
            info!(check = "redis", latency_ms, "ready check passed");
            CheckResult::Ok { latency_ms }
        }
        Ok(Err(detail)) => {
            warn!(check = "redis", latency_ms, error = %detail, "ready check failed");
            CheckResult::Error {
                error: class_of(&detail),
                latency_ms,
            }
        }
        Err(_) => {
            warn!(check = "redis", latency_ms, "ready check timed out");
            CheckResult::Error {
                error: "timed_out",
                latency_ms,
            }
        }
    };

    let ok = matches!(redis, CheckResult::Ok { .. });
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(ReadyResponse {
            status: if ok { "ready" } else { "not_ready" },
            checks: Checks { redis },
        }),
    )
}

async fn ping_redis(state: &AppState) -> Result<(), String> {
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| err.to_string())?;
    let reply: String = redis::cmd("PING")
        .query_async(&mut conn)
        .await
        .map_err(|err| err.to_string())?;
    if reply != "PONG" {
        return Err(format!("unexpected PING reply: {reply}"));
    }
    Ok(())
}

fn class_of(detail: &str) -> &'static str {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("connection refused") || lower.contains("os error 111") {
        "connection_refused"
    } else if lower.contains("auth") || lower.contains("wrong pass") {
        "auth_failed"
    } else {
        "unavailable"
    }
}
