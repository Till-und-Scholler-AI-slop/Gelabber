//! Expiring per-connection voice seats and exclusive live claims.
use super::hub::{ConnId, Gateway, VoiceRoster, VoiceSeat, redis_err};
use super::protocol::{ServerFrame, SigEvent, TrackKind, VoiceEntry};
use crate::error::ApiError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const VOICE_LEASE_MS: u64 = 5000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct LiveOwner {
    pub u: Uuid,
    pub s: Uuid,
    pub c: Uuid,
    pub session: String,
    pub seat: Uuid,
    pub nonce: Uuid,
}
#[derive(Clone, Serialize, Deserialize)]
struct SeatRecord {
    id: Uuid,
    u: Uuid,
    s: Uuid,
    c: Uuid,
    m: bool,
    d: bool,
    p: HashSet<TrackKind>,
}

// All indexes are leases too. Reads atomically prune expired/missing seats;
// no unbounded counter can survive an API crash or a partial join.
const WRITE_SEAT: &str = r#"
local now = redis.call('TIME')
local expires = tonumber(now[1])*1000 + math.floor(tonumber(now[2])/1000) + tonumber(ARGV[2])
redis.call('SET', KEYS[1], ARGV[1], 'PX', ARGV[2])
for i=2,3 do
  redis.call('ZADD', KEYS[i], expires, KEYS[1])
  redis.call('PEXPIRE', KEYS[i], tonumber(ARGV[2])*2)
end
return 1
"#;
const READ_SEATS: &str = r#"
local now = redis.call('TIME')
local ms = tonumber(now[1])*1000 + math.floor(tonumber(now[2])/1000)
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', ms)
local rows = {}
for _, key in ipairs(redis.call('ZRANGE', KEYS[1], 0, -1)) do
  local raw = redis.call('GET', key)
  if raw then table.insert(rows, raw) else redis.call('ZREM', KEYS[1], key) end
end
return rows
"#;
const DROP_SEAT: &str = r#"
redis.call('DEL', KEYS[1])
redis.call('ZREM', KEYS[2], KEYS[1])
redis.call('ZREM', KEYS[3], KEYS[1])
return 1
"#;
// Compare the entire owner (including seat + nonce), not just the user: a late
// leave/refresh from another tab or an old process cannot release a new claim.
const CLAIM_LIVE: &str = r#"
local current = redis.call('GET', KEYS[1])
if not current then
  redis.call('SET', KEYS[1], ARGV[1], 'PX', ARGV[2])
  return 1
end
if current == ARGV[1] then redis.call('PEXPIRE', KEYS[1], ARGV[2]); return 2 end
return 0
"#;
const REFRESH_LIVE: &str = r#"
if redis.call('GET', KEYS[1]) ~= ARGV[1] then return 0 end
redis.call('PEXPIRE', KEYS[1], ARGV[2]); return 1
"#;
const RELEASE_LIVE: &str = r#"
if redis.call('GET', KEYS[1]) ~= ARGV[1] then return 0 end
redis.call('DEL', KEYS[1]); return 1
"#;

// Read aggregate and publish in one Redis turn. A concurrent tab cannot make
// a delta stale between the aggregate read and Pub/Sub publication.
const AGGREGATE_SIGNAL: &str = r#"
local now=redis.call('TIME')
local ms=tonumber(now[1])*1000+math.floor(tonumber(now[2])/1000)
redis.call('ZREMRANGEBYSCORE',KEYS[1],'-inf',ms)
local raw=redis.call('GET',KEYS[2])
local live=nil
if raw then live=cjson.decode(raw) end
local count=0
local muted=true
local deafened=true
local pubs={}
for _,key in ipairs(redis.call('ZRANGE',KEYS[1],0,-1)) do
  local seat=redis.call('GET',key)
  if seat then
    local row=cjson.decode(seat)
    if row.u==ARGV[1] then
      count=count+1; muted=muted and row.m; deafened=deafened and row.d
      for _,kind in ipairs(row.p) do
        if (kind~='l' and kind~='la') or (live and live.u==row.u and live.seat==row.id) then pubs[kind]=true end
      end
    end
  else redis.call('ZREM',KEYS[1],key) end
end
local base='"s":"'..ARGV[2]..'","c":"'..ARGV[3]..'","u":"'..ARGV[1]..'"'
local function send(t,extra) redis.call('PUBLISH',KEYS[3],'{"t":"'..t..'",'..base..(extra or '')..'}') end
local function flags() send('m',',"on":'..tostring(muted)); send('d',',"on":'..tostring(deafened)) end
if ARGV[4]=='j' and count>0 then send('j',',"m":'..tostring(muted)..',"d":'..tostring(deafened))
elseif ARGV[4]=='m' and count>0 then send('m',',"on":'..tostring(muted))
elseif (ARGV[4]=='d' or ARGV[4]=='d+') and count>0 then
  send('d',',"on":'..tostring(deafened)); if ARGV[4]=='d+' then send('m',',"on":'..tostring(muted)) end
