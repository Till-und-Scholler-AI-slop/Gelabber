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
//! **Issue 10 (signaling)** uses `op: "sig"` for presence (join/leave,
//! pub/unpub, mute/deafen). SDP and ICE go to the media socket, not `/ws`.
//! Those frames are not mixed into `op: "e"`.

mod conn;
pub mod delivery;
mod hub;
mod leases;
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

use crate::auth::session::CurrentSession;
use crate::error::ApiError;
use crate::state::AppState;

pub use hub::{ConnTable, EventLog, Gateway, VoiceRoster};
pub use protocol::{Event, EventDraft, EventKind, PresenceStatus, Topic};

pub fn router() -> Router<AppState> {
    Router::new().route("/ws", get(upgrade))
}

async fn upgrade(
    State(state): State<AppState>,
    session: CurrentSession,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !origin_allowed(&headers, state.cookie_secure) {
        return Err(ApiError::Forbidden(
            "Cross-origin WebSocket is not allowed.",
        ));
    }
    Ok(ws.on_upgrade(move |socket| conn::run(socket, state, session)))
}

/// Same-origin check when the browser sends `Origin`. Non-browser clients
/// (tests, future native apps) omit it and pass. Vite and Caddy keep `Host`
/// as the page host when `changeOrigin` is false, so the hosts match.
///
/// A browser leaves the scheme's default port out of `Origin`; a proxy in
/// front may still write it into `Host` (`Host: example.com:443`). Both name
/// the same origin, so that port is ignored on either side.
///
/// `Host` does not say which scheme the browser used, so `secure_cookies`
/// (`API_COOKIE_SECURE`, set behind TLS) stands in for it: the app's own pages
/// are then `https://`. An `http://` page under the same host name is someone
/// else's, served by whoever can answer plain HTTP for that name, and Firefox
/// sends the session cookie along with that page's `wss://` handshake
/// (Chromium withholds it). Loopback hosts stay allowed: browsers keep
/// `Secure` cookies for `http://localhost`, and nobody on the network answers
/// for it.
pub fn origin_allowed(headers: &HeaderMap, secure_cookies: bool) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    // Present but not plain ASCII is nothing a browser sends, and it is not
    // the same as absent.
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if origin.eq_ignore_ascii_case("null") {
        return false;
    }
    let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let (origin_host, default_port) = if let Some(rest) = origin.strip_prefix("https://") {
        (rest, ":443")
    } else if let Some(rest) = origin.strip_prefix("http://") {
        if secure_cookies && !is_loopback(rest) {
            return false;
        }
        (rest, ":80")
    } else {
        return origin.eq_ignore_ascii_case(host);
    };
    without_port(origin_host, default_port).eq_ignore_ascii_case(without_port(host, default_port))
}

fn without_port<'a>(host: &'a str, port: &str) -> &'a str {
    host.strip_suffix(port).unwrap_or(host)
}

