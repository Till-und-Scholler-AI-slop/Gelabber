pub mod attachments;
pub mod auth;
pub mod avatars;
pub mod config;
pub mod cookies;
pub mod csrf;
pub mod dms;
pub mod error;
pub mod gateway;
pub mod gifs;
pub mod health;
pub mod json;
pub mod limits;
pub mod media;
pub mod messages;
pub mod metrics;
pub mod password;
pub mod path;
pub mod profile;
pub mod servers;
pub mod state;
pub mod storage;
pub mod telemetry;
pub mod themes;
pub mod token;

use axum::Router;
use axum::extract::State;
use axum::http::header::STRICT_TRANSPORT_SECURITY;
use axum::http::{HeaderValue, Request};
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Level, info_span};

pub use config::Config;
pub use gateway::{Event, EventDraft, EventKind, PresenceStatus, publish_channel, publish_server};
pub use state::AppState;

/// Builds the HTTP router. Kept separate from `main` so integration tests
/// can drive it in-process.
pub fn app(state: AppState) -> Router {
    state.gateway.ensure_delivery(state.clone());
    attachments::cleanup::start(state.clone());
    // Everything under /api is a browser-facing JSON route and goes through
    // the CSRF check; /health, /ready and /metrics stay outside (GET only).
    let api = Router::new()
        .merge(auth::router())
        .merge(profile::router())
        .merge(avatars::router())
        .merge(themes::router())
        .merge(servers::router())
        .merge(dms::router())
        .merge(messages::router())
        .merge(media::router())
        .merge(attachments::router())
        .merge(gifs::router())
        .layer(middleware::from_fn(csrf::require))
        .layer(middleware::from_fn_with_state(state.clone(), limits::gate));

    // `/ws` is a GET upgrade, not a JSON mutation — it stays outside the
    // CSRF layer and uses the session cookie the browser already sends.
    Router::new()
        .merge(health::router())
        .merge(metrics::router())
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            metrics::track,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), hsts))
        .with_state(state)
}

/// A year, this host only: subdomains may be other people's services.
const HSTS: HeaderValue = HeaderValue::from_static("max-age=31536000");

/// Behind TLS (`API_COOKIE_SECURE`) every response tells the browser to stay
/// on HTTPS for this host, whatever proxy terminates TLS. The web client
/// asks `/api/auth/session` on every start, so the first visit sets it.
async fn hsts(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    if state.cookie_secure {
        response
            .headers_mut()
            .entry(STRICT_TRANSPORT_SECURITY)
            .or_insert(HSTS);
    }
    response
}
