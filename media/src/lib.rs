//! Gelabber's own ticket/signaling/authority gateway with the pinned official
//! Rust mediasoup engine. Room = voice channel; UDP stays outside Caddy.

pub mod config;
pub mod error;
pub mod health;
pub mod live;
pub mod metrics;
mod native_command;
mod native_resource;
pub mod protocol;
pub mod redis_connection;
pub mod sfu;
pub mod state;
pub mod telemetry;
pub mod ticket;
pub mod ws;

use axum::Router;
use axum::http::Request;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Level, info_span};

pub use config::Config;
pub use state::AppState;

pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(metrics::router())
        .merge(ws::router())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    info_span!(
                        "http_request",
                        method = %request.method(),
                        path = %request.uri().path(),
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(()),
        )
        .with_state(state)
}
