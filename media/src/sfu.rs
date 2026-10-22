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

const STOP_TIMEOUT: Duration = Duration::from_millis(500);
const STOP_CONCURRENCY: usize = 64;
// The deadline includes both an in-flight resource gate and native pause ACK.
const LIVE_STOP_MARGIN: Duration = Duration::from_millis(1250);
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
    watch_user: Option<Uuid>,
    grant: Arc<Grant>,
    out: mpsc::Sender<ServerFrame>,
    rpc_gate: Mutex<()>,
    lease_gate: Mutex<()>,
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
    withdrawn_live: VecDeque<Uuid>,
    retired_producers: VecDeque<String>,
    retired_transports: VecDeque<String>,
}
struct LiveBinding {
    nonce: Uuid,
    deadline: StdMutex<Instant>,
    changed: Notify,
}
struct Publication {
    producer: NativeProducer,
    peer: Arc<Peer>,
    kind: SourceKind,
    epoch: Uuid,
    parent: Option<Arc<Publication>>,
    live: Option<Arc<LiveBinding>>,
    grant: Arc<Grant>,
    gate: Mutex<()>,
    height: u16,
    layers: u8,
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
            && self
                .live
                .as_ref()
                .is_none_or(|l| Instant::now() + LIVE_STOP_MARGIN < *l.deadline.lock().unwrap())
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
        // Clients reconnect on `gone`; a Live lease they reclaim before its
        // release below is answered `live_busy` and retried.
        for p in &peers {
            self.emit(&p.peer, ServerFrame::error("gone"));
        }
        for p in &peers {
            let _lease = p.peer.lease_gate.lock().await;
            if let Some(live) = p
                .publications
                .get(&SourceKind::Live)
                .and_then(|p| p.live.as_ref())
            {
                crate::live::release(&self.redis, live.nonce, p.peer.id.0).await;
            }
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
    pub fn metrics_text(&self) -> String {
        let c = &self.counters;
        let mut text = String::new();
        for (name, value) in [
            ("gelabber_sfu_rooms", self.room_count()),
            ("gelabber_sfu_peers", c.peers.load(Ordering::Relaxed)),
            ("gelabber_media_rooms", self.room_count()),
            ("gelabber_media_peers", c.peers.load(Ordering::Relaxed)),
            (
                "gelabber_mediasoup_producers",
                c.producers.load(Ordering::Relaxed),
            ),
            (
                "gelabber_mediasoup_consumers",
                c.consumers.load(Ordering::Relaxed),
            ),
            (
                "gelabber_mediasoup_transports",
                c.transports.load(Ordering::Relaxed),
            ),
            (
                "gelabber_mediasoup_stats_available",
                usize::from(c.stats_available.load(Ordering::Relaxed)),
            ),
        ] {
            text.push_str(&format!("# TYPE {name} gauge\n{name} {value}\n"));
        }
        if c.stats_available.load(Ordering::Relaxed) || c.transports.load(Ordering::Relaxed) == 0 {
            // Actual delta sum from native RTP counters, including retired transports.
            text.push_str(&format!("# HELP gelabber_media_forwarded_bytes_total Measured native RTP byte deltas including RTP headers; failed final samples may omit bytes.\n# TYPE gelabber_media_forwarded_bytes_total counter\ngelabber_media_forwarded_bytes_total {value}\n# TYPE gelabber_sfu_forwarded_bytes_total counter\ngelabber_sfu_forwarded_bytes_total {value}\n",value=c.rtp_sent.load(Ordering::Relaxed)));
        }
        text.push_str(&format!("# TYPE gelabber_mediasoup_ice_disconnects_total counter\ngelabber_mediasoup_ice_disconnects_total {}\n",c.ice_disconnects.load(Ordering::Relaxed)));
        text.push_str(&format!("# TYPE gelabber_media_outbound_overflows_total counter\ngelabber_media_outbound_overflows_total {}\n", c.outbound_overflows.load(Ordering::Relaxed)));
        let workers = self.workers();
        text.push_str(&format!("# HELP gelabber_media_workers Configured native mediasoup workers.\n# TYPE gelabber_media_workers gauge\ngelabber_media_workers {}\n# HELP gelabber_media_workers_live Workers that accept rooms; fewer than configured while a lost worker is replaced.\n# TYPE gelabber_media_workers_live gauge\ngelabber_media_workers_live {}\n", workers.total, workers.live));
        text.push_str(&format!("# TYPE gelabber_media_worker_failures_total counter\ngelabber_media_worker_failures_total {}\n# TYPE gelabber_media_worker_restarts_total counter\ngelabber_media_worker_restarts_total {}\n", c.worker_failures.load(Ordering::Relaxed), c.worker_restarts.load(Ordering::Relaxed)));
        let engines = self.engines.lock().unwrap().clone();
        text.push_str("# TYPE gelabber_media_worker_rooms gauge\n");
        for engine in &engines {
            text.push_str(&format!(
                "gelabber_media_worker_rooms{{worker=\"{}\"}} {}\n",
                engine.slot,
                engine.rooms.load(Ordering::Relaxed)
            ));
        }
        text.push_str("# HELP gelabber_media_worker_down_seconds Time since the slot lost its worker, 0 while it has a live one; a growing value means the replacement keeps failing (a hung worker holds its port until a restart).\n# TYPE gelabber_media_worker_down_seconds gauge\n");
        for engine in &engines {
            text.push_str(&format!(
                "gelabber_media_worker_down_seconds{{worker=\"{}\"}} {}\n",
                engine.slot,
                engine.down_for().as_secs()
            ));
        }
        text.push_str("# TYPE gelabber_media_worker_peers gauge\n");
        for engine in &engines {
            text.push_str(&format!(
                "gelabber_media_worker_peers{{worker=\"{}\"}} {}\n",
                engine.slot,
                engine.peers.load(Ordering::Relaxed)
            ));
        }
        text.push_str("# HELP gelabber_media_rtp_packets_total Measured native source packet deltas; failed final samples may omit packets; absent when current input stats are unavailable.\n# TYPE gelabber_media_rtp_packets_total counter\n# HELP gelabber_media_rtp_jitter_ms Maximum current native source jitter converted from RTP clock ticks; absent when unavailable.\n# TYPE gelabber_media_rtp_jitter_ms gauge\n# HELP gelabber_media_rtp_lost_total Unavailable: native signed loss gauges do not preserve the previous reorder-aware monotonic counter.\n");
        if c.input_available.load(Ordering::Relaxed) || c.producers.load(Ordering::Relaxed) == 0 {
            text.push_str(&format!(
                "gelabber_media_rtp_packets_total {}\n",
                c.rtp_received.load(Ordering::Relaxed)
            ));
        }
        if c.input_available.load(Ordering::Relaxed) {
            text.push_str(&format!(
                "gelabber_media_rtp_jitter_ms {}\n",
                c.jitter_microseconds.load(Ordering::Relaxed) as f64 / 1000.0
            ));
        }
        text
    }
    async fn room(&self, channel: Uuid) -> Result<Arc<Room>, SfuError> {
        self.rooms
            .lock()
            .await
            .get(&channel)
            .cloned()
            .ok_or(SfuError::NotInRoom)
    }
    pub async fn join_authorized_watch_version(
        self: &Arc<Self>,
        authority: AuthorizedTicketClaim,
        watch_user: Option<Uuid>,
        version: u8,
        out: mpsc::Sender<ServerFrame>,
    ) -> Result<PeerId, SfuError> {
        if version != MEDIA_PROTOCOL_VERSION {
            return Err(SfuError::ProtocolVersion);
        }
        if watch_user.is_some_and(|u| u.is_nil()) {
            return Err(SfuError::BadAnnounce);
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
            watch_user,
            grant: Grant::new(),
            out,
            rpc_gate: Mutex::new(()),
            lease_gate: Mutex::new(()),
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
                withdrawn_live: VecDeque::new(),
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
    pub async fn rpc(
        self: &Arc<Self>,
        id: PeerId,
        channel: Uuid,
        frame: ClientFrame,
    ) -> Result<Value, SfuError> {
        let room = self.room(channel).await?;
        let peer = room
            .data
            .lock()
            .await
            .peers
            .get(&id)
            .ok_or(SfuError::NotInRoom)?
            .peer
            .clone();
        let _rpc = peer.rpc_gate.lock().await;
        if !peer.grant.valid() || !self.room_ready(&room) {
            self.leave(id, channel).await;
            return Err(SfuError::Revoked);
        }
        if !crate::ticket::validate_authority(&self.redis, &peer.authority)
            .await
            .unwrap_or(false)
        {
            self.leave(id, channel).await;
            return Err(SfuError::Revoked);
        }
        match frame {
            ClientFrame::Capabilities { rtp, .. } => {
                let caps: RtpCapabilities =
                    serde_json::from_value(rtp).map_err(|_| SfuError::BadAnnounce)?;
                if caps.codecs.len() > 64 || caps.header_extensions.len() > 32 {
                    return Err(SfuError::BadAnnounce);
                }
                let mut data = room.data.lock().await;
                let p = data.peers.get_mut(&id).ok_or(SfuError::Revoked)?;
                if p.capabilities.is_some() {
                    return Err(SfuError::BadAnnounce);
                }
                p.capabilities = Some(caps);
                let changed = data.update_video_codecs(Some(id));
                let codecs =
                    json!({ "video": data.video_codecs, "revision": data.codecs_revision });
                drop(data);
                self.announce_video_codecs(changed);
                self.schedule_attach(channel, id).await;
                Ok(json!({ "codecs": codecs }))
            }
            ClientFrame::CreateTransport { direction, .. } => {
                let send = direction == TransportDirection::Send;
                if send && peer.watch_user.is_some() {
                    return Err(SfuError::Forbidden);
                }
                if room
                    .data
                    .lock()
                    .await
                    .peers
                    .get(&id)
                    .ok_or(SfuError::Revoked)?
                    .transports
                    .contains_key(&send)
                {
                    return Err(SfuError::BadAnnounce);
                }
                let mut opts = WebRtcTransportOptions::new_with_server(room.engine.server.clone());
                opts.enable_tcp = false;
                opts.prefer_udp = true;
                // Last sample belongs to the native transport lifetime, keeping
                // historical totals without retaining an unbounded ID cache.
                opts.app_data = AppData::new(AtomicU64::new(0));
                let transport = room
                    .engine
                    .native(
                        room.router.get().ok_or(SfuError::Unavailable)?.clone(),
                        move |router| async move { router.create_webrtc_transport(opts).await },
                    )
                    .await?;
                let weak = Arc::downgrade(self);
                transport
                    .on_ice_state_change(move |state| {
                        if state == IceState::Disconnected
                            && let Some(sfu) = weak.upgrade()
                        {
                            sfu.counters.ice_disconnects.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                    .detach();
                let result = json!({"id":transport.id(),"iceParameters":transport.ice_parameters(),"iceCandidates":transport.ice_candidates(),"dtlsParameters":transport.dtls_parameters()});
                let mut data = room.data.lock().await;
                let p = data
                    .peers
                    .get_mut(&id)
                    .filter(|p| p.peer.grant.valid())
                    .ok_or(SfuError::Revoked)?;
                p.transports.insert(send, transport);
                self.counters.transports.fetch_add(1, Ordering::Relaxed);
                drop(data);
                if !send {
                    self.schedule_attach(channel, id).await
                }
                Ok(result)
            }
            ClientFrame::ConnectTransport {
                transport_id, dtls, ..
            } => {
                let t = self.transport(&room, id, &transport_id).await?;
                let dtls_parameters = serde_json::from_value::<DtlsParameters>(dtls)
                    .map_err(|_| SfuError::BadAnnounce)?;
                room.engine
                    .native(t, move |t| async move {
                        t.connect(WebRtcTransportRemoteParameters { dtls_parameters })
                            .await
                    })
                    .await?;
                if !peer.grant.valid() {
                    return Err(SfuError::Revoked);
                }
                Ok(json!({}))
            }
            ClientFrame::RestartIce { transport_id, .. } => {
                let t = self.transport(&room, id, &transport_id).await?;
                let ice = room
                    .engine
                    .native(t, |t| async move { t.restart_ice().await })
                    .await?;
                if !peer.grant.valid() {
                    return Err(SfuError::Revoked);
                }
                Ok(json!({"iceParameters":ice}))
            }
            ClientFrame::CloseTransport { transport_id, .. } => {
                self.close_transport(&room, &peer, &transport_id).await?;
                Ok(json!({}))
            }
            ClientFrame::Produce {
                k,
                rtp,
                epoch,
                parent,
                lc,
                expected_old_producer_id,
                height,
                paused,
                ..
            } => {
                self.produce(
                    &room,
                    &peer,
                    k,
                    rtp,
                    epoch,
                    parent,
                    lc,
                    expected_old_producer_id,
                    height,
                    paused,
                )
                .await
            }
            ClientFrame::PauseProducer { producer_id, .. } => {
                let p = self.own_publication(&room, id, &producer_id).await?;
                let _gate = p.gate.lock().await;
                stop_producer(&p.producer).await;
                Ok(json!({}))
            }
            ClientFrame::ResumeProducer { producer_id, .. } => {
                let p = self.own_publication(&room, id, &producer_id).await?;
                let _gate = p.gate.lock().await;
                if !p.valid() {
                    return Err(SfuError::Forbidden);
                }
                if let Err(error) = room
                    .engine
                    .native(p.clone(), |resource| async move {
                        resource.producer.resume().await
                    })
                    .await
                {
                    p.producer.confirm_closed().await;
                    return Err(error);
                }
                if !p.valid() {
                    stop_producer(&p.producer).await;
                    return Err(SfuError::Forbidden);
                }
                Ok(json!({}))
            }
            ClientFrame::CloseProducer { producer_id, .. } => {
                if room
                    .data
                    .lock()
                    .await
                    .peers
                    .get(&id)
                    .is_some_and(|p| p.retired_producers.contains(&producer_id))
                {
                    return Ok(json!({}));
                }
                let p = self.own_publication(&room, id, &producer_id).await?;
                self.remove_publication(&room, &p, true).await;
                Ok(json!({}))
            }
            ClientFrame::ConsumerReady {
                consumer_id,
                generation,
                ..
            } => {
                let s = self
                    .own_consumer(&room, id, &consumer_id, generation)
                    .await?;
                let _gate = s.gate.lock().await;
                if !s.valid() {
                    return Err(SfuError::Forbidden);
                }
                if let Err(error) = room
                    .engine
                    .native(s.clone(), |resource| async move {
                        resource.consumer.resume().await
                    })
                    .await
                {
                    s.consumer.confirm_closed().await;
                    drop(_gate);
                    self.remove_consumer(&room, &s).await;
                    self.retry_attach(channel, peer.clone());
                    return Err(error);
                }
                if !s.valid() {
                    stop_consumer(&s.consumer).await;
                    return Err(SfuError::Forbidden);
                }
                peer.attach_failures.store(0, Ordering::Relaxed);
                self.emit(
                    &peer,
                    ServerFrame::ConsumerState {
                        consumer_id,
                        generation,
                        paused: s.consumer.paused() || s.consumer.producer_paused(),
                    },
                );
                Ok(json!({}))
            }
            ClientFrame::ConsumerFailed {
                consumer_id,
                generation,
                ..
            } => {
                let s = self
                    .own_consumer(&room, id, &consumer_id, generation)
                    .await?;
                self.remove_consumer(&room, &s).await;
                let retry = {
                    let mut data = room.data.lock().await;
                    let RoomData {
                        peers,
                        publications,
                        ..
                    } = &mut *data;
                    peers.get_mut(&id).is_some_and(|p| {
                        note_consumer_failure(
                            &mut p.failed_consumers,
                            |producer| publications.contains_key(producer),
                            s.publication.id(),
                        )
                    })
                };
                // A producer this browser cannot decode would otherwise be
                // re-consumed forever; a new producer ID starts a fresh budget.
                if retry {
                    self.retry_attach(channel, peer.clone());
                }
                Ok(json!({}))
            }
            ClientFrame::Watch { u, k, on, .. } => {
                self.set_watch(channel, &room, &peer, u, k, on).await?;
                Ok(json!({}))
            }
            ClientFrame::ViewerLayer {
                consumer_id,
                generation,
                h,
                congested,
                ..
            } => {
                let s = self
                    .own_consumer(&room, id, &consumer_id, generation)
                    .await?;
                let _gate = s.gate.lock().await;
                if !s.valid() || !s.publication.kind.is_video() {
                    return Err(SfuError::Forbidden);
                }
                if s.publication.layers > 1 {
                    room.engine
                        .native(s.clone(), move |s| async move {
                            s.consumer
                                .set_preferred_layers(ConsumerLayers {
                                    spatial_layer: preferred_layer(
                                        h,
                                        congested,
                                        s.publication.height,
                                        s.publication.layers,
                                    ),
                                    temporal_layer: None,
                                })
                                .await
                        })
                        .await?
                }
                Ok(json!({}))
            }
            ClientFrame::Leave { .. } => {
                self.leave(id, channel).await;
                Ok(json!({}))
            }
            ClientFrame::Join { .. } => Err(SfuError::BadAnnounce),
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn produce(
        self: &Arc<Self>,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        kind: SourceKind,
        rtp: Value,
        epoch: Uuid,
        parent_id: Option<String>,
        nonce: Option<Uuid>,
        expected_old: Option<String>,
        height: u16,
        paused: bool,
    ) -> Result<Value, SfuError> {
        if peer.watch_user.is_some() || epoch.is_nil() {
            return Err(SfuError::Forbidden);
        }
        if matches!(kind, SourceKind::Live | SourceKind::LiveAudio) && !peer.authority.claim.g {
            return Err(SfuError::Forbidden);
        }
        let mut parameters: RtpParameters =
            serde_json::from_value(rtp).map_err(|_| SfuError::BadAnnounce)?;
        // The pinned public Chrome handler returns an empty CNAME for RID-only
        // offers. Treat that absent identity as None so the official Rust SDK
        // chooses its documented transport CNAME. Some("") would poison that
        // SDK's transport-wide cache before the native worker rejects it.
        if parameters.rtcp.cname.as_deref() == Some("") {
            parameters.rtcp.cname = None;
        }
        validate_parameters(kind, &parameters)?;
        let _lease = if kind == SourceKind::Live {
            Some(peer.lease_gate.lock().await)
        } else {
            None
        };
        let (transport, old, parent) = {
            let data = room.data.lock().await;
            let p = data.peers.get(&peer.id).ok_or(SfuError::Revoked)?;
            let old = p.publications.get(&kind).cloned();
            if old.as_ref().map(|p| p.id()) != expected_old {
                return Err(SfuError::Forbidden);
            }
            let parent = if let Some(pk) = kind.parent() {
                let parent = p
                    .publications
                    .get(&pk)
                    .filter(|p| {
                        p.valid() && p.epoch == epoch && Some(p.id()).as_ref() == parent_id.as_ref()
                    })
                    .ok_or(SfuError::Forbidden)?
                    .clone();
                if kind == SourceKind::LiveAudio && parent.live.as_ref().map(|l| l.nonce) != nonce {
                    return Err(SfuError::Forbidden);
                }
                Some(parent)
            } else {
                if parent_id.is_some() {
                    return Err(SfuError::BadAnnounce);
                }
                None
            };
            if !matches!(kind, SourceKind::Live | SourceKind::LiveAudio) && nonce.is_some() {
                return Err(SfuError::BadAnnounce);
            }
            if nonce.is_some_and(|n| p.withdrawn_live.contains(&n)) {
                return Err(SfuError::Forbidden);
            }
            (
                p.transports
                    .get(&true)
                    .cloned()
                    .ok_or(SfuError::BadAnnounce)?,
                old,
                parent,
            )
        };
        let live = if kind == SourceKind::Live {
            let nonce = nonce.ok_or(SfuError::Forbidden)?;
            let checked = Instant::now();
            let ttl = match crate::live::try_acquire(&self.redis, &peer.authority, nonce, peer.id.0)
                .await
                .unwrap_or(crate::live::LiveAcquireOutcome::Denied)
            {
                crate::live::LiveAcquireOutcome::Acquired(ttl) => ttl,
                crate::live::LiveAcquireOutcome::Busy => return Err(SfuError::LiveBusy),
                crate::live::LiveAcquireOutcome::Denied => return Err(SfuError::Forbidden),
            };
            let deadline = checked + ttl;
            if Instant::now() + LIVE_STOP_MARGIN >= deadline {
                self.release_unpublished_live(room, peer, nonce).await;
                return Err(SfuError::LiveBusy);
            }
            Some(Arc::new(LiveBinding {
                nonce,
                deadline: StdMutex::new(deadline),
                changed: Notify::new(),
            }))
        } else {
            parent.as_ref().and_then(|p| p.live.clone())
        };
        let result = async {
            let layers = parameters.encodings.len().max(1) as u8;
            let mut options = ProducerOptions::new(
                if kind.is_video() {
                    MediaKind::Video
                } else {
                    MediaKind::Audio
                },
                parameters,
            );
            options.paused = true;
            let producer = room
                .engine
                .native(transport.clone(), move |transport| async move {
                    transport.produce(options).await
                })
                .await?;
            let pubn = Arc::new(Publication {
                producer: NativeProducer::new(
                    producer,
                    transport.clone(),
                    room.engine.lost.clone(),
                ),
                peer: peer.clone(),
                kind,
                epoch,
                parent,
                live,
                grant: Grant::new(),
                gate: Mutex::new(()),
                height,
                layers,
                packet_samples: StdMutex::new(HashMap::new()),
            });
            if !crate::ticket::validate_authority(&self.redis, &peer.authority)
                .await
                .unwrap_or(false)
                || !pubn.valid()
            {
                stop_producer(&pubn.producer).await;
                return Err(SfuError::Revoked);
            }
            // Atomically replace the slot with a still-paused new Producer. This actual
            // ID fences timeout/close callbacks from the retired Producer, even if the
            // browser kept the same capture epoch for a codec-only replacement.
            {
                let mut data = room.data.lock().await;
                let p = data
                    .peers
                    .get_mut(&peer.id)
                    .filter(|p| p.peer.grant.valid())
                    .ok_or(SfuError::Revoked)?;
                if p.publications.get(&kind).map(|p| p.id()) != expected_old
                    || old.as_ref().is_some_and(|o| !o.valid())
                    || !pubn.valid()
                {
                    return Err(SfuError::Forbidden);
                }
                if let Some(old) = &old {
                    old.grant.stop()
                }
                p.publications.insert(kind, pubn.clone());
            }
            if let Some(old) = &old {
                self.retire_publication(room, old).await;
            }
            let committed = {
                let mut data = room.data.lock().await;
                if !pubn.valid()
                    || !data
                        .peers
                        .get(&peer.id)
                        .and_then(|p| p.publications.get(&kind))
                        .is_some_and(|p| Arc::ptr_eq(p, &pubn))
                {
                    false
                } else {
                    data.publications.insert(pubn.id(), pubn.clone());
                    self.counters.producers.fetch_add(1, Ordering::Relaxed);
                    true
                }
            };
            if !committed {
                stop_producer(&pubn.producer).await;
                return Err(SfuError::Revoked);
            }
            if let Some(old) = old.as_ref().filter(|o| o.kind == SourceKind::Live)
                && let Some(live) = &old.live
                && pubn.live.as_ref().is_none_or(|l| l.nonce != live.nonce)
            {
                crate::live::release(&self.redis, live.nonce, peer.id.0).await;
            }
            if !paused {
                let _gate = pubn.gate.lock().await;
                if !pubn.valid() {
                    return Err(SfuError::Forbidden);
                }
                if let Err(error) = room
                    .engine
                    .native(pubn.clone(), |resource| async move {
                        resource.producer.resume().await
                    })
                    .await
                {
                    pubn.producer.confirm_closed().await;
                    return Err(error);
                }
                if !pubn.valid() {
                    stop_producer(&pubn.producer).await;
                    return Err(SfuError::Forbidden);
                }
            }
            self.observe_producer(room, &pubn);
            if kind == SourceKind::Live {
                Self::watch_live(
                    Arc::downgrade(self),
                    Arc::downgrade(room),
                    Arc::downgrade(&pubn),
                );
            }
            self.schedule_room_attach(peer.authority.claim.c).await;
            Ok(json!({"producerId":pubn.producer.id(),"epoch":epoch}))
        }
        .await;
        if result.is_err() {
            let failed = room
                .data
                .lock()
                .await
                .peers
                .get(&peer.id)
                .and_then(|p| p.publications.get(&kind))
                .filter(|p| Some(p.id()) != expected_old)
                .cloned();
            if let Some(failed) = failed {
                self.retire_publication(room, &failed).await;
            }
            if kind == SourceKind::Live
                && let Some(nonce) = nonce
            {
                // The lease gate is already held by this Produce operation.
                self.release_unpublished_live(room, peer, nonce).await;
            }
        }
        result
    }
    async fn release_unpublished_live(
        self: &Arc<Self>,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        nonce: Uuid,
    ) {
        let current = room
            .data
            .lock()
            .await
            .peers
            .get(&peer.id)
            .and_then(|p| p.publications.get(&SourceKind::Live))
            .filter(|p| p.live.as_ref().is_some_and(|l| l.nonce == nonce))
            .cloned();
        if let Some(current) = current {
            if current.valid() {
                return;
            }
            // A grant may have expired while native creation was in flight.
            // Confirm the old source is stopped before releasing its lease.
            self.retire_publication(room, &current).await;
        }
        crate::live::release(&self.redis, nonce, peer.id.0).await;
    }
    async fn transport(
        &self,
        room: &Room,
        peer: PeerId,
        id: &str,
    ) -> Result<WebRtcTransport, SfuError> {
        room.data
            .lock()
            .await
            .peers
            .get(&peer)
            .ok_or(SfuError::Revoked)?
            .transports
            .values()
            .find(|t| t.id().to_string() == id)
            .cloned()
            .ok_or(SfuError::Forbidden)
    }
    async fn close_transport(
        self: &Arc<Self>,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        id: &str,
    ) -> Result<(), SfuError> {
        let (direction, transport, publications, consumers) = {
            let data = room.data.lock().await;
            let p = data.peers.get(&peer.id).ok_or(SfuError::Revoked)?;
            if p.retired_transports.iter().any(|owned| owned == id) {
                return Ok(());
            }
            let (direction, transport) = p
                .transports
                .iter()
                .find(|(_, t)| t.id().to_string() == id)
                .ok_or(SfuError::Forbidden)?;
            let publications = p
                .publications
                .values()
                .filter(|p| p.producer.transport().id().to_string() == id)
                .cloned()
                .collect::<Vec<_>>();
            let consumers = p
                .consumers
                .values()
                .filter(|s| s.consumer.transport().id().to_string() == id)
                .cloned()
                .collect::<Vec<_>>();
            (*direction, transport.clone(), publications, consumers)
        };
        self.confirmed_stop_resources(&publications, &consumers)
            .await;
        for publication in publications {
            // Compaction retires the native resource, not the API's Live claim.
            self.remove_publication(room, &publication, false).await;
        }
        for consumer in consumers {
            self.forget_consumer(room, &consumer).await;
        }
        self.sample_transport(&transport).await;
        let mut data = room.data.lock().await;
        if let Some(p) = data.peers.get_mut(&peer.id)
            && p.transports
                .get(&direction)
                .is_some_and(|t| t.id() == transport.id())
        {
            p.transports.remove(&direction);
            remember_retired(&mut p.retired_transports, id.to_owned());
            self.counters.transports.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }
    async fn own_publication(
        &self,
        room: &Room,
        peer: PeerId,
        id: &str,
    ) -> Result<Arc<Publication>, SfuError> {
        room.data
            .lock()
            .await
            .publications
            .get(id)
            .filter(|p| p.peer.id == peer && p.grant.valid())
            .cloned()
            .ok_or(SfuError::Forbidden)
    }
    async fn own_consumer(
        &self,
        room: &Room,
        peer: PeerId,
        id: &str,
        generation: Uuid,
    ) -> Result<Arc<Subscription>, SfuError> {
        room.data
            .lock()
            .await
            .peers
            .get(&peer)
            .and_then(|p| p.consumers.get(id))
            .filter(|s| s.generation == generation)
            .cloned()
            .ok_or(SfuError::Forbidden)
    }
    async fn schedule_attach(self: &Arc<Self>, channel: Uuid, id: PeerId) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let peer = room
            .data
            .lock()
            .await
            .peers
            .get(&id)
            .map(|p| p.peer.clone());
        if let Some(peer) = peer {
            self.schedule_peer_attach(channel, peer);
        }
    }
    fn retry_attach(self: &Arc<Self>, channel: Uuid, peer: Arc<Peer>) {
        if !peer.grant.valid() || peer.retry_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let failures = peer.attach_failures.fetch_add(1, Ordering::Relaxed).min(5);
        let delay = Duration::from_millis(250 * (1 << failures));
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            peer.retry_scheduled.store(false, Ordering::SeqCst);
            if let Some(sfu) = weak.upgrade()
                && peer.grant.valid()
            {
                sfu.schedule_peer_attach(channel, peer);
            }
        });
    }
    fn schedule_peer_attach(self: &Arc<Self>, channel: Uuid, peer: Arc<Peer>) {
        if !peer.grant.valid() {
            return;
        }
        peer.attach_dirty.store(true, Ordering::SeqCst);
        if peer.attach_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                peer.attach_dirty.store(false, Ordering::SeqCst);
                if let Some(sfu) = weak.upgrade()
                    && peer.grant.valid()
                {
                    sfu.attach_existing(channel, peer.id).await;
                }
                peer.attach_scheduled.store(false, Ordering::SeqCst);
                if !peer.grant.valid()
                    || !peer.attach_dirty.load(Ordering::SeqCst)
                    || peer
                        .attach_scheduled
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                {
                    break;
                }
            }
        });
    }
    async fn schedule_room_attach(self: &Arc<Self>, channel: Uuid) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let peers: Vec<_> = room
            .data
            .lock()
            .await
            .peers
            .values()
            .map(|p| p.peer.clone())
            .collect();
        for peer in peers {
            self.schedule_peer_attach(channel, peer);
        }
    }
    async fn attach_existing(self: &Arc<Self>, channel: Uuid, id: PeerId) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let pubs: Vec<_> = room
            .data
            .lock()
            .await
            .publications
            .values()
            .cloned()
            .collect();
        let engine = room.engine.clone();
        let results = self
            .attach_batch(&room, id, pubs, |transport, options| {
                let engine = engine.clone();
                async move {
                    engine
                        .native(transport, move |transport| async move {
                            transport.consume(options).await
                        })
                        .await
                }
            })
            .await;
        for err in results.into_iter().filter_map(Result::err) {
            if !matches!(err, SfuError::Forbidden | SfuError::Unavailable) {
                warn!(?id, code = err.code(), "consumer attach failed");
            }
        }
    }
    /// One attach pass: creates paused consumers for `pubs`, a few at a time,
    /// then checks the viewer's authority once and announces them. The check
    /// follows every creation of the pass, so it is as late for each consumer
    /// as a check per consumer would be, without a Redis round trip each.
    async fn attach_batch<C, F>(
        self: &Arc<Self>,
        room: &Arc<Room>,
        id: PeerId,
        pubs: Vec<Arc<Publication>>,
        create: C,
    ) -> Vec<Result<(), SfuError>>
    where
        C: Fn(WebRtcTransport, ConsumerOptions) -> F + Sync,
        F: Future<Output = Result<Consumer, SfuError>> + Send,
    {
        let prepared: Vec<_> = stream::iter(pubs)
            .map(|pubn| self.prepare_consumer(room, id, pubn, &create))
            .buffer_unordered(ATTACH_CONCURRENCY)
            .collect()
            .await;
        let mut results = Vec::with_capacity(prepared.len());
        let mut reserved = Vec::new();
        for item in prepared {
            match item {
                Ok(Some(p)) => reserved.push(p),
                Ok(None) => results.push(Ok(())),
                Err(error) => results.push(Err(error)),
            }
        }
        let Some(peer) = reserved.first().map(|p| p.peer.clone()) else {
            return results;
        };
        let check = if reserved.iter().any(|p| p.created.is_ok()) {
            crate::ticket::validate_authority(&self.redis, &peer.authority).await
        } else {
            Ok(true)
        };
        // A Redis failure denies the pass like a revocation but is retried,
        // since it fails every consumer of the join at once.
        let redis_failed = check.is_err();
        let authorized = check.unwrap_or(false);
        for p in &mut reserved {
            if let Ok(sub) = &p.created
                && (!authorized || !sub.valid())
            {
                stop_consumer(&sub.consumer).await;
                p.created = Err(if redis_failed {
                    SfuError::Unavailable
                } else {
                    SfuError::Revoked
                });
            }
        }
        let (mut recreated, mut retry_native) = (false, redis_failed);
        for p in reserved {
            let result = match p.created {
                Ok(sub) => self.commit_consumer(room, id, &p.transport, sub).await,
                Err(error) => Err(error),
            };
            recreated |= {
                let mut data = room.data.lock().await;
                data.peers.get_mut(&id).is_some_and(|d| {
                    d.pending_consumers.remove(&p.producer_id);
                    d.transports
                        .get(&false)
                        .is_some_and(|t| t.id() != p.transport.id())
                })
            };
            retry_native |= p.retry_native;
            results.push(result);
        }
        if recreated {
            self.schedule_attach(peer.authority.claim.c, id).await;
        } else if retry_native {
            // Reservations are cleared before retry, and the next attempt rechecks
            // grants, capabilities, capacity and current receive transport.
            self.retry_attach(peer.authority.claim.c, peer);
        }
        results
    }
    /// Reserves `pubn` for the viewer and creates its paused native consumer;
    /// announces nothing. `Ok(None)`: already consumed, pending or given up.
    async fn prepare_consumer<C, F>(
        &self,
        room: &Room,
        id: PeerId,
        pubn: Arc<Publication>,
        create: &C,
    ) -> Result<Option<PreparedConsumer>, SfuError>
    where
        C: Fn(WebRtcTransport, ConsumerOptions) -> F + Sync,
        F: Future<Output = Result<Consumer, SfuError>> + Send,
    {
        let producer_id = pubn.id();
        let (peer, transport, capabilities, watch) = {
            let mut data = room.data.lock().await;
            let p = data.peers.get_mut(&id).ok_or(SfuError::Revoked)?;
            let watch = receives(p, &pubn)?;
            if p.pending_consumers.contains(&producer_id)
                || p.consumers
                    .values()
                    .any(|s| s.publication.id() == producer_id)
                || p.failed_consumers
                    .get(&producer_id)
                    .is_some_and(|failures| *failures >= CONSUMER_FAILURE_LIMIT)
            {
                return Ok(None);
            }
            if p.consumers.len() + p.pending_consumers.len() >= CONSUMER_LIMIT {
                return Err(SfuError::Unavailable);
            }
            let transport = p
                .transports
                .get(&false)
                .cloned()
                .ok_or(SfuError::Unavailable)?;
            let caps = p.capabilities.clone().ok_or(SfuError::Unavailable)?;
            p.pending_consumers.insert(producer_id.clone());
            (p.peer.clone(), transport, caps, watch)
        };
        let mut retry_native = false;
        let created = async {
            if !room
                .router
                .get()
                .ok_or(SfuError::Unavailable)?
                .can_consume(&pubn.producer.id(), &capabilities)
            {
                return Err(SfuError::UnsupportedCodec);
            }
            let mut options = ConsumerOptions::new(pubn.producer.id(), capabilities);
            options.paused = true;
            if pubn.kind.is_video() && pubn.layers > 1 {
                options.preferred_layers = Some(ConsumerLayers {
                    spatial_layer: 0,
                    temporal_layer: None,
                });
            }
            let consumer = {
                let _slot = room
                    .engine
                    .attach
                    .acquire()
                    .await
                    .map_err(|_| SfuError::Unavailable)?;
                match create(transport.clone(), options).await {
                    Ok(consumer) => consumer,
                    Err(error) => {
                        retry_native =
                            matches!(error, SfuError::Unavailable | SfuError::Negotiation(_));
                        return Err(error);
                    }
                }
            };
            Ok(Arc::new(Subscription {
                consumer: NativeConsumer::new(
                    consumer,
                    transport.clone(),
                    room.engine.lost.clone(),
                ),
                publication: pubn,
                peer: peer.clone(),
                watch,
                grant: Grant::new(),
                generation: Uuid::new_v4(),
                gate: Mutex::new(()),
            }))
        }
        .await;
        Ok(Some(PreparedConsumer {
            producer_id,
            transport,
            peer,
            created,
            retry_native,
        }))
    }
    /// Installs and announces an authorized consumer unless its grants or the
    /// viewer's receive transport changed meanwhile.
    async fn commit_consumer(
        self: &Arc<Self>,
        room: &Arc<Room>,
        id: PeerId,
        transport: &WebRtcTransport,
        sub: Arc<Subscription>,
    ) -> Result<(), SfuError> {
        {
            let mut data = room.data.lock().await;
            let p = data.peers.get_mut(&id).ok_or(SfuError::Revoked)?;
            if !sub.valid()
                || !p
                    .transports
                    .get(&false)
                    .is_some_and(|t| t.id() == transport.id())
            {
                return Err(SfuError::Forbidden);
            }
            p.consumers
                .insert(sub.consumer.id().to_string(), sub.clone());
            self.counters.consumers.fetch_add(1, Ordering::Relaxed);
        }
        self.observe_consumer(room, &sub);
        self.emit(
            &sub.peer,
            ServerFrame::Consumer {
                consumer_id: sub.consumer.id().to_string(),
                producer_id: sub.publication.id(),
                owner: sub.publication.peer.authority.claim.u,
                k: sub.publication.kind,
                epoch: sub.publication.epoch,
                generation: sub.generation,
                parent: sub.publication.parent.as_ref().map(|p| p.id()),
                kind: if sub.publication.kind.is_video() {
                    "video"
                } else {
                    "audio"
                }
                .into(),
                rtp_parameters: serde_json::to_value(sub.consumer.rtp_parameters())
                    .map_err(SfuError::negotiation)?,
                paused: sub.publication.producer.paused(),
            },
        );
        Ok(())
    }
    async fn set_watch(
        self: &Arc<Self>,
        channel: Uuid,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        user: Uuid,
        kind: WatchKind,
        on: bool,
    ) -> Result<(), SfuError> {
        if user.is_nil() || user == peer.authority.claim.u || peer.watch_user.is_some() {
            return Err(SfuError::Forbidden);
        }
        let stopped = {
            let mut data = room.data.lock().await;
            let p = data.peers.get_mut(&peer.id).ok_or(SfuError::Revoked)?;
            let key = (user, kind);
            if on {
                if p.watches.len() >= WATCH_LIMIT && !p.watches.contains_key(&key) {
                    return Err(SfuError::BadAnnounce);
                }
                p.watches.entry(key).or_insert_with(Grant::new);
                Vec::new()
            } else {
                if let Some(g) = p.watches.remove(&key) {
                    g.stop()
                }
                p.consumers
                    .values()
                    .filter(|s| !s.valid())
                    .cloned()
                    .collect::<Vec<_>>()
            }
        };
        self.confirmed_stop_resources(&[], &stopped).await;
        for sub in stopped {
            self.forget_consumer(room, &sub).await;
        }
        if on {
            self.schedule_attach(channel, peer.id).await
        }
        Ok(())
    }
    async fn confirmed_stop_resources(
        &self,
        publications: &[Arc<Publication>],
        subscriptions: &[Arc<Subscription>],
    ) {
        // Fence every resource before starting any native operation. The single
        // deadline includes queued work, gate acquisition and every pause ACK.
        for publication in publications {
            publication.grant.stop();
        }
        for subscription in subscriptions {
            subscription.grant.stop();
        }
        let resources: Vec<_> = publications
            .iter()
            .cloned()
            .map(StopResource::Publication)
            .chain(
                subscriptions
                    .iter()
                    .cloned()
                    .map(StopResource::Subscription),
            )
            .collect();
        let jobs = resources.clone().into_iter().map(|resource| async move {
            match resource {
                StopResource::Publication(p) => {
                    let _gate = stop_gate(&p.gate).await;
                    if _gate.is_some() {
                        stop_producer(&p.producer).await;
                    } else {
                        p.producer.confirm_closed().await;
                    }
                }
                StopResource::Subscription(s) => {
                    let _gate = stop_gate(&s.gate).await;
                    if _gate.is_some() {
                        stop_consumer(&s.consumer).await;
                    } else {
                        s.consumer.confirm_closed().await;
                    }
                }
            }
        });
        if stop_batch(jobs).await.is_err() {
            // Close only this batch, including resources still waiting for a
            // concurrency slot. No unrelated room loses its worker/transport.
            for resource in &resources {
                match resource {
                    StopResource::Publication(p) => p.producer.resource.close(),
                    StopResource::Subscription(s) => s.consumer.resource.close(),
                }
            }
            stream::iter(resources)
                .for_each_concurrent(Some(STOP_CONCURRENCY), |resource| async move {
                    match resource {
                        StopResource::Publication(p) => p.producer.confirm_closed().await,
                        StopResource::Subscription(s) => s.consumer.confirm_closed().await,
                    }
                })
                .await;
        }
    }
    async fn remove_consumer(self: &Arc<Self>, room: &Room, sub: &Arc<Subscription>) {
        self.confirmed_stop_resources(&[], std::slice::from_ref(sub))
            .await;
        self.forget_consumer(room, sub).await;
    }
    async fn forget_consumer(self: &Arc<Self>, room: &Room, sub: &Arc<Subscription>) {
        let mut data = room.data.lock().await;
        if let Some(p) = data.peers.get_mut(&sub.peer.id)
            && p.consumers
                .get(&sub.consumer.id().to_string())
                .is_some_and(|s| Arc::ptr_eq(s, sub))
        {
            p.consumers.remove(&sub.consumer.id().to_string());
            self.counters.consumers.fetch_sub(1, Ordering::Relaxed);
            self.emit(
                &sub.peer,
                ServerFrame::ConsumerClosed {
                    consumer_id: sub.consumer.id().to_string(),
                    generation: sub.generation,
                },
            );
        }
    }
    async fn retire_publication(self: &Arc<Self>, room: &Arc<Room>, pubn: &Arc<Publication>) {
        let (pubs, subs) = {
            let data = room.data.lock().await;
            let mut pubs = vec![pubn.clone()];
            pubs.extend(
                data.publications
                    .values()
                    .filter(|p| {
                        !Arc::ptr_eq(p, pubn)
                            && p.parent
                                .as_ref()
                                .is_some_and(|parent| Arc::ptr_eq(parent, pubn))
                    })
                    .cloned(),
            );
            for p in &pubs {
                p.grant.stop()
            }
            let subs: Vec<_> = data
                .peers
                .values()
                .flat_map(|p| p.consumers.values())
                .filter(|s| !s.valid())
                .cloned()
                .collect();
            for s in &subs {
                s.grant.stop()
            }
            (pubs, subs)
        };
        self.confirmed_stop_resources(&pubs, &subs).await;
        // Registry removal, UI events and retired-ID ACKs follow the confirmed
        // stop of both owned sources and every affected room subscription.
        let registered = {
            let mut data = room.data.lock().await;
            let mut registered = 0;
            for p in &pubs {
                if data.publications.remove(&p.id()).is_some() {
                    registered += 1;
                }
                if let Some(peer) = data.peers.get_mut(&p.peer.id)
                    && peer
                        .publications
                        .get(&p.kind)
                        .is_some_and(|current| Arc::ptr_eq(current, p))
                {
                    peer.publications.remove(&p.kind);
                }
            }
            registered
        };
        // An idempotent close ACK for an old ID is only legal after its native
        // pause has been confirmed, never merely after graph invalidation.
        // Optional telemetry follows stop; its pre-lease-release budget is 10ms.
        join_all(
            pubs.iter()
                .map(|p| self.sample_publication(p, Duration::from_millis(10))),
        )
        .await;
        {
            let mut data = room.data.lock().await;
            for p in &pubs {
                if let Some(peer) = data.peers.get_mut(&p.peer.id) {
                    remember_retired(&mut peer.retired_producers, p.id());
                }
            }
        }
        for sub in subs {
            self.forget_consumer(room, &sub).await;
        }
        self.counters
            .producers
            .fetch_sub(registered, Ordering::Relaxed);
        for p in pubs {
            self.emit(
                &p.peer,
                ServerFrame::ProducerClosed {
                    producer_id: p.id(),
                    epoch: p.epoch,
                },
            );
        }
    }
    async fn remove_publication(
        self: &Arc<Self>,
        room: &Arc<Room>,
        pubn: &Arc<Publication>,
        withdraw: bool,
    ) {
        // Pause before acquiring the lease gate: a native creation holding that
        // gate must never delay the existing producer beyond its lease deadline.
        self.retire_publication(room, pubn).await;
        if pubn.kind == SourceKind::Live
            && let Some(live) = &pubn.live
        {
            let _lease = pubn.peer.lease_gate.lock().await;
            let release = {
                let mut data = room.data.lock().await;
                match data.peers.get_mut(&pubn.peer.id) {
                    Some(p) => {
                        let same = p
                            .publications
                            .get(&SourceKind::Live)
                            .and_then(|p| p.live.as_ref())
                            .is_some_and(|l| l.nonce == live.nonce);
                        if withdraw && !same && !p.withdrawn_live.contains(&live.nonce) {
                            if p.withdrawn_live.len() == 64 {
                                p.withdrawn_live.pop_front();
                            }
                            p.withdrawn_live.push_back(live.nonce);
                        }
                        !same
                    }
                    None => true,
                }
            };
            if release {
                crate::live::release(&self.redis, live.nonce, pubn.peer.id.0).await;
                if withdraw {
                    self.emit(&pubn.peer, ServerFrame::live_withdrawn(live.nonce));
                }
            }
        }
    }
    pub async fn leave(self: &Arc<Self>, id: PeerId, channel: Uuid) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let (peer, pubs, subs) = {
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
                p.peer.clone(),
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
        let _lease = peer.lease_gate.lock().await;
        for p in &pubs {
            if p.kind == SourceKind::Live
                && let Some(live) = &p.live
            {
                crate::live::release(&self.redis, live.nonce, id.0).await;
            }
        }
        let transports: Vec<_> = room
            .data
            .lock()
            .await
            .peers
            .get(&id)
            .map(|p| p.transports.values().cloned().collect())
            .unwrap_or_default();
        // Sources are stopped and Live leases released above. Final transport
        // samples are optional and concurrent, bounded to 500ms total.
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
    fn announce_video_codecs(self: &Arc<Self>, changed: Option<(Vec<Arc<Peer>>, ServerFrame)>) {
        let Some((peers, frame)) = changed else {
            return;
        };
        for peer in peers {
            self.emit(&peer, frame.clone());
        }
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
    fn observe_producer(self: &Arc<Self>, room: &Arc<Room>, pubn: &Arc<Publication>) {
        let weak = Arc::downgrade(self);
        let room = Arc::downgrade(room);
        let publication = Arc::downgrade(pubn);
        let runtime = tokio::runtime::Handle::current();
        pubn.producer.resource.with(|producer| {
            producer
                .on_close(move || {
                    if let Some(p) = publication.upgrade() {
                        p.grant.stop()
                    }
                    runtime.spawn(async move {
                        if let (Some(sfu), Some(room), Some(p)) =
                            (weak.upgrade(), room.upgrade(), publication.upgrade())
                        {
                            sfu.remove_publication(&room, &p, true).await;
                        }
                    });
                })
                .detach();
        });
    }
    fn observe_consumer(self: &Arc<Self>, room: &Arc<Room>, sub: &Arc<Subscription>) {
        for paused in [true, false] {
            let weak = Arc::downgrade(self);
            let subweak = Arc::downgrade(sub);
            let runtime = tokio::runtime::Handle::current();
            let callback = move || {
                let weak = weak.clone();
                let subweak = subweak.clone();
                runtime.spawn(async move {
                    if let (Some(sfu), Some(s)) = (weak.upgrade(), subweak.upgrade())
                        && s.grant.valid()
                    {
                        sfu.emit(
                            &s.peer,
                            ServerFrame::ConsumerState {
                                consumer_id: s.consumer.id().to_string(),
                                generation: s.generation,
                                paused: s.consumer.paused() || s.consumer.producer_paused(),
                            },
                        );
                    }
                });
            };
            if paused {
                sub.consumer
                    .resource
                    .with(|c| c.on_producer_pause(callback).detach());
            } else {
                sub.consumer
                    .resource
                    .with(|c| c.on_producer_resume(callback).detach());
            }
        }
        let weak = Arc::downgrade(self);
        let subweak = Arc::downgrade(sub);
        let runtime = tokio::runtime::Handle::current();
        sub.consumer.resource.with(|consumer| {
            consumer
                .on_layers_change(move |layers| {
                    let weak = weak.clone();
                    let subweak = subweak.clone();
                    let layers = *layers;
                    runtime.spawn(async move {
                        if let (Some(sfu), Some(s)) = (weak.upgrade(), subweak.upgrade())
                            && s.valid()
                        {
                            sfu.emit(
                                &s.peer,
                                ServerFrame::Layers {
                                    consumer_id: s.consumer.id().to_string(),
                                    generation: s.generation,
                                    spatial_layer: layers.map(|l| l.spatial_layer),
                                    temporal_layer: layers.and_then(|l| l.temporal_layer),
                                },
                            );
                        }
                    });
                })
                .detach();
        });
        let weak = Arc::downgrade(self);
        let subweak = Arc::downgrade(sub);
        let room = Arc::downgrade(room);
        let runtime = tokio::runtime::Handle::current();
        sub.consumer.resource.with(|consumer| {
            consumer
                .on_close(move || {
                    if let Some(s) = subweak.upgrade() {
                        s.grant.stop()
                    }
                    runtime.spawn(async move {
                        if let (Some(sfu), Some(room), Some(s)) =
                            (weak.upgrade(), room.upgrade(), subweak.upgrade())
                        {
                            sfu.remove_consumer(&room, &s).await;
                        }
                    });
                })
                .detach();
        });
    }
    fn watch_authority(weak: Weak<Self>, channel: Uuid, peer: Weak<Peer>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let (Some(sfu), Some(peer)) = (weak.upgrade(), peer.upgrade()) else {
                    break;
                };
                if !peer.grant.valid() {
                    break;
                }
                if !sfu.ready()
                    || !crate::ticket::validate_authority(&sfu.redis, &peer.authority)
                        .await
                        .unwrap_or(false)
                {
                    sfu.leave(peer.id, channel).await;
                    sfu.emit(&peer, ServerFrame::error("unauthorized"));
                    break;
                }
                let Ok(room) = sfu.room(channel).await else {
                    break;
                };
                let pubn = room
                    .data
                    .lock()
                    .await
                    .peers
                    .get(&peer.id)
                    .and_then(|p| p.publications.get(&SourceKind::Live))
                    .cloned();
                if let Some(pubn) = pubn {
                    let _lease = peer.lease_gate.lock().await;
                    if !pubn.grant.valid() {
                        continue;
                    }
                    let Some(live) = &pubn.live else { continue };
                    let checked = Instant::now();
                    if let Some(ttl) = crate::live::validate_and_acquire(
                        &sfu.redis,
                        &peer.authority,
                        live.nonce,
                        peer.id.0,
                    )
                    .await
                    .unwrap_or(None)
                        && pubn.grant.valid()
                        && checked + ttl > Instant::now() + LIVE_STOP_MARGIN
                    {
                        *live.deadline.lock().unwrap() = checked + ttl;
                        live.changed.notify_one();
                        continue;
                    }
                    drop(_lease);
                    sfu.remove_publication(&room, &pubn, true).await;
                }
            }
        });
    }
    fn watch_live(weak: Weak<Self>, room: Weak<Room>, pubn: Weak<Publication>) {
        tokio::spawn(async move {
            loop {
                let Some(p) = pubn.upgrade() else { break };
                if !p.grant.valid() {
                    break;
                }
                let Some(live) = p.live.clone() else { break };
                let deadline = *live.deadline.lock().unwrap();
                let stop_at = deadline.checked_sub(LIVE_STOP_MARGIN).unwrap_or(deadline);
                drop(p);
                tokio::select! {_=tokio::time::sleep_until(stop_at.into())=>{},_=live.changed.notified()=>continue}
                let (Some(sfu), Some(room), Some(p)) =
                    (weak.upgrade(), room.upgrade(), pubn.upgrade())
                else {
                    break;
                };
                if Instant::now() + LIVE_STOP_MARGIN < *live.deadline.lock().unwrap() {
                    continue;
                }
                sfu.remove_publication(&room, &p, true).await;
                break;
            }
        });
    }
    fn watch_stats(weak: Weak<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut cursor = 0usize;
            loop {
                interval.tick().await;
                let Some(sfu) = weak.upgrade() else { break };
                let rooms: Vec<_> = sfu.rooms.lock().await.values().cloned().collect();
                let mut transports = Vec::new();
                let mut publications = Vec::new();
                for room in rooms {
                    let data = room.data.lock().await;
                    transports.extend(
                        data.peers
                            .values()
                            .flat_map(|p| p.transports.values())
                            .cloned(),
                    );
                    publications.extend(data.publications.values().cloned());
                }
                // At most 32 transport + 32 producer samples per five seconds.
                // Rotate stable native IDs so large rooms do not starve later IDs.
                transports.sort_by_key(|t| t.id().to_string());
                publications.sort_by_key(|p| p.id());
                if !transports.is_empty() {
                    let n = transports.len();
                    transports.rotate_left(cursor % n);
                }
                if !publications.is_empty() {
                    let n = publications.len();
                    publications.rotate_left(cursor % n);
                }
                cursor = cursor.wrapping_add(32);
                let mut ok = false;
                for transport in transports.into_iter().take(32) {
                    ok |= sfu.sample_transport(&transport).await;
                }
                sfu.counters.stats_available.store(ok, Ordering::Relaxed);
                let mut input_available = false;
                let mut jitter_microseconds = 0;
                for p in publications.into_iter().take(32) {
                    if let Some(jitter) = sfu.sample_publication(&p, STOP_TIMEOUT).await {
                        input_available = true;
                        jitter_microseconds = jitter_microseconds.max(jitter);
                    }
                }
                sfu.counters
                    .jitter_microseconds
                    .store(jitter_microseconds, Ordering::Relaxed);
                sfu.counters
                    .input_available
                    .store(input_available, Ordering::Relaxed);
            }
        });
    }
    async fn sample_publication(&self, p: &Publication, budget: Duration) -> Option<u64> {
        let Ok(Ok(stats)) = tokio::time::timeout(budget, p.producer.get_stats()).await else {
            return None;
        };
        if stats.is_empty() {
            return None;
        }
        let mut jitter_microseconds = 0;
        for stat in stats {
            let mut samples = p.packet_samples.lock().unwrap();
            let old = samples.entry(stat.ssrc).or_default();
            self.counters
                .rtp_received
                .fetch_add(stat.packet_count.saturating_sub(*old), Ordering::Relaxed);
            *old = (*old).max(stat.packet_count);
            let rate = if stat.kind == MediaKind::Audio {
                48000
            } else {
                90000
            };
            jitter_microseconds =
                jitter_microseconds.max(u64::from(stat.jitter) * 1_000_000 / rate);
        }
        Some(jitter_microseconds)
    }
    async fn sample_transport(&self, transport: &WebRtcTransport) -> bool {
        let Ok(Ok(stats)) = tokio::time::timeout(STOP_TIMEOUT, transport.get_stats()).await else {
            return false;
        };
        let available = !stats.is_empty();
        for stat in stats {
            let sample = transport
                .app_data()
                .downcast_ref::<AtomicU64>()
                .expect("gateway transport measurement");
            let old = sample.fetch_max(stat.rtp_bytes_sent, Ordering::SeqCst);
            self.counters
                .rtp_sent
                .fetch_add(stat.rtp_bytes_sent.saturating_sub(old), Ordering::Relaxed);
        }
        available
    }
}

