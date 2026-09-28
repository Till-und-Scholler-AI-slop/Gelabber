//! Short SFU join tickets (issue 11). Session + `join_voice` + voice
//! channel, then a 12-char code in Redis (`gb:mt:{code}`, 30 s). The
//! media binary consumes it. Not a LiveKit/JWT token.
//!
//! The payload is [`gelabber_shared::ticket::TicketClaim`], including
//! whether this caller may Go Live. The SFU refuses an `l` announce
//! without that bit.

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::post;
use gelabber_shared::ice::IceServer;
use gelabber_shared::ticket::{self, TicketClaim};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::session::CurrentSession;
use crate::error::ApiError;
use crate::path::Id;
use crate::servers::channel::{Channel, ChannelKind};
use crate::servers::membership;
use crate::servers::permissions::Permission;
use crate::state::AppState;

pub use gelabber_shared::ice::parse_ice_servers;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/channels/{id}/media-ticket", post(issue_ticket))
}

#[derive(Debug, Serialize)]
pub struct MediaTicket {
    pub ticket: String,
    pub expires_in: u64,
    pub media_path: &'static str,
    pub ice_servers: Vec<IceServer>,
}

async fn issue_ticket(
    State(state): State<AppState>,
    session: CurrentSession,
    Id(channel_id): Id,
) -> Result<Json<MediaTicket>, ApiError> {
    let user = &session.user;
    let channel = load_channel(&state.db, channel_id).await?;
    // DMs have no server — same 404 as `channel_for` / `messaging_channel`.
    let server_id = channel.server_id.ok_or(ApiError::NotFound)?;
    let mut session_guard = session
        .lock_live(&state.db)
        .await?
        .ok_or(ApiError::Unauthenticated)?;
    membership::lock_server_conn(&mut session_guard, server_id, false).await?;
    // Re-read after acquiring the server lock: a delete may have won the race.
    let channel = load_channel(&mut *session_guard, channel_id).await?;
    let member = membership::load(&mut *session_guard, server_id, user.id).await?;
    member.require(Permission::JoinVoice)?;
    if channel.kind != ChannelKind::Voice {
        return Err(ApiError::BadRequest(
            "Media tickets are for voice channels.".into(),
        ));
    }
    let go_live = member.can(Permission::GoLive);

    let (ticket, expires_in) = mint(
        &state.redis,
        TicketClaim {
            u: user.id,
            s: server_id,
            c: channel.id,
            g: go_live,
        },
        &session,
        state.media_ticket_ttl,
    )
    .await?;

    maintain_session(&state, &session).await?;
    Ok(Json(MediaTicket {
        ticket,
        expires_in,
        media_path: "/media/ws",
        ice_servers: state.ticket_ice_servers(user.id),
    }))
}

