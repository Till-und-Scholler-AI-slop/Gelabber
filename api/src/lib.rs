pub mod attachments;
pub mod auth;
pub mod config;
pub mod cookies;
pub mod csrf;
pub mod dms;
pub mod error;
pub mod gateway;
pub mod health;
pub mod json;
pub mod media;
pub mod messages;
pub mod password;
pub mod path;
pub mod profile;
pub mod servers;
pub mod state;
pub mod storage;
pub mod telemetry;
pub mod token;

use axum::Router;
use axum::http::Request;
use axum::middleware;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Level, info_span};

pub use config::Config;
pub use gateway::{Event, EventDraft, EventKind, PresenceStatus, publish_channel, publish_server};
pub use state::AppState;

/// Builds the HTTP router. Kept separate from `main` so integration tests
/// can drive it in-process.
pub fn app(state: AppState) -> Router {
    // Everything under /api is a browser-facing JSON route and goes through
    // the CSRF check; /health and /ready stay outside (GET only, no cookies).
    let api = Router::new()
        .merge(auth::router())
        .merge(profile::router())
        .merge(servers::router())
        .merge(dms::router())
        .merge(messages::router())
        .merge(media::router())
        .merge(attachments::router())
        .layer(middleware::from_fn(csrf::require));

    // `/ws` is a GET upgrade, not a JSON mutation — it stays outside the
    // CSRF layer and uses the session cookie the browser already sends.
    Router::new()
        .merge(health::router())
        .merge(gateway::router())
        .merge(api)
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
