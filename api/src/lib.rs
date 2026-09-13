pub mod config;
pub mod health;
pub mod state;
pub mod telemetry;

use axum::Router;
use axum::http::Request;
use tower_http::trace::{DefaultOnFailure, TraceLayer};
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
                // A 503 from `/ready` is already explained by the per-check
                // warnings; do not escalate it to an error on top.
                .on_failure(DefaultOnFailure::new().level(Level::WARN)),
        )
        .with_state(state)
}
