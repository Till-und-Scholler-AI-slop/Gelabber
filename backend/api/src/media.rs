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
use gelabber_shared::ticket::{
    self, TicketClaim, channel_authority_key, media_demand_key, member_authority_key,
    session_authority_key, session_owner_key,
};
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
    if !member.sees(&channel) {
        return Err(ApiError::NotFound);
    }
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
        state.session_ttl,
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
    sqlx::query_as::<_, Channel>(concat!(
        "SELECT ",
        crate::servers::channel::channel_columns!(),
        " FROM channels c WHERE c.id = $1"
    ))
    .bind(channel_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

/// The session's absolute end (`created_at + ttl`). Its row's expiry slides
/// with use (#195), so a copy of that taken at mint would end calls early.
fn session_end(session: &CurrentSession, session_ttl: Duration) -> i64 {
    chrono::Duration::from_std(session_ttl)
        .ok()
        .and_then(|ttl| session.created_at.checked_add_signed(ttl))
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MAX_UTC)
        .timestamp()
}

async fn mint(
    redis: &redis::Client,
    claim: TicketClaim,
    session: &CurrentSession,
    ttl: Duration,
    session_ttl: Duration,
) -> Result<(String, u64), ApiError> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
    let member_key = member_authority_key(claim.s, claim.u);
    let channel_key = channel_authority_key(claim.c);
    let session_key = session_authority_key(&session.key());
    let secs = ttl.as_secs().max(1).min(
        (session.expires_at - chrono::Utc::now())
            .num_seconds()
            .max(0) as u64,
    );
    let ends_at = session_end(session, session_ttl);
    if secs == 0 || ends_at <= chrono::Utc::now().timestamp() {
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
            .key(media_demand_key(&session.key()))
            .arg(&base)
            .arg(secs)
            .arg(Uuid::new_v4().to_string())
            .arg(Uuid::new_v4().to_string())
            .arg(session.key())
            .arg(ends_at)
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
// a ticket from an earlier Redis lifetime. These generations alone enforce
// kick/ban/leave, so a fresh membership can rejoin immediately.
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

/// Rotate the member generations of `user_ids` on one server in a single
/// pipeline (#294), so a server delete or a permission cut costs one round
/// trip instead of one connection per member. Safe to repeat on retry.
pub async fn revoke_members(
    conn: &mut redis::aio::MultiplexedConnection,
    server_id: Uuid,
    user_ids: &[Uuid],
) -> Result<(), redis::RedisError> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let mut pipe = redis::pipe();
    for user_id in user_ids {
        pipe.cmd("SET")
            .arg(member_authority_key(server_id, *user_id))
            .arg(Uuid::new_v4().to_string())
            .ignore();
    }
    pipe.query_async::<()>(conn).await
}
pub async fn revoke_channel(redis: &redis::Client, channel_id: Uuid) -> Result<(), ApiError> {
    rotate(redis, channel_authority_key(channel_id)).await
}
pub async fn revoke_session(redis: &redis::Client, hash: &str) -> Result<(), ApiError> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
    redis::cmd("DEL")
        .arg(session_authority_key(hash))
        .query_async::<()>(&mut conn)
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))
}

/// A DB-backed session lease bounds missed logout notifications to 3 seconds.
/// Refresh holds the same session row lock as ticket mint, so logout cannot race
/// a successful check followed by a lease resurrection. Redis/DB errors are
/// retried until the lease has run out; 03b must fail closed when it disappears.
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

/// The lease's TTL (`PX 3000` in the Lua above).
const SESSION_LEASE: Duration = Duration::from_secs(3);
/// The pause before retrying a failed refresh: inside the lease's slack.
const LEASE_RETRY: Duration = Duration::from_millis(250);

const CLEANUP_OWNER_LUA: &str = r#"
if redis.call('GET', KEYS[2]) ~= ARGV[1] then return 0 end
redis.call('DEL', KEYS[1], KEYS[2])
return 1
"#;

