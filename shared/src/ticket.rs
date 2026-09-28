//! Short join tickets. Same alphabet as server invites so they stay
//! readable and unguessable (30^12). Not a LiveKit/JWT token.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lower-case, no `0/o` / `1/l/i`. Same set as invite codes.
pub const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
pub const CODE_LEN: usize = 12;
pub const REDIS_PREFIX: &str = "gb:mt:";
/// Legacy kick/ban deny, retained for API compatibility during integration.
/// The authorized media envelope uses membership generations instead.
pub const DENY_PREFIX: &str = "gb:deny:";
pub const DENY_TTL_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketClaim {
    pub u: Uuid,
    pub s: Uuid,
    pub c: Uuid,
    /// Caller may announce a Go Live track (`l`). Absent means no.
    #[serde(default)]
    pub g: bool,
}

pub fn redis_key(code: &str) -> String {
    format!("{REDIS_PREFIX}{code}")
}

pub fn deny_key(server_id: Uuid, user_id: Uuid) -> String {
    format!("{DENY_PREFIX}{server_id}:{user_id}")
}

pub fn is_valid(raw: &str) -> bool {
    raw.len() == CODE_LEN && raw.bytes().all(|b| ALPHABET.contains(&b))
}

pub fn generate() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..CODE_LEN)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

pub fn encode(claim: &TicketClaim) -> Result<String, serde_json::Error> {
    serde_json::to_string(claim)
}

pub fn decode(raw: &str) -> Result<TicketClaim, serde_json::Error> {
    serde_json::from_str(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_short_and_in_alphabet() {
        let a = generate();
        let b = generate();
        assert_eq!(a.len(), CODE_LEN);
        assert!(is_valid(&a));
        assert_ne!(a, b);
        assert!(!is_valid("livekit-jwt"));
        assert!(!is_valid("SHORT"));
        assert!(!is_valid(&"a".repeat(CODE_LEN - 1)));
    }

    #[test]
    fn claim_round_trips_with_go_live() {
        let claim = TicketClaim {
            u: Uuid::from_u128(1),
            s: Uuid::from_u128(2),
            c: Uuid::from_u128(3),
            g: true,
        };
        let raw = encode(&claim).unwrap();
        assert_eq!(decode(&raw).unwrap(), claim);
        assert!(raw.contains("\"g\":true"));
        assert!(!raw.contains("token"));
        assert!(!raw.contains("livekit"));
    }

    #[test]
    fn missing_go_live_is_false() {
        let raw = r#"{"u":"00000000-0000-0000-0000-000000000001","s":"00000000-0000-0000-0000-000000000002","c":"00000000-0000-0000-0000-000000000003"}"#;
        let claim = decode(raw).unwrap();
        assert!(!claim.g);
    }
}

/// Mandatory media envelope from the API's access-revocation contract.
/// The base claim stays source-compatible for API minting and in-process tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizedTicketClaim {
    #[serde(flatten)]
    pub claim: TicketClaim,
    pub auth: TicketAuthorization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketAuthorization {
    pub session: String,
    pub expires_at: u64,
    pub member: Uuid,
    pub channel: Uuid,
}

impl AuthorizedTicketClaim {
    pub fn well_formed(&self) -> bool {
        let auth = &self.auth;
        !self.claim.u.is_nil()
            && !self.claim.s.is_nil()
            && !self.claim.c.is_nil()
            && !auth.member.is_nil()
            && !auth.channel.is_nil()
            && auth.session.len() == 64
            && auth
                .session
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && auth.expires_at > 0
            && auth.expires_at <= 9_007_199_254_740_991
    }
}

pub fn member_authority_key(server: Uuid, user: Uuid) -> String {
    format!("gb:auth:member:{server}:{user}")
}
pub fn channel_authority_key(channel: Uuid) -> String {
    format!("gb:auth:channel:{channel}")
}
pub fn session_authority_key(session: &str) -> String {
    format!("gb:auth:session:{session}")
}
pub fn media_demand_key(session: &str) -> String {
    format!("gb:auth:demand:{session}")
}

#[cfg(test)]
mod authorization_tests {
    use super::*;
    #[test]
    fn authorized_envelope_preserves_base_contract_and_requires_authority() {
        let claim = AuthorizedTicketClaim {
            claim: TicketClaim {
                u: Uuid::from_u128(1),
                s: Uuid::from_u128(2),
                c: Uuid::from_u128(3),
                g: true,
            },
            auth: TicketAuthorization {
                session: "a".repeat(64),
                expires_at: 1234567890,
                member: Uuid::from_u128(4),
                channel: Uuid::from_u128(5),
            },
        };
        let raw = serde_json::to_string(&claim).unwrap();
        assert_eq!(
            serde_json::from_str::<AuthorizedTicketClaim>(&raw).unwrap(),
            claim
        );
        assert!(claim.well_formed());
        assert_eq!(decode(&raw).unwrap(), claim.claim);
        assert!(
            serde_json::from_str::<AuthorizedTicketClaim>(&encode(&claim.claim).unwrap()).is_err()
        );
        let mut bad = claim.clone();
        bad.auth.session = "A".repeat(64);
        assert!(!bad.well_formed());
        bad = claim.clone();
        bad.auth.member = Uuid::nil();
        assert!(!bad.well_formed());
        bad = claim;
        bad.auth.expires_at = 9_007_199_254_740_992;
        assert!(!bad.well_formed());
    }
}
