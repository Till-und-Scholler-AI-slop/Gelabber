//! Presence and typing on Redis keys + ephemeral Pub/Sub (issue 8).
//!
//! Keys carry a TTL. Fan-out is `PUBLISH` without `INCR` — these frames
//! never enter the chat replay log and never bump a topic seq.

use std::collections::HashSet;

use uuid::Uuid;

use super::hub::{ConnId, Gateway};
use super::protocol::{
    LiveTopic, PresenceEntry, PresenceMode, PresenceStatus, REDIS_PREFIX, ServerFrame,
};
use crate::error::ApiError;

/// A member connected to a server ([`Gateway::present_members`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Present {
    pub u: Uuid,
    /// Invisible by hand (#590): connected, but shown to nobody.
    pub invisible: bool,
}

/// Set one connection's status, recompute the user aggregate.
/// KEYS: conn, user, conn-index, mode. ARGV: status, ttl_ms, conn-key.
/// Returns the previous and new aggregate and the manual mode (#590), read
/// in the same step, so what others are shown follows from one state.
const SET_PRESENCE_LUA: &str = r#"
redis.call('SET', KEYS[1], ARGV[1], 'PX', ARGV[2])
redis.call('SADD', KEYS[3], ARGV[3])
redis.call('PEXPIRE', KEYS[3], ARGV[2])
local prev = redis.call('GET', KEYS[2])
if not prev then prev = 'x' end
local members = redis.call('SMEMBERS', KEYS[3])
local online = false
local any = false
for _, k in ipairs(members) do
  local st = redis.call('GET', k)
  if st then
    any = true
    if st == 'o' then online = true end
  else
    redis.call('SREM', KEYS[3], k)
  end
end
local agg
if not any then
  redis.call('DEL', KEYS[2])
  agg = 'x'
elseif online then
  redis.call('SET', KEYS[2], 'o', 'PX', ARGV[2])
  agg = 'o'
else
  redis.call('SET', KEYS[2], 'i', 'PX', ARGV[2])
  agg = 'i'
end
return {prev, agg, redis.call('GET', KEYS[4]) or ''}
"#;

/// Drop one connection, recompute the user aggregate. KEYS as
/// [`SET_PRESENCE_LUA`]; ARGV: ttl_ms, conn-key.
const DROP_PRESENCE_LUA: &str = r#"
redis.call('DEL', KEYS[1])
redis.call('SREM', KEYS[3], ARGV[2])
local prev = redis.call('GET', KEYS[2])
if not prev then prev = 'x' end
local members = redis.call('SMEMBERS', KEYS[3])
local online = false
local any = false
for _, k in ipairs(members) do
  local st = redis.call('GET', k)
  if st then
    any = true
    if st == 'o' then online = true end
  else
    redis.call('SREM', KEYS[3], k)
  end
end
local agg
if not any then
  redis.call('DEL', KEYS[2])
  agg = 'x'
elseif online then
  redis.call('SET', KEYS[2], 'o', 'PX', ARGV[1])
  agg = 'o'
else
  redis.call('SET', KEYS[2], 'i', 'PX', ARGV[1])
  agg = 'i'
end
return {prev, agg, redis.call('GET', KEYS[4]) or ''}
"#;

/// Replace the manual mode (#590). KEYS: mode, user. ARGV: the mode letter
/// (empty: automatic, no key), its lifetime in ms (0: until changed).
/// Returns the mode it replaced and the sockets' aggregate.
const SET_MODE_LUA: &str = r#"
local old = redis.call('GET', KEYS[1]) or ''
if ARGV[1] == '' then
  redis.call('DEL', KEYS[1])
elseif tonumber(ARGV[2]) > 0 then
  redis.call('SET', KEYS[1], ARGV[1], 'PX', ARGV[2])
else
  redis.call('SET', KEYS[1], ARGV[1])
end
return {old, redis.call('GET', KEYS[2]) or 'x'}
"#;

/// Remove server-set members whose status is gone, checked in one atomic
/// step. KEYS: server set, then one status key per candidate. ARGV: the
/// candidate members, aligned with KEYS[2..].
///
/// A snapshot that read "no status" can race an announce that writes the
/// status for a member already in the set (its SADD is then a no-op). The
/// re-check here keeps that member instead of removing it for good.
/// Returns the members it removed.
const PRUNE_STALE_LUA: &str = r#"
local removed = {}
for i = 2, #KEYS do
  if redis.call('EXISTS', KEYS[i]) == 0 and redis.call('SREM', KEYS[1], ARGV[i - 1]) == 1 then
    table.insert(removed, ARGV[i - 1])
  end
