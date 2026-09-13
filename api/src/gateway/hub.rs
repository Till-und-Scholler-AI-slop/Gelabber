//! Local socket table + Redis 8 Pub/Sub fan-out.
//!
//! `PUBLISH` is the live path. A bounded Redis list per topic is the
//! reconnect buffer (Pub/Sub itself does not store messages). Each API
//! process keeps one `PSUBSCRIBE gb:*` connection and delivers matching
//! events to sockets subscribed on this process.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, warn};
use uuid::Uuid;

use super::protocol::{
    CatchUp, Event, EventDraft, LiveTopic, REDIS_PREFIX, ServerFrame, SigEvent, Topic, TrackKind,
    VoiceEntry, plan_catch_up,
};
use crate::error::ApiError;

const SUBSCRIBER_RETRY: Duration = Duration::from_millis(200);

/// One Redis turn: assign seq, append the replay list, PUBLISH.
/// `ARGV[1]` is the compact event JSON with `n` as a placeholder (0).
/// Last seat for a user: drop the roster field and their pubs. Returns 1
/// when the caller should broadcast leave.
const LEAVE_SEAT_LUA: &str = r#"
local n = redis.call('HINCRBY', KEYS[1], ARGV[1], -1)
if tonumber(n) <= 0 then
  redis.call('HDEL', KEYS[1], ARGV[1])
  redis.call('DEL', KEYS[2])
  return 1
end
return 0
"#;

const PUBLISH_LUA: &str = r#"
local n = redis.call('INCR', KEYS[1])
local event = cjson.decode(ARGV[1])
event['n'] = tonumber(n)
local raw = cjson.encode(event)
redis.call('LPUSH', KEYS[2], raw)
redis.call('LTRIM', KEYS[2], 0, tonumber(ARGV[2]))
redis.call('PUBLISH', KEYS[3], raw)
return {n, raw}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(u64);

impl ConnId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone)]
pub struct Gateway {
    redis: redis::Client,
    replay: usize,
    presence_ttl: Duration,
    typing_ttl: Duration,
    inner: Arc<Inner>,
}

struct Inner {
    next_id: AtomicU64,
    started: AtomicBool,
    subscribed: AtomicBool,
    conn: Mutex<Option<redis::aio::MultiplexedConnection>>,
    sockets: RwLock<HashMap<ConnId, Socket>>,
}

struct VoiceSeat {
    server_id: Uuid,
    pubs: HashSet<TrackKind>,
    muted: bool,
    deafened: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VoiceFlag {
    Mute,
    Deafen,
}

struct Occupancy {
    channel_id: Uuid,
    muted: bool,
    deafened: bool,
}

struct Socket {
    user_id: Uuid,
    /// Servers this socket has subscribed to (channel or server topic).
    servers: HashSet<Uuid>,
    /// Channels this socket started typing in (`server`, `channel`).
    typing: HashSet<(Uuid, Uuid)>,
    topics: HashSet<Topic>,
    /// Voice rooms this socket has joined (`op: "sig"`). Independent of
    /// chat topic subscriptions.
    rooms: HashMap<Uuid, VoiceSeat>,
    /// Live Pub/Sub frames held until catch-up finishes. Dropping them
    /// here would punch a hole the replay log can miss.
    catching_up: HashMap<Topic, Vec<Event>>,
    tx: mpsc::UnboundedSender<ServerFrame>,
}

impl Gateway {
    pub fn new(
        redis: redis::Client,
        replay: usize,
        presence_ttl: Duration,
        typing_ttl: Duration,
    ) -> Self {
        Self {
            redis,
            replay: replay.max(1),
            presence_ttl,
            typing_ttl,
            inner: Arc::new(Inner {
                next_id: AtomicU64::new(1),
                started: AtomicBool::new(false),
                subscribed: AtomicBool::new(false),
                conn: Mutex::new(None),
                sockets: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub fn presence_ttl(&self) -> Duration {
        self.presence_ttl
    }

    pub fn typing_ttl(&self) -> Duration {
        self.typing_ttl
    }

    pub fn ensure_subscriber(&self) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.run_subscriber().await;
        });
    }

    pub fn is_ready(&self) -> bool {
        self.inner.subscribed.load(Ordering::SeqCst)
    }

    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        self.ensure_subscriber();
        let started = tokio::time::Instant::now();
        while started.elapsed() < timeout {
            if self.is_ready() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Err("redis pub/sub subscriber did not become ready".into())
    }

    pub async fn attach(&self, user_id: Uuid, tx: mpsc::UnboundedSender<ServerFrame>) -> ConnId {
        self.ensure_subscriber();
        let id = ConnId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        self.inner.sockets.write().await.insert(
            id,
            Socket {
                user_id,
                servers: HashSet::new(),
                typing: HashSet::new(),
                topics: HashSet::new(),
                rooms: HashMap::new(),
                catching_up: HashMap::new(),
                tx,
            },
        );
        id
    }

    pub async fn detach(&self, id: ConnId) -> Option<(Uuid, HashSet<Uuid>, HashSet<(Uuid, Uuid)>)> {
        let socket = self.inner.sockets.write().await.remove(&id)?;
        for (channel_id, seat) in socket.rooms {
            match self
                .redis_leave_member(seat.server_id, channel_id, socket.user_id)
                .await
            {
                Ok(true) => {
                    let _ = self
                        .publish_sig(SigEvent::leave(seat.server_id, channel_id, socket.user_id))
                        .await;
                }
                Ok(false) => {}
                Err(err) => {
                    warn!(error = err.code(), "voice leave on detach failed");
                }
            }
        }
        Some((socket.user_id, socket.servers, socket.typing))
    }

    pub async fn watch_server(&self, id: ConnId, server_id: Uuid) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.servers.insert(server_id);
        }
    }

