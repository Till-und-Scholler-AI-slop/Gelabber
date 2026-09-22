//! Parse Compose `TURN_URLS` into the list the API puts on a media ticket.
//! The SFU does not apply these: it is ICE-lite.
//!
//! Optional `TURN_AUTH_SECRET` mints coturn REST (time-limited) credentials
//! per ticket so static long-term passwords are not required on the client.

use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use serde::{Deserialize, Serialize};

type HmacSha1 = Hmac<Sha1>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// Comma-separated STUN/TURN URLs. Credentials attach only to `turn:` /
/// `turns:` entries.
pub fn parse_ice_servers(
    urls: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) -> Vec<IceServer> {
    let Some(urls) = urls.filter(|raw| !raw.trim().is_empty()) else {
        return Vec::new();
    };
    let mut stun = Vec::new();
    let mut turn = Vec::new();
    for raw in urls.split(',') {
        let url = raw.trim();
        if url.is_empty() {
            continue;
        }
        if url.starts_with("turn:") || url.starts_with("turns:") {
            turn.push(url.to_owned());
        } else {
            stun.push(url.to_owned());
        }
    }
    let mut out = Vec::new();
    if !stun.is_empty() {
        out.push(IceServer {
            urls: stun,
            username: None,
            credential: None,
        });
    }
    if !turn.is_empty() {
        out.push(IceServer {
            urls: turn,
            username: username.map(str::to_owned).filter(|s| !s.is_empty()),
            credential: password.map(str::to_owned).filter(|s| !s.is_empty()),
        });
    }
    out
}

/// coturn REST username + HMAC-SHA1 credential (`expiry:user`).
pub fn turn_rest_credentials(secret: &str, user: &str, ttl_secs: u64, now_unix: u64) -> (String, String) {
    let expiry = now_unix.saturating_add(ttl_secs.max(1));
    let safe_user = if user.is_empty() { "gelabber" } else { user };
    let username = format!("{expiry}:{safe_user}");
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes()).expect("HMAC-SHA1 accepts any key");
    mac.update(username.as_bytes());
    let credential = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    (username, credential)
}

/// Overlay REST credentials onto TURN entries. STUN entries stay anonymous.
pub fn with_turn_credentials(servers: &[IceServer], username: &str, credential: &str) -> Vec<IceServer> {
    servers
        .iter()
        .map(|server| {
            let turn = server
                .urls
                .iter()
                .any(|url| url.starts_with("turn:") || url.starts_with("turns:"));
            if turn {
                IceServer {
                    urls: server.urls.clone(),
                    username: Some(username.to_owned()),
                    credential: Some(credential.to_owned()),
                }
            } else {
                server.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_stun_and_turn() {
        let servers = parse_ice_servers(
            Some("stun:127.0.0.1:3478,turn:127.0.0.1:3478"),
            Some("gelabber"),
            Some("secret"),
        );
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].urls, vec!["stun:127.0.0.1:3478"]);
        assert_eq!(servers[1].username.as_deref(), Some("gelabber"));
    }

    #[test]
    fn rest_credentials_are_time_limited_and_stable() {
        let (user, cred) = turn_rest_credentials("s3cret", "alice", 600, 1_700_000_000);
        assert_eq!(user, "1700000600:alice");
        let (again, cred2) = turn_rest_credentials("s3cret", "alice", 600, 1_700_000_000);
        assert_eq!(user, again);
        assert_eq!(cred, cred2);
        assert_ne!(
            cred,
            turn_rest_credentials("other", "alice", 600, 1_700_000_000).1
        );
    }

    #[test]
    fn rest_credentials_overlay_turn_only() {
        let servers = parse_ice_servers(
            Some("stun:127.0.0.1:3478,turn:127.0.0.1:3478"),
            Some("gelabber"),
            Some("static"),
        );
        let minted = with_turn_credentials(&servers, "1700000600:alice", "hmac");
        assert_eq!(minted[0].username, None);
        assert_eq!(minted[1].username.as_deref(), Some("1700000600:alice"));
        assert_eq!(minted[1].credential.as_deref(), Some("hmac"));
    }
}
