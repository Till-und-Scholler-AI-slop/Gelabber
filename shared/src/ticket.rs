//! Short join tickets. Same alphabet as server invites so they stay
//! readable and unguessable (30^12). Not a LiveKit/JWT token.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lower-case, no `0/o` / `1/l/i`. Same set as invite codes.
pub const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
pub const CODE_LEN: usize = 12;
pub const REDIS_PREFIX: &str = "gb:mt:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketClaim {
    pub u: Uuid,
    pub s: Uuid,
    pub c: Uuid,
    /// Caller may go live: a screen (`s`) and its sound (`sa`). Absent means no.
    #[serde(default)]
    pub g: bool,
}

pub fn redis_key(code: &str) -> String {
    format!("{REDIS_PREFIX}{code}")
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

pub const MEMBER_AUTHORITY_PREFIX: &str = "gb:auth:member:";
pub const CHANNEL_AUTHORITY_PREFIX: &str = "gb:auth:channel:";
pub const SESSION_AUTHORITY_PREFIX: &str = "gb:auth:session:";
pub const MEDIA_DEMAND_PREFIX: &str = "gb:auth:demand:";
pub const SESSION_OWNER_PREFIX: &str = "gb:auth:owner:";

pub fn member_authority_key(server: Uuid, user: Uuid) -> String {
    format!("{MEMBER_AUTHORITY_PREFIX}{server}:{user}")
}
pub fn channel_authority_key(channel: Uuid) -> String {
    format!("{CHANNEL_AUTHORITY_PREFIX}{channel}")
}
pub fn session_authority_key(session: &str) -> String {
    format!("{SESSION_AUTHORITY_PREFIX}{session}")
}
/// Media refreshes this demand key (SET 1 EX 35) every second while at
/// least one authenticated peer is using this session. Do not delete it on
/// peer leave: another tab or an outstanding ticket may still need it.
pub fn media_demand_key(session: &str) -> String {
    format!("{MEDIA_DEMAND_PREFIX}{session}")
}
/// API-internal: the worker generation that owns this session's lease
/// refresh and cleanup. Media never reads it.
pub fn session_owner_key(session: &str) -> String {
    format!("{SESSION_OWNER_PREFIX}{session}")
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

    // API and media production code build these keys from the helpers or the
    // prefix constants above (media's CONSUME Lua takes the prefixes as ARGV).
    // web/scripts/e2e/*.mjs hard-code the same strings, so a format change has
    // to update them as well.
    #[test]
    fn authority_key_formats_are_pinned() {
        let server = Uuid::from_u128(1);
        let user = Uuid::from_u128(2);
        let channel = Uuid::from_u128(3);
        let session = "0123456789abcdef".repeat(4);
        assert_eq!(
            member_authority_key(server, user),
            "gb:auth:member:00000000-0000-0000-0000-000000000001:00000000-0000-0000-0000-000000000002"
        );
        assert_eq!(
            channel_authority_key(channel),
            "gb:auth:channel:00000000-0000-0000-0000-000000000003"
        );
        assert_eq!(
            session_authority_key(&session),
            "gb:auth:session:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            media_demand_key(&session),
            "gb:auth:demand:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            session_owner_key(&session),
            "gb:auth:owner:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }
}
