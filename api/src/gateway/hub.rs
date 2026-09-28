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
/// `ARGV[1]` is compact event JSON with `n` as placeholder 0 (serde order
/// is `t`,`n`,`d`). Seq is string-substituted — Lua cjson would turn `[]`
/// into `{}`.
const PUBLISH_LUA: &str = r#"
local n = redis.call('INCR', KEYS[1])
local raw = string.gsub(ARGV[1], '"n":0', '"n":' .. n, 1)
redis.call('LPUSH', KEYS[2], raw)
redis.call('LTRIM', KEYS[2], 0, tonumber(ARGV[2]))
redis.call('PUBLISH', KEYS[3], raw)
return {n, raw}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(u64, Uuid);

impl ConnId {
    pub fn redis_id(self) -> Uuid {
        self.1
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

pub(super) struct Inner {
    next_id: AtomicU64,
    started: AtomicBool,
    subscribed: AtomicBool,
    conn: Mutex<Option<redis::aio::MultiplexedConnection>>,
    pub(super) sockets: RwLock<HashMap<ConnId, Socket>>,
    pub(super) voice_ops: Mutex<()>,
    media_sessions: Mutex<HashMap<String, (Uuid, std::time::Instant)>>,
}

#[derive(Clone)]
pub(super) struct VoiceSeat {
    pub(super) id: Uuid,
    pub(super) server_id: Uuid,
    pub(super) pubs: HashSet<TrackKind>,
    pub(super) muted: bool,
    pub(super) deafened: bool,
    pub(super) live: Option<super::leases::LiveOwner>,
}

pub(super) struct Socket {
    pub(super) user_id: Uuid,
    pub(super) session: Option<(String, tokio::sync::watch::Sender<bool>)>,
    access: HashMap<Uuid, (chrono::DateTime<chrono::Utc>, i32)>,
    /// Servers this socket has subscribed to (channel or server topic).
    pub(super) servers: HashSet<Uuid>,
    pub(super) voice_rosters: HashMap<Uuid, Vec<VoiceEntry>>,
    /// Channels this socket started typing in (`server`, `channel`).
    typing: HashSet<(Uuid, Uuid)>,
    topics: HashSet<Topic>,
    /// Voice rooms this socket has joined (`op: "sig"`). Independent of
    /// chat topic subscriptions.
    pub(super) rooms: HashMap<Uuid, VoiceSeat>,
    /// Live Pub/Sub frames held until catch-up finishes. Dropping them
    /// here would punch a hole the replay log can miss.
    catching_up: HashMap<Topic, Vec<Event>>,
    pub(super) tx: mpsc::UnboundedSender<ServerFrame>,
}

/// Local sockets, the Redis subscriber, and the multiplexed command connection.
#[derive(Clone)]
pub struct ConnTable {
    redis: redis::Client,
    pub(super) inner: Arc<Inner>,
}

/// Sequenced chat log (`publish` / `catch_up`). Messages live in Postgres;
/// this is the replay buffer and the fan-out, not voice and not RTP.
#[derive(Clone)]
pub struct EventLog {
    replay: usize,
    connections: ConnTable,
}

/// Voice occupancy, publications, and the Go Live claim. Ephemeral Redis,
/// separate from the chat log.
#[derive(Clone)]
pub struct VoiceRoster {
    pub(super) connections: ConnTable,
}

/// Facade over the connection table, the event log, and the voice roster so
/// `conn.rs` keeps one handle.
#[derive(Clone)]
pub struct Gateway {
    pub connections: ConnTable,
    pub events: EventLog,
    pub voice: VoiceRoster,
    presence_ttl: Duration,
    typing_ttl: Duration,
}

impl ConnTable {
    fn new(redis: redis::Client) -> Self {
        Self {
            redis,
            inner: Arc::new(Inner {
                next_id: AtomicU64::new(1),
                started: AtomicBool::new(false),
                subscribed: AtomicBool::new(false),
                conn: Mutex::new(None),
                sockets: RwLock::new(HashMap::new()),
                voice_ops: Mutex::new(()),
                media_sessions: Mutex::new(HashMap::new()),
            }),
        }
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
        let id = ConnId(
            self.inner.next_id.fetch_add(1, Ordering::Relaxed),
            Uuid::new_v4(),
        );
        self.inner.sockets.write().await.insert(
            id,
            Socket {
                user_id,
                session: None,
                access: HashMap::new(),
                servers: HashSet::new(),
                voice_rosters: HashMap::new(),
                typing: HashSet::new(),
                topics: HashSet::new(),
                rooms: HashMap::new(),
                catching_up: HashMap::new(),
                tx,
            },
        );
        id
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
    /// Register the topic and queue live Redis events until [`finish_catch_up`].
    pub async fn begin_catch_up(&self, id: ConnId, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.topics.insert(topic);
            socket.catching_up.entry(topic).or_default();
        }
    }

    /// Release the topic to live delivery and flush queued frames with
    /// `n > after_n` (already-replayed seqs are dropped).
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
        let server_id = match &frame {
            ServerFrame::Sig { s, .. } => *s,
            _ => return,
        };
        let sockets = self.inner.sockets.read().await;
        for socket in sockets.values() {
            let in_room = socket.rooms.contains_key(&channel_id);
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
}

impl EventLog {
    /// Assign seq, append the replay buffer, PUBLISH — one Lua turn so a
    /// concurrent writer cannot `PUBLISH` 2 before 1.
    pub async fn publish(&self, draft: EventDraft) -> Result<Event, ApiError> {
        self.connections.ensure_subscriber();
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
        let current = self.current_seq(topic).await?;
        let log = self.load_log(topic).await?;
        Ok((current, plan_catch_up(client_n, current, &log, |e| e.n)))
    }
}

impl VoiceRoster {
    /// Fan-out a live signaling frame. No seq, no replay list.
    pub async fn publish_sig(&self, event: SigEvent) -> Result<(), ApiError> {
        self.connections.ensure_subscriber();
        let raw = ServerFrame::sig(event.clone())
            .to_json()
            .map_err(|err| ApiError::Internal(format!("serialize sig: {err}")))?;
        let channel = voice_redis_channel(event.c);
        self.connections
            .with_conn(|mut conn| {
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

    /// Short Redis deny so the SFU drops this user's peer even if the tab
    /// ignores the gateway frame.
    pub async fn deny(&self, user_id: Uuid, server_id: Uuid) -> Result<(), ApiError> {
        let key = gelabber_shared::ticket::deny_key(server_id, user_id);
        let ttl = gelabber_shared::ticket::DENY_TTL_SECS;
        self.connections
            .with_conn(|mut conn| {
                let key = key.clone();
                async move {
                    redis::cmd("SET")
                        .arg(key)
                        .arg("1")
                        .arg("EX")
                        .arg(ttl)
                        .query_async::<()>(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)
    }
}

impl Gateway {
    pub fn new(
        redis: redis::Client,
        replay: usize,
        presence_ttl: Duration,
        typing_ttl: Duration,
    ) -> Self {
        let connections = ConnTable::new(redis);
        Self {
            connections: connections.clone(),
            events: EventLog {
                replay: replay.max(1),
                connections: connections.clone(),
            },
            voice: VoiceRoster { connections },
            presence_ttl,
            typing_ttl,
        }
    }

    pub fn presence_ttl(&self) -> Duration {
        self.presence_ttl
    }

    pub fn typing_ttl(&self) -> Duration {
        self.typing_ttl
    }

    pub async fn detach(&self, id: ConnId) -> Option<(Uuid, HashSet<Uuid>, HashSet<(Uuid, Uuid)>)> {
        let _guard = self.connections.inner.voice_ops.lock().await;
        let socket = self.connections.inner.sockets.write().await.remove(&id)?;
        for (channel_id, seat) in socket.rooms {
            if let Err(err) = self
                .voice
                .drop_seat(id, socket.user_id, channel_id, seat)
                .await
            {
                warn!(error = err.code(), "voice leave on detach failed");
            }
        }
        Some((socket.user_id, socket.servers, socket.typing))
    }

    /// Drop a user's live seats on one server after kick/ban: unsubscribe
    /// chat topics, leave voice rooms, tell their sockets why.
    pub async fn revoke_server(
        &self,
        user_id: Uuid,
        server_id: Uuid,
        channel_ids: &[Uuid],
        reason: &'static str,
    ) -> Result<(), ApiError> {
        crate::media::revoke_member(&self.connections.redis, server_id, user_id).await?;
        self.voice.deny(user_id, server_id).await?;
        let conns: Vec<ConnId> = self
            .connections
            .inner
            .sockets
            .read()
            .await
            .iter()
            .filter(|(_, socket)| socket.user_id == user_id)
            .map(|(id, _)| *id)
            .collect();
        for id in conns {
            self.clear_server_access(id, server_id, channel_ids, reason)
                .await;
        }
        Ok(())
    }

    async fn clear_server_access(
        &self,
        id: ConnId,
        server_id: Uuid,
        channel_ids: &[Uuid],
        reason: &'static str,
    ) {
        let Some((user_id, rooms, typing)) = ({
            let mut sockets = self.connections.inner.sockets.write().await;
            sockets.get_mut(&id).map(|socket| {
                socket.access.remove(&server_id);
                socket.servers.remove(&server_id);
                socket.topics.remove(&Topic::Server(server_id));
                socket.catching_up.remove(&Topic::Server(server_id));
                for channel_id in channel_ids {
                    socket.topics.remove(&Topic::Channel(*channel_id));
                    socket.catching_up.remove(&Topic::Channel(*channel_id));
                }
                let rooms: Vec<Uuid> = socket
                    .rooms
                    .iter()
                    .filter(|(_, seat)| seat.server_id == server_id)
                    .map(|(cid, _)| *cid)
                    .collect();
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
                (socket.user_id, rooms, typing)
            })
        }) else {
            return;
        };
        for cid in rooms {
            if let Err(err) = self.leave_voice(id, user_id, server_id, cid).await {
                warn!(error = err.code(), "voice cleanup failed");
            }
        }
        for (sid, cid) in typing {
            if let Err(err) = self.stop_typing(sid, cid, user_id).await {
                warn!(error = err.code(), "typing cleanup failed");
            }
        }
        if let Err(err) = self.forget_server_presence(user_id, server_id).await {
            warn!(error = err.code(), "server presence cleanup failed");
        }
    }

    pub async fn track_media_session(&self, key: &str) -> Option<Uuid> {
        let mut sessions = self.connections.inner.media_sessions.lock().await;
        if let Some((_, last_mint)) = sessions.get_mut(key) {
            *last_mint = std::time::Instant::now();
            return None;
        }
        let generation = Uuid::new_v4();
        sessions.insert(key.to_owned(), (generation, std::time::Instant::now()));
        Some(generation)
    }
    pub async fn media_session_recent(&self, key: &str, generation: Uuid) -> bool {
        self.connections
            .inner
            .media_sessions
            .lock()
            .await
            .get(key)
            .is_some_and(|(owner, last)| {
                *owner == generation && last.elapsed() < Duration::from_secs(35)
            })
    }
    pub async fn untrack_media_session(&self, key: &str, generation: Uuid) {
        let mut sessions = self.connections.inner.media_sessions.lock().await;
        if sessions
            .get(key)
            .is_some_and(|(owner, _)| *owner == generation)
        {
            sessions.remove(key);
        }
    }

    pub async fn attach_session(
        &self,
        user_id: Uuid,
        hash: String,
        tx: mpsc::UnboundedSender<ServerFrame>,
    ) -> (ConnId, tokio::sync::watch::Receiver<bool>) {
        let id = self.attach(user_id, tx).await;
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
            socket.session = Some((hash, cancel));
        }
        (id, receiver)
    }

    pub async fn revoke_session(&self, hash: &str) {
        let mut sockets = self.connections.inner.sockets.write().await;
        for socket in sockets.values_mut() {
            if let Some((session, cancel)) = &socket.session
                && session == hash
            {
                socket.topics.clear();
                socket.catching_up.clear();
                let _ = cancel.send(true);
            }
        }
    }

    pub async fn wants_frame(&self, id: ConnId, frame: &ServerFrame) -> bool {
        let sockets = self.connections.inner.sockets.read().await;
        let Some(socket) = sockets.get(&id) else {
            return false;
        };
        match frame {
            ServerFrame::Event { s, c, .. } => socket.topics.contains(&Topic::of(*s, *c)),
            ServerFrame::Sig { s, c, .. } => {
                socket.servers.contains(s)
                    || c.is_some_and(|cid| {
                        socket
                            .rooms
                            .get(&cid)
                            .is_some_and(|seat| seat.server_id == *s)
                    })
            }
            ServerFrame::Presence { s, .. } => socket.servers.contains(s),
            ServerFrame::Typing { c, .. } => socket.topics.contains(&Topic::Channel(*c)),
            _ => true,
        }
    }

    pub async fn bind_server(
        &self,
        id: ConnId,
        db: &mut sqlx::PgConnection,
        server_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ApiError> {
        let revision = crate::servers::membership::revision(&mut *db, server_id, user_id)
            .await?
            .ok_or(ApiError::NotFound)?;
        if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
            socket.access.insert(server_id, revision);
        }
        Ok(())
    }

    /// Revalidate existing subscriptions/seats, including on another API process.
    /// A new joined_at prevents leave + immediate rejoin from reviving old seats.
    pub async fn reconcile_access(
        &self,
        id: ConnId,
        db: &mut sqlx::PgConnection,
    ) -> Result<(), ApiError> {
        let Some((user_id, access, topics, rooms)) = self
            .connections
            .inner
            .sockets
            .read()
            .await
            .get(&id)
            .map(|socket| {
                (
                    socket.user_id,
                    socket.access.clone(),
                    socket.topics.clone(),
                    socket
                        .rooms
                        .iter()
                        .map(|(cid, seat)| (*cid, seat.server_id))
                        .collect::<Vec<_>>(),
                )
            })
        else {
            return Ok(());
        };
        for (sid, revision) in access {
            let current = crate::servers::membership::revision(&mut *db, sid, user_id).await?;
            if current.is_none_or(|now| now.0 != revision.0) {
                // Include deleted channels: their rows can no longer be queried.
                let mut cids: Vec<Uuid> =
                    sqlx::query_scalar("SELECT id FROM channels WHERE server_id = $1")
                        .bind(sid)
                        .fetch_all(&mut *db)
                        .await?;
                for topic in &topics {
                    if let Topic::Channel(cid) = topic
                        && crate::servers::channel::get(&mut *db, *cid)
                            .await?
                            .is_none()
                    {
                        cids.push(*cid);
                    }
                }
                self.clear_server_access(id, sid, &cids, "not_found").await;
            } else {
                let current = current.expect("checked");
                if revision.1 & !current.1 & 48 != 0 {
                    self.clear_voice_access(id, sid).await?;
                }
                if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
                    socket.access.insert(sid, current);
                }
            }
        }
        for topic in topics {
            if let Topic::Channel(cid) = topic
                && crate::servers::channel::get(&mut *db, cid).await?.is_none()
            {
                self.unsubscribe(id, topic).await;
            }
        }
        for (cid, sid) in rooms {
            let member = crate::servers::membership::load(&mut *db, sid, user_id).await;
            let allowed = match member {
                Ok(member) => member.can(crate::servers::permissions::Permission::JoinVoice),
                Err(ApiError::NotFound) => false,
                Err(err) => return Err(err),
            };
            if !allowed || crate::servers::channel::get(&mut *db, cid).await?.is_none() {
                self.leave_voice(id, user_id, sid, cid).await?;
            }
        }
        Ok(())
    }

    pub async fn revoke_voice_access(
        &self,
        user_id: Uuid,
        server_id: Uuid,
    ) -> Result<(), ApiError> {
        crate::media::revoke_member(&self.connections.redis, server_id, user_id).await?;
        self.voice.deny(user_id, server_id).await?;
        let ids: Vec<ConnId> = self
            .connections
            .inner
            .sockets
            .read()
            .await
            .iter()
            .filter(|(_, socket)| socket.user_id == user_id)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.clear_voice_access(id, server_id).await?;
        }
        Ok(())
    }

    async fn clear_voice_access(&self, id: ConnId, server_id: Uuid) -> Result<(), ApiError> {
        let Some((uid, rooms)) =
            self.connections
                .inner
                .sockets
                .read()
                .await
                .get(&id)
                .map(|socket| {
                    (
                        socket.user_id,
                        socket
                            .rooms
                            .iter()
                            .filter(|(_, seat)| seat.server_id == server_id)
                            .map(|(cid, _)| *cid)
                            .collect::<Vec<_>>(),
                    )
                })
        else {
            return Ok(());
        };
        for cid in rooms {
            self.leave_voice(id, uid, server_id, cid).await?;
        }
        Ok(())
    }

    pub async fn revoke_channel(&self, server_id: Uuid, channel_id: Uuid) -> Result<(), ApiError> {
        crate::media::revoke_channel(&self.connections.redis, channel_id).await?;
        let conns: Vec<(ConnId, Uuid)> = self
            .connections
            .inner
            .sockets
            .read()
            .await
            .iter()
            .map(|(id, socket)| (*id, socket.user_id))
            .collect();
        for (id, uid) in conns {
            self.unsubscribe(id, Topic::Channel(channel_id)).await;
            if let Err(err) = self.leave_voice(id, uid, server_id, channel_id).await {
                warn!(error = err.code(), "deleted channel voice cleanup failed");
            }
            self.note_typing(id, server_id, channel_id, false).await;
            if let Err(err) = self.stop_typing(server_id, channel_id, uid).await {
                warn!(error = err.code(), "deleted channel typing cleanup failed");
            }
        }
        Ok(())
    }

    pub fn ensure_subscriber(&self) {
        self.connections.ensure_subscriber()
    }

    pub fn is_ready(&self) -> bool {
        self.connections.is_ready()
    }

    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        self.connections.wait_ready(timeout).await
    }

    pub async fn attach(&self, user_id: Uuid, tx: mpsc::UnboundedSender<ServerFrame>) -> ConnId {
        self.connections.attach(user_id, tx).await
    }

    pub async fn watch_server(&self, id: ConnId, server_id: Uuid) {
        self.connections.watch_server(id, server_id).await
    }

    pub async fn socket_meta(&self, id: ConnId) -> Option<(Uuid, HashSet<Uuid>)> {
        self.connections.socket_meta(id).await
    }

    pub async fn note_typing(&self, id: ConnId, server_id: Uuid, channel_id: Uuid, on: bool) {
        self.connections
            .note_typing(id, server_id, channel_id, on)
            .await
    }

    /// Register the topic and queue live Redis events until [`finish_catch_up`].
    /// Register the topic and queue live Redis events until [`finish_catch_up`].
    pub async fn begin_catch_up(&self, id: ConnId, topic: Topic) {
        self.connections.begin_catch_up(id, topic).await
    }

    /// Release the topic to live delivery and flush queued frames with
    /// `n > after_n` (already-replayed seqs are dropped).
    /// Release the topic to live delivery and flush queued frames with
    /// `n > after_n` (already-replayed seqs are dropped).
    pub async fn finish_catch_up(&self, id: ConnId, topic: Topic, after_n: u64) {
        self.connections.finish_catch_up(id, topic, after_n).await
    }

    pub async fn queued_len(&self, id: ConnId, topic: Topic) -> usize {
        self.connections.queued_len(id, topic).await
    }

    pub async fn unsubscribe(&self, id: ConnId, topic: Topic) {
        self.connections.unsubscribe(id, topic).await
    }

    pub(super) async fn with_conn<T, F, Fut>(&self, op: F) -> Result<T, redis::RedisError>
    where
        F: FnMut(redis::aio::MultiplexedConnection) -> Fut,
        Fut: std::future::Future<Output = Result<T, redis::RedisError>>,
    {
        self.connections.with_conn(op).await
    }

    /// Assign seq, append the replay buffer, PUBLISH — one Lua turn so a
    /// concurrent writer cannot `PUBLISH` 2 before 1.
    pub async fn publish(&self, draft: EventDraft) -> Result<Event, ApiError> {
        self.events.publish(draft).await
    }

    pub async fn current_seq(&self, topic: Topic) -> Result<u64, ApiError> {
        self.events.current_seq(topic).await
    }

    pub async fn load_log(&self, topic: Topic) -> Result<Vec<Event>, ApiError> {
        self.events.load_log(topic).await
    }

    pub async fn catch_up(
        &self,
        topic: Topic,
        client_n: Option<u64>,
    ) -> Result<(u64, CatchUp<Event>), ApiError> {
        self.events.catch_up(topic, client_n).await
    }

    pub async fn in_voice(&self, id: ConnId, server_id: Uuid, channel_id: Uuid) -> bool {
        self.voice.in_voice(id, server_id, channel_id).await
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
        self.voice
            .join_voice(id, user_id, server_id, channel_id)
            .await
    }

    pub async fn leave_voice(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
    ) -> Result<bool, ApiError> {
        self.voice
            .leave_voice(id, user_id, server_id, channel_id)
            .await
    }

    pub async fn set_voice_mute(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
        on: bool,
    ) -> Result<bool, ApiError> {
        self.voice
            .set_voice_mute(id, user_id, server_id, channel_id, on)
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
        self.voice
            .set_voice_deafen(id, user_id, server_id, channel_id, on)
            .await
    }

    pub async fn voice_snapshot(&self, server_id: Uuid) -> Result<Vec<VoiceEntry>, ApiError> {
        self.voice.voice_snapshot(server_id).await
    }

    pub async fn set_voice_pub(
        &self,
        id: ConnId,
        user_id: Uuid,
        server_id: Uuid,
        channel_id: Uuid,
        kind: TrackKind,
        on: bool,
    ) -> Result<Option<bool>, ApiError> {
        self.voice
            .set_voice_pub(id, user_id, server_id, channel_id, kind, on)
            .await
    }

    /// Fan-out a live signaling frame. No seq, no replay list.
    /// Fan-out a live signaling frame. No seq, no replay list.
    pub async fn publish_sig(&self, event: SigEvent) -> Result<(), ApiError> {
        self.voice.publish_sig(event).await
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
