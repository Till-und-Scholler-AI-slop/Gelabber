//! Short join tickets. The API mints them after session + `join_voice`;
//! this process only consumes. Same alphabet as server invites so they
//! stay readable and unguessable (30^12). Not a LiveKit/JWT token.

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

/// Single-use: GETDEL. Missing/expired → `None`.
pub async fn consume(
    redis: &redis::Client,
    code: &str,
) -> Result<Option<TicketClaim>, redis::RedisError> {
    if !is_valid(code) {
        return Ok(None);
    }
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let raw: Option<String> = redis::cmd("GETDEL")
        .arg(redis_key(code))
        .query_async(&mut conn)
        .await?;
    Ok(raw.and_then(|value| decode(&value).ok()))
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
    fn claim_round_trips() {
        let claim = TicketClaim {
            u: Uuid::from_u128(1),
            s: Uuid::from_u128(2),
            c: Uuid::from_u128(3),
        };
        let raw = encode(&claim).unwrap();
        assert_eq!(decode(&raw).unwrap(), claim);
        assert!(!raw.contains("token"));
        assert!(!raw.contains("livekit"));
    }
}
