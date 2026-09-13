//! Native WebSocket gateway (issue #6).
//!
//! Same Axum process as the REST API. Session cookie on the upgrade,
//! compact JSON frames, Redis 8.10.1 Pub/Sub for fan-out, bounded Redis
//! lists for reconnect catch-up. No Socket.IO, no LiveKit.
//!
//! **Issue 5 (REST messages)** owns create/edit/delete persistence. After a
//! successful write it should call [`publish_channel`]. This module does
//! not add message tables or `/api/.../messages` routes.
//!
//! **Issue 8 (presence / typing)** uses ephemeral `op: "p"` / `op: "y"`
//! over Redis keys + TTL and Pub/Sub. They are not sequenced and never
//! enter the chat replay log.
//!
//! **Issue 10 (signaling)** uses `op: "sig"` (join/leave, offer/answer,
//! ice, pub/unpub). Those frames are not mixed into `op: "e"`.

mod conn;
mod hub;
mod live;
pub mod protocol;
mod signal;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;
use axum::http::header::{HOST, ORIGIN};
use axum::response::Response;
use axum::routing::get;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::ApiError;
use crate::state::AppState;

pub use hub::Gateway;
pub use protocol::{Event, EventDraft, EventKind, PresenceStatus, Topic};

pub fn router() -> Router<AppState> {
    Router::new().route("/ws", get(upgrade))
}

async fn upgrade(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !origin_allowed(&headers) {
        return Err(ApiError::Forbidden(
            "Cross-origin WebSocket is not allowed.",
        ));
    }
    Ok(ws.on_upgrade(move |socket| conn::run(socket, state, user)))
}

/// Same-origin check when the browser sends `Origin`. Non-browser clients
/// (tests, future native apps) omit it and pass. Vite and Caddy keep `Host`
/// as the page host when `changeOrigin` is false, so the hosts match.
pub fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) else {
        return true;
    };
    if origin.eq_ignore_ascii_case("null") {
        return false;
    }
    let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let origin_host = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(origin);
    origin_host.eq_ignore_ascii_case(host)
}

/// Publish a compact create/edit/delete event on a **channel** topic.
/// Issue 5 calls this after REST create/edit/delete of a message.
pub async fn publish_channel(
    state: &AppState,
    server_id: Uuid,
    channel_id: Uuid,
    kind: EventKind,
    entity_id: Option<Uuid>,
    delta: Option<Value>,
) -> Result<Event, ApiError> {
    state
        .gateway
        .publish(EventDraft {
            kind,
            server_id,
            channel_id: Some(channel_id),
            entity_id,
            delta,
        })
        .await
}

/// Publish on a **server** topic (structural events later; the subscribe
/// path is already here).
pub async fn publish_server(
    state: &AppState,
    server_id: Uuid,
    kind: EventKind,
    entity_id: Option<Uuid>,
    delta: Option<Value>,
) -> Result<Event, ApiError> {
    state
        .gateway
        .publish(EventDraft {
            kind,
            server_id,
            channel_id: None,
            entity_id,
            delta,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::HeaderValue;

    fn headers(origin: Option<&str>, host: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        if let Some(origin) = origin {
            map.insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        if let Some(host) = host {
            map.insert(HOST, HeaderValue::from_str(host).unwrap());
        }
        map
    }

    #[test]
    fn origin_optional_for_non_browsers() {
        assert!(origin_allowed(&headers(None, Some("localhost"))));
    }

    #[test]
    fn origin_must_match_host() {
        assert!(origin_allowed(&headers(
            Some("http://localhost:5173"),
            Some("localhost:5173")
        )));
        assert!(origin_allowed(&headers(
            Some("https://gelabber.example"),
            Some("gelabber.example")
        )));
        assert!(!origin_allowed(&headers(
            Some("https://evil.example"),
            Some("gelabber.example")
        )));
        assert!(!origin_allowed(&headers(Some("null"), Some("localhost"))));
    }
}
