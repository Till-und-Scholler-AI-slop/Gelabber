//! Prometheus scrape for the SFU. `GET /metrics` (and `/media/metrics` so
//! a Caddy `/media/*` prefix still works) exports rooms, peers, forwarded
//! RTP bytes/packets, ICE failures, queue drops and write errors. `/health`
//! and `/ready` are unchanged.

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/metrics", get(scrape))
        .route("/media/metrics", get(scrape))
}

async fn scrape(State(state): State<AppState>) -> Response {
    let body = state.sfu.metrics_text();
    (
        StatusCode::OK,
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
        )],
        body,
    )
        .into_response()
}
