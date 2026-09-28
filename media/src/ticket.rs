//! Short join tickets. The API mints them after session + `join_voice`;
//! this process only consumes. Alphabet, prefix, and payload live in
//! `gelabber-shared` so a renamed field cannot fail closed in silence.

use gelabber_shared::ticket;
pub use gelabber_shared::ticket::{ALPHABET, CODE_LEN, REDIS_PREFIX, TicketClaim};

pub fn redis_key(code: &str) -> String {
    ticket::redis_key(code)
}

pub fn is_valid(raw: &str) -> bool {
    ticket::is_valid(raw)
}

pub fn decode(raw: &str) -> Result<TicketClaim, serde_json::Error> {
    ticket::decode(raw)
}

pub use gelabber_shared::ticket::AuthorizedTicketClaim;
use std::time::Duration;

const VALIDATE: &str = r#"
local member = redis.call('GET', KEYS[1])
local channel = redis.call('GET', KEYS[2])
local session = redis.call('GET', KEYS[3])
local now = redis.call('TIME')
if member ~= ARGV[1] or channel ~= ARGV[2] or session ~= ARGV[3]
  or tonumber(now[1]) >= tonumber(ARGV[4]) then return 0 end
redis.call('SET', KEYS[4], '1', 'EX', 35)
return 1
"#;

const CONSUME: &str = r#"
local raw = redis.call('GETDEL', KEYS[1])
if not raw then return false end
local ok, claim = pcall(cjson.decode, raw)
if not ok or type(claim) ~= 'table' or type(claim.auth) ~= 'table' then return false end
local auth = claim.auth
if type(claim.u) ~= 'string' or type(claim.s) ~= 'string' or type(claim.c) ~= 'string'
  or type(auth.session) ~= 'string' or #auth.session ~= 64 or auth.session:find('[^0-9a-f]')
  or type(auth.member) ~= 'string' or type(auth.channel) ~= 'string'
  or type(auth.expires_at) ~= 'number' or auth.expires_at <= 0
  or auth.expires_at > 9007199254740991 or auth.expires_at ~= math.floor(auth.expires_at)
  then return false end
local now = redis.call('TIME')
if redis.call('GET', 'gb:auth:member:' .. claim.s .. ':' .. claim.u) ~= auth.member
  or redis.call('GET', 'gb:auth:channel:' .. claim.c) ~= auth.channel
  or redis.call('GET', 'gb:auth:session:' .. auth.session) ~= claim.u
  or tonumber(now[1]) >= auth.expires_at then return false end
redis.call('SET', 'gb:auth:demand:' .. auth.session, '1', 'EX', 35)
return raw
"#;

fn deadline_error() -> redis::RedisError {
    redis::RedisError::from((redis::ErrorKind::Io, "media authority deadline exceeded"))
}

/// Consumption and authority validation run in one Redis operation. Invalid
/// envelopes are consumed as well; Redis failure never grants a peer.
pub async fn consume(
    redis: &redis::Client,
    code: &str,
) -> Result<Option<AuthorizedTicketClaim>, redis::RedisError> {
    if !is_valid(code) {
        return Ok(None);
    }
    tokio::time::timeout(Duration::from_millis(500), async {
        let mut conn = redis.get_multiplexed_async_connection().await?;
        let raw: Option<String> = redis::cmd("EVAL")
            .arg(CONSUME)
            .arg(1)
            .arg(redis_key(code))
            .query_async(&mut conn)
            .await?;
        Ok(raw
            .and_then(|raw| serde_json::from_str::<AuthorizedTicketClaim>(&raw).ok())
            .filter(AuthorizedTicketClaim::well_formed))
    })
    .await
    .map_err(|_| deadline_error())?
}

/// Only the API renews the short session lease. Media refreshes demand after
/// atomic validation and never deletes another tab's demand on leave.
pub async fn validate_authority(
    redis: &redis::Client,
    claim: &AuthorizedTicketClaim,
) -> Result<bool, redis::RedisError> {
    if !claim.well_formed() {
        return Ok(false);
    }
    tokio::time::timeout(Duration::from_millis(500), async {
        let mut conn = redis.get_multiplexed_async_connection().await?;
        let result: i64 = redis::cmd("EVAL")
            .arg(VALIDATE)
            .arg(4)
            .arg(ticket::member_authority_key(claim.claim.s, claim.claim.u))
            .arg(ticket::channel_authority_key(claim.claim.c))
            .arg(ticket::session_authority_key(&claim.auth.session))
            .arg(ticket::media_demand_key(&claim.auth.session))
            .arg(claim.auth.member.to_string())
            .arg(claim.auth.channel.to_string())
            .arg(claim.claim.u.to_string())
            .arg(claim.auth.expires_at)
            .query_async(&mut conn)
            .await?;
        Ok(result == 1)
    })
    .await
    .map_err(|_| deadline_error())?
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
