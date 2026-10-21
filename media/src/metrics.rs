//! Prometheus scrape for the SFU. `GET /metrics` exports rooms, peers,
//! forwarded RTP bytes and ICE failures. Prometheus scrapes it on the
//! internal compose network (`media:8081/metrics`). There is no `/media/*`
//! alias on purpose, so a proxy that forwards `/media/*` cannot publish it.
//! `/health` and `/ready` are unchanged.

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/metrics", get(scrape))
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