    pub async fn socket_meta(&self, id: ConnId) -> Option<(Uuid, HashSet<Uuid>)> {
        self.inner
            .sockets
            .read()
            .await
            .get(&id)
            .map(|socket| (socket.user_id, socket.servers.clone()))
    }

    pub async fn note_typing(&self, id: ConnId, server_id: Uuid, channel_id: Uuid, on: bool) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            if on {
                socket.typing.insert((server_id, channel_id));
            } else {
                socket.typing.remove(&(server_id, channel_id));
            }
        }
    }

    /// Register the topic and queue live Redis events until [`finish_catch_up`].
    pub async fn begin_catch_up(&self, id: ConnId, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.topics.insert(topic);
            socket.catching_up.entry(topic).or_default();
        }
    }

    /// Release the topic to live delivery and flush queued frames with
    /// `n > after_n` (already-replayed seqs are dropped).
    pub async fn finish_catch_up(&self, id: ConnId, topic: Topic, after_n: u64) {
        let mut sockets = self.inner.sockets.write().await;
        let Some(socket) = sockets.get_mut(&id) else {
            return;
        };
        let Some(mut buf) = socket.catching_up.remove(&topic) else {
            return;
        };
        buf.sort_by_key(|event| event.n);
        let mut seen = after_n;
        for event in buf {
            if event.n <= seen {
                continue;
            }
            seen = event.n;
            let _ = socket.tx.send(ServerFrame::event(event));
        }
    }

    pub async fn queued_len(&self, id: ConnId, topic: Topic) -> usize {
        self.inner
            .sockets
            .read()
            .await
            .get(&id)
            .and_then(|socket| socket.catching_up.get(&topic))
            .map(Vec::len)
            .unwrap_or(0)
    }

    pub async fn unsubscribe(&self, id: ConnId, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.topics.remove(&topic);
            socket.catching_up.remove(&topic);
        }
    }

    /// Drop a user's live seats on one server after kick/ban: unsubscribe
    /// chat topics, leave voice rooms, tell their sockets why.
    pub async fn revoke_server(
        &self,
        user_id: Uuid,
        server_id: Uuid,
        channel_ids: &[Uuid],
        reason: &'static str,
    ) {
        let conns: Vec<ConnId> = {
            let sockets = self.inner.sockets.read().await;
            sockets
                .iter()
                .filter(|(_, socket)| socket.user_id == user_id)
                .map(|(id, _)| *id)
                .collect()
        };
        for id in conns {
            let rooms: Vec<Uuid> = {
                let sockets = self.inner.sockets.read().await;
                sockets
                    .get(&id)
                    .map(|socket| {
                        socket
                            .rooms
                            .iter()
                            .filter(|(_, seat)| seat.server_id == server_id)
                            .map(|(channel_id, _)| *channel_id)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            for channel_id in rooms {
                if let Err(err) = self.leave_voice(id, user_id, server_id, channel_id).await {
                    warn!(error = err.code(), "voice leave on revoke failed");
                }
            }

            let typing: Vec<(Uuid, Uuid)> = {
                let mut sockets = self.inner.sockets.write().await;
                let Some(socket) = sockets.get_mut(&id) else {
                    continue;
                };
                socket.topics.remove(&Topic::Server(server_id));
                socket.catching_up.remove(&Topic::Server(server_id));
                for channel_id in channel_ids {
                    let topic = Topic::Channel(*channel_id);
                    socket.topics.remove(&topic);
                    socket.catching_up.remove(&topic);
                }
                socket.servers.remove(&server_id);
                let typing: Vec<(Uuid, Uuid)> = socket
                    .typing
                    .iter()
                    .filter(|(sid, _)| *sid == server_id)
                    .copied()
                    .collect();
                for pair in &typing {
                    socket.typing.remove(pair);
                }
                let _ = socket
                    .tx
                    .send(ServerFrame::error(reason, Some(server_id), None));
                typing
            };
            for (sid, channel_id) in typing {
                if let Err(err) = self.stop_typing(sid, channel_id, user_id).await {
                    warn!(error = err.code(), "typing stop on revoke failed");
                }
            }
        }
    }

    async fn run_subscriber(&self) {
        loop {
            match self.subscribe_once().await {
                Ok(()) => warn!("redis pub/sub stream ended"),
                Err(err) => debug!(error = %err, "redis pub/sub not ready; retrying"),
            }
            self.inner.subscribed.store(false, Ordering::SeqCst);
            tokio::time::sleep(SUBSCRIBER_RETRY).await;
        }
    }

    async fn subscribe_once(&self) -> Result<(), redis::RedisError> {
        let mut pubsub = self.redis.get_async_pubsub().await?;
        pubsub.psubscribe(format!("{REDIS_PREFIX}*")).await?;
        self.inner.subscribed.store(true, Ordering::SeqCst);
        debug!("gateway subscribed to redis pub/sub");
        let mut stream = pubsub.on_message();
        while let Some(msg) = stream.next().await {
            let channel = msg.get_channel_name().to_owned();
            match msg.get_payload::<String>() {
                Ok(raw) => self.deliver_raw(&channel, &raw).await,
                Err(err) => warn!(error = %err, "invalid redis pub/sub payload"),
            }
        }
        Ok(())
    }

    async fn deliver_raw(&self, channel: &str, raw: &str) {
        if let Some(channel_id) = voice_channel_id(channel) {
            match serde_json::from_str::<SigEvent>(raw) {
                Ok(event) => {
                    self.deliver_sig(channel_id, ServerFrame::sig(event)).await;
                }
                Err(_) => warn!(channel, "redis payload is not a sig frame"),
            }
            return;
        }
        if let Some(live) = LiveTopic::from_redis_channel(channel) {
            match serde_json::from_str::<super::protocol::LiveFrame>(raw) {
                Ok(frame) => self.deliver_live(live, frame.into()).await,
                Err(err) => warn!(channel, error = %err, "redis payload is not a live frame"),
            }
            return;
        }
        let Some(topic) = Topic::from_redis_channel(channel) else {
            return;
        };
        let Ok(event) = serde_json::from_str::<Event>(raw) else {
            warn!(channel, "redis payload is not a compact event");
            return;
        };
        self.deliver(topic, event).await;
    }

    async fn deliver_sig(&self, channel_id: Uuid, frame: ServerFrame) {
        let (kind, server_id) = match &frame {
            ServerFrame::Sig { t, s, .. } => (*t, *s),
            _ => return,
        };
        let media = kind.room_only();
        let sockets = self.inner.sockets.read().await;
        for socket in sockets.values() {
            let in_room = socket.rooms.contains_key(&channel_id);
            if media {
                if in_room {
                    let _ = socket.tx.send(frame.clone());
                }
                continue;
            }
            if in_room || socket.servers.contains(&server_id) {
                let _ = socket.tx.send(frame.clone());
            }
        }
    }

    async fn deliver_live(&self, live: LiveTopic, frame: ServerFrame) {
        let sockets = self.inner.sockets.read().await;
        for socket in sockets.values() {
            let want = match live {
                LiveTopic::Presence(server_id) => socket.servers.contains(&server_id),
                LiveTopic::Typing(channel_id) => {
                    socket.topics.contains(&Topic::Channel(channel_id))
                }
            };
            if want {
                let _ = socket.tx.send(frame.clone());
            }
        }
    }

    async fn deliver(&self, topic: Topic, event: Event) {
        let mut sockets = self.inner.sockets.write().await;
        for socket in sockets.values_mut() {
            if !socket.topics.contains(&topic) {
                continue;
            }
            if let Some(buf) = socket.catching_up.get_mut(&topic) {
                buf.push(event.clone());
                continue;
            }
            let _ = socket.tx.send(ServerFrame::event(event.clone()));
        }
    }

    async fn redis_conn(&self) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
        let mut slot = self.inner.conn.lock().await;
        if let Some(conn) = slot.as_ref() {
            return Ok(conn.clone());
        }
        let conn = self.redis.get_multiplexed_async_connection().await?;
        *slot = Some(conn.clone());
        Ok(conn)
    }

    async fn reset_conn(&self) {
        *self.inner.conn.lock().await = None;
    }

    pub(super) async fn with_conn<T, F, Fut>(&self, mut op: F) -> Result<T, redis::RedisError>
    where
        F: FnMut(redis::aio::MultiplexedConnection) -> Fut,
        Fut: std::future::Future<Output = Result<T, redis::RedisError>>,
    {
        let conn = self.redis_conn().await?;
        match op(conn).await {
            Ok(value) => Ok(value),
            Err(err) if err.is_connection_dropped() || err.is_io_error() => {
                self.reset_conn().await;
                let conn = self.redis_conn().await?;
                op(conn).await
            }
            Err(err) => Err(err),
        }
    }

    /// Assign seq, append the replay buffer, PUBLISH — one Lua turn so a
    /// concurrent writer cannot `PUBLISH` 2 before 1.
    pub async fn publish(&self, draft: EventDraft) -> Result<Event, ApiError> {
        self.ensure_subscriber();
        let topic = draft.topic();
        let placeholder = Event {
            t: draft.kind,
            s: draft.server_id,
            c: draft.channel_id,
            n: 0,
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
                        .arg(template)
                        .arg(replay_end)
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
        let current = self.current_seq(topic).await?;
        let log = self.load_log(topic).await?;
        Ok((current, plan_catch_up(client_n, current, &log, |e| e.n)))
    }

    pub async fn in_voice(&self, id: ConnId, channel_id: Uuid) -> bool {
        self.inner
            .sockets
            .read()
            .await
            .get(&id)
            .is_some_and(|socket| socket.rooms.contains_key(&channel_id))
    }

    /// Seat this socket in `channel_id`, leaving any other voice room first.
    /// Returns other members (and their pubs) so the handler can snapshot
    /// them onto this socket without waiting for Pub/Sub.
    pub async fn join_voice(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
    ) -> Result<Vec<SigEvent>, ApiError> {
        self.ensure_subscriber();
        let previous: Vec<(Uuid, Uuid)> = {
            let sockets = self.inner.sockets.read().await;
            sockets
                .get(&id)
                .map(|socket| {
                    socket
                        .rooms
                        .iter()
                        .filter(|(cid, _)| **cid != channel_id)
                        .map(|(cid, seat)| (seat.server_id, *cid))
                        .collect()
                })
                .unwrap_or_default()
        };
        for (prev_server, prev_channel) in previous {
            self.leave_voice(id, user_id, prev_server, prev_channel)
                .await?;
        }

        let inserted = {
            let mut sockets = self.inner.sockets.write().await;
            if let Some(socket) = sockets.get_mut(&id) {
                match socket.rooms.entry(channel_id) {
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(VoiceSeat {
                            server_id,
                            pubs: HashSet::new(),
                            muted: false,
                            deafened: false,
                        });
                        true
                    }
                    std::collections::hash_map::Entry::Occupied(_) => false,
                }
            } else {
                false
            }
        };
        if inserted {
            self.redis_join_member(channel_id, user_id).await?;
            self.redis_put_occupancy(server_id, user_id, channel_id, false, false)
                .await?;
        }
        let occupancy = self.redis_occupancy(server_id).await?;
        let roster = self.redis_roster(channel_id).await?;
        let mut snapshot = Vec::new();
        for (uid, pubs) in roster {
            if uid == user_id {
                continue;
            }
            let (muted, deafened) = occupancy
                .get(&uid)
                .map(|row| (row.muted, row.deafened))
                .unwrap_or((false, false));
            snapshot.push(SigEvent::join_state(
                server_id, channel_id, uid, muted, deafened,
            ));
            for kind in pubs {
                snapshot.push(SigEvent::published(server_id, channel_id, uid, kind));
            }
        }
        self.publish_sig(SigEvent::join(server_id, channel_id, user_id))
            .await?;
        Ok(snapshot)
    }

    pub async fn leave_voice(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
    ) -> Result<bool, ApiError> {
        let was_in = {
            let mut sockets = self.inner.sockets.write().await;
            sockets
                .get_mut(&id)
                .is_some_and(|socket| socket.rooms.remove(&channel_id).is_some())
        };
        if !was_in {
            return Ok(false);
        }
        if self
            .redis_leave_member(server_id, channel_id, user_id)
            .await?
        {
            self.publish_sig(SigEvent::leave(server_id, channel_id, user_id))
                .await?;
        }
        Ok(true)
    }

    pub async fn set_voice_mute(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
        on: bool,
    ) -> Result<bool, ApiError> {
        self.set_voice_flag(id, user_id, server_id, channel_id, VoiceFlag::Mute, on)
            .await
    }

    pub async fn set_voice_deafen(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
        on: bool,
    ) -> Result<bool, ApiError> {
        self.set_voice_flag(id, user_id, server_id, channel_id, VoiceFlag::Deafen, on)
            .await
    }

    async fn set_voice_flag(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
        flag: VoiceFlag,
        on: bool,
    ) -> Result<bool, ApiError> {
        let (muted, deafened) = {
            let mut sockets = self.inner.sockets.write().await;
            let Some(seat) = sockets
                .get_mut(&id)
                .and_then(|socket| socket.rooms.get_mut(&channel_id))
            else {
                return Ok(false);
            };
            match flag {
                VoiceFlag::Mute => seat.muted = on,
                VoiceFlag::Deafen => {
                    seat.deafened = on;
                    if on {
                        seat.muted = true;
                    }
                }
            }
            (seat.muted, seat.deafened)
        };
        self.redis_write_occupancy(server_id, user_id, channel_id, muted, deafened)
            .await?;
        let event = match flag {
            VoiceFlag::Mute => SigEvent::muted(server_id, channel_id, user_id, on),
            VoiceFlag::Deafen => SigEvent::deafened(server_id, channel_id, user_id, on),
        };
        self.publish_sig(event).await?;
        if flag == VoiceFlag::Deafen && on {
            self.publish_sig(SigEvent::muted(server_id, channel_id, user_id, true))
                .await?;
        }
        Ok(true)
    }

    pub async fn voice_snapshot(&self, server_id: Uuid) -> Result<Vec<VoiceEntry>, ApiError> {
        let occupancy = self.redis_occupancy(server_id).await?;
        let mut rows: Vec<VoiceEntry> = occupancy
            .into_iter()
            .map(|(u, row)| VoiceEntry {
                u,
                c: row.channel_id,
                m: row.muted,
                d: row.deafened,
            })
            .collect();
        rows.sort_by_key(|row| row.u);
        Ok(rows)
    }

    pub async fn set_voice_pub(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
        kind: TrackKind,
        on: bool,
    ) -> Result<bool, ApiError> {
        {
            let mut sockets = self.inner.sockets.write().await;
            let Some(seat) = sockets
                .get_mut(&id)
                .and_then(|socket| socket.rooms.get_mut(&channel_id))
            else {
                return Ok(false);
            };
            if on {
                seat.pubs.insert(kind);
            } else {
                seat.pubs.remove(&kind);
            }
        }
        if on {
            self.redis_add_pub(channel_id, user_id, kind).await?;
            self.publish_sig(SigEvent::published(server_id, channel_id, user_id, kind))
                .await?;
        } else {
            self.redis_remove_pub(channel_id, user_id, kind).await?;
            self.publish_sig(SigEvent::unpublished(server_id, channel_id, user_id, kind))
                .await?;
        }
        Ok(true)
    }

    /// Fan-out a live signaling frame. No seq, no replay list.
    pub async fn publish_sig(&self, event: SigEvent) -> Result<(), ApiError> {
        self.ensure_subscriber();
        let raw = ServerFrame::sig(event.clone())
            .to_json()
            .map_err(|err| ApiError::Internal(format!("serialize sig: {err}")))?;
        let channel = voice_redis_channel(event.c);
        self.with_conn(|mut conn| {
            let channel = channel.clone();
            let raw = raw.clone();
            async move {
                redis::cmd("PUBLISH")
                    .arg(channel)
                    .arg(raw)
                    .query_async::<()>(&mut conn)
                    .await
            }
        })
        .await
        .map_err(redis_err)?;
        Ok(())
    }

    async fn redis_join_member(&self, channel_id: Uuid, user_id: Uuid) -> Result<(), ApiError> {
        let key = room_key(channel_id);
        let member = user_id.to_string();
        self.with_conn(|mut conn| {
            let key = key.clone();
            let member = member.clone();
            async move {
                redis::cmd("HINCRBY")
                    .arg(key)
                    .arg(member)
                    .arg(1)
                    .query_async::<i64>(&mut conn)
                    .await
                    .map(|_| ())
            }
        })
        .await
        .map_err(redis_err)
    }

    /// Decrement the per-user seat count. `true` = last socket left: drop
    /// the roster row, pubs, occupancy, caller should broadcast `t:"l"`.
    async fn redis_leave_member(
        &self,
        server_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> Result<bool, ApiError> {
        let members = room_key(channel_id);
        let pubs = pubs_key(channel_id, user_id);
        let member = user_id.to_string();
        let last: i32 = self
            .with_conn(|mut conn| {
                let members = members.clone();
                let pubs = pubs.clone();
                let member = member.clone();
                async move {
                    redis::Script::new(LEAVE_SEAT_LUA)
                        .key(members)
                        .key(pubs)
                        .arg(member)
                        .invoke_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)?;
        if last == 1 {
            self.redis_drop_occupancy(server_id, user_id).await?;
        }
        Ok(last == 1)
    }

    async fn redis_put_occupancy(
        &self,
        server_id: Uuid,
        user_id: Uuid,
        channel_id: Uuid,
        muted: bool,
        deafened: bool,
    ) -> Result<(), ApiError> {
        // Second tab of the same user must not wipe mute/deafen.
        let key = occupancy_key(server_id);
        let member = user_id.to_string();
        let value = encode_occupancy(channel_id, muted, deafened);
        self.with_conn(|mut conn| {
            let key = key.clone();
            let member = member.clone();
            let value = value.clone();
            async move {
                redis::cmd("HSETNX")
                    .arg(key)
                    .arg(member)
                    .arg(value)
                    .query_async::<i64>(&mut conn)
                    .await
                    .map(|_| ())
            }
        })
        .await
        .map_err(redis_err)
    }

    async fn redis_write_occupancy(
        &self,
        server_id: Uuid,
        user_id: Uuid,
        channel_id: Uuid,
        muted: bool,
        deafened: bool,
    ) -> Result<(), ApiError> {
        let key = occupancy_key(server_id);
        let member = user_id.to_string();
        let value = encode_occupancy(channel_id, muted, deafened);
        self.with_conn(|mut conn| {
            let key = key.clone();
            let member = member.clone();
            let value = value.clone();
            async move {
                redis::cmd("HSET")
                    .arg(key)
                    .arg(member)
                    .arg(value)
                    .query_async::<i64>(&mut conn)
                    .await
                    .map(|_| ())
            }
        })
        .await
        .map_err(redis_err)
    }

    async fn redis_drop_occupancy(&self, server_id: Uuid, user_id: Uuid) -> Result<(), ApiError> {
        let key = occupancy_key(server_id);
        let member = user_id.to_string();
        self.with_conn(|mut conn| {
            let key = key.clone();
            let member = member.clone();
            async move {
                redis::cmd("HDEL")
                    .arg(key)
                    .arg(member)
                    .query_async::<i64>(&mut conn)
                    .await
                    .map(|_| ())
            }
        })
        .await
        .map_err(redis_err)
    }

    async fn redis_occupancy(&self, server_id: Uuid) -> Result<HashMap<Uuid, Occupancy>, ApiError> {
        let key = occupancy_key(server_id);
        let seats: HashMap<String, String> = self
            .with_conn(|mut conn| {
                let key = key.clone();
                async move { redis::cmd("HGETALL").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(redis_err)?;
        let mut occupancy = HashMap::with_capacity(seats.len());
        for (raw, value) in seats {
            let Ok(uid) = Uuid::parse_str(&raw) else {
                continue;
            };
            let Some(row) = parse_occupancy(&value) else {
                continue;
            };
            occupancy.insert(uid, row);
        }
        Ok(occupancy)
    }

    async fn redis_add_pub(
        &self,
        channel_id: Uuid,
        user_id: Uuid,
        kind: TrackKind,
    ) -> Result<(), ApiError> {
        let key = pubs_key(channel_id, user_id);
        let kind = kind.as_str().to_owned();
        self.with_conn(|mut conn| {
            let key = key.clone();
            let kind = kind.clone();
            async move {
                redis::cmd("SADD")
                    .arg(key)
                    .arg(kind)
                    .query_async::<()>(&mut conn)
                    .await
            }
        })
        .await
        .map_err(redis_err)
    }

    async fn redis_remove_pub(
        &self,
        channel_id: Uuid,
        user_id: Uuid,
        kind: TrackKind,
    ) -> Result<(), ApiError> {
        let key = pubs_key(channel_id, user_id);
        let kind = kind.as_str().to_owned();
        self.with_conn(|mut conn| {
            let key = key.clone();
            let kind = kind.clone();
            async move {
                redis::cmd("SREM")
                    .arg(key)
                    .arg(kind)
                    .query_async::<()>(&mut conn)
                    .await
            }
        })
        .await
        .map_err(redis_err)
    }

    async fn redis_roster(
        &self,
        channel_id: Uuid,
    ) -> Result<Vec<(Uuid, Vec<TrackKind>)>, ApiError> {
        let key = room_key(channel_id);
        let seats: HashMap<String, String> = self
            .with_conn(|mut conn| {
                let key = key.clone();
                async move { redis::cmd("HGETALL").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(redis_err)?;
        let mut roster = Vec::with_capacity(seats.len());
        for (raw, count) in seats {
            let Ok(count) = count.parse::<i64>() else {
                continue;
            };
            if count <= 0 {
                continue;
            }
            let Ok(uid) = Uuid::parse_str(&raw) else {
                continue;
            };
            let pubs_key = pubs_key(channel_id, uid);
            let pubs: Vec<String> = self
                .with_conn(|mut conn| {
                    let pubs_key = pubs_key.clone();
                    async move {
                        redis::cmd("SMEMBERS")
                            .arg(pubs_key)
                            .query_async(&mut conn)
                            .await
                    }
                })
                .await
                .map_err(redis_err)?;
            let kinds = pubs
                .into_iter()
                .filter_map(|name| match name.as_str() {
                    "a" => Some(TrackKind::A),
                    "v" => Some(TrackKind::V),
                    "s" => Some(TrackKind::S),
                    _ => None,
                })
                .collect();
            roster.push((uid, kinds));
        }
        Ok(roster)
    }
}

pub(super) fn redis_err(err: redis::RedisError) -> ApiError {
    ApiError::Internal(format!("redis: {err}"))
}

pub fn voice_redis_channel(channel_id: Uuid) -> String {
    format!("{REDIS_PREFIX}v:{channel_id}")
}

fn voice_channel_id(name: &str) -> Option<Uuid> {
    let rest = name.strip_prefix(REDIS_PREFIX)?;
    let (kind, id) = rest.split_once(':')?;
    if kind != "v" {
        return None;
    }
    Uuid::parse_str(id).ok()
}

/// Per-channel hash: user_id → seat count (one increment per socket).
fn room_key(channel_id: Uuid) -> String {
    format!("{REDIS_PREFIX}vr:{channel_id}")
}

fn pubs_key(channel_id: Uuid, user_id: Uuid) -> String {
    format!("{REDIS_PREFIX}vp:{channel_id}:{user_id}")
}

/// Server-wide voice occupancy for the member list (`user` → channel + flags).
fn occupancy_key(server_id: Uuid) -> String {
    format!("{REDIS_PREFIX}vo:{server_id}")
}

fn encode_occupancy(channel_id: Uuid, muted: bool, deafened: bool) -> String {
    format!("{channel_id}:{}{}", u8::from(muted), u8::from(deafened))
}

fn parse_occupancy(raw: &str) -> Option<Occupancy> {
    let (id, flags) = raw.rsplit_once(':')?;
    if flags.len() != 2 {
        return None;
    }
    let bytes = flags.as_bytes();
    Some(Occupancy {
        channel_id: Uuid::parse_str(id).ok()?,
        muted: bytes[0] == b'1',
        deafened: bytes[1] == b'1',
    })
}
