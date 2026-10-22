//! Local socket table + Redis 8 Pub/Sub fan-out.
//!
//! `PUBLISH` is the live path. A bounded Redis list per topic is the
//! reconnect buffer (Pub/Sub itself does not store messages, see
//! `event_log`). Each API process keeps one `PSUBSCRIBE gb:*` connection and
//! delivers matching events to sockets subscribed on this process (`fanout`).

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

mod event_log;
mod fanout;

pub use event_log::EventLog;

/// A reconnecting socket receives one presence frame per member that comes
/// back with it while it is still busy subscribing, so this bounds how many
/// members can reconnect at once (after a deploy) without overflowing. 1024
/// is headroom over the largest measured storm (200 members, no closes), not
/// a measured peak depth. Overflow still closes the socket and the client
/// resyncs. The channel grows in 32-slot blocks as frames queue, so only a
/// backed-up socket pays for the larger bound.
pub const OUTBOUND_CAPACITY: usize = 1024;
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
    pub(super) delivery_started: AtomicBool,
    pub(super) delivery_scan: Mutex<super::delivery::ScanCursor>,
    pub(super) delivery_wake: tokio::sync::Notify,
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
}

pub(super) struct Socket {
    pub(super) user_id: Uuid,
    pub(super) session: Option<(String, tokio::sync::watch::Sender<bool>)>,
    access: HashMap<Uuid, (chrono::DateTime<chrono::Utc>, i32)>,
    /// Servers this socket has subscribed to (channel or server topic).
    pub(super) servers: HashSet<Uuid>,
    /// The server of each subscribed topic. When the last topic of a server
    /// goes and no voice seat is left there, the server leaves `servers`
    /// and `access` too, so its presence, signaling and rosters stop.
    scopes: HashMap<Topic, Uuid>,
    /// DM ids in `servers` (a DM topic is `s == c`). They keep presence and
    /// signaling fan-out, but have no voice roster to refresh.
    pub(super) dms: HashSet<Uuid>,
    pub(super) voice_rosters: HashMap<Uuid, Vec<VoiceEntry>>,
    /// Channels this socket started typing in (`server`, `channel`).
    typing: HashSet<(Uuid, Uuid)>,
    topics: HashSet<Topic>,
    epochs: HashMap<Topic, Uuid>,
    closed: tokio::sync::watch::Sender<bool>,
    /// Wakes the socket for an immediate session and access check after a
    /// revocation on any API process ([`Recheck`]). Carries no authority;
    /// the check reads the DB.
    recheck: Arc<tokio::sync::Notify>,
    /// Voice rooms this socket has joined (`op: "sig"`). Independent of
    /// chat topic subscriptions.
    pub(super) rooms: HashMap<Uuid, VoiceSeat>,
    /// Live Pub/Sub frames held until catch-up finishes. Dropping them
    /// here would punch a hole the replay log can miss.
    catching_up: HashMap<Topic, Vec<Event>>,
    pub(super) tx: mpsc::Sender<ServerFrame>,
}

impl Socket {
    pub(super) fn send(&self, frame: ServerFrame) {
        if *self.closed.borrow() {
            return;
        }
        if self.tx.try_send(frame).is_err() {
            self.closed.send_replace(true);
        }
    }
}

/// Local sockets, the Redis subscriber, and the multiplexed command connection.
#[derive(Clone)]
pub struct ConnTable {
    redis: redis::Client,
    pub(super) inner: Arc<Inner>,
}

