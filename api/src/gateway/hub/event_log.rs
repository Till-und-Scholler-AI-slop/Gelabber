//! The sequenced chat log: seq, epoch and replay list in Redis, written
//! and read in one Lua turn each.

use super::*;

/// One Redis turn: assign seq, append the replay list, PUBLISH.
/// `ARGV[1]` is compact event JSON with the placeholders `"n":0` and
/// `"ep":null`. Seq and epoch are string-substituted — Lua cjson would turn
/// `[]` into `{}` — and only the FIRST match of each is replaced. That is
/// the envelope's own field only while `Event` serializes `n` and `ep`
/// before `d` (today `t,s,c,n,ep,r,i,d`) and `ep` stays `null` when unset
/// (no `skip_serializing_if`); otherwise a delta's own `"n":0` would get
/// the seq. `protocol::tests::event_placeholders_precede_delta` pins this.
const PUBLISH_LUA: &str = r#"
for i=1,5 do
  local expected=({'string','list','none','string','hash'})[i]
  if i~=3 then
    local actual=redis.call('TYPE',KEYS[i]).ok
    if actual~='none' and actual~=expected then return redis.error_reply('wrong realtime key type') end
  end
end
local epoch = redis.call('GET', KEYS[4])
if not epoch then epoch = ARGV[3]; redis.call('SET', KEYS[4], epoch) end
if tonumber(ARGV[4]) > 0 then
  local last = tonumber(redis.call('HGET', KEYS[5], 'r') or '0')
  if last >= tonumber(ARGV[4]) then return {0, redis.call('HGET', KEYS[5], 'raw')} end
end
local n = redis.call('INCR', KEYS[1])
local raw = string.gsub(ARGV[1], '"n":0', '"n":' .. n, 1)
raw = string.gsub(raw, '"ep":null', '"ep":"' .. epoch .. '"', 1)
redis.call('LPUSH', KEYS[2], raw)
redis.call('LTRIM', KEYS[2], 0, tonumber(ARGV[2]))
redis.call('PUBLISH', KEYS[3], raw)
if tonumber(ARGV[4]) > 0 then redis.call('HSET', KEYS[5], 'r', ARGV[4], 'raw', raw) end
return {n, raw}
"#;
const SNAPSHOT_LUA: &str = r#"
local epoch = redis.call('GET', KEYS[3])
if not epoch then epoch = ARGV[1]; redis.call('SET', KEYS[3], epoch) end
redis.call('SET', KEYS[1], '0', 'NX')
return {redis.call('GET', KEYS[1]), epoch, redis.call('LRANGE', KEYS[2], 0, -1)}
"#;

/// Sequenced chat log (`publish` / `catch_up`). Messages live in Postgres;
/// this is the replay buffer and the fan-out, not voice and not RTP.
#[derive(Clone)]
pub struct EventLog {
    pub(super) replay: usize,
    pub(super) connections: ConnTable,
}

impl EventLog {
    /// Assign seq, append the replay buffer, PUBLISH — one Lua turn so a
    /// concurrent writer cannot `PUBLISH` 2 before 1.
    pub async fn publish(&self, draft: EventDraft) -> Result<Event, ApiError> {
        self.publish_revision(draft, None).await
    }

    pub(in crate::gateway) async fn publish_revision(
        &self,
        draft: EventDraft,
        revision: Option<i64>,
    ) -> Result<Event, ApiError> {
        self.connections.ensure_subscriber();
        let topic = draft.topic();
        let placeholder = Event {
            t: draft.kind,
            s: draft.server_id,
            c: draft.channel_id,
            n: 0,
            ep: None,
            r: revision,
            i: draft.entity_id,
            d: draft.delta,
        };
        let template = serde_json::to_string(&placeholder)
            .map_err(|err| ApiError::Internal(format!("serialize event: {err}")))?;
        let replay_end = (self.replay - 1) as i64;
        let seq_key = topic.seq_key();
        let log_key = topic.log_key();
        let channel = topic.redis_channel();

        let (n, raw): (u64, String) = self
            .connections
            .with_conn(|mut conn| {
                let seq_key = seq_key.clone();
                let log_key = log_key.clone();
                let channel = channel.clone();
                let template = template.clone();
                async move {
                    redis::Script::new(PUBLISH_LUA)
                        .key(seq_key)
                        .key(log_key)
                        .key(channel)
                        .key(topic.epoch_key())
                        .key(topic.delivery_key())
                        .arg(template)
                        .arg(replay_end)
                        .arg(Uuid::new_v4().to_string())
                        .arg(revision.unwrap_or(0))
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)?;

        match serde_json::from_str::<Event>(&raw) {
            Ok(event) => Ok(event),
            Err(_) => Ok(Event { n, ..placeholder }),
        }
    }

    pub async fn current_seq(&self, topic: Topic) -> Result<u64, ApiError> {
        let n: Option<u64> = self
            .connections
            .with_conn(|mut conn| {
                let key = topic.seq_key();
                async move { redis::cmd("GET").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(redis_err)?;
        Ok(n.unwrap_or(0))
    }

    pub async fn load_log(&self, topic: Topic) -> Result<Vec<Event>, ApiError> {
        let rows: Vec<String> = self
            .connections
            .with_conn(|mut conn| {
                let key = topic.log_key();
                async move {
                    redis::cmd("LRANGE")
                        .arg(key)
                        .arg(0)
                        .arg(-1)
                        .query_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)?;
        let mut events = Vec::with_capacity(rows.len());
        for raw in rows {
            match serde_json::from_str::<Event>(&raw) {
                Ok(event) => events.push(event),
                Err(err) => warn!(error = %err, "dropping unreadable replay row"),
            }
        }
        Ok(events)
    }

    pub async fn catch_up(
        &self,
        topic: Topic,
        client_n: Option<u64>,
    ) -> Result<(u64, CatchUp<Event>), ApiError> {
        let (current, _, plan) = self.catch_up_epoch(topic, client_n, None).await?;
        Ok((current, plan))
    }
    pub async fn catch_up_epoch(
        &self,
        topic: Topic,
        client_n: Option<u64>,
        client_epoch: Option<Uuid>,
    ) -> Result<(u64, Uuid, CatchUp<Event>), ApiError> {
        let (current, epoch, rows): (u64, String, Vec<String>) = self
            .connections
            .with_conn(|mut conn| async move {
                redis::Script::new(SNAPSHOT_LUA)
                    .key(topic.seq_key())
                    .key(topic.log_key())
                    .key(topic.epoch_key())
                    .arg(Uuid::new_v4().to_string())
                    .invoke_async(&mut conn)
                    .await
            })
            .await
            .map_err(redis_err)?;
        let epoch: Uuid = epoch
            .parse()
            .map_err(|_| ApiError::Internal("invalid Redis epoch".into()))?;
        let events: Vec<Event> = rows
            .into_iter()
            .filter_map(|row| serde_json::from_str(&row).ok())
            .filter(|event: &Event| event.ep == Some(epoch) && event.n <= current)
            .collect();
        let plan = if client_epoch.is_some_and(|previous| previous != epoch) {
            CatchUp::Gap
        } else {
            plan_catch_up(client_n, current, &events, |e| e.n)
        };
        Ok((current, epoch, plan))
    }
}