/// `localhost`, `127.0.0.0/8` or `[::1]`, with or without a port.
fn is_loopback(authority: &str) -> bool {
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    host.eq_ignore_ascii_case("localhost")
        || host == "[::1]"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|address| address.is_loopback())
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

    /// Plain HTTP (`API_COOKIE_SECURE=false`) and behind TLS.
    const PLAIN: bool = false;
    const TLS: bool = true;

    fn allowed(origin: &str, host: &str, secure_cookies: bool) -> bool {
        origin_allowed(&headers(Some(origin), Some(host)), secure_cookies)
    }

    #[test]
    fn origin_optional_for_non_browsers() {
        for mode in [PLAIN, TLS] {
            assert!(origin_allowed(&headers(None, Some("localhost")), mode));
        }
    }

    #[test]
    fn origin_must_match_host() {
        assert!(allowed("http://localhost:5173", "localhost:5173", PLAIN));
        for mode in [PLAIN, TLS] {
            assert!(allowed(
                "https://gelabber.example",
                "gelabber.example",
                mode
            ));
            assert!(!allowed("https://evil.example", "gelabber.example", mode));
            assert!(!allowed("null", "localhost", mode));
            assert!(!origin_allowed(
                &headers(Some("https://gelabber.example"), None),
                mode
            ));
        }
    }

    #[test]
    fn default_port_is_the_same_origin() {
        // An outer proxy with `Host $host:$server_port` in front of Caddy,
        // which forwards the port since v0.6.
        for (origin, host) in [
            ("https://gelabber.example", "gelabber.example:443"),
            ("https://gelabber.example:443", "gelabber.example"),
            ("https://[2001:db8::1]", "[2001:db8::1]:443"),
            ("https://Gelabber.Example", "gelabber.example:443"),
        ] {
            for mode in [PLAIN, TLS] {
                assert!(allowed(origin, host, mode), "{origin} {host} {mode}");
            }
        }
        assert!(allowed(
            "http://gelabber.example",
            "gelabber.example:80",
            PLAIN
        ));
    }

    #[test]
    fn other_ports_still_differ() {
        for (origin, host) in [
            // The other scheme's default is not this scheme's default.
            ("https://gelabber.example", "gelabber.example:80"),
            ("http://gelabber.example", "gelabber.example:443"),
            ("https://gelabber.example:8443", "gelabber.example"),
            ("https://gelabber.example:8443", "gelabber.example:443"),
            ("https://gelabber.example", "gelabber.example:8443"),
            ("https://gelabber.example", "gelabber.example:4430"),
            ("https://evil.example", "gelabber.example:443"),
            ("https://evil.example:443", "gelabber.example"),
            ("https://[2001:db8::1]:8443", "[2001:db8::1]"),
        ] {
            for mode in [PLAIN, TLS] {
                assert!(!allowed(origin, host, mode), "{origin} {host} {mode}");
            }
        }
        for mode in [PLAIN, TLS] {
            assert!(allowed(
                "https://gelabber.example:8443",
                "gelabber.example:8443",
                mode
            ));
        }
    }

    /// The host matches, the scheme does not: a page someone served as
    /// `http://` under the name of an HTTPS deployment.
    #[test]
    fn http_page_of_the_same_host_is_foreign_behind_tls() {
        for (origin, host) in [
            ("http://gelabber.example", "gelabber.example"),
            ("http://gelabber.example", "gelabber.example:80"),
            ("http://gelabber.example:80", "gelabber.example"),
            ("http://gelabber.example:8443", "gelabber.example:8443"),
            ("http://[2001:db8::1]", "[2001:db8::1]"),
            ("http://192.168.1.10:8088", "192.168.1.10:8088"),
        ] {
            assert!(!allowed(origin, host, TLS), "{origin} {host}");
            // A plain-HTTP deployment has nothing but such pages.
            assert!(allowed(origin, host, PLAIN), "{origin} {host}");
        }
    }

    /// Browsers keep `Secure` cookies for `http://` loopback pages, so a stack
    /// with `API_COOKIE_SECURE=true` tried on localhost keeps its socket.
    #[test]
    fn loopback_http_page_stays_same_origin_behind_tls() {
        for (origin, host) in [
            ("http://localhost", "localhost"),
            ("http://localhost:5173", "localhost:5173"),
            ("http://LOCALHOST:5173", "localhost:5173"),
            ("http://127.0.0.1:8080", "127.0.0.1:8080"),
            ("http://127.8.9.10", "127.8.9.10:80"),
            ("http://[::1]:8080", "[::1]:8080"),
        ] {
            assert!(allowed(origin, host, TLS), "{origin} {host}");
        }
        for (origin, host) in [
            // Loopback, but not the host that was asked.
            ("http://localhost:5173", "gelabber.example"),
            ("http://localhost:5173", "localhost:5174"),
            ("http://127.0.0.1", "localhost"),
            // Names that only look like loopback.
            ("http://localhost.evil.example", "localhost.evil.example"),
            ("http://127.0.0.1.evil.example", "127.0.0.1.evil.example"),
            ("http://evil.localhost", "evil.localhost"),
            ("http://notlocalhost", "notlocalhost"),
            ("http://128.0.0.1", "128.0.0.1"),
            ("http://[::2]", "[::2]"),
            ("http://localhost:", "localhost:"),
        ] {
            assert!(!allowed(origin, host, TLS), "{origin} {host}");
        }
    }

    #[test]
    fn origin_that_is_not_ascii_is_not_an_absent_origin() {
        for origin in [
            &b"https://evil.example\xff"[..],
            &b"https://gelabber.example\xff"[..],
            &b"\xff"[..],
        ] {
            let mut map = headers(None, Some("gelabber.example"));
            map.insert(ORIGIN, HeaderValue::from_bytes(origin).unwrap());
            for mode in [PLAIN, TLS] {
                assert!(!origin_allowed(&map, mode), "{origin:?} {mode}");
            }
        }
    }
}