async fn load_channel<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    channel_id: Uuid,
) -> Result<Channel, ApiError> {
    sqlx::query_as::<_, Channel>(
        "SELECT id, server_id, category_id, name, kind, created_at \
         FROM channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

async fn mint(
    redis: &redis::Client,
    claim: TicketClaim,
    session: &CurrentSession,
    ttl: Duration,
) -> Result<(String, u64), ApiError> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
    let member_key = member_key(claim.s, claim.u);
    let channel_key = channel_key(claim.c);
    let session_key = session_key(&session.key());
    let secs = ttl.as_secs().max(1).min(
        (session.expires_at - chrono::Utc::now())
            .num_seconds()
            .max(0) as u64,
    );
    if secs == 0 {
        return Err(ApiError::Unauthenticated);
    }
    let base = serde_json::to_string(&claim)
        .map_err(|err| ApiError::Internal(format!("serialize media ticket: {err}")))?;
    for _ in 0..8 {
        let code = ticket::generate();
        let key = ticket::redis_key(&code);
        let set: Option<String> = redis::Script::new(MINT_LUA)
            .key(&key)
            .key(&member_key)
            .key(&channel_key)
            .key(&session_key)
            .key(demand_key(&session.key()))
            .arg(&base)
            .arg(secs)
            .arg(Uuid::new_v4().to_string())
            .arg(Uuid::new_v4().to_string())
            .arg(session.key())
            .arg(session.expires_at.timestamp())
            .arg(claim.u.to_string())
            .invoke_async(&mut conn)
            .await
            .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
        if set.as_deref() == Some("OK") {
            return Ok((code, secs));
        }
    }
    Err(ApiError::Internal(
        "could not allocate a media ticket".into(),
    ))
}

// 03b must enforce this envelope at ticket consume, peer attach and throughout
// peer lifetime, fail closed on missing keys/Redis errors, and reject legacy
// tickets. A missing epoch creates a fresh nonce, so a Redis reset cannot revive
// a ticket from an earlier Redis lifetime. The existing 120 s deny is retained
// until 03b replaces it with these generations (immediate rejoin then works).
const MINT_LUA: &str = r#"
if redis.call('EXISTS', KEYS[1]) == 1 then return nil end
redis.call('SET', KEYS[2], ARGV[3], 'NX')
redis.call('SET', KEYS[3], ARGV[4], 'NX')
local now = redis.call('TIME')
local remaining = tonumber(ARGV[6]) * 1000 - (tonumber(now[1]) * 1000 + math.floor(tonumber(now[2]) / 1000))
if remaining <= 0 then return nil end
redis.call('SET', KEYS[4], ARGV[7], 'PX', math.min(3000, remaining))
redis.call('SET', KEYS[5], '1', 'EX', 35)
local claim = cjson.decode(ARGV[1])
claim.auth = {session = ARGV[5], expires_at = tonumber(ARGV[6]),
  member = redis.call('GET', KEYS[2]), channel = redis.call('GET', KEYS[3])}
redis.call('SET', KEYS[1], cjson.encode(claim), 'EX', ARGV[2])
return 'OK'
"#;

pub fn member_key(server_id: Uuid, user_id: Uuid) -> String {
    format!("gb:auth:member:{server_id}:{user_id}")
}
pub fn channel_key(channel_id: Uuid) -> String {
    format!("gb:auth:channel:{channel_id}")
}
pub fn session_key(hash: &str) -> String {
    format!("gb:auth:session:{hash}")
}

async fn rotate(redis: &redis::Client, key: String) -> Result<(), ApiError> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
    // Generations intentionally have no TTL: old tickets must never become valid
    // again after leave/rejoin or unban. A reset is fail closed until new mint.
    redis::cmd("SET")
        .arg(key)
        .arg(Uuid::new_v4().to_string())
        .query_async::<()>(&mut conn)
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))
}

pub async fn revoke_member(
    redis: &redis::Client,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    rotate(redis, member_key(server_id, user_id)).await
}
pub async fn revoke_channel(redis: &redis::Client, channel_id: Uuid) -> Result<(), ApiError> {
    rotate(redis, channel_key(channel_id)).await
}
pub async fn revoke_session(redis: &redis::Client, hash: &str) -> Result<(), ApiError> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
    redis::cmd("DEL")
        .arg(session_key(hash))
        .query_async::<()>(&mut conn)
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))
}

/// A DB-backed session lease bounds missed logout notifications to 3 seconds.
/// Refresh holds the same session row lock as ticket mint, so logout cannot race
/// a successful check followed by a lease resurrection. Redis/DB errors stop
/// renewal; 03b must fail closed when the lease disappears.
const REFRESH_SESSION_LUA: &str = r#"
if redis.call('GET', KEYS[3]) ~= ARGV[3] then return 'lost' end
if redis.call('EXISTS', KEYS[2]) == 0 then return 'idle' end
local now = redis.call('TIME')
local remaining = tonumber(ARGV[2]) * 1000 - (tonumber(now[1]) * 1000 + math.floor(tonumber(now[2]) / 1000))
if remaining <= 0 then return 'expired' end
redis.call('SET', KEYS[1], ARGV[1], 'PX', math.min(3000, remaining))
redis.call('PEXPIRE', KEYS[3], 5000)
return 'OK'
"#;

const CLEANUP_OWNER_LUA: &str = r#"
if redis.call('GET', KEYS[2]) ~= ARGV[1] then return 0 end
redis.call('DEL', KEYS[1], KEYS[2])
return 1
"#;

/// Media 03b refreshes this demand key (SET 1 EX 35) every second while at
/// least one authenticated peer is using this session. Do not delete it on
/// peer leave: another tab or an outstanding ticket may still need it.
pub fn demand_key(hash: &str) -> String {
    format!("gb:auth:demand:{hash}")
}
fn owner_key(hash: &str) -> String {
    format!("gb:auth:owner:{hash}")
}

