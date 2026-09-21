//! Parse Compose `TURN_URLS` into the list the API puts on a media ticket.
//! The SFU does not apply these: it is ICE-lite.

use serde::{Deserialize, Serialize};

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
}
