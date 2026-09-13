//! `GET /health` (process is up, no dependencies) and `GET /ready`
//! (Postgres and Redis both answer within the configured timeout).
//!
//! `/ready` is proxied unauthenticated by Caddy, so the response body only
//! carries a coarse error class per check. The full driver error goes to the
//! `warn!` log line, which is where operators look for the cause.

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
    Error { error: ErrorClass, latency_ms: u64 },
}

impl CheckResult {
    fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }
}

/// Public, coarse failure class. Anything that does not map onto the three
/// specific classes is reported as `unavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    ConnectionRefused,
    AuthFailed,
    TimedOut,
    Unavailable,
}

impl ErrorClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionRefused => "connection_refused",
            Self::AuthFailed => "auth_failed",
            Self::TimedOut => "timed_out",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Internal probe failure: the class that goes to the client plus the full
/// driver error that only goes to the log.
struct ProbeError {
    class: ErrorClass,
    detail: String,
}

impl From<sqlx::Error> for ProbeError {
    fn from(err: sqlx::Error) -> Self {
        let class = match &err {
            sqlx::Error::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => {
                ErrorClass::ConnectionRefused
            }
            // SQLSTATE class 28 = invalid authorization specification
            // (28000) / invalid password (28P01).
            sqlx::Error::Database(db) if db.code().is_some_and(|c| c.starts_with("28")) => {
                ErrorClass::AuthFailed
            }
            _ => ErrorClass::Unavailable,
        };
        Self {
            class,
            detail: err.to_string(),
        }
    }
}

impl From<redis::RedisError> for ProbeError {
    fn from(err: redis::RedisError) -> Self {
        let class = if err.is_connection_refusal() {
            ErrorClass::ConnectionRefused
        } else if err.kind() == redis::ErrorKind::AuthenticationFailed {
            ErrorClass::AuthFailed
        } else {
            ErrorClass::Unavailable
        };
        Self {
            class,
            detail: err.to_string(),
        }
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
/// on a stalled connection. Logs which check failed and the full cause;
/// returns only the coarse class for the response body.
async fn run_check(
    name: &'static str,
    state: &AppState,
    probe: impl Future<Output = Result<(), ProbeError>>,
) -> CheckResult {
    let started = Instant::now();
    let outcome = timeout(state.ready_timeout, probe).await;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    let ProbeError { class, detail } = match outcome {
        Ok(Ok(())) => {
            info!(check = name, latency_ms, "ready check passed");
            return CheckResult::Ok { latency_ms };
        }
        Ok(Err(err)) => err,
        Err(_elapsed) => ProbeError {
            class: ErrorClass::TimedOut,
            detail: format!("timed out after {}ms", state.ready_timeout.as_millis()),
        },
    };

    warn!(
        check = name,
        latency_ms,
        error_class = class.as_str(),
        error = %detail,
        "ready check failed"
    );
    CheckResult::Error {
        error: class,
        latency_ms,
    }
}

async fn check_postgres(state: &AppState) -> Result<(), ProbeError> {
    let mut conn = PgConnection::connect_with(&state.pg_connect).await?;
    let value: i32 = sqlx::query_scalar("SELECT 1").fetch_one(&mut conn).await?;
    // Drop instead of `close()`: the probe has succeeded once the query
    // answered, and a stalled Terminate round-trip must not eat the deadline.
    drop(conn);
    if value != 1 {
        return Err(ProbeError {
            class: ErrorClass::Unavailable,
            detail: format!("unexpected SELECT 1 result: {value}"),
        });
    }
    Ok(())
}

async fn check_redis(state: &AppState) -> Result<(), ProbeError> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    let reply: String = redis::cmd("PING").query_async(&mut conn).await?;
    if reply != "PONG" {
        return Err(ProbeError {
            class: ErrorClass::Unavailable,
            detail: format!("unexpected PING reply: {reply}"),
        });
    }
    Ok(())
}
