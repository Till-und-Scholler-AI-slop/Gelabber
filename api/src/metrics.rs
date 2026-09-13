//! Prometheus scrape for the API. `GET /metrics` is text/plain, not JSON,
//! and is not behind CSRF. `/health` and `/ready` stay as they are.
//!
//! Counters are process-local (one API container). Cardinality is bounded:
//! method + status, plus a small set of rate-limit bucket names.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/metrics", get(scrape))
}

async fn scrape(State(state): State<AppState>) -> Response {
    let body = state.metrics.render();
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

#[derive(Default)]
pub struct HttpMetrics {
    /// `(method, status)` → count.
    requests: Mutex<BTreeMap<(String, u16), u64>>,
    duration_ms_sum: AtomicU64,
    duration_count: AtomicU64,
    rate_limited: Mutex<BTreeMap<&'static str, u64>>,
}

impl HttpMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&self, method: &str, status: u16, duration_ms: u64) {
        let mut map = self.requests.lock().unwrap_or_else(|err| err.into_inner());
        *map.entry((method.to_ascii_uppercase(), status))
            .or_insert(0) += 1;
        self.duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.duration_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn rate_limited(&self, bucket: &'static str) {
        let mut map = self
            .rate_limited
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        *map.entry(bucket).or_insert(0) += 1;
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP gelabber_http_requests_total HTTP requests handled by the API.\n");
        out.push_str("# TYPE gelabber_http_requests_total counter\n");
        {
            let map = self.requests.lock().unwrap_or_else(|err| err.into_inner());
            for ((method, status), count) in map.iter() {
                out.push_str(&format!(
                    "gelabber_http_requests_total{{method=\"{method}\",status=\"{status}\"}} {count}\n"
                ));
            }
        }

        let dur_count = self.duration_count.load(Ordering::Relaxed);
        let dur_sum = self.duration_ms_sum.load(Ordering::Relaxed) as f64 / 1000.0;
        out.push_str(
            "# HELP gelabber_http_request_duration_seconds_sum Total request handling time.\n",
        );
        out.push_str("# TYPE gelabber_http_request_duration_seconds_sum counter\n");
        out.push_str(&format!(
            "gelabber_http_request_duration_seconds_sum {dur_sum}\n"
        ));
        out.push_str(
            "# HELP gelabber_http_request_duration_seconds_count Requests included in the sum.\n",
        );
        out.push_str("# TYPE gelabber_http_request_duration_seconds_count counter\n");
        out.push_str(&format!(
            "gelabber_http_request_duration_seconds_count {dur_count}\n"
        ));

        out.push_str(
            "# HELP gelabber_rate_limited_total Requests rejected by a rate limit or quota.\n",
        );
        out.push_str("# TYPE gelabber_rate_limited_total counter\n");
        {
            let map = self
                .rate_limited
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            for (bucket, count) in map.iter() {
                out.push_str(&format!(
                    "gelabber_rate_limited_total{{bucket=\"{bucket}\"}} {count}\n"
                ));
            }
        }
        out
    }
}

/// Records method + status for every request except `/metrics` itself.
pub async fn track(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request.uri().path() == "/metrics" {
        return next.run(request).await;
    }
    let method = request.method().as_str().to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    let ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    state
        .metrics
        .observe(&method, response.status().as_u16(), ms);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_help_and_type() {
        let metrics = HttpMetrics::new();
        metrics.observe("GET", 200, 12);
        metrics.rate_limited("auth");
        let text = metrics.render();
        assert!(text.contains("# TYPE gelabber_http_requests_total counter"));
        assert!(text.contains("method=\"GET\",status=\"200\""));
        assert!(text.contains("gelabber_rate_limited_total{bucket=\"auth\"} 1"));
        assert!(text.contains("gelabber_http_request_duration_seconds_count 1"));
    }
}