/// Counts a browser consumer failure; returns whether another attempt is allowed.
/// Entries for producers that left the room are dropped first.
fn note_consumer_failure(
    failed: &mut HashMap<String, u8>,
    live: impl Fn(&str) -> bool,
    producer: String,
) -> bool {
    failed.retain(|id, _| live(id));
    let failures = failed.entry(producer).or_default();
    *failures = failures.saturating_add(1);
    *failures < CONSUMER_FAILURE_LIMIT
}

fn remember_retired(ids: &mut VecDeque<String>, id: String) {
    if ids.contains(&id) {
        return;
    }
    if ids.len() == 256 {
        ids.pop_front();
    }
    ids.push_back(id);
}

fn receives(peer: &PeerData, pubn: &Publication) -> Result<Option<Arc<Grant>>, SfuError> {
    let owner = pubn.peer.authority.claim.u;
    if !peer.peer.grant.valid() || !pubn.valid() || owner == peer.peer.authority.claim.u {
        return Err(SfuError::Forbidden);
    }
    match pubn.kind {
        SourceKind::Mic => Ok(None),
        SourceKind::Camera if peer.peer.watch_user.is_none() => Ok(None),
        SourceKind::Live | SourceKind::LiveAudio if peer.peer.watch_user == Some(owner) => Ok(None),
        SourceKind::Screen | SourceKind::ScreenAudio if peer.peer.watch_user.is_none() => peer
            .watches
            .get(&(owner, WatchKind::Screen))
            .filter(|g| g.valid())
            .cloned()
            .map(Some)
            .ok_or(SfuError::Forbidden),
        SourceKind::Live | SourceKind::LiveAudio if peer.peer.watch_user.is_none() => peer
            .watches
            .get(&(owner, WatchKind::Live))
            .filter(|g| g.valid())
            .cloned()
            .map(Some)
            .ok_or(SfuError::Forbidden),
        _ => Err(SfuError::Forbidden),
    }
}
fn validate_parameters(kind: SourceKind, rtp: &RtpParameters) -> Result<(), SfuError> {
    if rtp.codecs.is_empty()
        || rtp.codecs.len() > 16
        || rtp.encodings.len() > 2
        || rtp.header_extensions.len() > 32
    {
        return Err(SfuError::BadAnnounce);
    }
    let mime = rtp.codecs[0].mime_type().as_str().to_ascii_lowercase();
    if kind.is_video() {
        if !matches!(
            mime.as_str(),
            "video/vp8" | "video/vp9" | "video/h264" | "video/av1"
        ) {
            return Err(SfuError::UnsupportedCodec);
        }
    } else if mime != "audio/opus" || rtp.encodings.len() > 1 {
        return Err(SfuError::UnsupportedCodec);
    }
    Ok(())
}
/// The router's video codecs, in its order, that every one of `receivers`
/// can decode; all of them without receivers. H264 counts in the form
/// publishers send it: packetization mode 1, Constrained Baseline (profile
/// 0x42 with constraint_set1), the profile every WebRTC H264 decoder has
/// and the one mediasoup matches a 42e01f producer against.
fn common_video_codecs<'a>(receivers: impl Iterator<Item = &'a RtpCapabilities>) -> Vec<String> {
    const ROUTER: [&str; 4] = ["video/vp8", "video/vp9", "video/h264", "video/av1"];
    let mut common: Vec<&str> = ROUTER.to_vec();
    for caps in receivers {
        let decodes: Vec<String> = caps
            .codecs
            .iter()
            .filter_map(|codec| serde_json::to_value(codec).ok())
            .filter(|codec| {
                let mime = codec["mimeType"].as_str().unwrap_or_default();
                if !mime.eq_ignore_ascii_case("video/h264") {
                    return true;
                }
                let parameters = &codec["parameters"];
                let mode = parameters["packetization-mode"].as_u64().unwrap_or(0);
                let profile = parameters["profile-level-id"].as_str().unwrap_or("42001f");
                let constrained = profile
                    .get(2..4)
                    .and_then(|iop| u8::from_str_radix(iop, 16).ok())
                    .is_some_and(|iop| iop & 0x40 != 0);
                mode == 1
                    && constrained
                    && profile
                        .get(..2)
                        .is_some_and(|p| p.eq_ignore_ascii_case("42"))
            })
            .filter_map(|codec| codec["mimeType"].as_str().map(str::to_ascii_lowercase))
            .collect();
        common.retain(|mime| decodes.iter().any(|d| d == mime));
    }
    common.into_iter().map(str::to_owned).collect()
}
fn preferred_layer(h: u16, congested: bool, height: u16, layers: u8) -> u8 {
    if layers <= 1 || congested || h == 0 || h <= height.saturating_div(4).max(1) {
        0
    } else {
        layers - 1
    }
}
fn router_codecs() -> Result<Vec<RtpCodecCapability>, SfuError> {
    let feedback = json!([{"type":"nack"},{"type":"nack","parameter":"pli"},{"type":"ccm","parameter":"fir"},{"type":"goog-remb"},{"type":"transport-cc"}]);
    let mut codecs = vec![
        json!({"kind":"audio","mimeType":"audio/opus","clockRate":48000,"channels":2,"rtcpFeedback":[{"type":"nack"},{"type":"transport-cc"}]}),
        json!({"kind":"video","mimeType":"video/VP8","clockRate":90000,"rtcpFeedback":feedback}),
    ];
    for profile in [0, 1] {
        codecs.push(json!({"kind":"video","mimeType":"video/VP9","clockRate":90000,"parameters":{"profile-id":profile},"rtcpFeedback":feedback}));
    }
    for (profile, mode) in [
        ("42e01f", 1),
        ("42001f", 1),
        ("42e01f", 0),
        ("42001f", 0),
        ("640032", 1),
    ] {
        codecs.push(json!({"kind":"video","mimeType":"video/H264","clockRate":90000,"parameters":{"profile-level-id":profile,"packetization-mode":mode,"level-asymmetry-allowed":1},"rtcpFeedback":feedback}));
    }
    codecs.push(json!({"kind":"video","mimeType":"video/AV1","clockRate":90000,"parameters":{"profile":0},"rtcpFeedback":feedback}));
    serde_json::from_value(Value::Array(codecs)).map_err(SfuError::negotiation)
}
async fn stop_gate(gate: &Mutex<()>) -> Option<tokio::sync::MutexGuard<'_, ()>> {
    tokio::time::timeout(STOP_TIMEOUT, gate.lock()).await.ok()
}
async fn stop_batch<F: Future<Output = ()>>(
    jobs: impl IntoIterator<Item = F>,
) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(
        STOP_TIMEOUT,
        stream::iter(jobs).for_each_concurrent(Some(STOP_CONCURRENCY), |job| async move {
            // Keep the deadline observable even when many already-stopped
            // resources can finish without a native await.
            tokio::task::yield_now().await;
            job.await;
        }),
    )
    .await
}
async fn stop_producer(p: &NativeProducer) {
    if p.worker_lost() || p.closed() || p.paused() {
        return;
    }
    match tokio::time::timeout(STOP_TIMEOUT, p.pause()).await {
        Ok(Ok(())) if p.paused() => {}
        _ => p.confirm_closed().await,
    }
}
async fn stop_consumer(c: &NativeConsumer) {
    if c.worker_lost() || c.closed() || c.paused() {
        return;
    }
    match tokio::time::timeout(STOP_TIMEOUT, c.pause()).await {
        Ok(Ok(())) if c.paused() => {}
        _ => c.confirm_closed().await,
    }
}
pub(crate) fn fail_closed(reason: &'static str) -> ! {
    error!(
        reason,
        "terminating media service to stop native forwarding"
    );
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gelabber_shared::ticket::{TicketAuthorization, TicketClaim};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn first_engine(sfu: &Sfu) -> Arc<Engine> {
        sfu.engines.lock().unwrap()[0].clone()
    }

    fn receiver(codecs: Value) -> RtpCapabilities {
        serde_json::from_value(json!({ "codecs": codecs, "headerExtensions": [] })).unwrap()
    }

    #[test]
    fn video_codecs_are_those_every_receiver_decodes() {
        let h264 = |profile: &str, mode: u8| {
            json!({"kind":"video","mimeType":"video/H264","clockRate":90000,"preferredPayloadType":102,
                "parameters":{"profile-level-id":profile,"packetization-mode":mode}})
        };
        let vp8 = json!({"kind":"video","mimeType":"video/VP8","clockRate":90000,"preferredPayloadType":96});
        let av1 = json!({"kind":"video","mimeType":"video/AV1","clockRate":90000,"preferredPayloadType":45});
        let opus = json!({"kind":"audio","mimeType":"audio/opus","clockRate":48000,"channels":2,"preferredPayloadType":111});
        let none: [RtpCapabilities; 0] = [];
        assert_eq!(
            common_video_codecs(none.iter()),
            ["video/vp8", "video/vp9", "video/h264", "video/av1"]
        );
        let chrome = receiver(json!([opus, vp8, h264("42e01f", 1), av1]));
        let desktop = receiver(json!([opus, vp8, h264("42e01f", 1)]));
        assert_eq!(
            common_video_codecs([&chrome].into_iter()),
            ["video/vp8", "video/h264", "video/av1"]
        );
        assert_eq!(
            common_video_codecs([&chrome, &desktop].into_iter()),
            ["video/vp8", "video/h264"]
        );
        // Firefox without its H264 plugin; decoders of plain Baseline, High
        // or mode 0 only.
        let firefox = receiver(json!([opus, vp8, av1]));
        let high_only = receiver(json!([
            opus,
            vp8,
            h264("42001f", 1),
            h264("640032", 1),
            h264("42e01f", 0)
        ]));
        assert_eq!(
            common_video_codecs([&chrome, &firefox].into_iter()),
            ["video/vp8", "video/av1"]
        );
        assert_eq!(common_video_codecs([&high_only].into_iter()), ["video/vp8"]);
        // A room starts with the set of no receivers: peers that state no
        // capabilities (old clients, tests) are never sent a codecs frame.
        assert!(RoomData::default().update_video_codecs(None).is_none());
    }

    #[test]
    fn consumer_failures_stop_retrying_per_producer() {
        let mut failed = HashMap::new();
        let live = |id: &str| id != "gone";
        failed.insert("gone".to_owned(), 3);
        for _ in 1..CONSUMER_FAILURE_LIMIT {
            assert!(note_consumer_failure(&mut failed, live, "a".to_owned()));
        }
        assert!(!note_consumer_failure(&mut failed, live, "a".to_owned()));
        assert!(!failed.contains_key("gone"));
        // Another producer, e.g. the same source republished, has its own budget.
        assert!(note_consumer_failure(&mut failed, live, "b".to_owned()));
        assert!(!note_consumer_failure(&mut failed, live, "a".to_owned()));
        assert_eq!(failed["a"], CONSUMER_FAILURE_LIMIT + 1);
    }

    #[tokio::test]
    async fn stop_batch_deadline_includes_queued_unpaused_resources() {
        let started = Arc::new(AtomicUsize::new(0));
        let paused = Arc::new(AtomicUsize::new(0));
        let jobs = (0..=STOP_CONCURRENCY).map(|index| {
            let started = started.clone();
            let paused = paused.clone();
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                if index < STOP_CONCURRENCY {
                    std::future::pending::<()>().await;
                }
                paused.fetch_add(1, Ordering::SeqCst);
            }
        });
        assert!(stop_batch(jobs).await.is_err());
        assert_eq!(started.load(Ordering::SeqCst), STOP_CONCURRENCY);
        assert_eq!(paused.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stop_batch_waits_for_concurrent_pause_acknowledgements() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let paused = Arc::new(AtomicUsize::new(0));
        let jobs = (0..2).map(|_| {
            let barrier = barrier.clone();
            let paused = paused.clone();
            async move {
                barrier.wait().await;
                paused.fetch_add(1, Ordering::SeqCst);
            }
        });
        assert!(stop_batch(jobs).await.is_ok());
        assert_eq!(paused.load(Ordering::SeqCst), 2);
    }

    async fn stopping_fixture() -> (
        Arc<Sfu>,
        Arc<Peer>,
        Arc<Publication>,
        mpsc::Receiver<ServerFrame>,
    ) {
        let config = Config::from_source(|key| match key {
            "REDIS_URL" => Some(std::env::var(key).expect("isolated REDIS_URL required")),
            "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
            _ => None,
        })
        .unwrap();
        let sfu = Sfu::with_redis(
            &config,
            redis::Client::open(config.redis_url.as_str()).unwrap(),
        )
        .await
        .unwrap();
        let engine = first_engine(&sfu);
        let router = engine
            .native(engine.worker.clone(), |worker| async move {
                worker
                    .create_router(RouterOptions::new(router_codecs().unwrap()))
                    .await
            })
            .await
            .unwrap();
        let mut options = WebRtcTransportOptions::new_with_server(engine.server.clone());
        options.app_data = AppData::new(AtomicU64::new(0));
        let transport = engine
            .native(router.clone(), move |router| async move {
                router.create_webrtc_transport(options).await
            })
            .await
            .unwrap();
        let rtp = serde_json::from_value(json!({
            "codecs":[{"mimeType":"audio/opus","payloadType":111,"clockRate":48000,
                "channels":2,"parameters":{},"rtcpFeedback":[]}],
            "headerExtensions":[],"encodings":[{"ssrc":1234}],
            "rtcp":{"cname":"stop-order-test","reducedSize":true}
        }))
        .unwrap();
        let producer = engine
            .native(transport.clone(), move |transport| async move {
                transport
                    .produce(ProducerOptions::new(MediaKind::Audio, rtp))
                    .await
            })
            .await
            .unwrap();
        assert!(!producer.paused());
        let (out, rx) = mpsc::channel(16);
        // Deliberately absent authority keys trigger the real denial path.
        // The native resources are installed directly to hold their gate before
        // starting revocation; this fixture exercises stop ordering, not join.
        let channel = Uuid::new_v4();
        let peer = Arc::new(Peer {
            id: PeerId(Uuid::new_v4()),
            authority: AuthorizedTicketClaim {
                claim: TicketClaim {
                    u: Uuid::new_v4(),
                    s: Uuid::new_v4(),
                    c: channel,
                    g: false,
                },
                auth: TicketAuthorization {
                    session: Uuid::new_v4().simple().to_string().repeat(2),
                    expires_at: 9_007_199_254_740_991,
                    member: Uuid::new_v4(),
                    channel: Uuid::new_v4(),
                },
            },
            watch_user: None,
            grant: Grant::new(),
            out,
            rpc_gate: Mutex::new(()),
            lease_gate: Mutex::new(()),
            attach_scheduled: AtomicBool::new(false),
            attach_dirty: AtomicBool::new(false),
            retry_scheduled: AtomicBool::new(false),
            attach_failures: AtomicUsize::new(0),
        });
        let publication = Arc::new(Publication {
            producer: NativeProducer::new(producer, transport.clone(), engine.lost.clone()),
            peer: peer.clone(),
            kind: SourceKind::Mic,
            epoch: Uuid::new_v4(),
            parent: None,
            live: None,
            grant: Grant::new(),
            gate: Mutex::new(()),
            height: 0,
            layers: 1,
            packet_samples: StdMutex::new(HashMap::new()),
        });
        let data = RoomData {
            peers: HashMap::from([(
                peer.id,
                PeerData {
                    peer: peer.clone(),
                    capabilities: None,
                    transports: HashMap::from([(true, transport)]),
                    publications: HashMap::from([(SourceKind::Mic, publication.clone())]),
                    consumers: HashMap::new(),
                    pending_consumers: HashSet::new(),
                    failed_consumers: HashMap::new(),
                    watches: HashMap::new(),
                    withdrawn_live: VecDeque::new(),
                    retired_producers: VecDeque::new(),
                    retired_transports: VecDeque::new(),
                },
            )]),
            publications: HashMap::from([(publication.id(), publication.clone())]),
            ..RoomData::default()
        };
        let mut room = Room::new(engine.clone(), Some(router));
        *room.data.get_mut() = data;
        sfu.rooms.lock().await.insert(channel, Arc::new(room));
        engine.peers.store(1, Ordering::Relaxed);
        for counter in [
            &sfu.counters.rooms,
            &sfu.counters.peers,
            &sfu.counters.producers,
            &sfu.counters.transports,
        ] {
            counter.store(1, Ordering::Relaxed);
        }
        (sfu, peer, publication, rx)
    }

    /// A second member joined to the fixture's room with its authority keys in
    /// Redis, a receive transport and the router's capabilities, ready to consume.
    struct Viewer {
        id: PeerId,
        events: mpsc::Receiver<ServerFrame>,
        transport: WebRtcTransport,
        keys: Vec<String>,
    }
    impl Viewer {
        async fn join(sfu: &Arc<Sfu>, owner: &Peer, room: &Room, capacity: usize) -> Self {
            use crate::redis_connection::ConnectionSource;
            use gelabber_shared::ticket;
            let channel = owner.authority.claim.c;
            let mut authority = owner.authority.clone();
            authority.claim.u = Uuid::new_v4();
            authority.auth.member = Uuid::new_v4();
            authority.auth.session = Uuid::new_v4().simple().to_string().repeat(2);
            let keys = [
                (
                    ticket::member_authority_key(authority.claim.s, authority.claim.u),
                    authority.auth.member.to_string(),
                ),
                (
                    ticket::channel_authority_key(channel),
                    authority.auth.channel.to_string(),
                ),
                (
                    ticket::session_authority_key(&authority.auth.session),
                    authority.claim.u.to_string(),
                ),
            ];
            let mut redis = sfu.redis.connection().await.unwrap();
            for (key, value) in &keys {
                let _: () = redis::cmd("SET")
                    .arg(key)
                    .arg(value)
                    .arg("EX")
                    .arg(30)
                    .query_async(&mut redis)
                    .await
                    .unwrap();
            }
            let (out, events) = mpsc::channel(capacity);
            let id = sfu
                .join_authorized_watch_version(authority, None, MEDIA_PROTOCOL_VERSION, out)
                .await
                .unwrap();
            let router = room.router.get().unwrap();
            let mut options = WebRtcTransportOptions::new_with_server(room.engine.server.clone());
            options.app_data = AppData::new(AtomicU64::new(0));
            let transport = room
                .engine
                .native(router.clone(), move |router| async move {
                    router.create_webrtc_transport(options).await
                })
                .await
                .unwrap();
            {
                let mut data = room.data.lock().await;
                let peer = data.peers.get_mut(&id).unwrap();
                peer.transports.insert(false, transport.clone());
                peer.capabilities = Some(
                    serde_json::from_value(
                        serde_json::to_value(router.rtp_capabilities()).unwrap(),
                    )
                    .unwrap(),
                );
            }
            sfu.counters.transports.fetch_add(1, Ordering::Relaxed);
            Self {
                id,
                events,
                transport,
                keys: keys.into_iter().map(|(key, _)| key).collect(),
            }
        }
        async fn revoke(&self, sfu: &Sfu) {
            use crate::redis_connection::ConnectionSource;
            let mut redis = sfu.redis.connection().await.unwrap();
            for key in &self.keys {
                let _: () = redis::cmd("DEL")
                    .arg(key)
                    .query_async(&mut redis)
                    .await
                    .unwrap();
            }
        }
    }

    /// Adds `count` audio publications of the fixture's owner to its room, each
    /// on its own native producer, as a full voice room has.
    async fn publish_many(owner: &Arc<Peer>, room: &Room, count: u32) -> Vec<Arc<Publication>> {
        let transport = room.data.lock().await.peers[&owner.id].transports[&true].clone();
        let mut publications = Vec::new();
        for index in 0..count {
            let rtp = serde_json::from_value(json!({
                "codecs":[{"mimeType":"audio/opus","payloadType":111,"clockRate":48000,
                    "channels":2,"parameters":{},"rtcpFeedback":[]}],
                "headerExtensions":[],"encodings":[{"ssrc":10_000 + index}],
                "rtcp":{"cname":format!("attach-{index}"),"reducedSize":true}
            }))
            .unwrap();
            let producer = room
                .engine
                .native(transport.clone(), move |transport| async move {
                    transport
                        .produce(ProducerOptions::new(MediaKind::Audio, rtp))
                        .await
                })
                .await
                .unwrap();
            publications.push(Arc::new(Publication {
                producer: NativeProducer::new(
                    producer,
                    transport.clone(),
                    room.engine.lost.clone(),
                ),
                peer: owner.clone(),
                kind: SourceKind::Mic,
                epoch: Uuid::new_v4(),
                parent: None,
                live: None,
                grant: Grant::new(),
                gate: Mutex::new(()),
                height: 0,
                layers: 1,
                packet_samples: StdMutex::new(HashMap::new()),
            }));
        }
        let mut data = room.data.lock().await;
        for publication in &publications {
            data.publications
                .insert(publication.id(), publication.clone());
        }
        publications
    }

    /// Joining a room with 40 publications: every one is consumed and announced
    /// in one attach pass. Prints the pass time to compare join latency.
    #[tokio::test]
    async fn attach_pass_announces_every_publication_of_a_full_room() {
        const PUBLICATIONS: u32 = 40;
        let (sfu, owner, _publication, _owner_events) = stopping_fixture().await;
        let channel = owner.authority.claim.c;
        let room = sfu.room(channel).await.unwrap();
        publish_many(&owner, &room, PUBLICATIONS - 1).await;
        let mut viewer = Viewer::join(&sfu, &owner, &room, 64).await;
        let started = Instant::now();
        sfu.attach_existing(channel, viewer.id).await;
        let elapsed = started.elapsed();
        // Written past libtest's capture so the CI log keeps the number.
        let _ = std::io::Write::write_all(
            &mut std::io::stderr(),
            format!("attach pass for {PUBLICATIONS} publications took {elapsed:?}\n").as_bytes(),
        );
        let mut announced = HashSet::new();
        while let Ok(frame) = viewer.events.try_recv() {
            if let ServerFrame::Consumer { producer_id, .. } = frame {
                announced.insert(producer_id);
            }
        }
        assert_eq!(announced.len(), PUBLICATIONS as usize);
        {
            let data = room.data.lock().await;
            let peer = &data.peers[&viewer.id];
            assert_eq!(peer.consumers.len(), PUBLICATIONS as usize);
            assert!(peer.pending_consumers.is_empty());
        }
        assert_eq!(
            viewer.transport.dump().await.unwrap().consumer_ids.len(),
            PUBLICATIONS as usize
        );
        sfu.leave(viewer.id, channel).await;
        viewer.revoke(&sfu).await;
    }

    /// Authority that lapses while a pass creates its consumers: none of them
    /// is announced or kept, and every native consumer is closed again.
    #[tokio::test]
    async fn attach_pass_announces_nothing_after_revocation() {
        const PUBLICATIONS: usize = 12;
        let (sfu, owner, publication, _owner_events) = stopping_fixture().await;
        let channel = owner.authority.claim.c;
        let room = sfu.room(channel).await.unwrap();
        let mut pubs = publish_many(&owner, &room, PUBLICATIONS as u32 - 1).await;
        pubs.push(publication);
        let mut viewer = Viewer::join(&sfu, &owner, &room, 64).await;
        let created = Arc::new(AtomicUsize::new(0));
        let client = redis::Client::open(std::env::var("REDIS_URL").unwrap()).unwrap();
        let probe = (
            created.clone(),
            client,
            viewer.keys.clone(),
            room.engine.clone(),
        );
        let results = sfu
            .attach_batch(&room, viewer.id, pubs, move |transport, options| {
                let (created, client, keys, engine) = probe.clone();
                async move {
                    engine
                        .native(transport, move |transport| async move {
                            let consumer = transport.consume(options).await;
                            // The last creation of the pass revokes before it
                            // returns, so the pass's single check must see the
                            // revocation.
                            if created.fetch_add(1, Ordering::SeqCst) + 1 == PUBLICATIONS {
                                let mut redis =
                                    client.get_multiplexed_async_connection().await.unwrap();
                                let _: () = redis::cmd("DEL")
                                    .arg(&keys)
                                    .query_async(&mut redis)
                                    .await
                                    .unwrap();
                            }
                            consumer
                        })
                        .await
                }
            })
            .await;
        assert_eq!(created.load(Ordering::SeqCst), PUBLICATIONS);
        assert_eq!(results.len(), PUBLICATIONS);
        assert!(
            results.iter().all(|r| matches!(r, Err(SfuError::Revoked))),
            "{results:?}"
        );
        while let Ok(frame) = viewer.events.try_recv() {
            assert!(!matches!(frame, ServerFrame::Consumer { .. }));
        }
        // The authority watcher may already have removed the viewer.
        if let Some(peer) = room.data.lock().await.peers.get(&viewer.id) {
            assert!(peer.consumers.is_empty());
            assert!(peer.pending_consumers.is_empty());
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while viewer
                .transport
                .dump()
                .await
                .is_ok_and(|dump| !dump.consumer_ids.is_empty())
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("revoked consumers must be closed natively");
        sfu.leave(viewer.id, channel).await;
    }

    #[tokio::test]
    async fn native_consume_timeout_retries_without_room_activity_and_retires_late_consumer() {
        let (sfu, owner, publication, _owner_events) = stopping_fixture().await;
        let channel = owner.authority.claim.c;
        let room = sfu.room(channel).await.unwrap();
        let mut joined = Viewer::join(&sfu, &owner, &room, 16).await;
        let (viewer, transport) = (joined.id, joined.transport.clone());
        let (complete, pending) = tokio::sync::oneshot::channel();
        let (late_created, late_ready) = tokio::sync::oneshot::channel();
        let probe = StdMutex::new(Some((pending, late_created)));
        let engine = room.engine.clone();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            sfu.attach_batch(
                &room,
                viewer,
                vec![publication.clone()],
                move |transport, options| {
                    let (pending, late_created) = probe.lock().unwrap().take().unwrap();
                    let engine = engine.clone();
                    async move {
                        engine
                            .native(transport, move |transport| async move {
                                pending.await.unwrap();
                                let result = transport.consume(options).await;
                                assert!(result.is_ok(), "probe must create a real late consumer");
                                late_created.send(()).unwrap();
                                result
                            })
                            .await
                    }
                },
            ),
        )
        .await
        .expect("attach must release its pending reservation after timeout");
        assert!(matches!(result[..], [Err(SfuError::Unavailable)]));
        let event = tokio::time::timeout(Duration::from_secs(3), joined.events.recv())
            .await
            .unwrap()
            .unwrap();
        let ServerFrame::Consumer {
            producer_id,
            consumer_id,
            ..
        } = event
        else {
            panic!("retry did not announce a consumer");
        };
        assert_eq!(producer_id, publication.id());
        {
            let data = room.data.lock().await;
            let peer = data.peers.get(&viewer).unwrap();
            assert_eq!(peer.consumers.len(), 1);
            assert!(peer.pending_consumers.is_empty());
        }
        complete.send(()).unwrap();
        late_ready.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let dump = transport.dump().await.unwrap();
                if dump.consumer_ids.len() == 1 && dump.consumer_ids[0].to_string() == consumer_id {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("late unannounced consumer must be closed");
        sfu.leave(viewer, channel).await;
        joined.revoke(&sfu).await;
    }

    async fn assert_terminal_after_native_stop(revoke: bool) {
        let (sfu, peer, publication, mut rx) = stopping_fixture().await;
        let gate = publication.gate.lock().await;
        let shutdown = if revoke {
            Sfu::watch_authority(
                Arc::downgrade(&sfu),
                peer.authority.claim.c,
                Arc::downgrade(&peer),
            );
            None
        } else {
            let engine = sfu.clone();
            Some(tokio::spawn(async move { engine.shutdown().await }))
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while peer.grant.valid() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "terminal event preceded the confirmed native stop"
        );
        assert!(!publication.producer.paused());
        assert!(sfu.peer_present(peer.id, peer.authority.claim.c).await);
        drop(gate);
        let expected = if revoke { "unauthorized" } else { "gone" };
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(ServerFrame::Err { e, .. }) = rx.recv().await {
                    assert_eq!(e, expected);
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(publication.producer.paused());
        assert!(!sfu.peer_present(peer.id, peer.authority.claim.c).await);
        if let Some(task) = shutdown {
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn shutdown_terminal_event_follows_confirmed_native_stop() {
        assert_terminal_after_native_stop(false).await;
    }

    #[tokio::test]
    async fn revocation_terminal_event_follows_confirmed_native_stop() {
        assert_terminal_after_native_stop(true).await;
    }

    #[tokio::test]
    async fn revoked_rpc_waits_for_confirmed_native_stop() {
        let (sfu, peer, publication, _rx) = stopping_fixture().await;
        let gate = publication.gate.lock().await;
        peer.grant.stop();
        let rpc = sfu.rpc(
            peer.id,
            peer.authority.claim.c,
            ClientFrame::Capabilities {
                id: 1,
                rtp: json!({}),
            },
        );
        tokio::pin!(rpc);
        // The operation gate represents an in-flight native operation. A revoked
        // RPC must not finish while that operation still prevents confirmed stop.
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut rpc)
                .await
                .is_err()
        );
        assert!(!publication.producer.paused());
        assert!(sfu.peer_present(peer.id, peer.authority.claim.c).await);
        drop(gate);
        assert!(matches!(rpc.await, Err(SfuError::Revoked)));
        assert!(publication.producer.paused());
        assert!(!sfu.peer_present(peer.id, peer.authority.claim.c).await);
    }

    #[tokio::test]
    async fn slow_resource_gate_closes_only_its_native_producer() {
        let (sfu, _peer, publication, _rx) = stopping_fixture().await;
        let transport = publication.producer.transport();
        // Reuse the complete parameters accepted by the native fixture. Only
        // the second source's SSRC differs; no partial SDK JSON schema here.
        let mut rtp = publication
            .producer
            .resource
            .with(|producer| producer.rtp_parameters().clone())
            .unwrap();
        rtp.encodings[0].ssrc = Some(5678);
        let engine = first_engine(&sfu);
        let other = engine
            .native(transport.clone(), move |transport| async move {
                transport
                    .produce(ProducerOptions::new(MediaKind::Audio, rtp))
                    .await
            })
            .await
            .unwrap();
        let _gate = publication.gate.lock().await;
        tokio::time::timeout(
            Duration::from_secs(2),
            sfu.confirmed_stop_resources(std::slice::from_ref(&publication), &[]),
        )
        .await
        .unwrap();
        assert!(publication.producer.closed());
        let dump = transport.dump().await.unwrap();
        assert!(!dump.producer_ids.contains(&publication.producer.id()));
        assert!(dump.producer_ids.contains(&other.id()));
        assert!(!other.paused());
        assert!(sfu.ready());
    }

    async fn pool(workers: usize) -> Arc<Sfu> {
        let workers = workers.to_string();
        let config = Config::from_source(|key| match key {
            "REDIS_URL" => Some(std::env::var(key).expect("isolated REDIS_URL required")),
            "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
            "MEDIA_WORKERS" => Some(workers.clone()),
            _ => None,
        })
        .unwrap();
        Sfu::with_redis(
            &config,
            redis::Client::open(config.redis_url.as_str()).unwrap(),
        )
        .await
        .unwrap()
    }

    /// A room with one peer and a receive transport on `engine`, counted like a join.
    async fn seat(
        sfu: &Arc<Sfu>,
        engine: &Arc<Engine>,
    ) -> (Uuid, Arc<Peer>, mpsc::Receiver<ServerFrame>) {
        let router = engine
            .native(engine.worker.clone(), |worker| async move {
                worker
                    .create_router(RouterOptions::new(router_codecs().unwrap()))
                    .await
            })
            .await
            .unwrap();
        let mut options = WebRtcTransportOptions::new_with_server(engine.server.clone());
        options.app_data = AppData::new(AtomicU64::new(0));
        let transport = engine
            .native(router.clone(), move |router| async move {
                router.create_webrtc_transport(options).await
            })
            .await
            .unwrap();
        let channel = Uuid::new_v4();
        let (out, rx) = mpsc::channel(16);
        let peer = Arc::new(Peer {
            id: PeerId(Uuid::new_v4()),
            authority: AuthorizedTicketClaim {
                claim: TicketClaim {
                    u: Uuid::new_v4(),
                    s: Uuid::new_v4(),
                    c: channel,
                    g: false,
                },
                auth: TicketAuthorization {
                    session: Uuid::new_v4().simple().to_string().repeat(2),
                    expires_at: 9_007_199_254_740_991,
                    member: Uuid::new_v4(),
                    channel: Uuid::new_v4(),
                },
            },
            watch_user: None,
            grant: Grant::new(),
            out,
            rpc_gate: Mutex::new(()),
            lease_gate: Mutex::new(()),
            attach_scheduled: AtomicBool::new(false),
            attach_dirty: AtomicBool::new(false),
            retry_scheduled: AtomicBool::new(false),
            attach_failures: AtomicUsize::new(0),
        });
        let mut room = Room::new(engine.clone(), Some(router));
        room.data.get_mut().peers.insert(
            peer.id,
            PeerData {
                peer: peer.clone(),
                capabilities: None,
                transports: HashMap::from([(false, transport)]),
                publications: HashMap::new(),
                consumers: HashMap::new(),
                pending_consumers: HashSet::new(),
                failed_consumers: HashMap::new(),
                watches: HashMap::new(),
                withdrawn_live: VecDeque::new(),
                retired_producers: VecDeque::new(),
                retired_transports: VecDeque::new(),
            },
        );
        sfu.rooms.lock().await.insert(channel, Arc::new(room));
        for counter in [
            &sfu.counters.rooms,
            &sfu.counters.peers,
            &sfu.counters.transports,
        ] {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        engine.peers.fetch_add(1, Ordering::Relaxed);
        (channel, peer, rx)
    }

    #[tokio::test]
    async fn new_rooms_go_to_the_least_loaded_live_worker() {
        let sfu = pool(3).await;
        let engines = sfu.engines.lock().unwrap().clone();
        assert_eq!(sfu.workers(), WorkerStatus { live: 3, total: 3 });
        // Each worker has its own WebRtcServer and so its own UDP port.
        let mut ports = HashSet::new();
        for engine in &engines {
            let (channel, _, _) = seat(&sfu, engine).await;
            let room = sfu.rooms.lock().await.remove(&channel).unwrap();
            let data = room.data.lock().await;
            let transport = data.peers.values().next().unwrap().transports[&false].clone();
            ports.insert(transport.ice_candidates()[0].port);
            drop(data);
            let mut data = room.data.lock().await;
            data.peers.clear();
            for counter in [
                &sfu.counters.rooms,
                &sfu.counters.peers,
                &sfu.counters.transports,
            ] {
                counter.fetch_sub(1, Ordering::Relaxed);
            }
            engine.peers.fetch_sub(1, Ordering::Relaxed);
        }
        assert_eq!(ports.len(), 3);
        assert_eq!(sfu.pick_engine().unwrap().slot, 0);
        let (_a, _pa, _ra) = seat(&sfu, &engines[0]).await;
        assert_eq!(sfu.pick_engine().unwrap().slot, 1);
        let (_b, _pb, _rb) = seat(&sfu, &engines[1]).await;
        let (_c, _pc, _rc) = seat(&sfu, &engines[1]).await;
        assert_eq!(sfu.pick_engine().unwrap().slot, 2);
        engines[2].lost.store(true, Ordering::SeqCst);
        assert_eq!(sfu.pick_engine().unwrap().slot, 0);
        assert_eq!(sfu.workers(), WorkerStatus { live: 2, total: 3 });
        assert!(sfu.ready());
        let metrics = sfu.metrics_text();
        assert!(metrics.contains("gelabber_media_workers 3\n"), "{metrics}");
        assert!(
            metrics.contains("gelabber_media_workers_live 2\n"),
            "{metrics}"
        );
        assert!(
            metrics.contains("gelabber_media_worker_peers{worker=\"1\"} 2\n"),
            "{metrics}"
        );
    }

    #[tokio::test]
    async fn lost_worker_closes_only_its_rooms_and_is_replaced() {
        let sfu = pool(2).await;
        let engines = sfu.engines.lock().unwrap().clone();
        let (lost_channel, lost_peer, mut lost_rx) = seat(&sfu, &engines[0]).await;
        let (kept_channel, kept_peer, mut kept_rx) = seat(&sfu, &engines[1]).await;
        sfu.engine_lost(&engines[0], "test: worker lost").await;

        let frame = lost_rx.try_recv().expect("peer of the lost worker is told");
        assert!(
            matches!(&frame, ServerFrame::Err { e, .. } if e == "gone"),
            "{frame:?}"
        );
        assert!(!lost_peer.grant.valid());
        assert!(!sfu.peer_present(lost_peer.id, lost_channel).await);
        assert!(sfu.room(lost_channel).await.is_err());

        assert!(
            kept_rx.try_recv().is_err(),
            "other worker's peers are untouched"
        );
        assert!(kept_peer.grant.valid());
        assert!(sfu.peer_present(kept_peer.id, kept_channel).await);
        assert!(engines[1].alive());
        assert!(sfu.ready());
        assert_eq!(sfu.room_count(), 1);
        assert_eq!(sfu.counters.peers.load(Ordering::Relaxed), 1);
        assert_eq!(sfu.counters.transports.load(Ordering::Relaxed), 1);
        assert_eq!(sfu.workers(), WorkerStatus { live: 1, total: 2 });
        // New rooms avoid the lost worker until its slot is replaced.
        assert_eq!(sfu.pick_engine().unwrap().slot, 1);

        tokio::time::timeout(Duration::from_secs(10), async {
            while sfu.workers().live < 2 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("lost slot gets a new worker");
        let replaced = first_engine(&sfu);
        assert!(!Arc::ptr_eq(&replaced, &engines[0]));
        assert!(replaced.alive());
        assert_eq!(sfu.counters.worker_failures.load(Ordering::Relaxed), 1);
        assert_eq!(sfu.counters.worker_restarts.load(Ordering::Relaxed), 1);
        assert_eq!(sfu.pick_engine().unwrap().slot, 0);
    }

    #[tokio::test]
    async fn worker_exit_seen_by_a_heartbeat_tick_first_still_closes_its_rooms() {
        let sfu = pool(2).await;
        let engines = sfu.engines.lock().unwrap().clone();
        let (channel, peer, mut rx) = seat(&sfu, &engines[0]).await;
        // `on_dead`'s order: `lost` first, the notification afterwards. A tick
        // in between must not end the supervisor without handling the loss.
        engines[0].lost.store(true, Ordering::SeqCst);
        tokio::time::sleep(HEARTBEAT_INTERVAL * 2).await;
        engines[0].exited.notify_one();
        let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the supervisor handles the loss")
            .unwrap();
        assert!(
            matches!(&frame, ServerFrame::Err { e, .. } if e == "gone"),
            "{frame:?}"
        );
        assert!(!sfu.peer_present(peer.id, channel).await);
        // Handled once although both the tick and the notification saw it.
        tokio::time::sleep(HEARTBEAT_INTERVAL * 2).await;
        assert_eq!(sfu.counters.worker_failures.load(Ordering::Relaxed), 1);
        assert!(engines[0].down_for() > Duration::ZERO);
        let metrics = sfu.metrics_text();
        assert!(
            metrics.contains("gelabber_media_worker_down_seconds{worker=\"1\"} 0\n"),
            "{metrics}"
        );
    }

    #[tokio::test]
    async fn rooms_of_a_lost_worker_refuse_rpcs_while_the_pool_stays_ready() {
        let sfu = pool(2).await;
        let engine = first_engine(&sfu);
        let (channel, _peer, _rx) = seat(&sfu, &engine).await;
        let room = sfu.room(channel).await.unwrap();
        engine.lost.store(true, Ordering::SeqCst);
        assert!(!sfu.room_ready(&room));
        assert!(sfu.ready());
    }

    #[test]
    fn fail_closed_terminates_child_process_with_exit_one() {
        const CHILD: &str = "GELABBER_TEST_FAIL_CLOSED_CHILD";
        if std::env::var_os(CHILD).is_some() {
            fail_closed("test child executes the real termination function");
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("sfu::tests::fail_closed_terminates_child_process_with_exit_one")
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(1));
    }
}
