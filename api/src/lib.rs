pub mod config;
pub mod health;
pub mod state;
pub mod telemetry;

use axum::Router;
use axum::http::Request;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Level, info_span};

pub use config::Config;
pub use state::AppState;

/// Builds the HTTP router. Kept separate from `main` so integration tests
/// can drive it in-process.
pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(health::router())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    info_span!(
                        "http_request",
                        method = %request.method(),
                        path = %request.uri().path(),
                        version = ?request.version(),
                    )
                })
                // One INFO access-log line per response (status + latency).
                // The default on_failure would add a second line for 5xx; a
                // 503 from `/ready` is already explained by the per-check
                // warnings, so it is disabled.
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(()),
        )
        .with_state(state)
}