elseif ARGV[4]=='leave' then
  if count==0 then send('l') else
    flags()
    for _,kind in ipairs(cjson.decode(ARGV[6])) do if not pubs[kind] then send('u',',"k":"'..kind..'"') end end
  end
elseif ARGV[4]=='u' then
  for _,kind in ipairs(cjson.decode(ARGV[6])) do if not pubs[kind] then send('u',',"k":"'..kind..'"') end end
elseif ARGV[4]=='p' and pubs[ARGV[5]] then
  if ARGV[5]=='l' or ARGV[5]=='la' then
    if raw~=ARGV[7] then return 0 end
    send('p',',"k":"'..ARGV[5]..'","lc":"'..live.nonce..'"')
  else send('p',',"k":"'..ARGV[5]..'"') end
end
return 1
"#;

fn seat_key(id: Uuid) -> String {
    format!("gb:voice:seat:{id}")
}
fn room_key(c: Uuid) -> String {
    format!("gb:voice:channel:{c}")
}
fn server_key(s: Uuid) -> String {
    format!("gb:voice:server:{s}")
}
pub fn live_key(c: Uuid) -> String {
    format!("gb:live:{c}")
}
fn encode<T: Serialize>(value: &T) -> Result<String, ApiError> {
    serde_json::to_string(value).map_err(|err| ApiError::Internal(format!("voice state: {err}")))
}

