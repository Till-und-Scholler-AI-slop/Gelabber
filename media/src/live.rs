//! The API owns channel Live claims; media owns one exact peer lease per nonce.
use gelabber_shared::ticket::{self, AuthorizedTicketClaim};
use std::time::Duration;
use uuid::Uuid;

pub fn peer_key(nonce: Uuid) -> String {
    format!("gb:live:peer:{nonce}")
}

const ACQUIRE: &str = r#"
local now = redis.call('TIME')
if redis.call('GET', KEYS[1]) ~= ARGV[1]
 or redis.call('GET', KEYS[2]) ~= ARGV[2]
 or redis.call('GET', KEYS[3]) ~= ARGV[3]
 or tonumber(now[1]) >= tonumber(ARGV[4]) then return 0 end
local raw = redis.call('GET', KEYS[4])
if not raw then return 0 end
local ok, claim = pcall(cjson.decode, raw)
if not ok or type(claim) ~= 'table'
 or claim.u ~= ARGV[3] or claim.s ~= ARGV[5] or claim.c ~= ARGV[6]
 or claim.session ~= ARGV[7] or claim.nonce ~= ARGV[8]
 or type(claim.seat) ~= 'string'
 or not string.match(claim.seat, '^%x%x%x%x%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$')
 or claim.seat == '00000000-0000-0000-0000-000000000000' then return 0 end
local ttl = redis.call('PTTL', KEYS[4])
if ttl <= 0 or ttl > 5000 then return 0 end
local peer = redis.call('GET', KEYS[5])
if peer and peer ~= ARGV[9] then return 0 end
redis.call('SET', KEYS[5], ARGV[9], 'PX', ttl)
return ttl
"#;

pub async fn validate_and_acquire(
    redis: &redis::Client,
    authority: &AuthorizedTicketClaim,
    nonce: Uuid,
    peer: Uuid,
) -> Result<Option<Duration>, redis::RedisError> {
    if !authority.well_formed() || nonce.is_nil() || peer.is_nil() || !authority.claim.g {
        return Ok(None);
    }
    tokio::time::timeout(Duration::from_millis(500), async {
        let mut conn = redis.get_multiplexed_async_connection().await?;
        let result: i64 = redis::cmd("EVAL")
            .arg(ACQUIRE)
            .arg(5)
            .arg(ticket::member_authority_key(
                authority.claim.s,
                authority.claim.u,
            ))
            .arg(ticket::channel_authority_key(authority.claim.c))
            .arg(ticket::session_authority_key(&authority.auth.session))
            .arg(format!("gb:live:{}", authority.claim.c))
            .arg(peer_key(nonce))
            .arg(authority.auth.member.to_string())
            .arg(authority.auth.channel.to_string())
            .arg(authority.claim.u.to_string())
            .arg(authority.auth.expires_at)
            .arg(authority.claim.s.to_string())
            .arg(authority.claim.c.to_string())
            .arg(&authority.auth.session)
            .arg(nonce.to_string())
            .arg(peer.to_string())
            .query_async(&mut conn)
            .await?;
        Ok((result > 0).then(|| Duration::from_millis(result as u64)))
    })
    .await
    .map_err(|_| {
        redis::RedisError::from((redis::ErrorKind::Io, "live authority deadline exceeded"))
    })?
}

pub async fn release(redis: &redis::Client, nonce: Uuid, peer: Uuid) {
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        let mut conn = redis.get_multiplexed_async_connection().await?;
        redis::cmd("EVAL")
            .arg("if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) end return 0")
            .arg(1)
            .arg(peer_key(nonce))
            .arg(peer.to_string())
            .query_async::<i64>(&mut conn)
            .await
    }).await;
}