end
return removed
"#;

/// Refresh TTLs without changing status. Empty string = key was gone.
const TOUCH_PRESENCE_LUA: &str = r#"
local st = redis.call('GET', KEYS[1])
if not st then
  return ''
end
redis.call('PEXPIRE', KEYS[1], ARGV[1])
redis.call('PEXPIRE', KEYS[2], ARGV[1])
redis.call('PEXPIRE', KEYS[3], ARGV[1])
return st
"#;

fn conn_key(user_id: Uuid, conn: ConnId) -> String {
    format!("{REDIS_PREFIX}p:c:{user_id}:{}", conn.redis_id())
}

fn user_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:u:{user_id}")
}

fn conn_index_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:n:{user_id}")
}

/// The account's manual mode (#590) as one letter (`i`, `d`, `v`), with the
/// mode's end as its expiry; no key is automatic. `users.presence_mode` is
/// the record, this the copy the presence scripts read.
fn mode_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:m:{user_id}")
}

/// Set once the manual mode's copy was checked against the record (#590).
fn mode_check_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:mc:{user_id}")
}

/// A day: an account's copy is checked again at least that often.
const MODE_CHECK_MS: i64 = 24 * 60 * 60 * 1000;
/// How long a check may stay `pending` before another socket claims it.
const MODE_CHECK_PENDING_MS: i64 = 30 * 1000;

/// Claims the check (`pending`) unless one is under way or done; returns
/// `claimed`, `pending` or `done`.
const CLAIM_MODE_CHECK_LUA: &str = r#"
if redis.call('SET', KEYS[1], 'pending', 'NX', 'PX', ARGV[1]) then
  return 'claimed'
end
return redis.call('GET', KEYS[1]) or 'claimed'
"#;

/// Where the manual mode's check against the record stands (#590).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeCheck {
    /// This socket checks.
    Claimed,
    /// Another socket is checking: this one checks too, after it.
    Pending,
    /// Checked; nothing to do.
    Done,
}

fn server_set_key(server_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:s:{server_id}")
}

fn user_servers_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:u:{user_id}:s")
}

/// Sockets of a user whose page is in front (focused and visible), as a
/// sorted set: member the socket, score when its claim runs out (Unix ms).
/// Web Push (#243) skips a user with one of them.
fn focus_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}fx:{user_id}")
}

/// The user aggregate around one write: what others were shown before and
/// are shown now (the manual mode applied), and whether any socket is left.
struct Aggregate {
    prev: PresenceStatus,
    now: PresenceStatus,
    connected: bool,
    /// The manual mode read in the same step.
    mode: PresenceMode,
}

impl Aggregate {
    fn from_script((prev, agg, mode): (String, String, String)) -> Self {
        let mode = PresenceMode::from_redis(&mode);
        let agg = parse_st(&agg);
        Self {
            prev: parse_st(&prev).shown(mode),
            now: agg.shown(mode),
            connected: agg != PresenceStatus::Offline,
            mode,
        }
    }
}

fn ttl_ms(d: std::time::Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX).max(1)
}

impl Gateway {
    /// This socket's page came to the front (`on`) or left it. A claim runs
    /// out on its own after `ttl` unless renewed: a socket that dies without
    /// a word does not keep push away for long.
    pub async fn set_focus(
        &self,
        user_id: Uuid,
        id: ConnId,
        on: bool,
        ttl: std::time::Duration,
    ) -> Result<(), ApiError> {
        let key = focus_key(user_id);
        let member = id.redis_id().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let until = now + ttl_ms(ttl);
        let px = ttl_ms(ttl);
        self.with_conn(|mut conn| {
            let key = key.clone();
            let member = member.clone();
            async move {
                let mut pipe = redis::pipe();
                if on {
                    // Renewals keep the key alive, so claims of sockets that
                    // died go here rather than with the key.
                    pipe.cmd("ZREMRANGEBYSCORE")
                        .arg(&key)
                        .arg("-inf")
                        .arg(now)
                        .ignore();
                    pipe.cmd("ZADD").arg(&key).arg(until).arg(&member).ignore();
                    pipe.cmd("PEXPIRE").arg(&key).arg(px).ignore();
                } else {
                    pipe.cmd("ZREM").arg(&key).arg(&member).ignore();
                }
                pipe.query_async::<()>(&mut conn).await
            }
        })
        .await
        .map_err(super::hub::redis_err)
    }

    /// Whether any page of the user is in front right now.
    pub async fn has_focus(&self, user_id: Uuid) -> Result<bool, ApiError> {
        Ok(self.in_front(&[user_id]).await?.contains(&user_id))
    }

