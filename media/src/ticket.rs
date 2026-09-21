//! Short join tickets. The API mints them after session + `join_voice`;
//! this process only consumes. Alphabet, prefix, and payload live in
//! `gelabber-shared` so a renamed field cannot fail closed in silence.

use gelabber_shared::ticket;
pub use gelabber_shared::ticket::{ALPHABET, CODE_LEN, REDIS_PREFIX, TicketClaim};
use tracing::warn;

pub fn redis_key(code: &str) -> String {
    ticket::redis_key(code)
}

pub fn is_valid(raw: &str) -> bool {
    ticket::is_valid(raw)
}

pub fn decode(raw: &str) -> Result<TicketClaim, serde_json::Error> {
    ticket::decode(raw)
}

/// Single-use: GETDEL. Missing/expired → `None`. A payload that does not
/// match [`TicketClaim`] is logged and treated as unauthorized, not as a
/// missing key with no trace.
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
    Ok(match raw {
        None => None,
        Some(value) => match decode(&value) {
            Ok(claim) => Some(claim),
            Err(err) => {
                warn!(error = %err, "media ticket payload did not match");
                None
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gelabber_shared::ticket::{self, TicketClaim};
    use uuid::Uuid;

    #[test]
    fn claim_round_trips() {
        let claim = TicketClaim {
            u: Uuid::from_u128(1),
            s: Uuid::from_u128(2),
            c: Uuid::from_u128(3),
            g: false,
        };
        let raw = ticket::encode(&claim).unwrap();
        assert_eq!(decode(&raw).unwrap(), claim);
        assert!(!raw.contains("token"));
        assert!(!raw.contains("livekit"));
    }
}
