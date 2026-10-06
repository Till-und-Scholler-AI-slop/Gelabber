//! Own product authority/control; mediasoup is the sole production media engine.
//! Graph locks only reserve/commit. Native awaits use per-resource gates so a
//! denied grant cannot be resumed after its confirmed native pause.
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
use tokio::sync::{Mutex, Notify, OnceCell, mpsc};
use tracing::{error, warn};
use uuid::Uuid;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_TIMEOUT: Duration = Duration::from_millis(500);
const STOP_CONCURRENCY: usize = 64;
// The deadline includes both an in-flight resource gate and native pause ACK.
const LIVE_STOP_MARGIN: Duration = Duration::from_millis(1250);
const WATCH_LIMIT: usize = 64;
const CONSUMER_LIMIT: usize = 1024;

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
}
struct PeerData {
    peer: Arc<Peer>,
    capabilities: Option<RtpCapabilities>,
    transports: HashMap<bool, WebRtcTransport>, // true = send
    publications: HashMap<SourceKind, Arc<Publication>>,
    consumers: HashMap<String, Arc<Subscription>>,
    pending_consumers: HashSet<String>,
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
    producer: Producer,
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
    consumer: Consumer,
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
enum StopResource {
    Publication(Arc<Publication>),
    Subscription(Arc<Subscription>),
}
#[derive(Default)]
struct RoomData {
    peers: HashMap<PeerId, PeerData>,
    publications: HashMap<String, Arc<Publication>>,
}
struct Room {
    router: OnceCell<Router>,
    data: Mutex<RoomData>,
    joining: AtomicUsize,
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
}
pub struct Sfu {
    redis: redis::Client,
    worker: Worker,
    server: WebRtcServer,
    rooms: Mutex<HashMap<Uuid, Arc<Room>>>,
    counters: Counters,
    dead: Arc<AtomicBool>,
    draining: AtomicBool,
}
impl Sfu {
    pub async fn with_redis(config: &Config, redis: redis::Client) -> Result<Arc<Self>, SfuError> {
        let bind: std::net::SocketAddr = config.ice_bind.parse().map_err(SfuError::negotiation)?;
        let mut settings = WorkerSettings::default();
        settings.log_level = WorkerLogLevel::Error;
        let worker = native(WorkerManager::new().create_worker(settings)).await?;
        let dead = Arc::new(AtomicBool::new(false));
        let exited = dead.clone();
        worker
            .on_dead(move |result| {
                exited.store(true, Ordering::SeqCst);
                error!(?result, "mediasoup worker exited");
                // Unexpected worker death is terminal so the process supervisor
                // can restart a fresh engine; readiness alone does not restart it.
                fail_closed("mediasoup worker unexpectedly exited");
            })
            .detach();
        let listen = ListenInfo {
            protocol: Protocol::Udp,
            ip: bind.ip(),
            announced_address: config.advertised_ip.clone(),
            expose_internal_ip: false,
            port: (bind.port() != 0).then_some(bind.port()),
            // Explicit bounded local test allocation; production binds fixed UDP.
            port_range: (bind.port() == 0).then_some(20000..=30000),
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        };
        let server = native(worker.create_webrtc_server(WebRtcServerOptions::new(
            WebRtcServerListenInfos::new(listen),
        )))
        .await?;
        let sfu = Arc::new(Self {
            redis,
            worker,
            server,
            rooms: Mutex::new(HashMap::new()),
            counters: Counters::default(),
            dead,
            draining: AtomicBool::new(false),
        });
        Self::watch_stats(Arc::downgrade(&sfu));
        Ok(sfu)
    }
    pub fn ready(&self) -> bool {
        !self.draining.load(Ordering::SeqCst)
            && !self.dead.load(Ordering::SeqCst)
            && !self.worker.closed()
            && !self.server.closed()
    }
    pub fn room_count(&self) -> usize {
        self.counters.rooms.load(Ordering::Relaxed)
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
            let room = rooms
                .entry(channel)
                .or_insert_with(|| {
                    self.counters.rooms.fetch_add(1, Ordering::Relaxed);
                    Arc::new(Room {
                        router: OnceCell::new(),
                        data: Mutex::new(RoomData::default()),
                        joining: AtomicUsize::new(0),
                    })
                })
                .clone();
            room.joining.fetch_add(1, Ordering::SeqCst);
            room
        };
        let built = room
            .router
            .get_or_try_init(|| async {
                native(
                    self.worker
                        .create_router(RouterOptions::new(router_codecs()?)),
                )
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
            || !self.ready()
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
        });
        room.data.lock().await.peers.insert(
            id,
            PeerData {
                peer: peer.clone(),
                capabilities: None,
                transports: HashMap::new(),
                publications: HashMap::new(),
                consumers: HashMap::new(),
                pending_consumers: HashSet::new(),
                watches: HashMap::new(),
                withdrawn_live: VecDeque::new(),
                retired_producers: VecDeque::new(),
                retired_transports: VecDeque::new(),
            },
        );
        self.counters.peers.fetch_add(1, Ordering::Relaxed);
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
        if !peer.grant.valid() || !self.ready() {
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
                drop(data);
                self.schedule_attach(channel, id).await;
                Ok(json!({}))
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
                let mut opts = WebRtcTransportOptions::new_with_server(self.server.clone());
                opts.enable_tcp = false;
                opts.prefer_udp = true;
                // Last sample belongs to the native transport lifetime, keeping
                // historical totals without retaining an unbounded ID cache.
                opts.app_data = AppData::new(AtomicU64::new(0));
                let transport = native(
                    room.router
                        .get()
                        .ok_or(SfuError::Unavailable)?
                        .create_webrtc_transport(opts),
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
                native(t.connect(WebRtcTransportRemoteParameters { dtls_parameters })).await?;
                if !peer.grant.valid() {
                    return Err(SfuError::Revoked);
                }
                Ok(json!({}))
            }
            ClientFrame::RestartIce { transport_id, .. } => {
                let t = self.transport(&room, id, &transport_id).await?;
                let ice = native(t.restart_ice()).await?;
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
                stop_producer(&p.producer, &self.dead).await;
                Ok(json!({}))
            }
            ClientFrame::ResumeProducer { producer_id, .. } => {
                let p = self.own_publication(&room, id, &producer_id).await?;
                let _gate = p.gate.lock().await;
                if !p.valid() {
                    return Err(SfuError::Forbidden);
                }
                native(p.producer.resume()).await?;
                if !p.valid() {
                    stop_producer(&p.producer, &self.dead).await;
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
                native(s.consumer.resume()).await?;
                if !s.valid() {
                    stop_consumer(&s.consumer, &self.dead).await;
                    return Err(SfuError::Forbidden);
                }
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
                    native(s.consumer.set_preferred_layers(ConsumerLayers {
                        spatial_layer: preferred_layer(
                            h,
                            congested,
                            s.publication.height,
                            s.publication.layers,
                        ),
                        temporal_layer: None,
                    }))
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
            let producer = native(transport.produce(options)).await?;
            let pubn = Arc::new(Publication {
                producer,
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
                stop_producer(&pubn.producer, &self.dead).await;
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
                stop_producer(&pubn.producer, &self.dead).await;
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
                native(pubn.producer.resume()).await?;
                if !pubn.valid() {
                    stop_producer(&pubn.producer, &self.dead).await;
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
        for pubn in pubs {
            if let Err(err) = self.attach(&room, id, pubn).await
                && !matches!(err, SfuError::Forbidden | SfuError::Unavailable)
            {
                warn!(?id, code = err.code(), "consumer attach failed");
            }
        }
    }
    async fn attach(
        self: &Arc<Self>,
        room: &Arc<Room>,
        id: PeerId,
        pubn: Arc<Publication>,
    ) -> Result<(), SfuError> {
        let producer_id = pubn.id();
        let (peer, transport, capabilities, watch) = {
            let mut data = room.data.lock().await;
            let p = data.peers.get_mut(&id).ok_or(SfuError::Revoked)?;
            let watch = receives(p, &pubn)?;
            if p.pending_consumers.contains(&producer_id)
                || p.consumers
                    .values()
                    .any(|s| s.publication.id() == producer_id)
            {
                return Ok(());
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
        let result = async {
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
            let consumer = native(transport.consume(options)).await?;
            let sub = Arc::new(Subscription {
                consumer,
                publication: pubn,
                peer: peer.clone(),
                watch,
                grant: Grant::new(),
                generation: Uuid::new_v4(),
                gate: Mutex::new(()),
            });
            if !sub.valid()
                || !crate::ticket::validate_authority(&self.redis, &peer.authority)
                    .await
                    .unwrap_or(false)
            {
                stop_consumer(&sub.consumer, &self.dead).await;
                return Err(SfuError::Revoked);
            }
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
                &peer,
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
        .await;
        let recreated = {
            let mut data = room.data.lock().await;
            data.peers.get_mut(&id).is_some_and(|p| {
                p.pending_consumers.remove(&producer_id);
                p.transports
                    .get(&false)
                    .is_some_and(|t| t.id() != transport.id())
            })
        };
        if recreated {
            self.schedule_attach(peer.authority.claim.c, id).await;
        }
        result
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
        let jobs = resources.into_iter().map(|resource| async move {
            match resource {
                StopResource::Publication(p) => {
                    let _gate = stop_gate(&p.gate).await;
                    stop_producer(&p.producer, &self.dead).await;
                }
                StopResource::Subscription(s) => {
                    let _gate = stop_gate(&s.gate).await;
                    stop_consumer(&s.consumer, &self.dead).await;
                }
            }
        });
        if stop_batch(jobs).await.is_err() {
            fail_closed("native stop batch exceeded its aggregate deadline");
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
        if let Some(p) = room.data.lock().await.peers.remove(&id) {
            self.counters
                .transports
                .fetch_sub(p.transports.len(), Ordering::Relaxed);
            self.counters.peers.fetch_sub(1, Ordering::Relaxed);
        }
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
        if peer.out.try_send(frame).is_err() && peer.grant.stop_once() {
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
        pubn.producer
            .on_transport_close(move || {
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
                sub.consumer.on_producer_pause(callback).detach()
            } else {
                sub.consumer.on_producer_resume(callback).detach()
            }
        }
        let weak = Arc::downgrade(self);
        let subweak = Arc::downgrade(sub);
        let runtime = tokio::runtime::Handle::current();
        sub.consumer
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
        let weak = Arc::downgrade(self);
        let subweak = Arc::downgrade(sub);
        let room = Arc::downgrade(room);
        let runtime = tokio::runtime::Handle::current();
        sub.consumer
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
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
                let mut ok = false;
                for transport in transports {
                    ok |= sfu.sample_transport(&transport).await;
                }
                sfu.counters.stats_available.store(ok, Ordering::Relaxed);
                let mut input_available = false;
                let mut jitter_microseconds = 0;
                for p in publications {
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
    let first = serde_json::to_value(&rtp.codecs[0]).map_err(SfuError::negotiation)?;
    let mime = first["mimeType"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
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
async fn native<T, E: std::fmt::Display>(
    command: impl Future<Output = Result<T, E>>,
) -> Result<T, SfuError> {
    match tokio::time::timeout(COMMAND_TIMEOUT, command).await {
        Ok(r) => r.map_err(SfuError::negotiation),
        Err(_) => fail_closed("native command timed out with an unconfirmed result"),
    }
}
async fn stop_gate(gate: &Mutex<()>) -> tokio::sync::MutexGuard<'_, ()> {
    match tokio::time::timeout(STOP_TIMEOUT, gate.lock()).await {
        Ok(g) => g,
        Err(_) => fail_closed("native operation blocked revocation"),
    }
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
async fn stop_producer(p: &Producer, dead: &AtomicBool) {
    if dead.load(Ordering::SeqCst) || p.closed() || p.paused() {
        return;
    }
    match tokio::time::timeout(STOP_TIMEOUT, p.pause()).await {
        Ok(Ok(())) if p.paused() => {}
        _ => fail_closed("native producer pause unconfirmed"),
    }
}
async fn stop_consumer(c: &Consumer, dead: &AtomicBool) {
    if dead.load(Ordering::SeqCst) || c.closed() || c.paused() {
        return;
    }
    match tokio::time::timeout(STOP_TIMEOUT, c.pause()).await {
        Ok(Ok(())) if c.paused() => {}
        _ => fail_closed("native consumer pause unconfirmed"),
    }
}
fn fail_closed(reason: &'static str) -> ! {
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
        let router = native(
            sfu.worker
                .create_router(RouterOptions::new(router_codecs().unwrap())),
        )
        .await
        .unwrap();
        let mut options = WebRtcTransportOptions::new_with_server(sfu.server.clone());
        options.app_data = AppData::new(AtomicU64::new(0));
        let transport = native(router.create_webrtc_transport(options))
            .await
            .unwrap();
        let rtp = serde_json::from_value(json!({
            "codecs":[{"mimeType":"audio/opus","payloadType":111,"clockRate":48000,
                "channels":2,"parameters":{},"rtcpFeedback":[]}],
            "headerExtensions":[],"encodings":[{"ssrc":1234}],
            "rtcp":{"cname":"stop-order-test","reducedSize":true}
        }))
        .unwrap();
        let producer = native(transport.produce(ProducerOptions::new(MediaKind::Audio, rtp)))
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
        });
        let publication = Arc::new(Publication {
            producer,
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
                    watches: HashMap::new(),
                    withdrawn_live: VecDeque::new(),
                    retired_producers: VecDeque::new(),
                    retired_transports: VecDeque::new(),
                },
            )]),
            publications: HashMap::from([(publication.id(), publication.clone())]),
        };
        sfu.rooms.lock().await.insert(
            channel,
            Arc::new(Room {
                router: OnceCell::new_with(Some(router)),
                data: Mutex::new(data),
                joining: AtomicUsize::new(0),
            }),
        );
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