    /// Which of `users` have a page in front right now, in one round trip.
    pub async fn in_front(
        &self,
        users: &[Uuid],
    ) -> Result<std::collections::HashSet<Uuid>, ApiError> {
        if users.is_empty() {
            return Ok(Default::default());
        }
        let now = chrono::Utc::now().timestamp_millis();
        let counts: Vec<i64> = self
            .with_conn(|mut conn| {
                let mut pipe = redis::pipe();
                for user in users {
                    pipe.cmd("ZCOUNT")
                        .arg(focus_key(*user))
                        .arg(format!("({now}"))
                        .arg("+inf");
                }
                async move { pipe.query_async(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(users
            .iter()
            .zip(counts)
            .filter(|(_, count)| *count > 0)
            .map(|(user, _)| *user)
            .collect())
    }

    /// Which of `users` have a socket on this process that is in front and
    /// subscribed to `channel_id`, in one round trip. A page in front that
    /// shows another server does not follow this channel and still gets the
    /// push; a socket on another process counts as not in front.
    pub async fn in_front_in(
        &self,
        users: &[Uuid],
        channel_id: Uuid,
    ) -> Result<HashSet<Uuid>, ApiError> {
        let wanted: HashSet<Uuid> = users.iter().copied().collect();
        let topic = super::protocol::Topic::Channel(channel_id);
        let candidates: Vec<(Uuid, String)> = self
            .connections
            .inner
            .sockets
            .read()
            .await
            .iter()
            .filter(|(_, socket)| {
                wanted.contains(&socket.user_id) && socket.topics.contains(&topic)
            })
            .map(|(id, socket)| (socket.user_id, id.redis_id().to_string()))
            .collect();
        if candidates.is_empty() {
            return Ok(HashSet::new());
        }
        let now = chrono::Utc::now().timestamp_millis();
        let scores: Vec<Option<f64>> = self
            .with_conn(|mut conn| {
                let mut pipe = redis::pipe();
                for (user, member) in &candidates {
                    pipe.cmd("ZSCORE").arg(focus_key(*user)).arg(member);
                }
                async move { pipe.query_async(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(candidates
            .into_iter()
            .zip(scores)
            .filter(|(_, until)| until.is_some_and(|until| until > now as f64))
            .map(|((user, _), _)| user)
            .collect())
    }

    /// Mark this connection online/idle and fan-out if the user aggregate changed.
    pub async fn set_conn_status(
        &self,
        id: ConnId,
        status: PresenceStatus,
    ) -> Result<PresenceStatus, ApiError> {
        let Some((user_id, local)) = self.socket_meta(id).await else {
            return Ok(status);
        };
        if matches!(status, PresenceStatus::Offline) {
            return self.drop_conn_presence(id, user_id, &local).await;
        }
        let agg = self.write_presence(user_id, id, status).await?;
        if agg.prev != agg.now {
            let servers = self.presence_targets(user_id, &local).await?;
            self.broadcast_presence(user_id, agg.now, &servers).await?;
        }
        Ok(agg.now)
    }

    /// The account's manual mode changed (#590): store the copy the presence
    /// scripts read, then tell every server it is on what it shows now. The
    /// mode runs out on its own at `until`; the sweep in `crate::presence`
    /// announces that.
    pub async fn set_presence_mode(
        &self,
        user_id: Uuid,
        mode: PresenceMode,
        until: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), ApiError> {
        let lifetime = match until {
            None => 0,
            Some(until) => (until - chrono::Utc::now()).num_milliseconds(),
        };
        // Already over: automatic, as the record reads from now on.
        let mode = if until.is_some() && lifetime <= 0 {
            PresenceMode::Auto
        } else {
            mode
        };
        let (old, agg): (String, String) = self
            .with_conn(|mut conn| async move {
                redis::Script::new(SET_MODE_LUA)
                    .key(mode_key(user_id))
                    .key(user_key(user_id))
                    .arg(mode.redis().unwrap_or(""))
                    .arg(lifetime.max(0))
                    .invoke_async(&mut conn)
                    .await
            })
            .await
            .map_err(super::hub::redis_err)?;
        let agg = parse_st(&agg);
        let before = agg.shown(PresenceMode::from_redis(&old));
        let after = agg.shown(mode);
        if before != after {
            let servers = self.presence_targets(user_id, &HashSet::new()).await?;
            self.broadcast_presence(user_id, after, &servers).await?;
        }
        Ok(())
    }

    /// A mode ran out (#590): its Redis copy is gone by now, so the members
    /// are told what `old` hid, from what the sockets report.
    pub async fn presence_mode_ended(
        &self,
        user_id: Uuid,
        old: PresenceMode,
    ) -> Result<(), ApiError> {
        let (_, agg): (String, String) = self
            .with_conn(|mut conn| async move {
                redis::Script::new(SET_MODE_LUA)
                    .key(mode_key(user_id))
                    .key(user_key(user_id))
                    .arg("")
                    .arg(0)
                    .invoke_async(&mut conn)
                    .await
            })
            .await
            .map_err(super::hub::redis_err)?;
        let agg = parse_st(&agg);
        let (before, after) = (agg.shown(old), agg.shown(PresenceMode::Auto));
        if before != after {
            let servers = self.presence_targets(user_id, &HashSet::new()).await?;
            self.broadcast_presence(user_id, after, &servers).await?;
        }
        Ok(())
    }

    /// Whether the manual mode's copy still has to be checked against the
    /// record: claimed once per Redis lifetime and account (a day at most),
    /// as the marker goes with the copy when Redis restarts.
    pub async fn claim_mode_check(&self, user_id: Uuid) -> Result<ModeCheck, ApiError> {
        let state: String = self
            .with_conn(|mut conn| async move {
                redis::Script::new(CLAIM_MODE_CHECK_LUA)
                    .key(mode_check_key(user_id))
                    .arg(MODE_CHECK_PENDING_MS)
                    .invoke_async(&mut conn)
                    .await
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(match state.as_str() {
            "claimed" => ModeCheck::Claimed,
            "pending" => ModeCheck::Pending,
            _ => ModeCheck::Done,
        })
    }

    /// The copy matches the record: later sockets skip the check.
    pub async fn finish_mode_check(&self, user_id: Uuid) -> Result<(), ApiError> {
        self.with_conn(|mut conn| async move {
            redis::cmd("SET")
                .arg(mode_check_key(user_id))
                .arg("done")
                .arg("PX")
                .arg(MODE_CHECK_MS)
                .query_async::<()>(&mut conn)
                .await
        })
        .await
        .map_err(super::hub::redis_err)?;
        Ok(())
    }

    pub async fn drop_mode_check(&self, user_id: Uuid) -> Result<(), ApiError> {
        self.with_conn(|mut conn| async move {
            redis::cmd("DEL")
                .arg(mode_check_key(user_id))
                .query_async::<i64>(&mut conn)
                .await
        })
        .await
        .map_err(super::hub::redis_err)?;
        Ok(())
    }

    /// The manual mode the presence scripts see now; automatic without Redis.
    pub async fn presence_mode(&self, user_id: Uuid) -> Result<PresenceMode, ApiError> {
        let raw: Option<String> = self
            .with_conn(|mut conn| async move {
                redis::cmd("GET")
                    .arg(mode_key(user_id))
                    .query_async(&mut conn)
                    .await
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(raw.map_or(PresenceMode::Auto, |raw| PresenceMode::from_redis(&raw)))
    }

    /// Heartbeat: keep Redis keys alive without flipping idle → online.
    pub async fn touch_presence(&self, id: ConnId) -> Result<(), ApiError> {
        let Some((user_id, servers)) = self.socket_meta(id).await else {
            return Ok(());
        };
        let ck = conn_key(user_id, id);
        let uk = user_key(user_id);
        let nk = conn_index_key(user_id);
        let px = ttl_ms(self.presence_ttl());
        let st: String = self
            .with_conn(|mut conn| {
                let ck = ck.clone();
                let uk = uk.clone();
                let nk = nk.clone();
                async move {
                    redis::Script::new(TOUCH_PRESENCE_LUA)
                        .key(ck)
                        .key(uk)
                        .key(nk)
                        .arg(px)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(super::hub::redis_err)?;
        if st.is_empty() {
            return Ok(());
        }
        let targets = self.presence_targets(user_id, &servers).await?;
        self.expire_server_sets(user_id, &targets).await?;
        Ok(())
    }

    /// Mark this connection online and add the user to a server's presence
    /// set. Publishes at most one frame per server: an aggregate change
    /// already reaches this server, otherwise a first announce publishes the
    /// current status.
    ///
    /// Order matters. The status is written before the SADD: a concurrent
    /// `presence_snapshot` treats a set member without status as stale and
    /// removes it, which would hide this user from every later snapshot.
    /// (A member already in the set with an expired status is covered by
    /// the snapshot itself, which re-checks the status atomically before
    /// removing anyone.)
    /// Both publishes follow the SADD, so a concurrent snapshot either lists
    /// the user or its socket receives the frame.
    pub async fn announce_server(&self, id: ConnId, server_id: Uuid) -> Result<(), ApiError> {
        let Some((user_id, local)) = self.socket_meta(id).await else {
            return Ok(());
        };
        let agg = self
            .write_presence(user_id, id, PresenceStatus::Online)
            .await?;
        let px = ttl_ms(self.presence_ttl());
        let added: i64 = self
            .with_conn(|mut conn| {
                let sk = server_set_key(server_id);
                let usk = user_servers_key(user_id);
                let uid = user_id.to_string();
                let sid = server_id.to_string();
                async move {
                    let added: i64 = redis::cmd("SADD")
                        .arg(&sk)
                        .arg(&uid)
                        .query_async(&mut conn)
                        .await?;
                    let _: i64 = redis::cmd("SADD")
                        .arg(&usk)
                        .arg(&sid)
                        .query_async(&mut conn)
                        .await?;
                    let _: i64 = redis::cmd("PEXPIRE")
                        .arg(&sk)
                        .arg(px)
                        .query_async(&mut conn)
                        .await?;
                    let _: i64 = redis::cmd("PEXPIRE")
                        .arg(&usk)
                        .arg(px)
                        .query_async(&mut conn)
                        .await?;
                    Ok(added)
                }
            })
            .await
            .map_err(super::hub::redis_err)?;
        if agg.prev != agg.now {
            let mut servers = self.presence_targets(user_id, &local).await?;
            servers.insert(server_id);
            self.broadcast_presence(user_id, agg.now, &servers).await?;
        } else if added == 1 && agg.now != PresenceStatus::Offline {
            // Invisible: the server's members have nothing to hear.
            self.publish_presence(server_id, user_id, agg.now).await?;
        }
        Ok(())
    }

    /// The status each of `users` is shown with, in one round trip: the
    /// sockets' aggregate with the manual mode (#590) on top; someone
    /// without a live socket, or invisible, is left out. For the friend
    /// list (#563), which is not a server topic and so has no live frames.
    pub async fn statuses_of(
        &self,
        users: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, PresenceStatus>, ApiError> {
        if users.is_empty() {
            return Ok(Default::default());
        }
        let mut statuses: Vec<Option<String>> = self
            .with_conn(|mut conn| {
                let mut cmd = redis::cmd("MGET");
                for user_id in users {
                    cmd.arg(user_key(*user_id));
                }
                for user_id in users {
                    cmd.arg(mode_key(*user_id));
                }
                async move { cmd.query_async::<Vec<Option<String>>>(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;
        let modes = statuses.split_off(users.len().min(statuses.len()));
        Ok(users
            .iter()
            .zip(statuses)
            .enumerate()
            .filter_map(|(index, (user, raw))| {
                let mode = modes
                    .get(index)
                    .and_then(Option::as_deref)
                    .map_or(PresenceMode::Auto, PresenceMode::from_redis);
                let shown = raw.as_deref().and_then(PresenceStatus::parse)?.shown(mode);
                (shown != PresenceStatus::Offline).then_some((*user, shown))
            })
            .collect())
    }

    /// Who is shown on a server and how, the manual modes (#590) applied:
    /// an invisible member is left out.
    pub async fn presence_snapshot(&self, server_id: Uuid) -> Result<Vec<PresenceEntry>, ApiError> {
        Ok(self
            .snapshot(server_id, true)
            .await?
            .into_iter()
            .map(|(entry, _)| entry)
            .collect())
    }

    /// Who is really connected on a server, whatever they show (`@here`:
    /// an invisible member is there all the same, and says so).
    pub async fn present_members(&self, server_id: Uuid) -> Result<Vec<Present>, ApiError> {
        Ok(self
            .snapshot(server_id, false)
            .await?
            .into_iter()
            .filter(|(entry, _)| entry.st != PresenceStatus::Offline)
            .map(|(entry, mode)| Present {
                u: entry.u,
                invisible: mode == PresenceMode::Invisible,
            })
            .collect())
    }

    async fn snapshot(
        &self,
        server_id: Uuid,
        shown: bool,
    ) -> Result<Vec<(PresenceEntry, PresenceMode)>, ApiError> {
        let users: Vec<String> = self
            .with_conn(|mut conn| {
                let key = server_set_key(server_id);
                async move { redis::cmd("SMEMBERS").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;

        let mut ids = Vec::new();
        let mut malformed = Vec::new();
        for raw in users {
            match raw.parse::<Uuid>() {
                Ok(user_id) => ids.push(user_id),
                Err(_) => malformed.push(raw),
            }
        }
        // Every member's status and manual mode in one round trip. MGET
        // without a key is an error, so an empty set skips it.
        let mut values: Vec<Option<String>> = if ids.is_empty() {
            Vec::new()
        } else {
            self.with_conn(|mut conn| {
                let mut cmd = redis::cmd("MGET");
                for user_id in &ids {
                    cmd.arg(user_key(*user_id));
                }
                for user_id in &ids {
                    cmd.arg(mode_key(*user_id));
                }
                async move { cmd.query_async::<Vec<Option<String>>>(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?
        };
        let modes = values.split_off(ids.len().min(values.len()));
        let mut snap = Vec::new();
        let mut stale = Vec::new();
        for (index, (user_id, raw)) in ids.into_iter().zip(values).enumerate() {
            let mode = modes
                .get(index)
                .and_then(Option::as_deref)
                .map_or(PresenceMode::Auto, PresenceMode::from_redis);
            match raw.as_deref().and_then(PresenceStatus::parse) {
                Some(st) if !shown => snap.push((PresenceEntry { u: user_id, st }, mode)),
                Some(st) => match st.shown(mode) {
                    // Invisible, and still connected: not stale.
                    PresenceStatus::Offline => {}
                    st => snap.push((PresenceEntry { u: user_id, st }, mode)),
                },
                None => stale.push((user_id, mode)),
            }
        }
        if !malformed.is_empty() {
            let _ = self
                .with_conn(|mut conn| {
                    let key = server_set_key(server_id);
                    let malformed = malformed.clone();
                    async move {
                        let mut cmd = redis::cmd("SREM");
                        cmd.arg(key);
                        for member in malformed {
                            cmd.arg(member);
                        }
                        cmd.query_async::<i64>(&mut conn).await
                    }
                })
                .await;
        }
        // The status may have been written since it was read above (an
        // announce for a member already in the set), so the removal re-checks
        // it atomically instead of trusting this read.
        if !stale.is_empty() {
            let pruned = self
                .with_conn(|mut conn| {
                    let stale: Vec<Uuid> = stale.iter().map(|(user_id, _)| *user_id).collect();
                    async move {
                        let prune = redis::Script::new(PRUNE_STALE_LUA);
                        let mut call = prune.prepare_invoke();
                        call.key(server_set_key(server_id));
                        for user_id in &stale {
                            call.key(user_key(*user_id));
                        }
                        for user_id in &stale {
                            call.arg(user_id.to_string());
                        }
                        call.invoke_async::<Vec<String>>(&mut conn).await
                    }
                })
                .await;
            // A status that ran out never said offline: whoever saw the
            // member online hears it now, from the first snapshot to notice.
            // An invisible one (#590) was never seen, so it stays quiet.
            for user_id in pruned.unwrap_or_default() {
                if let Ok(user_id) = user_id.parse::<Uuid>()
                    && !stale
                        .iter()
                        .any(|(id, mode)| *id == user_id && *mode == PresenceMode::Invisible)
                    && let Err(err) = self
                        .publish_presence(server_id, user_id, PresenceStatus::Offline)
                        .await
                {
                    tracing::debug!(error = err.code(), "offline after prune not published");
                }
            }
        }
        snap.sort_by_key(|(entry, _)| entry.u);
        Ok(snap)
    }

    /// Drop this socket: stop its typing immediately, then recompute presence.
    /// Offline / idle fan-out uses `gb:p:u:{user}:s`, not only this socket.
    pub async fn clear_conn(
        &self,
        id: ConnId,
        user_id: Uuid,
        servers: &HashSet<Uuid>,
        typing: &HashSet<(Uuid, Uuid)>,
    ) {
        for (server_id, channel_id) in typing {
            if let Err(err) = self
                .publish_typing_stop(*server_id, *channel_id, user_id)
                .await
            {
                tracing::debug!(error = err.code(), "typing stop on detach failed");
            }
        }
        if let Err(err) = self.drop_conn_presence(id, user_id, servers).await {
            tracing::debug!(error = err.code(), "presence cleanup failed");
        }
    }

    pub async fn set_typing(
        &self,
        conn: ConnId,
        server_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
        on: bool,
    ) -> Result<(), ApiError> {
        self.note_typing(conn, server_id, channel_id, on).await;
        self.publish_live(
            LiveTopic::Typing(channel_id),
            &ServerFrame::typing(server_id, channel_id, user_id, on),
        )
        .await
    }

    pub async fn stop_typing(
        &self,
        server_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ApiError> {
        self.publish_typing_stop(server_id, channel_id, user_id)
            .await
    }

    /// Kick/ban/leave or server delete: drop these users from one server's
    /// presence set and tell remaining members they went offline there.
    /// Other servers stay. One pipeline for all users; a retry repeats only
    /// idempotent removals and offline frames.
    pub async fn forget_server_presence(
        &self,
        user_ids: &[Uuid],
        server_id: Uuid,
    ) -> Result<(), ApiError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let channel = LiveTopic::Presence(server_id).redis_channel();
        let frames = user_ids
            .iter()
            .map(|user_id| {
                ServerFrame::presence(server_id, *user_id, PresenceStatus::Offline)
                    .to_json()
                    .map_err(|err| ApiError::Internal(format!("serialize live frame: {err}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.with_conn(|mut conn| {
            let mut pipe = redis::pipe();
            pipe.cmd("SREM")
                .arg(server_set_key(server_id))
                .arg(user_ids.iter().map(Uuid::to_string).collect::<Vec<_>>())
                .ignore();
            for user_id in user_ids {
                pipe.cmd("SREM")
                    .arg(user_servers_key(*user_id))
                    .arg(server_id.to_string())
                    .ignore();
            }
            for raw in &frames {
                pipe.cmd("PUBLISH").arg(&channel).arg(raw).ignore();
            }
            async move { pipe.query_async::<()>(&mut conn).await }
        })
        .await
        .map_err(super::hub::redis_err)
    }

    async fn publish_typing_stop(
        &self,
        server_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ApiError> {
        self.publish_live(
            LiveTopic::Typing(channel_id),
            &ServerFrame::typing(server_id, channel_id, user_id, false),
        )
        .await
    }

    pub async fn drop_conn_presence(
        &self,
        id: ConnId,
        user_id: Uuid,
        local: &HashSet<Uuid>,
    ) -> Result<PresenceStatus, ApiError> {
        let agg = self.delete_presence(user_id, id).await?;
        let servers = self.presence_targets(user_id, local).await?;
        if !agg.connected {
            self.forget_user_servers(user_id, &servers).await?;
            // Invisible (#590): nobody saw this account online, so nobody
            // hears it leave either.
            if agg.mode != PresenceMode::Invisible {
                self.broadcast_presence(user_id, PresenceStatus::Offline, &servers)
                    .await?;
            }
        } else if agg.prev != agg.now {
            self.broadcast_presence(user_id, agg.now, &servers).await?;
        }
        Ok(agg.now)
    }

    /// Every server this user has announced on, plus this socket's set
    /// (in case Redis has not been written yet).
    async fn presence_targets(
        &self,
        user_id: Uuid,
        extra: &HashSet<Uuid>,
    ) -> Result<HashSet<Uuid>, ApiError> {
        let mut servers = self.load_user_servers(user_id).await?;
        servers.extend(extra.iter().copied());
        Ok(servers)
    }

    async fn load_user_servers(&self, user_id: Uuid) -> Result<HashSet<Uuid>, ApiError> {
        let raw: Vec<String> = self
            .with_conn(|mut conn| {
                let key = user_servers_key(user_id);
                async move { redis::cmd("SMEMBERS").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(raw
            .into_iter()
            .filter_map(|id| id.parse::<Uuid>().ok())
            .collect())
    }

    async fn write_presence(
        &self,
        user_id: Uuid,
        id: ConnId,
        status: PresenceStatus,
    ) -> Result<Aggregate, ApiError> {
        let ck = conn_key(user_id, id);
        let uk = user_key(user_id);
        let nk = conn_index_key(user_id);
        let px = ttl_ms(self.presence_ttl());
        let result: (String, String, String) = self
            .with_conn(|mut conn| {
                let ck = ck.clone();
                let uk = uk.clone();
                let nk = nk.clone();
                let member = ck.clone();
                async move {
                    redis::Script::new(SET_PRESENCE_LUA)
                        .key(ck)
                        .key(uk)
                        .key(nk)
                        .key(mode_key(user_id))
                        .arg(status.as_str())
                        .arg(px)
                        .arg(member)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(Aggregate::from_script(result))
    }

    async fn delete_presence(&self, user_id: Uuid, id: ConnId) -> Result<Aggregate, ApiError> {
        let ck = conn_key(user_id, id);
        let uk = user_key(user_id);
        let nk = conn_index_key(user_id);
        let px = ttl_ms(self.presence_ttl());
        let result: (String, String, String) = self
            .with_conn(|mut conn| {
                let ck = ck.clone();
                let uk = uk.clone();
                let nk = nk.clone();
                let member = ck.clone();
                async move {
                    redis::Script::new(DROP_PRESENCE_LUA)
                        .key(ck)
                        .key(uk)
                        .key(nk)
                        .key(mode_key(user_id))
                        .arg(px)
                        .arg(member)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(Aggregate::from_script(result))
    }

    async fn broadcast_presence(
        &self,
        user_id: Uuid,
        status: PresenceStatus,
        servers: &HashSet<Uuid>,
    ) -> Result<(), ApiError> {
        for server_id in servers {
            self.publish_presence(*server_id, user_id, status).await?;
        }
        Ok(())
    }

    async fn publish_presence(
        &self,
        server_id: Uuid,
        user_id: Uuid,
        status: PresenceStatus,
    ) -> Result<(), ApiError> {
        self.publish_live(
            LiveTopic::Presence(server_id),
            &ServerFrame::presence(server_id, user_id, status),
        )
        .await
    }

    async fn publish_live(&self, topic: LiveTopic, frame: &ServerFrame) -> Result<(), ApiError> {
        let raw = frame
            .to_json()
            .map_err(|err| ApiError::Internal(format!("serialize live frame: {err}")))?;
        let channel = topic.redis_channel();
        self.with_conn(|mut conn| {
            let channel = channel.clone();
            let raw = raw.clone();
            async move {
                redis::cmd("PUBLISH")
                    .arg(channel)
                    .arg(raw)
                    .query_async::<i64>(&mut conn)
                    .await
            }
        })
        .await
        .map_err(super::hub::redis_err)?;
        Ok(())
    }

    async fn expire_server_sets(
        &self,
        user_id: Uuid,
        servers: &HashSet<Uuid>,
    ) -> Result<(), ApiError> {
        let px = ttl_ms(self.presence_ttl());
        self.with_conn(|mut conn| {
            let usk = user_servers_key(user_id);
            let servers: Vec<Uuid> = servers.iter().copied().collect();
            async move {
                let _: i64 = redis::cmd("PEXPIRE")
                    .arg(&usk)
                    .arg(px)
                    .query_async(&mut conn)
                    .await?;
                for server_id in servers {
                    let _: i64 = redis::cmd("PEXPIRE")
                        .arg(server_set_key(server_id))
                        .arg(px)
                        .query_async(&mut conn)
                        .await?;
                }
                Ok(())
            }
        })
        .await
        .map_err(super::hub::redis_err)
    }

    async fn forget_user_servers(
        &self,
        user_id: Uuid,
        servers: &HashSet<Uuid>,
    ) -> Result<(), ApiError> {
        self.with_conn(|mut conn| {
            let usk = user_servers_key(user_id);
            let uid = user_id.to_string();
            let servers: Vec<Uuid> = servers.iter().copied().collect();
            async move {
                for server_id in servers {
                    let _: i64 = redis::cmd("SREM")
                        .arg(server_set_key(server_id))
                        .arg(&uid)
                        .query_async(&mut conn)
                        .await?;
                }
                let _: i64 = redis::cmd("DEL").arg(usk).query_async(&mut conn).await?;
                Ok(())
            }
        })
        .await
        .map_err(super::hub::redis_err)
    }
}

fn parse_st(raw: &str) -> PresenceStatus {
    PresenceStatus::parse(raw).unwrap_or(PresenceStatus::Offline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// A member whose status ran out is pruned by the next snapshot, and the
    /// server's watchers hear that it went offline.
    #[tokio::test]
    async fn a_pruned_member_goes_offline_for_the_watchers() {
        let g = Gateway::new(
            redis::Client::open(std::env::var("REDIS_URL").unwrap()).unwrap(),
            8,
            Duration::from_secs(10),
            Duration::from_secs(2),
        );
        g.wait_ready(Duration::from_secs(2)).await.unwrap();
        let (server, gone) = (Uuid::new_v4(), Uuid::new_v4());
        let (tx, mut rx) = mpsc::channel(128);
        let (watcher, _) = g.attach_session(Uuid::new_v4(), "watcher".into(), tx).await;
        g.watch_server(watcher, server).await;
        let mut conn = redis::Client::open(std::env::var("REDIS_URL").unwrap())
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        redis::cmd("SADD")
            .arg(server_set_key(server))
            .arg(gone.to_string())
            .query_async::<i64>(&mut conn)
            .await
            .unwrap();

        assert!(g.presence_snapshot(server).await.unwrap().is_empty());
        // The roster pass sends voice snapshots meanwhile.
        let frame = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Some(ServerFrame::Sig { .. }) => continue,
                    other => return other,
                }
            }
        })
        .await
        .expect("offline frame")
        .unwrap();
        assert_eq!(
            frame,
            ServerFrame::presence(server, gone, PresenceStatus::Offline)
        );
        g.detach(watcher).await;
    }
}
