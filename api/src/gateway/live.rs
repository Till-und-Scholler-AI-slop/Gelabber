//! Presence and typing on Redis keys + ephemeral Pub/Sub (issue 8).
//!
//! Keys carry a TTL. Fan-out is `PUBLISH` without `INCR` — these frames
//! never enter the chat replay log and never bump a topic seq.

use std::collections::HashSet;

use uuid::Uuid;

use super::hub::{ConnId, Gateway};
use super::protocol::{
    LiveTopic, PresenceEntry, PresenceStatus, REDIS_PREFIX, ServerFrame,
};
use crate::error::ApiError;

/// Set one connection's status, recompute the user aggregate.
/// KEYS: conn, user, conn-index. ARGV: status, ttl_ms, conn-key.
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
return {prev, agg}
"#;

/// Drop one connection, recompute the user aggregate.
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
return {prev, agg}
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
    format!("{REDIS_PREFIX}p:c:{user_id}:{}", conn.as_u64())
}

fn user_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:u:{user_id}")
}

fn conn_index_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:n:{user_id}")
}

fn server_set_key(server_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:s:{server_id}")
}

fn user_servers_key(user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}p:u:{user_id}:s")
}

fn typing_key(channel_id: Uuid, user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}y:{channel_id}:{user_id}")
}

fn ttl_ms(d: std::time::Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX).max(1)
}

impl Gateway {
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
        let (prev, agg) = self.write_presence(user_id, id, status).await?;
        if prev != agg {
            let servers = self.presence_targets(user_id, &local).await?;
            self.broadcast_presence(user_id, agg, &servers).await?;
        }
        Ok(agg)
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

    /// Add the user to a server's presence set. First time: publish current status.
    pub async fn announce_server(&self, id: ConnId, server_id: Uuid) -> Result<(), ApiError> {
        let Some((user_id, _)) = self.socket_meta(id).await else {
            return Ok(());
        };
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
        if added == 1 {
            let st = self
                .user_status(user_id)
                .await?
                .unwrap_or(PresenceStatus::Online);
            self.publish_presence(server_id, user_id, st).await?;
        }
        Ok(())
    }

    pub async fn presence_snapshot(
        &self,
        server_id: Uuid,
    ) -> Result<Vec<PresenceEntry>, ApiError> {
        let users: Vec<String> = self
            .with_conn(|mut conn| {
                let key = server_set_key(server_id);
                async move { redis::cmd("SMEMBERS").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;

        let mut snap = Vec::new();
        let mut stale = Vec::new();
        for raw in users {
            let Ok(user_id) = raw.parse::<Uuid>() else {
                stale.push(raw);
                continue;
            };
            match self.user_status(user_id).await? {
                Some(st) => snap.push(PresenceEntry { u: user_id, st }),
                None => stale.push(raw),
            }
        }
        if !stale.is_empty() {
            let _ = self
                .with_conn(|mut conn| {
                    let key = server_set_key(server_id);
                    let stale = stale.clone();
                    async move {
                        let mut cmd = redis::cmd("SREM");
                        cmd.arg(key);
                        for member in stale {
                            cmd.arg(member);
                        }
                        cmd.query_async::<i64>(&mut conn).await
                    }
                })
                .await;
        }
        snap.sort_by_key(|entry| entry.u);
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
        let key = typing_key(channel_id, user_id);
        let px = ttl_ms(self.typing_ttl());
        if on {
            self.with_conn(|mut conn| {
                let key = key.clone();
                async move {
                    redis::cmd("SET")
                        .arg(key)
                        .arg("1")
                        .arg("PX")
                        .arg(px)
                        .query_async::<String>(&mut conn)
                        .await
                }
            })
            .await
            .map_err(super::hub::redis_err)?;
        } else {
            self.with_conn(|mut conn| {
                let key = key.clone();
                async move { redis::cmd("DEL").arg(key).query_async::<i64>(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;
        }
        self.publish_live(
            LiveTopic::Typing(channel_id),
            &ServerFrame::typing(server_id, channel_id, user_id, on),
        )
        .await
    }

    async fn publish_typing_stop(
        &self,
        server_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ApiError> {
        let key = typing_key(channel_id, user_id);
        self.with_conn(|mut conn| {
            let key = key.clone();
            async move { redis::cmd("DEL").arg(key).query_async::<i64>(&mut conn).await }
        })
        .await
        .map_err(super::hub::redis_err)?;
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
        let (prev, agg) = self.delete_presence(user_id, id).await?;
        let servers = self.presence_targets(user_id, local).await?;
        if agg == PresenceStatus::Offline {
            self.forget_user_servers(user_id, &servers).await?;
            self.broadcast_presence(user_id, PresenceStatus::Offline, &servers)
                .await?;
        } else if prev != agg {
            self.broadcast_presence(user_id, agg, &servers).await?;
        }
        Ok(agg)
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
    ) -> Result<(PresenceStatus, PresenceStatus), ApiError> {
        let ck = conn_key(user_id, id);
        let uk = user_key(user_id);
        let nk = conn_index_key(user_id);
        let px = ttl_ms(self.presence_ttl());
        let (prev, agg): (String, String) = self
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
                        .arg(status.as_str())
                        .arg(px)
                        .arg(member)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok((parse_st(&prev), parse_st(&agg)))
    }

    async fn delete_presence(
        &self,
        user_id: Uuid,
        id: ConnId,
    ) -> Result<(PresenceStatus, PresenceStatus), ApiError> {
        let ck = conn_key(user_id, id);
        let uk = user_key(user_id);
        let nk = conn_index_key(user_id);
        let px = ttl_ms(self.presence_ttl());
        let (prev, agg): (String, String) = self
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
                        .arg(px)
                        .arg(member)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok((parse_st(&prev), parse_st(&agg)))
    }

    async fn user_status(&self, user_id: Uuid) -> Result<Option<PresenceStatus>, ApiError> {
        let raw: Option<String> = self
            .with_conn(|mut conn| {
                let key = user_key(user_id);
                async move { redis::cmd("GET").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(super::hub::redis_err)?;
        Ok(raw.as_deref().and_then(PresenceStatus::parse))
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