impl VoiceRoster {
    pub async fn in_voice(&self, id: ConnId, s: Uuid, c: Uuid) -> bool {
        self.connections
            .inner
            .sockets
            .read()
            .await
            .get(&id)
            .is_some_and(|socket| socket.rooms.get(&c).is_some_and(|seat| seat.server_id == s))
    }
    async fn write_seat(
        &self,
        _id: ConnId,
        u: Uuid,
        c: Uuid,
        seat: &VoiceSeat,
    ) -> Result<(), ApiError> {
        let raw = encode(&SeatRecord {
            id: seat.id,
            u,
            s: seat.server_id,
            c,
            m: seat.muted,
            d: seat.deafened,
            p: seat.pubs.clone(),
        })?;
        self.connections
            .with_conn(|mut conn| {
                let raw = raw.clone();
                async move {
                    redis::Script::new(WRITE_SEAT)
                        .key(seat_key(seat.id))
                        .key(room_key(c))
                        .key(server_key(seat.server_id))
                        .arg(raw)
                        .arg(VOICE_LEASE_MS)
                        .invoke_async::<i32>(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)?;
        Ok(())
    }
    async fn records(&self, key: String) -> Result<Vec<SeatRecord>, ApiError> {
        let rows: Vec<String> = self
            .connections
            .with_conn(|mut conn| {
                let key = key.clone();
                async move {
                    redis::Script::new(READ_SEATS)
                        .key(key)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)?;
        rows.into_iter()
            .map(|raw| {
                serde_json::from_str(&raw)
                    .map_err(|err| ApiError::Internal(format!("voice lease: {err}")))
            })
            .collect()
    }
    async fn live_op(&self, script: &'static str, owner: &LiveOwner) -> Result<i32, ApiError> {
        let raw = encode(owner)?;
        self.connections
            .with_conn(|mut conn| {
                let raw = raw.clone();
                let owner = owner.clone();
                async move {
                    redis::Script::new(script)
                        .key(live_key(owner.c))
                        .arg(raw)
                        .arg(VOICE_LEASE_MS)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)
    }
    async fn live_owner(&self, c: Uuid) -> Result<Option<LiveOwner>, ApiError> {
        let raw: Option<String> = self
            .connections
            .with_conn(|mut conn| async move {
                redis::cmd("GET")
                    .arg(live_key(c))
                    .query_async(&mut conn)
                    .await
            })
            .await
            .map_err(redis_err)?;
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|err| ApiError::Internal(format!("live claim: {err}")))
        })
        .transpose()
    }
    async fn aggregate_signal(
        &self,
        u: Uuid,
        s: Uuid,
        c: Uuid,
        mode: &str,
        kinds: &HashSet<TrackKind>,
        live: Option<&LiveOwner>,
    ) -> Result<bool, ApiError> {
        let mode = mode.to_owned();
        let kinds_raw = encode(kinds)?;
        let kind = kinds
            .iter()
            .next()
            .map(|kind| kind.as_str())
            .unwrap_or("")
            .to_owned();
        let owner = live.map(encode).transpose()?.unwrap_or_default();
        let result: i32 = self
            .connections
            .with_conn(|mut conn| {
                let (mode, kinds_raw, kind, owner) =
                    (mode.clone(), kinds_raw.clone(), kind.clone(), owner.clone());
                async move {
                    redis::Script::new(AGGREGATE_SIGNAL)
                        .key(room_key(c))
                        .key(live_key(c))
                        .key(super::hub::voice_redis_channel(c))
                        .arg(u.to_string())
                        .arg(s.to_string())
                        .arg(c.to_string())
                        .arg(mode)
                        .arg(kind)
                        .arg(kinds_raw)
                        .arg(owner)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)?;
        Ok(result == 1)
    }
    pub async fn join_voice(
        &self,
        id: ConnId,
        u: Uuid,
        s: Uuid,
        c: Uuid,
    ) -> Result<Vec<SigEvent>, ApiError> {
        let _guard = self.connections.inner.voice_ops.lock().await;
        let old = {
            let mut sockets = self.connections.inner.sockets.write().await;
            let socket = sockets.get_mut(&id).ok_or(ApiError::Unauthenticated)?;
            let rooms: Vec<_> = socket
                .rooms
                .keys()
                .filter(|cid| **cid != c)
                .copied()
                .collect();
            rooms
                .into_iter()
                .filter_map(|cid| socket.rooms.remove(&cid).map(|seat| (cid, seat)))
                .collect::<Vec<_>>()
        };
        for (cid, seat) in old {
            self.drop_seat(id, u, cid, seat).await?;
        }
        let seat = {
            let mut sockets = self.connections.inner.sockets.write().await;
            let socket = sockets.get_mut(&id).ok_or(ApiError::Unauthenticated)?;
            socket
                .rooms
                .entry(c)
                .or_insert(VoiceSeat {
                    id: Uuid::new_v4(),
                    server_id: s,
                    pubs: HashSet::new(),
                    muted: false,
                    deafened: false,
                    live: None,
                })
                .clone()
        };
        if seat.server_id != s {
            return Err(ApiError::NotFound);
        }
        if let Err(err) = self.write_seat(id, u, c, &seat).await {
            if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
                socket.rooms.remove(&c);
            }
            return Err(err);
        }
        let result = async {
            let live = self.live_owner(c).await?;
            let rows = self.records(room_key(c)).await?;
            let mut peers: HashMap<Uuid, (bool, bool, HashSet<TrackKind>)> = HashMap::new();
            for mut row in rows.into_iter().filter(|row| row.u != u) {
                if !live
                    .as_ref()
                    .is_some_and(|owner| owner.seat == row.id && owner.u == row.u && owner.s == s)
                {
                    row.p.remove(&TrackKind::L);
                    row.p.remove(&TrackKind::La);
                }
                let peer = peers.entry(row.u).or_insert((true, true, HashSet::new()));
                peer.0 &= row.m;
                peer.1 &= row.d;
                peer.2.extend(row.p);
            }
            let mut events = Vec::new();
            for (uid, (m, d, pubs)) in peers {
                events.push(SigEvent::join_state(s, c, uid, m, d));
                for kind in pubs {
                    let mut event = SigEvent::published(s, c, uid, kind);
                    if matches!(kind, TrackKind::L | TrackKind::La) {
                        event.lc = live
                            .as_ref()
                            .filter(|owner| owner.u == uid)
                            .map(|owner| owner.nonce);
                    }
                    events.push(event);
                }
            }
            self.aggregate_signal(u, s, c, "j", &HashSet::new(), None)
                .await?;
            Ok(events)
        }
        .await;
        if result.is_err() {
            if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
                socket.rooms.remove(&c);
            }
            let _ = self.drop_seat(id, u, c, seat).await;
        }
        result
    }
    pub async fn leave_voice(
        &self,
        id: ConnId,
        u: Uuid,
        s: Uuid,
        c: Uuid,
    ) -> Result<bool, ApiError> {
        let _guard = self.connections.inner.voice_ops.lock().await;
        let seat = {
            let mut sockets = self.connections.inner.sockets.write().await;
            let Some(socket) = sockets.get_mut(&id) else {
                return Ok(false);
            };
            if !socket.rooms.get(&c).is_some_and(|seat| seat.server_id == s) {
                return Ok(false);
            }
            socket.rooms.remove(&c).expect("checked")
        };
        self.drop_seat(id, u, c, seat).await?;
        Ok(true)
    }
    pub(super) async fn drop_seat(
        &self,
        _id: ConnId,
        u: Uuid,
        c: Uuid,
        seat: VoiceSeat,
    ) -> Result<(), ApiError> {
        self.connections
            .with_conn(|mut conn| async move {
                redis::Script::new(DROP_SEAT)
                    .key(seat_key(seat.id))
                    .key(room_key(c))
                    .key(server_key(seat.server_id))
                    .invoke_async::<i32>(&mut conn)
                    .await
            })
            .await
            .map_err(redis_err)?;
        if let Some(owner) = &seat.live {
            self.live_op(RELEASE_LIVE, owner).await?;
        }
        self.aggregate_signal(u, seat.server_id, c, "leave", &seat.pubs, None)
            .await?;
        Ok(())
    }
    pub async fn set_voice_mute(
        &self,
        id: ConnId,
        u: Uuid,
        s: Uuid,
        c: Uuid,
        on: bool,
    ) -> Result<bool, ApiError> {
        self.set_flag(id, u, s, c, on, false).await
    }
    pub async fn set_voice_deafen(
        &self,
        id: ConnId,
        u: Uuid,
        s: Uuid,
        c: Uuid,
        on: bool,
    ) -> Result<bool, ApiError> {
        self.set_flag(id, u, s, c, on, true).await
    }
    async fn set_flag(
        &self,
        id: ConnId,
        u: Uuid,
        s: Uuid,
        c: Uuid,
        on: bool,
        deafen: bool,
    ) -> Result<bool, ApiError> {
        let _guard = self.connections.inner.voice_ops.lock().await;
        let seat = {
            let mut sockets = self.connections.inner.sockets.write().await;
            let Some(seat) = sockets
                .get_mut(&id)
                .and_then(|socket| socket.rooms.get_mut(&c))
                .filter(|seat| seat.server_id == s)
            else {
                return Ok(false);
            };
            if deafen {
                seat.deafened = on;
                if on {
                    seat.muted = true;
                }
            } else {
                seat.muted = on;
            }
            seat.clone()
        };
        self.write_seat(id, u, c, &seat).await?;
        self.aggregate_signal(
            u,
            s,
            c,
            if deafen {
                if on { "d+" } else { "d" }
            } else {
                "m"
            },
            &HashSet::new(),
            None,
        )
        .await?;
        Ok(true)
    }
    pub async fn set_voice_pub(
        &self,
        id: ConnId,
        u: Uuid,
        s: Uuid,
        c: Uuid,
        kind: TrackKind,
        on: bool,
    ) -> Result<Option<bool>, ApiError> {
        let _guard = self.connections.inner.voice_ops.lock().await;
        let (mut seat, session) = {
            let sockets = self.connections.inner.sockets.read().await;
            let Some(socket) = sockets.get(&id) else {
                return Ok(None);
            };
            let Some(seat) = socket.rooms.get(&c).filter(|seat| seat.server_id == s) else {
                return Ok(None);
            };
            (
                seat.clone(),
                socket
                    .session
                    .as_ref()
                    .map(|(hash, _)| hash.clone())
                    .unwrap_or_default(),
            )
        };
        let previous = seat.clone();
        if on && let Some(parent) = kind.source_parent() {
            if !seat.pubs.contains(&parent) {
                return Ok(None);
            }
            if parent == TrackKind::L
                && (seat.live.is_none() || self.live_owner(c).await?.as_ref() != seat.live.as_ref())
            {
                return Ok(None);
            }
        }
        let mut started = false;
        if kind == TrackKind::L {
            if on {
                if session.is_empty() {
                    return Ok(None);
                }
                let current = self.live_owner(c).await?;
                let owner = seat
                    .live
                    .clone()
                    .filter(|owner| current.as_ref() == Some(owner))
                    .unwrap_or(LiveOwner {
                        u,
                        s,
                        c,
                        session,
                        seat: seat.id,
                        nonce: Uuid::new_v4(),
                    });
                let result = self.live_op(CLAIM_LIVE, &owner).await?;
                if result == 0 {
                    return Ok(None);
                }
                started = result == 1;
                seat.live = Some(owner);
            } else if let Some(owner) = seat.live.take() {
                self.live_op(RELEASE_LIVE, &owner).await?;
            }
        }
        if on {
            seat.pubs.insert(kind);
        } else {
            seat.pubs.remove(&kind);
            if let Some(audio) = kind.source_audio() {
                seat.pubs.remove(&audio);
            }
        }
        if let Err(err) = self.write_seat(id, u, c, &seat).await {
            if kind == TrackKind::L
                && started
                && let Some(owner) = &seat.live
            {
                let _ = self.live_op(RELEASE_LIVE, owner).await;
            }
            return Err(err);
        }
        if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
            socket.rooms.insert(c, seat.clone());
        }
        if on {
            let result = self
                .aggregate_signal(u, s, c, "p", &HashSet::from([kind]), seat.live.as_ref())
                .await;
            if result.as_ref().is_ok_and(|accepted| !accepted) {
                seat.live = None;
                seat.pubs.remove(&TrackKind::L);
                seat.pubs.remove(&TrackKind::La);
                if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
                    socket.rooms.insert(c, seat.clone());
                }
                let _ = self.write_seat(id, u, c, &seat).await;
                return Ok(None);
            }
            if let Err(err) = result {
                if kind == TrackKind::L
                    && started
                    && let Some(owner) = &seat.live
                {
                    let _ = self.live_op(RELEASE_LIVE, owner).await;
                }
                let mut restored = previous;
                if kind == TrackKind::L && started {
                    restored.live = None;
                    restored.pubs.remove(&TrackKind::L);
                    restored.pubs.remove(&TrackKind::La);
                }
                if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
                    socket.rooms.insert(c, restored.clone());
                }
                let _ = self.write_seat(id, u, c, &restored).await;
                return Err(err);
            }
        } else {
            let mut removed = HashSet::from([kind]);
            if let Some(audio) = kind.source_audio()
                && previous.pubs.contains(&audio)
            {
                removed.insert(audio);
            }
            self.aggregate_signal(u, s, c, "u", &removed, None).await?;
        }
        Ok(Some(started))
    }
    pub async fn voice_snapshot(&self, s: Uuid) -> Result<Vec<VoiceEntry>, ApiError> {
        let rows = self.records(server_key(s)).await?;
        let mut groups: HashMap<(Uuid, Uuid), (bool, bool)> = HashMap::new();
        for row in rows {
            let flags = groups.entry((row.u, row.c)).or_insert((true, true));
            flags.0 &= row.m;
            flags.1 &= row.d;
        }
        let mut users: HashMap<Uuid, VoiceEntry> = HashMap::new();
        for ((u, c), (m, d)) in groups {
            let l = self
                .live_owner(c)
                .await?
                .is_some_and(|owner| owner.u == u && owner.s == s);
            let entry = VoiceEntry { u, c, m, d, l };
            match users.entry(u) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(entry);
                }
                std::collections::hash_map::Entry::Occupied(mut slot)
                    if l || (!slot.get().l && c < slot.get().c) =>
                {
                    slot.insert(entry);
                }
                _ => {}
            }
        }
        let mut rows: Vec<_> = users.into_values().collect();
        rows.sort_by_key(|row| row.u);
        Ok(rows)
    }
    pub async fn refresh(&self, id: ConnId) -> Result<(), ApiError> {
        let _guard = self.connections.inner.voice_ops.lock().await;
        let Some((u, rooms)) = self
            .connections
            .inner
            .sockets
            .read()
            .await
            .get(&id)
            .map(|socket| (socket.user_id, socket.rooms.clone()))
        else {
            return Ok(());
        };
        for (c, mut seat) in rooms {
            if let Some(owner) = &seat.live
                && self.live_op(REFRESH_LIVE, owner).await? == 0
            {
                seat.live = None;
                seat.pubs.remove(&TrackKind::L);
                let had_audio = seat.pubs.remove(&TrackKind::La);
                if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
                    socket.rooms.insert(c, seat.clone());
                }
                let mut removed = HashSet::from([TrackKind::L]);
                if had_audio {
                    removed.insert(TrackKind::La);
                }
                self.aggregate_signal(u, seat.server_id, c, "u", &removed, None)
                    .await?;
            }
            self.write_seat(id, u, c, &seat).await?;
            if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
                socket.rooms.insert(c, seat);
            }
        }
        Ok(())
    }
}
impl Gateway {
    pub async fn refresh_voice(&self, id: ConnId) -> Result<(), ApiError> {
        self.voice.refresh(id).await?;
        let servers = self
            .connections
            .inner
            .sockets
            .read()
            .await
            .get(&id)
            .map(|socket| socket.servers.clone())
            .unwrap_or_default();
        for server in servers {
            let snapshot = self.voice.voice_snapshot(server).await?;
            if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id)
                && socket.voice_rosters.get(&server) != Some(&snapshot)
            {
                socket.voice_rosters.insert(server, snapshot.clone());
                socket.send(ServerFrame::voice_snap(server, snapshot));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn gateway() -> Gateway {
        Gateway::new(
            redis::Client::open(std::env::var("REDIS_URL").unwrap()).unwrap(),
            8,
            Duration::from_secs(10),
            Duration::from_secs(2),
        )
    }
    async fn socket(g: &Gateway, u: Uuid) -> (ConnId, mpsc::Receiver<ServerFrame>) {
        // Initial subscriber readiness emits resync; this fixture attaches only
        // after that handshake, like the API integration fixtures do.
        g.wait_ready(Duration::from_secs(2)).await.unwrap();
        let (tx, rx) = mpsc::channel(128);
        let (id, _) = g.attach_session(u, Uuid::new_v4().to_string(), tx).await;
        (id, rx)
    }
    async fn redis() -> redis::aio::MultiplexedConnection {
        redis::Client::open(std::env::var("REDIS_URL").unwrap())
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .unwrap()
    }
    async fn corrupt_aggregate(c: Uuid) -> String {
        let key = seat_key(Uuid::new_v4());
        // WRITE_SEAT can still update both indexes, but AGGREGATE_SIGNAL fails
        // decoding this other seat after the requested seat has been written.
        redis::pipe()
            .atomic()
            .cmd("SET")
            .arg(&key)
            .arg("invalid JSON")
            .arg("PX")
            .arg(VOICE_LEASE_MS)
            .ignore()
            .cmd("ZADD")
            .arg(room_key(c))
            .arg("+inf")
            .arg(&key)
            .ignore()
            .query_async::<()>(&mut redis().await)
            .await
            .unwrap();
        key
    }
    async fn assert_seat_restored(g: &Gateway, id: ConnId, u: Uuid, c: Uuid, expected: &VoiceSeat) {
        let seat = g.connections.inner.sockets.read().await[&id].rooms[&c].clone();
        assert_eq!(seat.id, expected.id);
        assert_eq!(seat.server_id, expected.server_id);
        assert_eq!(seat.muted, expected.muted);
        assert_eq!(seat.deafened, expected.deafened);
        assert_eq!(seat.pubs, expected.pubs);
        assert_eq!(seat.live, expected.live);
        let raw: String = redis::cmd("GET")
            .arg(seat_key(expected.id))
            .query_async(&mut redis().await)
            .await
            .unwrap();
        let record: SeatRecord = serde_json::from_str(&raw).unwrap();
        assert_eq!(record.id, expected.id);
        assert_eq!((record.u, record.s, record.c), (u, expected.server_id, c));
        assert_eq!((record.m, record.d), (expected.muted, expected.deafened));
        assert_eq!(record.p, expected.pubs);
    }

    #[tokio::test]
    async fn failed_live_retry_preserves_existing_claim_and_previous_seat() {
        let g = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (id, _rx) = socket(&g, u).await;
        g.join_voice(id, u, s, c).await.unwrap();
        g.set_voice_deafen(id, u, s, c, true).await.unwrap();
        g.set_voice_pub(id, u, s, c, TrackKind::V, true)
            .await
            .unwrap();
        assert_eq!(
            g.set_voice_pub(id, u, s, c, TrackKind::L, true)
                .await
                .unwrap(),
            Some(true)
        );
        g.set_voice_pub(id, u, s, c, TrackKind::La, true)
            .await
            .unwrap();
        let previous = g.connections.inner.sockets.read().await[&id].rooms[&c].clone();
        let owner = g.voice.live_owner(c).await.unwrap().unwrap();
        let corrupt = corrupt_aggregate(c).await;
        let error = g
            .set_voice_pub(id, u, s, c, TrackKind::L, true)
            .await
            .unwrap_err();
        assert!(
            matches!(error, ApiError::Internal(ref message) if message.contains("Expected value"))
        );
        assert_eq!(g.voice.live_owner(c).await.unwrap(), Some(owner.clone()));
        assert_seat_restored(&g, id, u, c, &previous).await;
        redis::cmd("DEL")
            .arg(corrupt)
            .query_async::<i32>(&mut redis().await)
            .await
            .unwrap();
        assert!(g.voice_snapshot(s).await.unwrap()[0].l);
        assert_eq!(
            g.set_voice_pub(id, u, s, c, TrackKind::L, true)
                .await
                .unwrap(),
            Some(false)
        );
        assert_eq!(g.voice.live_owner(c).await.unwrap(), Some(owner));
        g.detach(id).await;
    }

    #[tokio::test]
    async fn failed_live_start_releases_new_claim_and_restores_previous_seat() {
        let g = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (id, _rx) = socket(&g, u).await;
        g.join_voice(id, u, s, c).await.unwrap();
        g.set_voice_deafen(id, u, s, c, true).await.unwrap();
        g.set_voice_pub(id, u, s, c, TrackKind::V, true)
            .await
            .unwrap();
        let previous = g.connections.inner.sockets.read().await[&id].rooms[&c].clone();
        let corrupt = corrupt_aggregate(c).await;
        let error = g
            .set_voice_pub(id, u, s, c, TrackKind::L, true)
            .await
            .unwrap_err();
        assert!(
            matches!(error, ApiError::Internal(ref message) if message.contains("Expected value"))
        );
        assert!(g.voice.live_owner(c).await.unwrap().is_none());
        assert_seat_restored(&g, id, u, c, &previous).await;
        redis::cmd("DEL")
            .arg(corrupt)
            .query_async::<i32>(&mut redis().await)
            .await
            .unwrap();
        assert!(!g.voice_snapshot(s).await.unwrap()[0].l);
        assert_eq!(
            g.set_voice_pub(id, u, s, c, TrackKind::L, true)
                .await
                .unwrap(),
            Some(true)
        );
        g.detach(id).await;
    }

    #[tokio::test]
    async fn crash_releases_live_and_roster_and_stale_leave_preserves_new_owner() {
        let a = gateway();
        let b = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (old, _rx) = socket(&a, u).await;
        a.join_voice(old, u, s, c).await.unwrap();
        assert_eq!(
            a.set_voice_pub(old, u, s, c, TrackKind::L, true)
                .await
                .unwrap(),
            Some(true)
        );
        let old_owner = a.voice.live_owner(c).await.unwrap().unwrap();
        // No renewal or detach models abrupt process death, not graceful leave.
        tokio::time::sleep(Duration::from_millis(5200)).await;
        assert!(b.voice_snapshot(s).await.unwrap().is_empty());
        assert!(b.voice.live_owner(c).await.unwrap().is_none());
        let (new, _rx2) = socket(&b, u).await;
        b.join_voice(new, u, s, c).await.unwrap();
        assert_eq!(
            b.set_voice_pub(new, u, s, c, TrackKind::L, true)
                .await
                .unwrap(),
            Some(true)
        );
        let owner = b.voice.live_owner(c).await.unwrap().unwrap();
        assert_ne!(owner.nonce, old_owner.nonce);
        assert_eq!(a.voice.live_op(RELEASE_LIVE, &old_owner).await.unwrap(), 0);
        a.leave_voice(old, u, s, c).await.unwrap();
        assert_eq!(b.voice.live_owner(c).await.unwrap(), Some(owner));
        assert_eq!(b.voice_snapshot(s).await.unwrap().len(), 1);
        b.detach(new).await;
    }

    #[tokio::test]
    async fn two_tabs_of_same_user_have_one_live_owner_and_repeat_start_is_idempotent() {
        let a = gateway();
        let b = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (x, _rx) = socket(&a, u).await;
        let (y, _ry) = socket(&b, u).await;
        assert_ne!(x.redis_id(), y.redis_id());
        a.join_voice(x, u, s, c).await.unwrap();
        b.join_voice(y, u, s, c).await.unwrap();
        let (one, two) = tokio::join!(
            a.set_voice_pub(x, u, s, c, TrackKind::L, true),
            b.set_voice_pub(y, u, s, c, TrackKind::L, true)
        );
        let one = one.unwrap();
        let two = two.unwrap();
        assert!(matches!(
            (one, two),
            (Some(true), None) | (None, Some(true))
        ));
        let (winner, id) = if one.is_some() { (&a, x) } else { (&b, y) };
        let before = winner.voice.live_owner(c).await.unwrap();
        assert_eq!(
            winner
                .set_voice_pub(id, u, s, c, TrackKind::L, true)
                .await
                .unwrap(),
            Some(false)
        );
        assert_eq!(winner.voice.live_owner(c).await.unwrap(), before);
        a.detach(x).await;
        b.detach(y).await;
    }

    #[tokio::test]
    async fn active_owner_renews_beyond_lease_and_expired_claim_is_never_reacquired() {
        let g = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (id, _rx) = socket(&g, u).await;
        g.join_voice(id, u, s, c).await.unwrap();
        g.set_voice_pub(id, u, s, c, TrackKind::L, true)
            .await
            .unwrap();
        let owner = g.voice.live_owner(c).await.unwrap().unwrap();
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            g.refresh_voice(id).await.unwrap();
        }
        assert_eq!(g.voice.live_owner(c).await.unwrap(), Some(owner));
        assert!(g.voice_snapshot(s).await.unwrap()[0].l);
        redis::cmd("DEL")
            .arg(live_key(c))
            .query_async::<i32>(&mut redis().await)
            .await
            .unwrap();
        g.refresh_voice(id).await.unwrap();
        assert!(g.voice.live_owner(c).await.unwrap().is_none());
        assert!(!g.voice_snapshot(s).await.unwrap()[0].l);
        let (other, _ry) = socket(&g, Uuid::new_v4()).await;
        let snapshot = g.join_voice(other, Uuid::new_v4(), s, c).await.unwrap();
        assert!(!snapshot.iter().any(|event| event.k == Some(TrackKind::L)));
        g.detach(id).await;
        g.detach(other).await;
    }

    #[tokio::test]
    async fn partial_join_failure_is_not_renewed_and_all_indexes_are_bounded() {
        let g = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (id, _rx) = socket(&g, u).await;
        // Redis script writes the seat then fails on a corrupt server index.
        redis::cmd("SET")
            .arg(server_key(s))
            .arg("wrong type")
            .arg("PX")
            .arg(1000)
            .query_async::<()>(&mut redis().await)
            .await
            .unwrap();
        assert!(g.join_voice(id, u, s, c).await.is_err());
        assert!(!g.in_voice(id, s, c).await);
        tokio::time::sleep(Duration::from_millis(5200)).await;
        g.refresh_voice(id).await.unwrap();
        assert!(g.voice_snapshot(s).await.unwrap().is_empty());
        assert!(g.voice.records(room_key(c)).await.unwrap().is_empty());
        g.detach(id).await;
    }

    #[tokio::test]
    async fn passive_watcher_gets_replacement_roster_after_crash_lease_expires() {
        let a = gateway();
        let b = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (id, _rx) = socket(&a, u).await;
        a.join_voice(id, u, s, c).await.unwrap();
        let (watcher, mut rx) = socket(&b, Uuid::new_v4()).await;
        b.watch_server(watcher, s).await;
        b.refresh_voice(watcher).await.unwrap();
        assert!(
            matches!(rx.recv().await,Some(ServerFrame::Sig { snap: Some(rows), .. }) if rows.len()==1)
        );
        tokio::time::sleep(Duration::from_millis(5200)).await;
        b.refresh_voice(watcher).await.unwrap();
        assert!(
            matches!(rx.recv().await,Some(ServerFrame::Sig { snap: Some(rows), .. }) if rows.is_empty())
        );
        a.detach(id).await;
        b.detach(watcher).await;
    }

    #[tokio::test]
    async fn live_audio_is_same_seat_and_expires_with_parent_without_clearing_microphone() {
        let g = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (owner, _rx) = socket(&g, u).await;
        let (other_tab, _rx) = socket(&g, u).await;
        for id in [owner, other_tab] {
            g.join_voice(id, u, s, c).await.unwrap();
        }
        assert_eq!(
            g.set_voice_pub(owner, u, s, c, TrackKind::A, true)
                .await
                .unwrap(),
            Some(false)
        );
        assert_eq!(
            g.set_voice_pub(owner, u, s, c, TrackKind::L, true)
                .await
                .unwrap(),
            Some(true)
        );
        let nonce = g.voice.live_owner(c).await.unwrap().unwrap().nonce;
        assert_eq!(
            g.set_voice_pub(other_tab, u, s, c, TrackKind::La, true)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            g.set_voice_pub(owner, u, s, c, TrackKind::La, true)
                .await
                .unwrap(),
            Some(false)
        );
        assert_eq!(g.voice.live_owner(c).await.unwrap().unwrap().nonce, nonce);
        redis::cmd("DEL")
            .arg(live_key(c))
            .query_async::<()>(&mut redis().await)
            .await
            .unwrap();
        assert_eq!(
            g.set_voice_pub(owner, u, s, c, TrackKind::La, true)
                .await
                .unwrap(),
            None
        );
        g.refresh_voice(owner).await.unwrap();
        let seat = g.connections.inner.sockets.read().await[&owner].rooms[&c].clone();
        assert_eq!(seat.pubs, HashSet::from([TrackKind::A]));
        assert!(seat.live.is_none());
        g.detach(owner).await;
        g.detach(other_tab).await;
    }

    #[tokio::test]
    async fn late_seat_cleanup_cannot_delete_rejoin_generation() {
        let g = gateway();
        let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (id, _rx) = socket(&g, u).await;
        g.join_voice(id, u, s, c).await.unwrap();
        let old = g.connections.inner.sockets.read().await[&id].rooms[&c].clone();
        g.leave_voice(id, u, s, c).await.unwrap();
        g.join_voice(id, u, s, c).await.unwrap();
        let new = g.connections.inner.sockets.read().await[&id].rooms[&c].clone();
        assert_ne!(old.id, new.id);
        g.voice.drop_seat(id, u, c, old).await.unwrap();
        assert_eq!(g.voice_snapshot(s).await.unwrap().len(), 1);
        g.detach(id).await;
    }
}
