//! `GET /health` (process is up, no dependencies) and `GET /ready`
//! (Postgres and Redis both answer within the configured timeout).

use std::future::Future;
use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use serde::Serialize;
use sqlx::{Connection, PgConnection};
use tokio::time::timeout;
use tracing::{info, warn};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub checks: Checks,
}

#[derive(Debug, Serialize)]
pub struct Checks {
    pub postgres: CheckResult,
    pub redis: CheckResult,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckResult {
    Ok { latency_ms: u64 },
    Error { error: String, latency_ms: u64 },
}

impl CheckResult {
    fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadyResponse>) {
    let (postgres, redis) = tokio::join!(
        run_check("postgres", &state, check_postgres(&state)),
        run_check("redis", &state, check_redis(&state)),
    );

    let all_ok = postgres.is_ok() && redis.is_ok();
    let status_code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = ReadyResponse {
        status: if all_ok { "ready" } else { "not_ready" },
        checks: Checks { postgres, redis },
    };
    (status_code, Json(body))
}

/// Runs one dependency probe with a hard deadline so `/ready` can never hang
/// on a stalled connection, and reports which check failed and why.
async fn run_check(
    name: &'static str,
    state: &AppState,
    probe: impl Future<Output = Result<(), String>>,
) -> CheckResult {
    let started = Instant::now();
    let outcome = timeout(state.ready_timeout, probe).await;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    match outcome {
        Ok(Ok(())) => {
            info!(check = name, latency_ms, "ready check passed");
            CheckResult::Ok { latency_ms }
        }
        Ok(Err(error)) => {
            warn!(check = name, latency_ms, error = %error, "ready check failed");
            CheckResult::Error { error, latency_ms }
        }
        Err(_elapsed) => {
            let error = format!("timed out after {}ms", state.ready_timeout.as_millis());
            warn!(check = name, latency_ms, error = %error, "ready check failed");
            CheckResult::Error { error, latency_ms }
        }
    }
}

async fn check_postgres(state: &AppState) -> Result<(), String> {
    let mut conn = PgConnection::connect_with(&state.pg_connect)
        .await
        .map_err(|err| err.to_string())?;
    let value: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut conn)
        .await
        .map_err(|err| err.to_string())?;
    // Best effort: a failed close does not change the verdict.
    let _ = conn.close().await;
    if value != 1 {
        return Err(format!("unexpected SELECT 1 result: {value}"));
    }
    Ok(())
}

async fn check_redis(state: &AppState) -> Result<(), String> {
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
