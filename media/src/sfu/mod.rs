//! Own product authority/control; mediasoup is the sole production media engine.
//! Graph locks only reserve/commit. Native awaits use per-resource gates so a
//! denied grant cannot be resumed after its confirmed native pause.
//! A pool of engines (one worker, WebRtcServer and UDP port each) carries the
//! rooms; a lost engine closes only its own rooms while the others keep running.
use crate::native_command::{command_slots, native};
use crate::native_resource::{NativeConsumer, NativeProducer};
use crate::{
    config::Config,
    error::SfuError,
    protocol::{
        ClientFrame, MEDIA_PROTOCOL_VERSION, ServerFrame, SourceKind, TransportDirection, WatchKind,
    },
    ticket::AuthorizedTicketClaim,
};
use futures_util::{StreamExt, future::join_all, stream};
use mediasoup::{
    prelude::*,
    types::data_structures::{AppData, IceState},
    worker::WorkerLogLevel,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    sync::{
        Arc, Mutex as StdMutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Notify, OnceCell, Semaphore, mpsc};
use tracing::{error, info, warn};
use uuid::Uuid;

mod attach;
mod authority;
mod codecs;
mod rpc;
mod stats;
mod stop;
#[cfg(test)]
mod tests;

use codecs::{common_video_codecs, router_codecs, validate_parameters};
use stop::{remember_retired, stop_consumer, stop_producer};

const STOP_TIMEOUT: Duration = Duration::from_millis(500);
const STOP_CONCURRENCY: usize = 64;
const WATCH_LIMIT: usize = 64;
const CONSUMER_LIMIT: usize = 1024;
/// Browser-reported failures per producer before a viewer stops re-consuming it.
const CONSUMER_FAILURE_LIMIT: u8 = 5;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
const RESPAWN_FIRST: Duration = Duration::from_secs(1);
const RESPAWN_MAX: Duration = Duration::from_secs(60);
/// Consumers one attach pass creates at the same time.
const ATTACH_CONCURRENCY: usize = 8;
// Consumer creations of all attach passes on one worker together. Native
// commands fail fast once the worker's 256 slots are taken, so joins must leave
// room for RPCs and stops.
const ATTACH_SLOTS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerId(pub Uuid);
struct Grant(AtomicBool);
impl Grant {
    fn new() -> Arc<Self> {
        Arc::new(Self(AtomicBool::new(true)))
    }
    fn valid(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn stop(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
    fn stop_once(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }
}
struct Peer {
    id: PeerId,
    authority: AuthorizedTicketClaim,
    grant: Arc<Grant>,
    out: mpsc::Sender<ServerFrame>,
    rpc_gate: Mutex<()>,
    attach_scheduled: AtomicBool,
    attach_dirty: AtomicBool,
    retry_scheduled: AtomicBool,
    attach_failures: AtomicUsize,
}
struct PeerData {
    peer: Arc<Peer>,
    capabilities: Option<RtpCapabilities>,
    transports: HashMap<bool, WebRtcTransport>, // true = send
    publications: HashMap<SourceKind, Arc<Publication>>,
    consumers: HashMap<String, Arc<Subscription>>,
    pending_consumers: HashSet<String>,
    /// Producer ID -> `consumerFailed` reports from this viewer for that producer.
    failed_consumers: HashMap<String, u8>,
    watches: HashMap<(Uuid, WatchKind), Arc<Grant>>,
    retired_producers: VecDeque<String>,
    retired_transports: VecDeque<String>,
}
struct Publication {
    producer: NativeProducer,
    peer: Arc<Peer>,
    kind: SourceKind,
    epoch: Uuid,
    parent: Option<Arc<Publication>>,
    grant: Arc<Grant>,
    gate: Mutex<()>,
    packet_samples: StdMutex<HashMap<u32, u64>>,
}
impl Publication {
    fn id(&self) -> String {
        self.producer.id().to_string()
    }
    fn valid(&self) -> bool {
        self.grant.valid()
            && self.peer.grant.valid()
            && self.parent.as_ref().is_none_or(|p| p.valid())
    }
}
struct Subscription {
    consumer: NativeConsumer,
    publication: Arc<Publication>,
    peer: Arc<Peer>,
    watch: Option<Arc<Grant>>,
    grant: Arc<Grant>,
    generation: Uuid,
    gate: Mutex<()>,
}
impl Subscription {
    fn valid(&self) -> bool {
        self.grant.valid()
            && self.peer.grant.valid()
            && self.publication.valid()
            && self.watch.as_ref().is_none_or(|g| g.valid())
    }
}
/// A consumer an attach pass reserved and tried to create, not yet announced.
struct PreparedConsumer {
    producer_id: String,
    transport: WebRtcTransport,
    peer: Arc<Peer>,
    created: Result<Arc<Subscription>, SfuError>,
    retry_native: bool,
}
#[derive(Clone)]
enum StopResource {
    Publication(Arc<Publication>),
    Subscription(Arc<Subscription>),
}
struct RoomData {
    peers: HashMap<PeerId, PeerData>,
    publications: HashMap<String, Arc<Publication>>,
    /// The common video codecs as last announced (`ServerFrame::Codecs`),
    /// and how often they changed.
    video_codecs: Vec<String>,
    codecs_revision: u64,
}
impl Default for RoomData {
    /// What a room without receivers has: every router codec. Peers that
    /// state no capabilities change nothing, so no frame goes out for them.
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            publications: HashMap::new(),
            video_codecs: common_video_codecs(std::iter::empty()),
            codecs_revision: 0,
        }
    }
}
impl RoomData {
    /// Recomputes the common video codecs; the peers to tell and the frame
    /// when they changed. `except` gets them in its RPC result instead.
    fn update_video_codecs(
        &mut self,
        except: Option<PeerId>,
    ) -> Option<(Vec<Arc<Peer>>, ServerFrame)> {
        let next = common_video_codecs(self.peers.values().filter_map(|p| p.capabilities.as_ref()));
        if next == self.video_codecs {
            return None;
        }
        self.video_codecs = next;
        self.codecs_revision += 1;
        let peers = self
            .peers
            .iter()
            .filter(|(id, _)| Some(**id) != except)
            .map(|(_, p)| p.peer.clone())
            .collect();
        let frame = ServerFrame::Codecs {
            video: self.video_codecs.clone(),
            revision: self.codecs_revision,
        };
        Some((peers, frame))
    }
}
/// One mediasoup worker (a native thread) with its own WebRtcServer, UDP
/// port and command slots. A room stays on its engine for its lifetime.
struct Engine {
    slot: usize,
    worker: Worker,
    server: WebRtcServer,
    /// Set when the worker died or missed its heartbeat; never cleared. The
    /// slot gets a new engine instead.
    lost: Arc<AtomicBool>,
    exited: Arc<Notify>,
    /// When `engine_lost` handled this engine; it runs once per engine.
    lost_at: StdMutex<Option<Instant>>,
    commands: Arc<Semaphore>,
    attach: Semaphore,
    rooms: AtomicUsize,
    peers: AtomicUsize,
}
impl Engine {
    async fn start(
        manager: &WorkerManager,
        slot: usize,
        listen: ListenInfo,
    ) -> Result<Arc<Self>, SfuError> {
        let commands = command_slots();
        let mut settings = WorkerSettings::default();
        settings.log_level = WorkerLogLevel::Error;
        let worker = native(&commands, manager.clone(), move |manager| async move {
            manager.create_worker(settings).await
        })
        .await?;
        let lost = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(Notify::new());
        {
            let (lost, exited) = (lost.clone(), exited.clone());
            worker
                .on_dead(move |result| {
                    lost.store(true, Ordering::SeqCst);
                    error!(slot, ?result, "mediasoup worker exited");
                    exited.notify_one();
                })
                .detach();
        }
        let server = native(&commands, worker.clone(), move |worker| async move {
            worker
                .create_webrtc_server(WebRtcServerOptions::new(WebRtcServerListenInfos::new(
                    listen,
                )))
                .await
        })
        .await?;
        Ok(Arc::new(Self {
            slot,
            worker,
            server,
            lost,
            exited,
            lost_at: StdMutex::new(None),
            commands,
            attach: Semaphore::new(ATTACH_SLOTS),
            rooms: AtomicUsize::new(0),
            peers: AtomicUsize::new(0),
        }))
    }
    fn alive(&self) -> bool {
        !self.lost.load(Ordering::SeqCst) && !self.worker.closed() && !self.server.closed()
    }
    /// How long this slot has been without a live worker; zero while alive.
    fn down_for(&self) -> Duration {
        self.lost_at
            .lock()
            .unwrap()
            .map(|at| at.elapsed())
            .unwrap_or_default()
    }
    async fn native<R, T, E, F>(
        &self,
        resource: R,
        command: impl FnOnce(R) -> F + Send + 'static,
    ) -> Result<T, SfuError>
    where
        R: Send + 'static,
        T: Send + 'static,
        E: std::fmt::Display + Send + 'static,
        F: Future<Output = Result<T, E>> + Send + 'static,
    {
        native(&self.commands, resource, command).await
    }
}
/// Worker `slot` binds the configured port + slot; test port 0 lets each
/// worker pick its own port from the bounded range.
fn listen_info(bind: std::net::SocketAddr, announced: Option<String>, slot: usize) -> ListenInfo {
    let port = bind.port();
    ListenInfo {
        protocol: Protocol::Udp,
        ip: bind.ip(),
        announced_address: announced,
        expose_internal_ip: false,
        // `Config` checked that port + slot stays within u16.
        port: (port != 0).then(|| port + slot as u16),
        // Explicit bounded local test allocation; production binds fixed UDP.
        port_range: (port == 0).then_some(20000..=30000),
        flags: None,
        send_buffer_size: None,
        recv_buffer_size: None,
    }
}
struct Room {
    engine: Arc<Engine>,
    router: OnceCell<Router>,
    data: Mutex<RoomData>,
    joining: AtomicUsize,
}
impl Room {
    fn new(engine: Arc<Engine>, router: Option<Router>) -> Self {
        engine.rooms.fetch_add(1, Ordering::Relaxed);
        Self {
            engine,
            router: OnceCell::new_with(router),
            data: Mutex::new(RoomData::default()),
            joining: AtomicUsize::new(0),
        }
    }
}
impl Drop for Room {
    fn drop(&mut self) {
        self.engine.rooms.fetch_sub(1, Ordering::Relaxed);
    }
}
#[derive(Default)]
struct Counters {
    rooms: AtomicUsize,
    peers: AtomicUsize,
    producers: AtomicUsize,
    consumers: AtomicUsize,
    transports: AtomicUsize,
    rtp_sent: AtomicU64,
    stats_available: AtomicBool,
    rtp_received: AtomicU64,
    input_available: AtomicBool,
    jitter_microseconds: AtomicU64,
    ice_disconnects: AtomicU64,
    outbound_overflows: AtomicU64,
    worker_failures: AtomicU64,
    worker_restarts: AtomicU64,
}
/// Live and configured workers, for `/ready` and the metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStatus {
    pub live: usize,
    pub total: usize,
}
pub struct Sfu {
    redis: crate::redis_connection::CachedRedis,
    manager: WorkerManager,
    bind: std::net::SocketAddr,
    announced_ip: Option<String>,
    /// One engine per slot; a lost engine is replaced in its slot.
    engines: StdMutex<Vec<Arc<Engine>>>,
    rooms: Mutex<HashMap<Uuid, Arc<Room>>>,
    counters: Counters,
    draining: AtomicBool,
}
impl Sfu {
    pub async fn with_redis(config: &Config, redis: redis::Client) -> Result<Arc<Self>, SfuError> {
        let bind: std::net::SocketAddr = config.ice_bind.parse().map_err(SfuError::negotiation)?;
        let manager = WorkerManager::new();
        let mut engines = Vec::with_capacity(config.workers);
        for slot in 0..config.workers.max(1) {
            engines.push(
                Engine::start(
                    &manager,
                    slot,
                    listen_info(bind, config.advertised_ip.clone(), slot),
                )
                .await?,
            );
        }
        let sfu = Arc::new(Self {
            redis: crate::redis_connection::CachedRedis::new(redis),
            manager,
            bind,
            announced_ip: config.advertised_ip.clone(),
            engines: StdMutex::new(engines.clone()),
            rooms: Mutex::new(HashMap::new()),
            counters: Counters::default(),
            draining: AtomicBool::new(false),
        });
        Self::watch_stats(Arc::downgrade(&sfu));
        for engine in engines {
            Self::supervise(Arc::downgrade(&sfu), engine);
        }
        Ok(sfu)
    }
    /// Watches one engine until its worker exits or misses the heartbeat.
    fn supervise(weak: Weak<Self>, engine: Arc<Engine>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let reason = loop {
                tokio::select! {
                    _ = engine.exited.notified() => break "mediasoup worker unexpectedly exited",
                    _ = interval.tick() => {}
                }
                if weak.strong_count() == 0 {
                    return;
                }
                // `on_dead` sets `lost` before it notifies, so this tick can
                // see the exit first.
                if engine.lost.load(Ordering::SeqCst) {
                    break "mediasoup worker unexpectedly exited";
                }
                if !matches!(
                    tokio::time::timeout(HEARTBEAT_TIMEOUT, engine.worker.dump()).await,
                    Ok(Ok(_))
                ) {
                    break "native worker heartbeat unresponsive";
                }
            };
            if let Some(sfu) = weak.upgrade() {
                sfu.engine_lost(&engine, reason).await;
            }
        });
    }
    /// A lost worker forwards nothing more (exited) or cannot be reclaimed
    /// (hung): its rooms are closed and their peers told `gone`, so clients
    /// rejoin on a live worker. Without a live worker the process exits, so
    /// the process supervisor can restart a fresh engine.
    async fn engine_lost(self: &Arc<Self>, engine: &Arc<Engine>, reason: &'static str) {
        engine.lost.store(true, Ordering::SeqCst);
        {
            let mut lost_at = engine.lost_at.lock().unwrap();
            if lost_at.is_some() {
                return;
            }
            *lost_at = Some(Instant::now());
        }
        self.counters
            .worker_failures
            .fetch_add(1, Ordering::Relaxed);
        let live = self.workers().live;
        error!(slot = engine.slot, reason, live, "media worker lost");
        if live == 0 {
            fail_closed(reason);
        }
        self.close_engine_rooms(engine).await;
        self.respawn(engine.slot);
    }
    async fn close_engine_rooms(self: &Arc<Self>, engine: &Arc<Engine>) {
        let lost: Vec<_> = {
            let mut rooms = self.rooms.lock().await;
            let channels: Vec<_> = rooms
                .iter()
                .filter(|(_, room)| Arc::ptr_eq(&room.engine, engine))
                .map(|(channel, _)| *channel)
                .collect();
            channels
                .into_iter()
                .filter_map(|channel| rooms.remove(&channel))
                .collect()
        };
        // A later join for the same channel builds a new room on a live engine.
        self.counters.rooms.fetch_sub(lost.len(), Ordering::Relaxed);
        for room in lost {
            self.close_lost_room(&room).await;
        }
    }
    /// Removes every peer of a room whose engine is lost. Grants stop first;
    /// native handles are only dropped, because the worker cannot confirm a
    /// pause any more. A late join or RPC on this room sees the lost engine.
    async fn close_lost_room(self: &Arc<Self>, room: &Arc<Room>) {
        let (peers, producers) = {
            let mut data = room.data.lock().await;
            let producers = data.publications.len();
            data.publications.clear();
            (
                data.peers.drain().map(|(_, p)| p).collect::<Vec<_>>(),
                producers,
            )
        };
        let c = &self.counters;
        c.producers.fetch_sub(producers, Ordering::Relaxed);
        c.peers.fetch_sub(peers.len(), Ordering::Relaxed);
        room.engine.peers.fetch_sub(peers.len(), Ordering::Relaxed);
        for p in &peers {
            p.peer.grant.stop();
            for watch in p.watches.values() {
                watch.stop();
            }
            for publication in p.publications.values() {
                publication.grant.stop();
                publication.producer.resource.close();
            }
            for sub in p.consumers.values() {
                sub.grant.stop();
                sub.consumer.resource.close();
            }
            c.consumers.fetch_sub(p.consumers.len(), Ordering::Relaxed);
            c.transports
                .fetch_sub(p.transports.len(), Ordering::Relaxed);
        }
        // Clients reconnect on `gone`.
        for p in &peers {
            self.emit(&p.peer, ServerFrame::error("gone"));
        }
    }
    /// Replaces a lost engine in its slot, on the same UDP port. A hung worker
    /// still holds that port, so retries back off up to a minute.
    fn respawn(self: &Arc<Self>, slot: usize) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut delay = RESPAWN_FIRST;
            loop {
                tokio::time::sleep(delay).await;
                let Some(sfu) = weak.upgrade() else { return };
                if sfu.draining.load(Ordering::SeqCst) {
                    return;
                }
                let listen = listen_info(sfu.bind, sfu.announced_ip.clone(), slot);
                match Engine::start(&sfu.manager, slot, listen).await {
                    Ok(engine) => {
                        sfu.engines.lock().unwrap()[slot] = engine.clone();
                        sfu.counters.worker_restarts.fetch_add(1, Ordering::Relaxed);
                        info!(slot, "media worker replaced");
                        Self::supervise(weak, engine);
                        return;
                    }
                    Err(err) => {
                        let down_s = sfu.engines.lock().unwrap()[slot].down_for().as_secs();
                        // A hung worker keeps its port until the process restarts.
                        warn!(
                            slot,
                            down_s,
                            error = %err,
                            "media worker restart failed; restart the service if this persists"
                        );
                        delay = (delay * 2).min(RESPAWN_MAX);
                    }
                }
            }
        });
    }
    /// The live engine with the fewest peers, then rooms, for a new room.
    fn pick_engine(&self) -> Option<Arc<Engine>> {
        self.engines
            .lock()
            .unwrap()
            .iter()
            .filter(|engine| engine.alive())
            .min_by_key(|engine| {
                (
                    engine.peers.load(Ordering::Relaxed),
                    engine.rooms.load(Ordering::Relaxed),
                    engine.slot,
                )
            })
            .cloned()
    }
    pub fn workers(&self) -> WorkerStatus {
        let engines = self.engines.lock().unwrap();
        WorkerStatus {
            live: engines.iter().filter(|engine| engine.alive()).count(),
            total: engines.len(),
        }
    }
    pub fn ready(&self) -> bool {
        !self.draining.load(Ordering::SeqCst) && self.workers().live > 0
    }
    fn room_ready(&self, room: &Room) -> bool {
        !self.draining.load(Ordering::SeqCst) && room.engine.alive()
    }
    pub fn room_count(&self) -> usize {
        self.counters.rooms.load(Ordering::Relaxed)
    }
    /// The engine's one reconnecting Redis connection, for the join path.
    pub fn redis(&self) -> &crate::redis_connection::CachedRedis {
        &self.redis
    }
    async fn room(&self, channel: Uuid) -> Result<Arc<Room>, SfuError> {
        self.rooms
            .lock()
            .await
            .get(&channel)
            .cloned()
            .ok_or(SfuError::NotInRoom)
    }
    pub async fn join_authorized_version(
        self: &Arc<Self>,
        authority: AuthorizedTicketClaim,
        version: u8,
        out: mpsc::Sender<ServerFrame>,
    ) -> Result<PeerId, SfuError> {
        if version != MEDIA_PROTOCOL_VERSION {
            return Err(SfuError::ProtocolVersion);
        }
        if !self.ready() {
            return Err(SfuError::Unavailable);
        }
        if !crate::ticket::validate_authority(&self.redis, &authority)
            .await
            .unwrap_or(false)
        {
            return Err(SfuError::Revoked);
        }
        let channel = authority.claim.c;
        let room = {
            let mut rooms = self.rooms.lock().await;
            let room = match rooms.get(&channel) {
                Some(room) => room.clone(),
                None => {
                    let engine = self.pick_engine().ok_or(SfuError::Unavailable)?;
                    let room = Arc::new(Room::new(engine, None));
                    self.counters.rooms.fetch_add(1, Ordering::Relaxed);
                    rooms.insert(channel, room.clone());
                    room
                }
            };
            room.joining.fetch_add(1, Ordering::SeqCst);
            room
        };
        let built = room
            .router
            .get_or_try_init(|| async {
                room.engine
                    .native(room.engine.worker.clone(), |worker| async move {
                        worker
                            .create_router(RouterOptions::new(router_codecs()?))
                            .await
                            .map_err(SfuError::negotiation)
                    })
                    .await
            })
            .await;
        if let Err(err) = built {
            room.joining.fetch_sub(1, Ordering::SeqCst);
            self.remove_empty_room(channel, &room).await;
            return Err(err);
        }
        if !crate::ticket::validate_authority(&self.redis, &authority)
            .await
            .unwrap_or(false)
            || !self.room_ready(&room)
        {
            room.joining.fetch_sub(1, Ordering::SeqCst);
            self.remove_empty_room(channel, &room).await;
            return Err(SfuError::Revoked);
        }
        let id = PeerId(Uuid::new_v4());
        let peer = Arc::new(Peer {
            id,
            authority,
            grant: Grant::new(),
            out,
            rpc_gate: Mutex::new(()),
            attach_scheduled: AtomicBool::new(false),
            attach_dirty: AtomicBool::new(false),
            retry_scheduled: AtomicBool::new(false),
            attach_failures: AtomicUsize::new(0),
        });
        let mut data = room.data.lock().await;
        // A lost engine's rooms are closed under this lock after `lost` is
        // set: either that close sees this peer, or this check sees `lost`.
        if !room.engine.alive() {
            drop(data);
            room.joining.fetch_sub(1, Ordering::SeqCst);
            self.remove_empty_room(channel, &room).await;
            return Err(SfuError::Unavailable);
        }
        data.peers.insert(
            id,
            PeerData {
                peer: peer.clone(),
                capabilities: None,
                transports: HashMap::new(),
                publications: HashMap::new(),
                consumers: HashMap::new(),
                pending_consumers: HashSet::new(),
                failed_consumers: HashMap::new(),
                watches: HashMap::new(),
                retired_producers: VecDeque::new(),
                retired_transports: VecDeque::new(),
            },
        );
        drop(data);
        self.counters.peers.fetch_add(1, Ordering::Relaxed);
        room.engine.peers.fetch_add(1, Ordering::Relaxed);
        room.joining.fetch_sub(1, Ordering::SeqCst);
        Self::watch_authority(Arc::downgrade(self), channel, Arc::downgrade(&peer));
        Ok(id)
    }
    pub async fn join_data(&self, id: PeerId, channel: Uuid) -> Result<Value, SfuError> {
        let room = self.room(channel).await?;
        let data = room.data.lock().await;
        let peer = &data.peers.get(&id).ok_or(SfuError::NotInRoom)?.peer;
        Ok(
            json!({"c":channel,"u":peer.authority.claim.u,"v":MEDIA_PROTOCOL_VERSION,"generation":id.0,"routerRtpCapabilities":room.router.get().ok_or(SfuError::Unavailable)?.rtp_capabilities()}),
        )
    }
    /// Signaling may close only after leave has confirmed native resource stop
    /// and removed this peer from the graph. An invalid grant alone is earlier.
    pub async fn peer_present(&self, id: PeerId, channel: Uuid) -> bool {
        let Ok(room) = self.room(channel).await else {
            return false;
        };
        room.data.lock().await.peers.contains_key(&id)
    }
    pub async fn leave(self: &Arc<Self>, id: PeerId, channel: Uuid) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let (pubs, subs) = {
            let data = room.data.lock().await;
            let Some(p) = data.peers.get(&id) else { return };
            p.peer.grant.stop();
            for p in p.publications.values() {
                p.grant.stop()
            }
            for s in p.consumers.values() {
                s.grant.stop()
            }
            (
                p.publications.values().cloned().collect::<Vec<_>>(),
                p.consumers.values().cloned().collect::<Vec<_>>(),
            )
        };
        self.confirmed_stop_resources(&pubs, &subs).await;
        for p in &pubs {
            self.retire_publication(&room, p).await;
        }
        for sub in subs {
            self.forget_consumer(&room, &sub).await;
        }
        let transports: Vec<_> = room
            .data
            .lock()
            .await
            .peers
            .get(&id)
            .map(|p| p.transports.values().cloned().collect())
            .unwrap_or_default();
        // Sources are stopped above. Final transport samples are optional
        // and concurrent, bounded to 500ms total.
        join_all(transports.iter().map(|t| self.sample_transport(t))).await;
        let changed = {
            let mut data = room.data.lock().await;
            let removed = data.peers.remove(&id);
            if let Some(p) = &removed {
                self.counters
                    .transports
                    .fetch_sub(p.transports.len(), Ordering::Relaxed);
                self.counters.peers.fetch_sub(1, Ordering::Relaxed);
                room.engine.peers.fetch_sub(1, Ordering::Relaxed);
            }
            removed.and_then(|_| data.update_video_codecs(None))
        };
        // The one who could not decode the others' codec left: they may
        // send a better one again.
        self.announce_video_codecs(changed);
        self.remove_empty_room(channel, &room).await;
    }
    async fn remove_empty_room(&self, channel: Uuid, room: &Arc<Room>) {
        let mut rooms = self.rooms.lock().await;
        if room.joining.load(Ordering::SeqCst) == 0
            && room.data.lock().await.peers.is_empty()
            && rooms.get(&channel).is_some_and(|r| Arc::ptr_eq(r, room))
        {
            rooms.remove(&channel);
            self.counters.rooms.fetch_sub(1, Ordering::Relaxed);
        }
    }
    pub async fn shutdown(self: &Arc<Self>) {
        self.draining.store(true, Ordering::SeqCst);
        let rooms: Vec<_> = self
            .rooms
            .lock()
            .await
            .iter()
            .map(|(c, r)| (*c, r.clone()))
            .collect();
        for (channel, room) in rooms {
            let peers: Vec<_> = room
                .data
                .lock()
                .await
                .peers
                .values()
                .map(|p| p.peer.clone())
                .collect();
            for peer in peers {
                peer.grant.stop();
                self.leave(peer.id, channel).await;
                self.emit(&peer, ServerFrame::error("gone"));
            }
        }
    }
    fn emit(self: &Arc<Self>, peer: &Arc<Peer>, frame: ServerFrame) {
        let Err(error) = peer.out.try_send(frame) else {
            return;
        };
        if matches!(error, mpsc::error::TrySendError::Full(_)) {
            self.counters
                .outbound_overflows
                .fetch_add(1, Ordering::Relaxed);
        }
        if peer.grant.stop_once() {
            let weak = Arc::downgrade(self);
            let id = peer.id;
            let channel = peer.authority.claim.c;
            tokio::spawn(async move {
                if let Some(sfu) = weak.upgrade() {
                    sfu.leave(id, channel).await;
                }
            });
        }
    }
}

pub(crate) fn fail_closed(reason: &'static str) -> ! {
    error!(
        reason,
        "terminating media service to stop native forwarding"
    );
    std::process::exit(1)
}