async fn maintain_session(state: &AppState, session: &CurrentSession) -> Result<(), ApiError> {
    let key = session.key();
    let Some(generation) = state.gateway.track_media_session(&key).await else {
        return Ok(());
    };
    let registered = tokio::time::timeout(Duration::from_secs(1), async {
        let mut conn = state
            .redis
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
        redis::cmd("SET")
            .arg(owner_key(&key))
            .arg(generation.to_string())
            .arg("PX")
            .arg(5000)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|err| ApiError::Internal(format!("redis: {err}")))
    })
    .await;
    if !matches!(registered, Ok(Ok(()))) {
        state.gateway.untrack_media_session(&key, generation).await;
        return Err(ApiError::Internal("media session lease unavailable".into()));
    }
    let state = state.clone();
    let session = session.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let refreshed = tokio::time::timeout(Duration::from_secs(1), async {
                let Some(_guard) = session.lock_live(&state.db).await? else {
                    return Ok::<String, ApiError>("expired".into());
                };
                let mut conn = state
                    .redis
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
                redis::Script::new(REFRESH_SESSION_LUA)
                    .key(session_key(&key))
                    .key(demand_key(&key))
                    .key(owner_key(&key))
                    .arg(session.user.id.to_string())
                    .arg(session.expires_at.timestamp())
                    .arg(generation.to_string())
                    .invoke_async::<String>(&mut conn)
                    .await
                    .map_err(|err| ApiError::Internal(format!("redis: {err}")))
            })
            .await;
            match refreshed {
                Ok(Ok(status)) if status == "OK" => {}
                Ok(Ok(status))
                    if status == "idle"
                        && state.gateway.media_session_recent(&key, generation).await => {}
                _ => break,
            }
        }
        state.gateway.untrack_media_session(&key, generation).await;
        // A delayed worker/retry may only delete authority it still owns.
        for _ in 0..4 {
            let result = tokio::time::timeout(Duration::from_millis(500), async {
                let mut conn = state.redis.get_multiplexed_async_connection().await?;
                redis::Script::new(CLEANUP_OWNER_LUA)
                    .key(session_key(&key))
                    .key(owner_key(&key))
                    .arg(generation.to_string())
                    .invoke_async::<i32>(&mut conn)
                    .await
            })
            .await;
            if matches!(result, Ok(Ok(_))) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    Ok(())
}

pub async fn cleanup_session(redis: redis::Client, hash: String) {
    if matches!(
        tokio::time::timeout(Duration::from_millis(500), revoke_session(&redis, &hash)).await,
        Ok(Ok(()))
    ) {
        return;
    }
    tracing::warn!("media session cleanup deferred; session lease will expire");
    tokio::spawn(async move {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if matches!(
                tokio::time::timeout(Duration::from_millis(500), revoke_session(&redis, &hash))
                    .await,
                Ok(Ok(()))
            ) {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_media_contract() {
        let code = ticket::generate();
        assert_eq!(code.len(), ticket::CODE_LEN);
        assert!(code.bytes().all(|b| ticket::ALPHABET.contains(&b)));
    }
    #[tokio::test]
    async fn old_owner_cleanup_cannot_delete_restarted_session_authority() {
        let redis = redis::Client::open(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
        )
        .unwrap();
        let mut conn = redis
            .get_multiplexed_async_connection()
            .await
            .expect("test Redis is required");
        let hash = Uuid::new_v4().to_string();
        let new_owner = Uuid::new_v4().to_string();
        redis::cmd("SET")
            .arg(session_key(&hash))
            .arg("user")
            .arg("EX")
            .arg(10)
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        redis::cmd("SET")
            .arg(owner_key(&hash))
            .arg(&new_owner)
            .arg("EX")
            .arg(10)
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        let removed: i32 = redis::Script::new(CLEANUP_OWNER_LUA)
            .key(session_key(&hash))
            .key(owner_key(&hash))
            .arg(Uuid::new_v4().to_string())
            .invoke_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(removed, 0);
        let user: String = redis::cmd("GET")
            .arg(session_key(&hash))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(user, "user");
        let removed: i32 = redis::Script::new(CLEANUP_OWNER_LUA)
            .key(session_key(&hash))
            .key(owner_key(&hash))
            .arg(new_owner)
            .invoke_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(removed, 1);
    }
}