async fn maintain_session(state: &AppState, session: &CurrentSession) -> Result<(), ApiError> {
    let key = session.key();
    let Some(generation) = state.gateway.track_media_session(&key).await else {
        return Ok(());
    };
    // The gateway's cached connection, not a new one per call. Every call
    // below is safe to retry: SET PX and the refresh are idempotent, and
    // the cleanup is a compare-and-delete.
    let registered = tokio::time::timeout(Duration::from_secs(1), async {
        state
            .gateway
            .with_redis(|mut conn| {
                let (owner, generation) = (session_owner_key(&key), generation.to_string());
                async move {
                    redis::cmd("SET")
                        .arg(owner)
                        .arg(generation)
                        .arg("PX")
                        .arg(5000)
                        .query_async::<()>(&mut conn)
                        .await
                }
            })
            .await
            .map_err(|err| ApiError::Internal(format!("redis: {err}")))
    })
    .await;
    if !matches!(registered, Ok(Ok(()))) {
        state.gateway.untrack_media_session(&key, generation).await;
        return Err(ApiError::Internal("media session lease unavailable".into()));
    }
    let ends_at = session_end(session, state.session_ttl);
    let state = state.clone();
    let session = session.clone();
    tokio::spawn(async move {
        // Mint set the lease just before this worker started.
        let mut renewed_at = tokio::time::Instant::now();
        let mut pause = Duration::from_secs(1);
        loop {
            tokio::time::sleep(pause).await;
            let attempt = tokio::time::Instant::now();
            let refreshed = tokio::time::timeout(Duration::from_secs(1), async {
                let Some(_guard) = session.lock_live(&state.db).await? else {
                    return Ok::<String, ApiError>("expired".into());
                };
                state
                    .gateway
                    .with_redis(|mut conn| {
                        let (lease, demand, owner) = (
                            session_authority_key(&key),
                            media_demand_key(&key),
                            session_owner_key(&key),
                        );
                        let user = session.user.id.to_string();
                        let generation = generation.to_string();
                        async move {
                            redis::Script::new(REFRESH_SESSION_LUA)
                                .key(lease)
                                .key(demand)
                                .key(owner)
                                .arg(user)
                                .arg(ends_at)
                                .arg(generation)
                                .invoke_async::<String>(&mut conn)
                                .await
                        }
                    })
                    .await
                    .map_err(|err| ApiError::Internal(format!("redis: {err}")))
            })
            .await;
            match refreshed {
                Ok(Ok(status)) if status == "OK" => {
                    renewed_at = attempt;
                    pause = Duration::from_secs(1);
                }
                Ok(Ok(status))
                    if status == "idle"
                        && state.gateway.media_session_recent(&key, generation).await =>
                {
                    pause = Duration::from_secs(1);
                }
                // `expired`, `lost`, or idle without a recent mint: definite.
                Ok(Ok(_)) => break,
                // A failed or slow refresh is retried while the lease lives.
                _ if renewed_at.elapsed() < SESSION_LEASE => pause = LEASE_RETRY,
                _ => break,
            }
        }
        state.gateway.untrack_media_session(&key, generation).await;
        // A delayed worker/retry may only delete authority it still owns.
        for _ in 0..4 {
            let result = tokio::time::timeout(
                Duration::from_millis(500),
                state.gateway.with_redis(|mut conn| {
                    let (lease, owner) = (session_authority_key(&key), session_owner_key(&key));
                    let generation = generation.to_string();
                    async move {
                        redis::Script::new(CLEANUP_OWNER_LUA)
                            .key(lease)
                            .key(owner)
                            .arg(generation)
                            .invoke_async::<i32>(&mut conn)
                            .await
                    }
                }),
            )
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
    async fn test_session(
        pool: &sqlx::PgPool,
        ttl_hours: &str,
    ) -> (AppState, CurrentSession, redis::aio::MultiplexedConnection) {
        let config = crate::Config::from_source(|key| match key {
            "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".into()),
            "REDIS_URL" => Some(std::env::var("REDIS_URL").unwrap()),
            "API_SESSION_TTL_HOURS" => Some(ttl_hours.to_owned()),
            "API_SESSION_IDLE_HOURS" => Some("1".to_owned()),
            _ => None,
        })
        .unwrap();
        let state = AppState::with_pool(&config, pool.clone()).unwrap();
        let user = crate::auth::user::insert(
            pool,
            "media@example.test",
            "media",
            "unused",
            Default::default(),
        )
        .await
        .unwrap();
        let raw = crate::auth::session::create(pool, user.id, Duration::from_secs(3600))
            .await
            .unwrap();
        let hash = crate::token::hash(&raw);
        let (created_at, expires_at) =
            sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                "SELECT created_at, expires_at FROM sessions WHERE token_hash = $1",
            )
            .bind(&hash)
            .fetch_one(pool)
            .await
            .unwrap();
        let conn = state
            .redis
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let session = CurrentSession {
            user,
            hash,
            created_at,
            expires_at,
        };
        (state, session, conn)
    }

    /// Holds the session row so the worker's `FOR SHARE` waits, for `hold`.
    async fn block_session_row(pool: &sqlx::PgPool, session: &CurrentSession, hold: Duration) {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT 1 FROM sessions WHERE token_hash = $1 FOR UPDATE")
            .bind(&session.hash)
            .execute(&mut *tx)
            .await
            .unwrap();
        tokio::time::sleep(hold).await;
        tx.commit().await.unwrap();
    }

    #[sqlx::test]
    async fn one_slow_refresh_keeps_the_lease_and_a_lasting_fault_ends_it(pool: sqlx::PgPool) {
        let (state, session, mut conn) = test_session(&pool, "2").await;
        let key = session.key();
        let user = session.user.id.to_string();
        // What a mint leaves behind: the 3 s lease and media's demand.
        redis::cmd("SET")
            .arg(session_authority_key(&key))
            .arg(&user)
            .arg("PX")
            .arg(3000)
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        redis::cmd("SET")
            .arg(media_demand_key(&key))
            .arg("1")
            .arg("EX")
            .arg(35)
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        maintain_session(&state, &session).await.unwrap();
        // The first refresh (at 1 s) times out at 2 s; the retry gets the row.
        block_session_row(&pool, &session, Duration::from_millis(2300)).await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let lease: Option<String> = redis::cmd("GET")
            .arg(session_authority_key(&key))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            lease.as_deref(),
            Some(user.as_str()),
            "lease survives one miss"
        );

        // The worker untracks the session when it gives up, so a new mint
        // would start another one.
        let blocked = block_session_row(&pool, &session, Duration::from_secs(7));
        let ended = tokio::time::timeout(Duration::from_secs(6), async {
            while state.gateway.track_media_session(&key).await.is_none() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        let (_, ended) = tokio::join!(blocked, ended);
        ended.expect("the worker gives up once failures used the lease up");
    }

    #[sqlx::test]
    async fn ticket_and_lease_end_with_the_session_not_its_expiry_at_mint(pool: sqlx::PgPool) {
        let (state, mut session, mut conn) = test_session(&pool, "2").await;
        let key = session.key();
        let claim = TicketClaim {
            u: session.user.id,
            s: Uuid::new_v4(),
            c: Uuid::new_v4(),
            g: false,
        };
        let (code, _) = mint(
            &state.redis,
            claim,
            &session,
            state.media_ticket_ttl,
            state.session_ttl,
        )
        .await
        .unwrap();
        let raw: String = redis::cmd("GET")
            .arg(ticket::redis_key(&code))
            .query_async(&mut conn)
            .await
            .unwrap();
        let ticket: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let end = (session.created_at + chrono::Duration::hours(2)).timestamp();
        assert_eq!(ticket["auth"]["expires_at"], end, "{ticket}");

        // The worker's copy is from mint; the row has slid on since.
        session.expires_at = chrono::Utc::now() + chrono::Duration::milliseconds(500);
        redis::cmd("SET")
            .arg(media_demand_key(&key))
            .arg("1")
            .arg("EX")
            .arg(35)
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        maintain_session(&state, &session).await.unwrap();
        tokio::time::sleep(Duration::from_millis(3500)).await;
        let lease: Option<String> = redis::cmd("GET")
            .arg(session_authority_key(&key))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(lease, Some(session.user.id.to_string()));
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
            .arg(session_authority_key(&hash))
            .arg("user")
            .arg("EX")
            .arg(10)
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        redis::cmd("SET")
            .arg(session_owner_key(&hash))
            .arg(&new_owner)
            .arg("EX")
            .arg(10)
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        let removed: i32 = redis::Script::new(CLEANUP_OWNER_LUA)
            .key(session_authority_key(&hash))
            .key(session_owner_key(&hash))
            .arg(Uuid::new_v4().to_string())
            .invoke_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(removed, 0);
        let user: String = redis::cmd("GET")
            .arg(session_authority_key(&hash))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(user, "user");
        let removed: i32 = redis::Script::new(CLEANUP_OWNER_LUA)
            .key(session_authority_key(&hash))
            .key(session_owner_key(&hash))
            .arg(new_owner)
            .invoke_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(removed, 1);
    }
}