/// Voice occupancy and publications. Ephemeral Redis,
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
                delivery_started: AtomicBool::new(false),
                delivery_scan: Mutex::new(super::delivery::ScanCursor::default()),
                delivery_wake: tokio::sync::Notify::new(),
                subscribed: AtomicBool::new(false),
                conn: Mutex::new(None),
                sockets: RwLock::new(HashMap::new()),
                voice_ops: Mutex::new(()),
                media_sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub async fn attach(&self, user_id: Uuid, tx: mpsc::Sender<ServerFrame>) -> ConnId {
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
                scopes: HashMap::new(),
                dms: HashSet::new(),
                voice_rosters: HashMap::new(),
                typing: HashSet::new(),
                topics: HashSet::new(),
                epochs: HashMap::new(),
                closed: tokio::sync::watch::channel(false).0,
                recheck: Arc::new(tokio::sync::Notify::new()),
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

    /// [`Self::watch_server`] for a topic the socket subscribes to, so the
    /// server can be dropped again with its last topic.
    pub async fn watch_topic(&self, id: ConnId, server_id: Uuid, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.servers.insert(server_id);
            socket.scopes.insert(topic, server_id);
        }
    }

    pub async fn mark_dm(&self, id: ConnId, dm_id: Uuid) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.dms.insert(dm_id);
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
        self.finish_catch_up_epoch(id, topic, after_n, None).await;
    }
    pub async fn finish_catch_up_epoch(
        &self,
        id: ConnId,
        topic: Topic,
        after_n: u64,
        epoch: Option<Uuid>,
    ) {
        let mut sockets = self.inner.sockets.write().await;
        let Some(socket) = sockets.get_mut(&id) else {
            return;
        };
        if let Some(epoch) = epoch {
            socket.epochs.insert(topic, epoch);
        }
        let Some(mut buf) = socket.catching_up.remove(&topic) else {
            return;
        };
        if epoch.is_some_and(|epoch| buf.iter().any(|event| event.ep != Some(epoch))) {
            socket.send(ServerFrame::Resync);
            return;
        }
        buf.sort_by_key(|event| event.n);
        let mut seen = after_n;
        for event in buf {
            if event.n <= seen {
                continue;
            }
            seen = event.n;
            socket.send(ServerFrame::event(event));
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

    /// Drop the topic and, with the last topic of its server, the server.
    /// A voice seat on that server keeps it: signaling and the membership
    /// recheck still need it. Presence keys stay; the user is still online
    /// and `clear_conn` reads the user's Redis server set on detach.
    pub async fn unsubscribe(&self, id: ConnId, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.topics.remove(&topic);
            socket.catching_up.remove(&topic);
            if let Some(server) = socket.scopes.remove(&topic)
                && !socket.scopes.values().any(|sid| *sid == server)
                && !socket.rooms.values().any(|seat| seat.server_id == server)
            {
                socket.servers.remove(&server);
                socket.dms.remove(&server);
                socket.access.remove(&server);
                socket.voice_rosters.remove(&server);
            }
        }
    }

    /// Wake local sockets that `target` may concern for an immediate check.
    fn recheck_local(sockets: &HashMap<ConnId, Socket>, target: &Recheck) {
        for socket in sockets.values() {
            let wanted = match target {
                Recheck::User(user_id) => socket.user_id == *user_id,
                Recheck::Server(server_id) => {
                    socket.access.contains_key(server_id)
                        || socket.servers.contains(server_id)
                        || socket
                            .rooms
                            .values()
                            .any(|seat| seat.server_id == *server_id)
                }
                Recheck::Channel(channel_id) => {
                    socket.topics.contains(&Topic::Channel(*channel_id))
                        || socket.rooms.contains_key(channel_id)
                }
                Recheck::Session(key) => socket
                    .session
                    .as_ref()
                    .is_some_and(|(session, _)| session == key),
            };
            if wanted {
                socket.recheck.notify_one();
            }
        }
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

    /// Drop users' live seats on one server after kick/ban/leave or a server
    /// delete: rotate their media generations, unsubscribe chat topics,
    /// leave voice rooms, tell their sockets why. The rotations share one
    /// pipeline and the socket table is scanned once for all users. Presence
    /// is the caller's job ([`Gateway::forget_server_presence`]), once per
    /// user rather than once per socket.
    pub async fn revoke_server(
        &self,
        user_ids: &[Uuid],
        server_id: Uuid,
        channel_ids: &[Uuid],
        reason: &'static str,
    ) -> Result<(), ApiError> {
        self.revoke_members(server_id, user_ids).await?;
        for id in self.local_conns(user_ids).await {
            self.drop_server_access(id, server_id, channel_ids, reason, false)
                .await;
        }
        Ok(())
    }

    async fn revoke_members(&self, server_id: Uuid, user_ids: &[Uuid]) -> Result<(), ApiError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        self.connections
            .with_conn(|mut conn| {
                let user_ids = user_ids.to_vec();
                async move { crate::media::revoke_members(&mut conn, server_id, &user_ids).await }
            })
            .await
            .map_err(redis_err)
    }

    /// This instance's sockets of any of `user_ids`, from one table scan.
    async fn local_conns(&self, user_ids: &[Uuid]) -> Vec<ConnId> {
        let users: HashSet<Uuid> = user_ids.iter().copied().collect();
        self.connections
            .inner
            .sockets
            .read()
            .await
            .iter()
            .filter(|(_, socket)| users.contains(&socket.user_id))
            .map(|(id, _)| *id)
            .collect()
    }

    async fn clear_server_access(
        &self,
        id: ConnId,
        server_id: Uuid,
        channel_ids: &[Uuid],
        reason: &'static str,
    ) {
        // A removal on this process may have cleared the server since the
        // caller read the socket's access; it already sent the error frame
        // and cleaned presence, so do not repeat either.
        let Some(user_id) = self
            .drop_server_access(id, server_id, channel_ids, reason, true)
            .await
        else {
            return;
        };
        // The membership-revision recheck relies on this per-socket cleanup.
        if let Err(err) = self.forget_server_presence(&[user_id], server_id).await {
            warn!(error = err.code(), "server presence cleanup failed");
        }
    }

    /// Everything [`Gateway::clear_server_access`] does except presence.
    /// Returns the socket's user, or `None` when the socket is gone or, with
    /// `only_if_held`, no longer holds access to the server.
    async fn drop_server_access(
        &self,
        id: ConnId,
        server_id: Uuid,
        channel_ids: &[Uuid],
        reason: &'static str,
        only_if_held: bool,
    ) -> Option<Uuid> {
        let (user_id, rooms, typing) = {
            let mut sockets = self.connections.inner.sockets.write().await;
            let socket = sockets.get_mut(&id)?;
            if only_if_held && !socket.access.contains_key(&server_id) {
                return None;
            }
            socket.access.remove(&server_id);
            socket.servers.remove(&server_id);
            socket.dms.remove(&server_id);
            socket.scopes.retain(|_, sid| *sid != server_id);
            socket.voice_rosters.remove(&server_id);
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
            socket.send(ServerFrame::error(reason, Some(server_id), None));
            (socket.user_id, rooms, typing)
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
        Some(user_id)
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
        tx: mpsc::Sender<ServerFrame>,
    ) -> (ConnId, tokio::sync::watch::Receiver<bool>) {
        let id = self.attach(user_id, tx).await;
        let cancel = self.connections.inner.sockets.read().await[&id]
            .closed
            .clone();
        let receiver = cancel.subscribe();
        if let Some(socket) = self.connections.inner.sockets.write().await.get_mut(&id) {
            socket.session = Some((hash, cancel));
        }
        (id, receiver)
    }

    /// Close this process's sockets of a session whose row is already
    /// deleted, then tell the other API processes to check theirs.
    pub async fn revoke_session(&self, hash: &str) {
        {
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
        self.announce_recheck(Recheck::Session(hash.to_owned()))
            .await;
    }

    /// Ask every API process to check the sockets `target` may concern now
    /// instead of at their next periodic check. Call it after the change is
    /// committed: the check reads the DB, and a check that runs first sees
    /// nothing to revoke. Best effort; a lost notice leaves the periodic
    /// check, and a Pub/Sub reconnect resyncs (and so checks) every socket.
    pub async fn announce_recheck(&self, target: Recheck) {
        let raw = match serde_json::to_string(&target) {
            Ok(raw) => raw,
            Err(err) => {
                warn!(error = %err, "recheck notice not serialized");
                return;
            }
        };
        // Bounded like the logout cleanup: the change is committed already,
        // and a slow Redis must not hold the response.
        let published = tokio::time::timeout(
            Duration::from_millis(500),
            self.connections.with_conn(|mut conn| {
                let raw = raw.clone();
                async move {
                    redis::cmd("PUBLISH")
                        .arg(RECHECK_CHANNEL)
                        .arg(raw)
                        .query_async::<()>(&mut conn)
                        .await
                }
            }),
        )
        .await;
        match published {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                warn!(error = %err, "recheck notice not published; periodic checks remain")
            }
            Err(_) => warn!("recheck notice timed out; periodic checks remain"),
        }
    }

    /// Resolves when this socket should check now. `None` once detached.
    pub async fn recheck_signal(&self, id: ConnId) -> Option<Arc<tokio::sync::Notify>> {
        self.connections
            .inner
            .sockets
            .read()
            .await
            .get(&id)
            .map(|socket| socket.recheck.clone())
    }

    /// Whether this socket holds a voice seat, whose lease the security tick
    /// renews every second right after its check.
    pub async fn holds_seat(&self, id: ConnId) -> bool {
        self.connections
            .inner
            .sockets
            .read()
            .await
            .get(&id)
            .is_some_and(|socket| !socket.rooms.is_empty())
    }

    pub async fn wants_frame(&self, id: ConnId, frame: &ServerFrame) -> bool {
        let sockets = self.connections.inner.sockets.read().await;
        let Some(socket) = sockets.get(&id) else {
            return false;
        };
        match frame {
            ServerFrame::Resync | ServerFrame::Dm { .. } => true,
            ServerFrame::Event { s, c, .. } | ServerFrame::Gap { s, c, .. } => {
                socket.topics.contains(&Topic::of(*s, *c))
            }
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
        // One query for the memberships and one for the channels, not one per
        // server, topic and room: this runs on every tick, frame and batch.
        let revisions = if access.is_empty() {
            HashMap::new()
        } else {
            let sids: Vec<Uuid> = access.keys().copied().collect();
            crate::servers::membership::revisions(&mut *db, user_id, &sids).await?
        };
        let wanted: Vec<Uuid> = topics
            .iter()
            .filter_map(|topic| match topic {
                Topic::Channel(cid) => Some(*cid),
                Topic::Server(_) => None,
            })
            .chain(rooms.iter().map(|(cid, _)| *cid))
            .collect();
        // DMs are channel rows too.
        let existing: HashSet<Uuid> = if wanted.is_empty() {
            HashSet::new()
        } else {
            sqlx::query_scalar("SELECT id FROM channels WHERE id = ANY($1)")
                .bind(&wanted)
                .fetch_all(&mut *db)
                .await?
                .into_iter()
                .collect()
        };
        for (sid, revision) in access {
            let current = revisions.get(&sid).copied();
            if current.is_none_or(|now| now.0 != revision.0) {
                // Include deleted channels: their rows can no longer be queried.
                let mut cids: Vec<Uuid> =
                    sqlx::query_scalar("SELECT id FROM channels WHERE server_id = $1")
                        .bind(sid)
                        .fetch_all(&mut *db)
                        .await?;
                for topic in &topics {
                    if let Topic::Channel(cid) = topic
                        && !existing.contains(cid)
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
                // Refresh only: a removal that cleared the server after the
                // snapshot above must not get its access back from this check.
                if let Some(held) = self
                    .connections
                    .inner
                    .sockets
                    .write()
                    .await
                    .get_mut(&id)
                    .and_then(|socket| socket.access.get_mut(&sid))
                {
                    *held = current;
                }
            }
        }
        for topic in topics {
            if let Topic::Channel(cid) = topic
                && !existing.contains(&cid)
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
            if !allowed || !existing.contains(&cid) {
                self.leave_voice(id, user_id, sid, cid).await?;
            }
        }
        Ok(())
    }

    /// Cut voice for `user_ids` after a permission reduction: one rotation
    /// pipeline and one socket table scan for all of them.
    pub async fn revoke_voice_access(
        &self,
        user_ids: &[Uuid],
        server_id: Uuid,
    ) -> Result<(), ApiError> {
        self.revoke_members(server_id, user_ids).await?;
        for id in self.local_conns(user_ids).await {
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

    pub async fn attach(&self, user_id: Uuid, tx: mpsc::Sender<ServerFrame>) -> ConnId {
        self.connections.attach(user_id, tx).await
    }

    pub async fn watch_server(&self, id: ConnId, server_id: Uuid) {
        self.connections.watch_server(id, server_id).await
    }

    pub async fn watch_topic(&self, id: ConnId, server_id: Uuid, topic: Topic) {
        self.connections.watch_topic(id, server_id, topic).await
    }

    pub async fn mark_dm(&self, id: ConnId, dm_id: Uuid) {
        self.connections.mark_dm(id, dm_id).await
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
    pub async fn begin_catch_up(&self, id: ConnId, topic: Topic) {
        self.connections.begin_catch_up(id, topic).await
    }

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
        self.with_redis(op).await
    }

    /// The cached command connection for callers outside the gateway (the
    /// media session lease worker). It reconnects and retries `op` once on
    /// an I/O error, so `op` must be safe to repeat.
    pub async fn with_redis<T, F, Fut>(&self, op: F) -> Result<T, redis::RedisError>
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
    pub async fn publish_sig(&self, event: SigEvent) -> Result<(), ApiError> {
        self.voice.publish_sig(event).await
    }
}

pub(super) fn redis_err(err: redis::RedisError) -> ApiError {
    ApiError::Internal(format!("redis: {err}"))
}

/// Frames for every socket of one account, whatever it subscribed to: its
/// read positions and notification settings (`gb:u:{user}`).
const USER_PREFIX: &str = "gb:u:";

/// Revocation notices between API processes, see [`Recheck`].
const RECHECK_CHANNEL: &str = "gb:rv";

/// Which sockets a committed revocation may concern: an account (kick, ban,
/// leave), every socket bound to a server (server delete, lost voice
/// permissions), a channel (delete) or one session (logout, password
/// change). Receivers only check those sockets against the DB.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Recheck {
    #[serde(rename = "u")]
    User(Uuid),
    #[serde(rename = "s")]
    Server(Uuid),
    #[serde(rename = "c")]
    Channel(Uuid),
    #[serde(rename = "k")]
    Session(String),
}

impl Gateway {
    /// Best effort: a socket that misses one catches up with the next REST
    /// snapshot (focus, poll, reconnect).
    pub async fn publish_user(
        &self,
        user: Uuid,
        frame: &super::protocol::UserFrame,
    ) -> Result<(), ApiError> {
        self.connections.ensure_subscriber();
        let raw = serde_json::to_string(frame)
            .map_err(|err| ApiError::Internal(format!("serialize user frame: {err}")))?;
        let channel = format!("{USER_PREFIX}{user}");
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
            .map_err(redis_err)
    }
}

pub fn voice_redis_channel(channel_id: Uuid) -> String {
    format!("{REDIS_PREFIX}v:{channel_id}")
}

#[cfg(test)]
mod resilience_tests {
    use super::fanout::CATCH_UP_CAPACITY;
    use super::*;
    fn gateway() -> Gateway {
        Gateway::new(
            redis::Client::open(std::env::var("REDIS_URL").unwrap()).unwrap(),
            512,
            Duration::from_secs(10),
            Duration::from_secs(2),
        )
    }
    fn event(s: Uuid, c: Uuid, n: u64) -> Event {
        Event {
            t: super::super::protocol::EventKind::C,
            s,
            c: Some(c),
            n,
            ep: Some(Uuid::nil()),
            r: None,
            i: None,
            d: None,
        }
    }
    #[tokio::test]
    async fn slow_consumer_and_catch_up_queues_are_bounded_without_blocking_another_socket() {
        let g = gateway();
        g.wait_ready(Duration::from_secs(2)).await.unwrap();
        let (s, c) = (Uuid::new_v4(), Uuid::new_v4());
        let topic = Topic::Channel(c);
        let (tx, rx) = mpsc::channel(2);
        let (slow, closed) = g.attach_session(Uuid::new_v4(), "slow".into(), tx).await;
        let (tx, mut fast_rx) = mpsc::channel(128);
        let (fast, fast_closed) = g.attach_session(Uuid::new_v4(), "fast".into(), tx).await;
        for id in [slow, fast] {
            g.begin_catch_up(id, topic).await;
            g.finish_catch_up(id, topic, 0).await;
        }
        for n in 1..=20 {
            g.connections.deliver(topic, event(s, c, n)).await;
        }
        assert!(*closed.borrow());
        assert!(!*fast_closed.borrow());
        assert!(rx.len() <= 2);
        assert_eq!(fast_rx.len(), 20);
        for n in 1..=20 {
            assert!(matches!(fast_rx.recv().await,Some(ServerFrame::Event { n:got,.. }) if got==n));
        }
        let (tx, _rx) = mpsc::channel(128);
        let (catching, closed) = g
            .attach_session(Uuid::new_v4(), "catching".into(), tx)
            .await;
        g.begin_catch_up(catching, topic).await;
        for n in 1..=256 {
            g.connections.deliver(topic, event(s, c, n)).await;
        }
        assert!(*closed.borrow());
        assert!(g.queued_len(catching, topic).await <= CATCH_UP_CAPACITY);
        g.detach(slow).await;
        g.detach(fast).await;
        g.detach(catching).await;
    }
    /// Whether the socket still watches a server and holds its membership.
    async fn watched(g: &Gateway, id: ConnId) -> (bool, bool) {
        let sockets = g.connections.inner.sockets.read().await;
        let socket = &sockets[&id];
        (!socket.servers.is_empty(), !socket.access.is_empty())
    }

    #[tokio::test]
    async fn last_unsubscribe_of_a_server_drops_it_unless_a_seat_is_there() {
        let g = gateway();
        g.wait_ready(Duration::from_secs(2)).await.unwrap();
        let (u, s, text, other) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let (tx, _rx) = mpsc::channel(128);
        let (id, _) = g.attach_session(u, "prune".into(), tx).await;
        let g = &g;
        let bind = |id| async move {
            if let Some(socket) = g.connections.inner.sockets.write().await.get_mut(&id) {
                socket.access.insert(s, (chrono::Utc::now(), 0));
                socket.voice_rosters.insert(s, Vec::new());
            }
        };
        bind(id).await;
        for topic in [Topic::Server(s), Topic::Channel(text)] {
            g.watch_topic(id, s, topic).await;
            g.begin_catch_up(id, topic).await;
            g.finish_catch_up(id, topic, 0).await;
        }
        g.unsubscribe(id, Topic::Channel(text)).await;
        assert_eq!(
            watched(g, id).await,
            (true, true),
            "the server topic is left"
        );
        g.unsubscribe(id, Topic::Server(s)).await;
        assert_eq!(watched(g, id).await, (false, false));
        assert!(
            g.connections.inner.sockets.read().await[&id]
                .voice_rosters
                .is_empty()
        );
        // No presence or signaling for it any more.
        assert!(
            !g.wants_frame(
                id,
                &ServerFrame::presence(s, u, super::super::protocol::PresenceStatus::Online)
            )
            .await
        );

        // A seat on the server keeps it through the last unsubscribe.
        bind(id).await;
        g.watch_topic(id, s, Topic::Channel(text)).await;
        g.join_voice(id, u, s, other).await.unwrap();
        g.unsubscribe(id, Topic::Channel(text)).await;
        assert_eq!(watched(g, id).await, (true, true));
        g.detach(id).await;
    }

    #[tokio::test]
    async fn recheck_notice_wakes_only_the_sockets_it_names_on_every_process() {
        let (a, b) = (gateway(), gateway());
        a.wait_ready(Duration::from_secs(2)).await.unwrap();
        b.wait_ready(Duration::from_secs(2)).await.unwrap();
        let (u, other, s, c) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let (tx, _rx) = mpsc::channel(128);
        let (mine, _) = b.attach_session(u, "mine".into(), tx).await;
        let (tx, _rx2) = mpsc::channel(128);
        let (theirs, _) = b.attach_session(other, "theirs".into(), tx).await;
        b.watch_topic(theirs, s, Topic::Channel(c)).await;
        b.begin_catch_up(theirs, Topic::Channel(c)).await;
        let mine_signal = b.recheck_signal(mine).await.unwrap();
        let theirs_signal = b.recheck_signal(theirs).await.unwrap();
        let woke = |signal: Arc<tokio::sync::Notify>| async move {
            tokio::time::timeout(Duration::from_millis(500), signal.notified())
                .await
                .is_ok()
        };

        // Published on process `a`, delivered to the sockets of `b`.
        a.announce_recheck(Recheck::User(u)).await;
        assert!(woke(mine_signal.clone()).await);
        assert!(!woke(theirs_signal.clone()).await);
        a.announce_recheck(Recheck::Channel(c)).await;
        assert!(woke(theirs_signal.clone()).await);
        assert!(!woke(mine_signal.clone()).await);
        a.announce_recheck(Recheck::Server(s)).await;
        assert!(woke(theirs_signal.clone()).await);
        a.announce_recheck(Recheck::Session("mine".into())).await;
        assert!(woke(mine_signal.clone()).await);
        assert!(!woke(theirs_signal).await);
        b.detach(mine).await;
        b.detach(theirs).await;
    }

    #[tokio::test]
    async fn seq_epoch_and_log_snapshot_are_atomic_under_concurrent_publish() {
        let g = gateway();
        let (s, c) = (Uuid::new_v4(), Uuid::new_v4());
        let writer = g.clone();
        let task = tokio::spawn(async move {
            for _ in 0..128 {
                writer
                    .publish(EventDraft {
                        kind: super::super::protocol::EventKind::C,
                        server_id: s,
                        channel_id: Some(c),
                        entity_id: None,
                        delta: None,
                    })
                    .await
                    .unwrap();
            }
        });
        for _ in 0..128 {
            let (n, ep, plan) = g
                .events
                .catch_up_epoch(Topic::Channel(c), Some(0), None)
                .await
                .unwrap();
            match plan {
                CatchUp::None => assert_eq!(n, 0),
                CatchUp::Replay(events) => {
                    assert_eq!(events.len(), n as usize);
                    assert!(events.iter().all(|event| event.ep == Some(ep)));
                }
                CatchUp::Gap => panic!("atomic snapshot must not invent a gap"),
            }
        }
        task.await.unwrap();
    }
}
