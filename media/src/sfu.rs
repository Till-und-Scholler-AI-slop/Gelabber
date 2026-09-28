//! Single-node SFU: a room is a voice channel. RTP from a publisher is
//! written onto `TrackLocalStaticRTP`s of every other peer. No mesh, no
//! recording, no second node.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::media_engine::{
    MIME_TYPE_AV1, MIME_TYPE_H264, MIME_TYPE_HEVC, MIME_TYPE_OPUS, MIME_TYPE_RTX, MIME_TYPE_VP8,
    MIME_TYPE_VP9,
};
use rtc::rtcp;
use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp;
use rtc::rtp_transceiver::rtp_sender::{
    RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters,
    RTCRtpEncodingParameters, RtpCodecKind,
};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, watch};
use tracing::{debug, info, warn};
use uuid::Uuid;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::TrackLocalEvent;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceCandidateType, RTCIceGatheringState,
    RTCSessionDescription, SettingEngine, register_default_interceptors,
};
use webrtc::rtp_transceiver::RtpSender;

use crate::config::Config;
use crate::error::SfuError;
use crate::protocol::ServerFrame;
use crate::ticket::{AuthorizedTicketClaim, TicketClaim};

const RTP_Q: usize = 512;

#[path = "sfu_feedback.rs"]
mod feedback;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerId(pub Uuid);

enum PcEvent {
    Ice(RTCIceCandidateInit),
    Track(Arc<dyn TrackRemote>),
    IceFailed,
    Closed,
}

struct Handler {
    tx: mpsc::UnboundedSender<PcEvent>,
    gathered: watch::Sender<u64>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: webrtc::peer_connection::RTCPeerConnectionIceEvent) {
        if event.candidate.address.is_empty() {
            return;
        }
        if let Ok(init) = event.candidate.to_json() {
            let _ = self.tx.send(PcEvent::Ice(init));
        }
    }

    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let next = *self.gathered.borrow() + 1;
            let _ = self.gathered.send(next);
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let _ = self.tx.send(PcEvent::Track(track));
    }

    async fn on_connection_state_change(
        &self,
        state: webrtc::peer_connection::RTCPeerConnectionState,
    ) {
        use webrtc::peer_connection::RTCPeerConnectionState;
        if state == RTCPeerConnectionState::Failed {
            let _ = self.tx.send(PcEvent::IceFailed);
        }
        if matches!(
            state,
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed
        ) {
            let _ = self.tx.send(PcEvent::Closed);
        }
    }
}

async fn local_sdp_after_gather(
    pc: &Arc<dyn PeerConnection>,
    gathered: &watch::Receiver<u64>,
    before: u64,
    closing: &watch::Sender<bool>,
) -> Option<String> {
    let mut rx = gathered.clone();
    if *rx.borrow() <= before {
        let mut closed = closing.subscribe();
        tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => return None,
            _ = tokio::time::timeout(Duration::from_secs(3), rx.wait_for(|n| *n > before)) => {}
        }
    }
    if *closing.borrow() {
        return None;
    }
    let sdp = pc.local_description().await.map(|desc| desc.sdp)?;
    sdp.contains("ice-ufrag").then_some(sdp)
}

/// Published host UDP ports. Port `0` stays ephemeral (in-process tests).
/// A fixed range is a pool: a port comes back on leave and on a failed bind.
struct IcePorts {
    ip: std::net::IpAddr,
    min: u16,
    max: u16,
    free: Mutex<VecDeque<u16>>,
}

impl IcePorts {
    fn from_config(config: &Config) -> Self {
        let addr: SocketAddr = config
            .ice_bind
            .parse()
            .unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap());
        let min = addr.port();
        let max = config.ice_port_max.unwrap_or(min).max(min);
        let mut free = VecDeque::new();
        if min != 0 {
            let mut port = min;
            loop {
                free.push_back(port);
                if port == max {
                    break;
                }
                port = port.saturating_add(1);
            }
        }
        Self {
            ip: addr.ip(),
            min,
            max,
            free: Mutex::new(free),
        }
    }

    /// `None` when every published port is still bound. Does not wrap.
    async fn take(&self) -> Option<String> {
        if self.min == 0 {
            return Some(SocketAddr::new(self.ip, 0).to_string());
        }
        let port = self.free.lock().await.pop_front()?;
        Some(SocketAddr::new(self.ip, port).to_string())
    }

    async fn release(&self, addr: &str) {
        if self.min == 0 {
            return;
        }
        let Ok(parsed) = addr.parse::<SocketAddr>() else {
            return;
        };
        let port = parsed.port();
        if port < self.min || port > self.max {
            return;
        }
        let mut free = self.free.lock().await;
        if !free.contains(&port) {
            free.push_back(port);
        }
    }
}

struct PublicationLife {
    stop: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}

#[derive(Clone)]
struct Published {
    id: String,
    publisher: PeerId,
    track_id: String,
    stream_id: String,
    kind: RtpCodecKind,
    codec: RTCRtpCodec,
    packets: broadcast::Sender<rtp::Packet>,
    keyframe: Option<mpsc::UnboundedSender<()>>,
    life: Arc<PublicationLife>,
    live_deadline: Option<Arc<StdMutex<Instant>>>,
}

struct Subscription {
    sender: Arc<dyn RtpSender>,
    task: tokio::task::JoinHandle<()>,
    codec: RTCRtpCodec,
    payload_type: watch::Sender<Option<u8>>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum SubscriptionState {
    Pending(Published),
    Active(Subscription),
}

struct PeerSdp {
    negotiated: bool,
    have_local_offer: bool,
    closed: bool,
    closing: watch::Sender<bool>,
    dirty: bool,
    /// The reservation exists from enqueue until cleanup, including during SDP.
    subscriptions: HashMap<String, SubscriptionState>,
    /// Only these new senders are discarded if the current offer fails.
    offered: HashSet<String>,
    pending_ice: Vec<(String, Option<String>)>,
}

impl PeerSdp {
    fn new() -> Self {
        Self {
            negotiated: false,
            have_local_offer: false,
            closed: false,
            closing: watch::channel(false).0,
            dirty: false,
            subscriptions: HashMap::new(),
            offered: HashSet::new(),
            pending_ice: Vec::new(),
        }
    }
}

struct Peer {
    #[allow(dead_code)]
    id: PeerId,
    user_id: Uuid,
    #[allow(dead_code)]
    channel_id: Uuid,
    /// From the join ticket. `l` is refused when this is false.
    go_live: bool,
    watch_user: Option<Uuid>,
    authority: Option<AuthorizedTicketClaim>,
    live_claim: Option<LiveBinding>,
    withdrawn_live: Option<Uuid>,
    /// Host UDP address taken from [`IcePorts`], returned on leave.
    ice_addr: String,
    pc: Arc<dyn PeerConnection>,
    out: mpsc::UnboundedSender<ServerFrame>,
    gathered: watch::Receiver<u64>,
    sdp: Arc<Mutex<PeerSdp>>,
    closing: watch::Sender<bool>,
    /// Explicit MSID track identity; legacy tags are bound in SDP order, never RTP order.
    video_kinds: HashMap<String, String>,
    legacy_kinds: VecDeque<String>,
    current_video: HashSet<String>,
    remote_tracks: HashMap<String, Arc<dyn TrackRemote>>,
}

struct Room {
    peers: HashMap<PeerId, Peer>,
    pubs: HashMap<String, Published>,
}

#[derive(Clone)]
struct LiveBinding {
    nonce: Uuid,
    track_id: String,
    deadline: Arc<StdMutex<Instant>>,
}

impl PartialEq for LiveBinding {
    fn eq(&self, other: &Self) -> bool {
        self.nonce == other.nonce
            && self.track_id == other.track_id
            && Arc::ptr_eq(&self.deadline, &other.deadline)
    }
}

fn live_expired(deadline: &Option<Arc<StdMutex<Instant>>>) -> bool {
    deadline
        .as_ref()
        .is_some_and(|deadline| Instant::now() >= *deadline.lock().unwrap())
}

fn live_wakeup(deadline: &Option<Arc<StdMutex<Instant>>>) -> tokio::time::Instant {
    let at = deadline.as_ref().map_or_else(
        || Instant::now() + Duration::from_secs(3600),
        |deadline| *deadline.lock().unwrap(),
    );
    at.into()
}

impl Peer {
    fn receives(&self, publication: &Published) -> bool {
        self.watch_user.is_none_or(|user| {
            publication.kind == RtpCodecKind::Audio || publication.stream_id == format!("{user}:l")
        })
    }
}

struct Forward {
    pc: Arc<dyn PeerConnection>,
    out: mpsc::UnboundedSender<ServerFrame>,
    gathered: watch::Receiver<u64>,
    sdp: Arc<Mutex<PeerSdp>>,
    publication: Published,
}

pub struct Sfu {
    ice_ports: IcePorts,
    advertised_ip: Option<String>,
    /// Shared with the API. `None` in unit tests that never mint tickets.
    redis: Option<redis::Client>,
    rooms: RwLock<HashMap<Uuid, Arc<Mutex<Room>>>>,
    stats: Arc<SfuStats>,
}

#[derive(Default)]
struct SfuStats {
    rooms: AtomicU64,
    peers: AtomicU64,
    forwarded_bytes: AtomicU64,
    ice_fails: AtomicU64,
    rtp_packets_total: AtomicU64,
    rtp_lost_total: AtomicU64,
    /// Latest RFC 3550 interarrival jitter, microseconds. Not a lifetime mean.
    rtp_jitter_micros: AtomicU64,
}

impl Sfu {
    pub fn new(config: &Config) -> Self {
        Self::with_redis(config, None)
    }

    pub fn with_redis(config: &Config, redis: Option<redis::Client>) -> Self {
        Self {
            ice_ports: IcePorts::from_config(config),
            advertised_ip: config.advertised_ip.clone(),
            redis,
            rooms: RwLock::new(HashMap::new()),
            stats: Arc::new(SfuStats::default()),
        }
    }

    pub fn room_count(&self) -> usize {
        // test helper; cheap snapshot
        self.rooms.try_read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn metrics_text(&self) -> String {
        let rooms = self.stats.rooms.load(Ordering::Relaxed);
        let peers = self.stats.peers.load(Ordering::Relaxed);
        let forwarded = self.stats.forwarded_bytes.load(Ordering::Relaxed);
        let ice_fails = self.stats.ice_fails.load(Ordering::Relaxed);
        let rtp_packets = self.stats.rtp_packets_total.load(Ordering::Relaxed);
        let rtp_lost = self.stats.rtp_lost_total.load(Ordering::Relaxed);
        let jitter_ms = self.stats.rtp_jitter_micros.load(Ordering::Relaxed) as f64 / 1_000.0;
        format!(
            "# HELP gelabber_media_rooms Active SFU rooms (voice channels with at least one peer).\n\
             # TYPE gelabber_media_rooms gauge\n\
             gelabber_media_rooms {rooms}\n\
             # HELP gelabber_media_peers Connected peers across all rooms.\n\
             # TYPE gelabber_media_peers gauge\n\
             gelabber_media_peers {peers}\n\
             # HELP gelabber_media_forwarded_bytes_total RTP payload bytes forwarded to subscribers.\n\
             # TYPE gelabber_media_forwarded_bytes_total counter\n\
             gelabber_media_forwarded_bytes_total {forwarded}\n\
             # HELP gelabber_media_ice_fails_total Peer connections that entered the ICE failed state.\n\
             # TYPE gelabber_media_ice_fails_total counter\n\
             gelabber_media_ice_fails_total {ice_fails}\n\
             # HELP gelabber_media_rtp_packets_total RTP packets received from publishers.\n\
             # TYPE gelabber_media_rtp_packets_total counter\n\
             gelabber_media_rtp_packets_total {rtp_packets}\n\
             # HELP gelabber_media_rtp_lost_total RTP packets declared lost after the reorder window.\n\
             # TYPE gelabber_media_rtp_lost_total counter\n\
             gelabber_media_rtp_lost_total {rtp_lost}\n\
             # HELP gelabber_media_rtp_jitter_ms Latest publisher RFC 3550 interarrival jitter in milliseconds.\n\
             # TYPE gelabber_media_rtp_jitter_ms gauge\n\
             gelabber_media_rtp_jitter_ms {jitter_ms:.3}\n"
        )
    }

    pub async fn join(
        self: &Arc<Self>,
        claim: TicketClaim,
        out: mpsc::UnboundedSender<ServerFrame>,
    ) -> Result<PeerId, SfuError> {
        // This helper is exclusively for in-process RTC tests without Redis.
        // A configured production SFU accepts only authorized ticket envelopes.
        if self.redis.is_some() {
            return Err(SfuError::Revoked);
        }
        self.join_inner(claim, None, None, out).await
    }

    pub async fn join_authorized(
        self: &Arc<Self>,
        claim: AuthorizedTicketClaim,
        out: mpsc::UnboundedSender<ServerFrame>,
    ) -> Result<PeerId, SfuError> {
        self.join_authorized_watch(claim, None, out).await
    }

    pub async fn join_authorized_watch(
        self: &Arc<Self>,
        claim: AuthorizedTicketClaim,
        watch_user: Option<Uuid>,
        out: mpsc::UnboundedSender<ServerFrame>,
    ) -> Result<PeerId, SfuError> {
        if watch_user.is_some_and(|user| user.is_nil()) {
            return Err(SfuError::BadAnnounce);
        }
        if !self.authorized(&claim).await {
            return Err(SfuError::Revoked);
        }
        self.join_inner(claim.claim.clone(), Some(claim), watch_user, out)
            .await
    }

    async fn join_inner(
        self: &Arc<Self>,
        claim: TicketClaim,
        authority: Option<AuthorizedTicketClaim>,
        watch_user: Option<Uuid>,
        out: mpsc::UnboundedSender<ServerFrame>,
    ) -> Result<PeerId, SfuError> {
        let peer_id = PeerId(Uuid::new_v4());
        let ice_addr = self.ice_ports.take().await.ok_or(SfuError::Unavailable)?;
        let built = self.build_pc(&ice_addr).await;
        let (pc, mut events, gathered) = match built {
            Ok(parts) => parts,
            Err(err) => {
                self.ice_ports.release(&ice_addr).await;
                return Err(SfuError::negotiation(err));
            }
        };

        {
            let mut rooms = self.rooms.write().await;
            // Revalidate after PC construction and any attach-lock wait.
            if let Some(authority) = &authority
                && !self.authorized(authority).await
            {
                drop(rooms);
                if pc.close().await.is_ok() {
                    self.ice_ports.release(&ice_addr).await;
                }
                return Err(SfuError::Revoked);
            }
            let gate = PeerSdp::new();
            let closing = gate.closing.clone();
            let room = rooms.entry(claim.c).or_insert_with(|| {
                Arc::new(Mutex::new(Room {
                    peers: HashMap::new(),
                    pubs: HashMap::new(),
                }))
            });
            let mut room = room.lock().await;
            let first = room.peers.is_empty();
            room.peers.insert(
                peer_id,
                Peer {
                    id: peer_id,
                    user_id: claim.u,
                    channel_id: claim.c,
                    go_live: claim.g,
                    watch_user,
                    authority: authority.clone(),
                    live_claim: None,
                    withdrawn_live: None,
                    ice_addr: ice_addr.clone(),
                    pc: pc.clone(),
                    out: out.clone(),
                    gathered,
                    sdp: Arc::new(Mutex::new(gate)),
                    closing,
                    video_kinds: HashMap::new(),
                    legacy_kinds: VecDeque::new(),
                    current_video: HashSet::new(),
                    remote_tracks: HashMap::new(),
                },
            );
            if first {
                self.stats.rooms.fetch_add(1, Ordering::Relaxed);
            }
            self.stats.peers.fetch_add(1, Ordering::Relaxed);
        }

        info!(
            peer = %peer_id.0,
            user = %claim.u,
            channel = %claim.c,
            go_live = claim.g,
            "sfu join"
        );

        let sfu = Arc::clone(self);
        tokio::spawn(async move {
            sfu.drive(peer_id, claim.c, pc, &mut events).await;
        });
        if let Some(authority) = authority {
            self.watch_revoke(peer_id, claim.c, authority, out);
        }

        Ok(peer_id)
    }

    pub async fn apply_remote(
        self: &Arc<Self>,
        peer_id: PeerId,
        channel_id: Uuid,
        sdp_text: String,
        as_offer: bool,
    ) -> Result<(), SfuError> {
        let room = self
            .find_room(channel_id)
            .await
            .ok_or(SfuError::NotInRoom)?;
        let (pc, out, gathered, sdp) = {
            let room = room.lock().await;
            let peer = room.peers.get(&peer_id).ok_or(SfuError::NotInRoom)?;
            (
                peer.pc.clone(),
                peer.out.clone(),
                peer.gathered.clone(),
                peer.sdp.clone(),
            )
        };
        let mut gate = sdp.lock().await;
        if gate.closed || *gate.closing.borrow() {
            return Err(SfuError::NotInRoom);
        }
        if as_offer && gate.have_local_offer {
            return Ok(());
        }
        let parsed = if as_offer {
            RTCSessionDescription::offer(sdp_text.clone())
        } else {
            RTCSessionDescription::answer(sdp_text.clone())
        };
        let desc = match parsed {
            Ok(desc) => desc,
            Err(err) => {
                if !as_offer {
                    self.rollback_locked(&pc, &mut gate).await;
                    self.negotiate_locked(&pc, &out, &gathered, &mut gate).await;
                }
                return Err(SfuError::negotiation(err));
            }
        };
        // Bind old announcements before set_remote_description can deliver on_track.
        if as_offer {
            let mut room = room.lock().await;
            let peer = room.peers.get_mut(&peer_id).ok_or(SfuError::NotInRoom)?;
            for source in video_sources(&sdp_text) {
                if !peer.video_kinds.contains_key(&source) {
                    let kind = peer.legacy_kinds.pop_front().unwrap_or_else(|| "v".into());
                    peer.video_kinds.insert(source, kind);
                }
            }
        }
        if let Err(err) = pc.set_remote_description(desc).await {
            if !as_offer {
                self.rollback_locked(&pc, &mut gate).await;
                self.negotiate_locked(&pc, &out, &gathered, &mut gate).await;
            }
            return Err(SfuError::negotiation(err));
        }
        let accepted_answer;
        if as_offer {
            for state in gate.subscriptions.values() {
                if let SubscriptionState::Active(sub) = state
                    && let Some(mid) = sender_mid(&pc, &sub.sender).await
                    && let Some(pt) = negotiated_payload_type(&sdp_text, &mid, &sub.codec)
                {
                    limit_forward_codec_with_pt(&pc, &sub.sender, &sub.codec, pt).await;
                }
            }
            let result = async {
                let answer = pc.create_answer(None).await?;
                let before = *gathered.borrow();
                pc.set_local_description(answer).await?;
                let local = local_sdp_after_gather(&pc, &gathered, before, &gate.closing)
                    .await
                    .ok_or(webrtc::error::Error::ErrUnknownType)?;
                let mut answer = RTCSessionDescription::answer(local)?;
                // The pinned media engine keeps its earlier PTs on reoffer.
                // Answers must use the PT offered on this exact subscriber MID.
                for state in gate.subscriptions.values() {
                    if let SubscriptionState::Active(sub) = state
                        && let Some(mid) = sender_mid(&pc, &sub.sender).await
                        && let Some(offered_pt) =
                            negotiated_payload_type(&sdp_text, &mid, &sub.codec)
                        && let Some(answer_pt) =
                            negotiated_payload_type(&answer.sdp, &mid, &sub.codec)
                        && offered_pt != answer_pt
                    {
                        answer = remap_answer_payload(answer, &mid, answer_pt, offered_pt)?;
                    }
                }
                // Local SDP munging is rejected by this pinned core. The wire
                // answer uses the offer's PTs; refresh_subscriber_bindings also
                // reconciles the concrete sender before any RTP is released.
                out.send(ServerFrame::Answer {
                    sdp: answer.sdp.clone(),
                })
                .map_err(|_| webrtc::error::Error::ErrUnknownType)?;
                Ok::<RTCSessionDescription, webrtc::error::Error>(answer)
            }
            .await;
            accepted_answer = match result {
                Ok(answer) => Some(answer),
                Err(err) => {
                    if let Ok(rollback) = RTCSessionDescription::rollback(None) {
                        let _ = pc.set_remote_description(rollback).await;
                    }
                    return Err(SfuError::negotiation(err));
                }
            };
            gate.negotiated = true;
        } else {
            accepted_answer = pc.remote_description().await;
        }
        gate.have_local_offer = false;
        gate.offered.clear();
        refresh_subscriber_bindings(&pc, &gate, accepted_answer.as_ref()).await;
        let queued_ice = std::mem::take(&mut gate.pending_ice);
        flush_ice(&pc, peer_id, queued_ice).await;
        self.negotiate_locked(&pc, &out, &gathered, &mut gate).await;
        drop(gate);
        if as_offer {
            // A recvonly/inactive m-line is a stop even when OnEnded is not emitted.
            let active = video_sources(&sdp_text);
            let ended = {
                let room = room.lock().await;
                room.pubs
                    .values()
                    .filter(|p| {
                        p.publisher == peer_id
                            && p.kind == RtpCodecKind::Video
                            && !active.contains(&p.track_id)
                    })
                    .map(|p| (p.id.clone(), p.life.clone()))
                    .collect::<Vec<_>>()
            };
            for (id, life) in ended {
                self.remove_publication(channel_id, &id, &life).await;
            }
            let reusable = {
                let mut room = room.lock().await;
                let Some(peer) = room.peers.get_mut(&peer_id) else {
                    return Err(SfuError::NotInRoom);
                };
                let previous =
                    std::mem::replace(&mut peer.current_video, active.iter().cloned().collect());
                peer.video_kinds.retain(|id, kind| {
                    active.contains(id) || (!previous.contains(id) && !kind.is_empty())
                });
                // Stopped sender/transceiver reuse may leave the same remote track
                // object alive through an inactive offer, without another on_track.
                // Keep its bounded receiver binding; only an accepted active offer
                // can create a new publication/reader from it.
                peer.remote_tracks
                    .iter()
                    .filter(|(id, _)| {
                        active.contains(id)
                            && peer
                                .video_kinds
                                .get(*id)
                                .is_some_and(|kind| !kind.is_empty())
                    })
                    .map(|(_, track)| track.clone())
                    .collect::<Vec<_>>()
            };
            for track in reusable {
                self.publish(peer_id, channel_id, track).await?;
            }
            self.attach_existing_pubs(peer_id, channel_id).await;
        }
        Ok(())
    }

    pub async fn announce(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        kind: &str,
    ) -> Result<(), SfuError> {
        self.announce_track(peer_id, channel_id, kind, None).await
    }

    pub async fn announce_track(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        kind: &str,
        track_id: Option<&str>,
    ) -> Result<(), SfuError> {
        self.announce_with_claim(peer_id, channel_id, kind, track_id, None)
            .await
    }

    pub async fn announce_with_claim(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        kind: &str,
        track_id: Option<&str>,
        nonce: Option<Uuid>,
    ) -> Result<(), SfuError> {
        if !matches!(kind, "v" | "s" | "l") {
            return Err(SfuError::BadAnnounce);
        }
        let room = self
            .find_room(channel_id)
            .await
            .ok_or(SfuError::NotInRoom)?;
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&peer_id).ok_or(SfuError::NotInRoom)?;
        if peer.watch_user.is_some() {
            return Err(SfuError::Forbidden);
        }
        if kind == "l" && !peer.go_live {
            return Err(SfuError::Forbidden);
        }
        if let Some(id) = track_id {
            if id.is_empty()
                || id.len() > 256
                || (peer.video_kinds.len() >= 64 && !peer.video_kinds.contains_key(id))
            {
                return Err(SfuError::BadAnnounce);
            }
            if kind != "l"
                && peer
                    .live_claim
                    .as_ref()
                    .is_some_and(|live| live.track_id == id)
            {
                return Err(SfuError::BadAnnounce);
            }
        }
        let mut replaced = None;
        if kind == "l"
            && let Some(redis) = &self.redis
        {
            let nonce = nonce.ok_or(SfuError::Forbidden)?;
            if peer.withdrawn_live == Some(nonce) {
                return Err(SfuError::Forbidden);
            }
            let id = track_id
                .filter(|id| !id.is_empty() && id.len() <= 256)
                .ok_or(SfuError::BadAnnounce)?;
            let authority = peer.authority.as_ref().ok_or(SfuError::Forbidden)?;
            if peer
                .live_claim
                .as_ref()
                .is_some_and(|live| live.nonce == nonce && live.track_id != id)
            {
                return Err(SfuError::Forbidden);
            }
            // Start before Redis I/O: the local forwarding deadline must never
            // outlive the atomic peer lease when another peer takes ownership.
            let checked_at = Instant::now();
            let Some(ttl) = crate::live::validate_and_acquire(redis, authority, nonce, peer_id.0)
                .await
                .unwrap_or(None)
            else {
                return Err(SfuError::Forbidden);
            };
            let deadline = peer
                .live_claim
                .as_ref()
                .filter(|live| live.nonce == nonce && live.track_id == id)
                .map(|live| live.deadline.clone())
                .unwrap_or_else(|| Arc::new(StdMutex::new(checked_at + ttl)));
            *deadline.lock().unwrap() = checked_at + ttl;
            let binding = LiveBinding {
                nonce,
                track_id: id.into(),
                deadline,
            };
            if let Some(old) = peer.live_claim.replace(binding.clone())
                && old != binding
            {
                peer.video_kinds.insert(old.track_id.clone(), String::new());
                replaced = Some(old);
            }
        }
        if let Some(id) = track_id {
            peer.video_kinds.insert(id.into(), kind.into());
        } else if !peer.legacy_kinds.iter().any(|k| k == kind) {
            peer.legacy_kinds.push_back(kind.into());
        }
        let ended = replaced.as_ref().and_then(|old| {
            let id = format!("{}:{}", peer_id.0, old.track_id);
            room.pubs
                .get(&id)
                .map(|publication| (id, publication.life.clone()))
        });
        drop(room);
        if let Some(old) = replaced {
            if let Some((_, life)) = &ended {
                life.stop.send_replace(true);
            }
            if let Some(redis) = &self.redis {
                crate::live::release(redis, old.nonce, peer_id.0).await;
            }
            if let Some((id, life)) = ended {
                self.remove_publication(channel_id, &id, &life).await;
            }
        }
        Ok(())
    }

    pub async fn retract(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        kind: &str,
    ) -> Result<(), SfuError> {
        self.retract_track(peer_id, channel_id, kind, None).await
    }

    pub async fn retract_track(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        kind: &str,
        track_id: Option<&str>,
    ) -> Result<(), SfuError> {
        if !matches!(kind, "v" | "s" | "l") {
            return Err(SfuError::BadAnnounce);
        }
        if track_id.is_some_and(|id| id.is_empty() || id.len() > 256) {
            return Err(SfuError::BadAnnounce);
        }
        let room = self
            .find_room(channel_id)
            .await
            .ok_or(SfuError::NotInRoom)?;
        let (ended, released) = {
            let mut room = room.lock().await;
            let peer = room.peers.get_mut(&peer_id).ok_or(SfuError::NotInRoom)?;
            let released = if kind == "l"
                && peer
                    .live_claim
                    .as_ref()
                    .is_some_and(|live| track_id.is_none_or(|id| id == live.track_id))
            {
                peer.live_claim.take()
            } else {
                None
            };
            if let Some(live) = &released {
                peer.withdrawn_live = Some(live.nonce);
            }
            peer.legacy_kinds.retain(|k| k != kind);
            // Keep the identity until the next SDP stops sending it: packets already
            // queued by on_track must not resurrect a retracted source as a camera.
            if let Some(id) = track_id {
                peer.video_kinds.insert(id.into(), String::new());
            } else {
                for k in peer.video_kinds.values_mut() {
                    if k == kind {
                        k.clear();
                    }
                }
            }
            let ended = room
                .pubs
                .values()
                .filter(|p| {
                    p.publisher == peer_id
                        && p.stream_id.ends_with(&format!(":{kind}"))
                        && track_id.is_none_or(|id| id == p.track_id)
                })
                .map(|p| (p.id.clone(), p.life.clone()))
                .collect::<Vec<_>>();
            (ended, released)
        };
        // Stop RTP before releasing the exact peer lease to a recovery peer.
        for (_, life) in &ended {
            life.stop.send_replace(true);
        }
        if let Some(live) = released
            && let Some(redis) = &self.redis
        {
            crate::live::release(redis, live.nonce, peer_id.0).await;
        }
        for (id, life) in ended {
            self.remove_publication(channel_id, &id, &life).await;
        }
        Ok(())
    }

    /// Subscriber could not complete our offer. Roll signaling back to stable
    /// and forward anything that queued behind that offer.
    pub async fn abort_offer(&self, peer_id: PeerId, channel_id: Uuid) -> Result<(), SfuError> {
        let room = self
            .find_room(channel_id)
            .await
            .ok_or(SfuError::NotInRoom)?;
        let (pc, out, gathered, sdp) = {
            let room = room.lock().await;
            let peer = room.peers.get(&peer_id).ok_or(SfuError::NotInRoom)?;
            (
                peer.pc.clone(),
                peer.out.clone(),
                peer.gathered.clone(),
                peer.sdp.clone(),
            )
        };
        info!(peer = %peer_id.0, "abort subscriber offer");
        self.fail_local_offer(&pc, &out, &gathered, &sdp, false)
            .await;
        Ok(())
    }

    /// An answer was refused before `apply_remote` (oversized SDP or frame).
    /// Abort only while this offer is still outstanding, so a late reject
    /// cannot roll back the next one.
    pub async fn abort_outstanding_offer(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
    ) -> Result<(), SfuError> {
        let room = self
            .find_room(channel_id)
            .await
            .ok_or(SfuError::NotInRoom)?;
        let (pc, out, gathered, sdp) = {
            let room = room.lock().await;
            let peer = room.peers.get(&peer_id).ok_or(SfuError::NotInRoom)?;
            (
                peer.pc.clone(),
                peer.out.clone(),
                peer.gathered.clone(),
                peer.sdp.clone(),
            )
        };
        self.fail_local_offer(&pc, &out, &gathered, &sdp, true)
            .await;
        Ok(())
    }

    pub async fn add_ice(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        ice: String,
        mid: Option<String>,
    ) -> Result<(), SfuError> {
        let room = self
            .find_room(channel_id)
            .await
            .ok_or(SfuError::NotInRoom)?;
        let (pc, gate) = {
            let room = room.lock().await;
            let peer = room.peers.get(&peer_id).ok_or(SfuError::NotInRoom)?;
            (peer.pc.clone(), peer.sdp.clone())
        };
        {
            let mut gate = gate.lock().await;
            // The subscriber's new m-line does not exist until they answer our
            // offer. Candidates gathered during that answer used to fail one
            // by one and each failure became a toast.
            if gate.have_local_offer {
                if gate.pending_ice.len() < 64 {
                    gate.pending_ice.push((ice, mid));
                }
                return Ok(());
            }
        }
        pc.add_ice_candidate(ice_init(ice, mid))
            .await
            .map_err(SfuError::ice)
    }

    pub async fn leave(&self, peer_id: PeerId, channel_id: Uuid) {
        let (peer, publications, subscribers) = {
            let mut rooms = self.rooms.write().await;
            let Some(room) = rooms.get(&channel_id).cloned() else {
                return;
            };
            let mut room = room.lock().await;
            let Some(peer) = room.peers.remove(&peer_id) else {
                return;
            };
            let ids = room
                .pubs
                .values()
                .filter(|p| p.publisher == peer_id)
                .map(|p| p.id.clone())
                .collect::<Vec<_>>();
            let publications = ids
                .into_iter()
                .filter_map(|id| room.pubs.remove(&id))
                .collect::<Vec<_>>();
            let subscribers = room
                .peers
                .values()
                .map(|p| {
                    (
                        p.pc.clone(),
                        p.out.clone(),
                        p.gathered.clone(),
                        p.sdp.clone(),
                    )
                })
                .collect::<Vec<_>>();
            if room.peers.is_empty() {
                rooms.remove(&channel_id);
                saturating_dec(&self.stats.rooms);
            }
            (peer, publications, subscribers)
        };
        saturating_dec(&self.stats.peers);
        // Stop both directions before waiting for an outstanding SDP operation.
        peer.closing.send_replace(true);
        for publication in &publications {
            publication.life.stop.send_replace(true);
        }
        if let Some(live) = &peer.live_claim
            && let Some(redis) = &self.redis
        {
            crate::live::release(redis, live.nonce, peer_id.0).await;
        }
        {
            let mut gate = peer.sdp.lock().await;
            gate.closed = true;
            for (_, state) in gate.subscriptions.drain() {
                if let SubscriptionState::Active(mut sub) = state {
                    sub.task.abort();
                    let _ = (&mut sub.task).await;
                }
            }
            gate.offered.clear();
            gate.pending_ice.clear();
            // A port is reusable only once the PC has actually closed.
            if peer.pc.close().await.is_ok() {
                self.ice_ports.release(&peer.ice_addr).await;
            }
        }
        for publication in publications {
            publication.life.stop.send_replace(true);
            for (pc, out, gathered, sdp) in &subscribers {
                self.detach_subscription(pc, out, gathered, sdp, &publication.id)
                    .await;
            }
            let mut done = publication.life.done.clone();
            let _ = done.wait_for(|done| *done).await;
        }
        info!(peer = %peer_id.0, channel = %channel_id, "sfu leave");
    }

    async fn authorized(&self, claim: &AuthorizedTicketClaim) -> bool {
        let Some(redis) = &self.redis else {
            return false;
        };
        match crate::ticket::validate_authority(redis, claim).await {
            Ok(valid) => valid,
            Err(err) => {
                warn!(error = %err, "media authority unavailable; closing peer");
                false
            }
        }
    }

    fn watch_revoke(
        self: &Arc<Self>,
        peer_id: PeerId,
        channel_id: Uuid,
        authority: AuthorizedTicketClaim,
        out: mpsc::UnboundedSender<ServerFrame>,
    ) {
        let sfu = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                if !sfu.has_peer(peer_id, channel_id).await {
                    break;
                }
                if !sfu.authorized(&authority).await {
                    let _ = out.send(ServerFrame::error("unauthorized"));
                    sfu.leave(peer_id, channel_id).await;
                    break;
                }
                let live = if let Some(room) = sfu.find_room(channel_id).await {
                    room.lock()
                        .await
                        .peers
                        .get(&peer_id)
                        .and_then(|peer| peer.live_claim.clone())
                } else {
                    None
                };
                if let Some(live) = live
                    && let Some(redis) = &sfu.redis
                {
                    let checked_at = Instant::now();
                    if let Some(ttl) =
                        crate::live::validate_and_acquire(redis, &authority, live.nonce, peer_id.0)
                            .await
                            .unwrap_or(None)
                    {
                        *live.deadline.lock().unwrap() = checked_at + ttl;
                        continue;
                    }
                    // Live teardown can wait on subscriber SDP; authorization
                    // checks must keep running independently of that cleanup.
                    let cleanup = sfu.clone();
                    tokio::spawn(async move {
                        cleanup.stop_live_binding(peer_id, channel_id, &live).await;
                    });
                }
            }
        });
    }

    async fn has_peer(&self, peer_id: PeerId, channel_id: Uuid) -> bool {
        let rooms = self.rooms.read().await;
        let Some(room) = rooms.get(&channel_id) else {
            return false;
        };
        room.lock().await.peers.contains_key(&peer_id)
    }

    async fn stop_live_binding(&self, peer_id: PeerId, channel_id: Uuid, expected: &LiveBinding) {
        let Some(room) = self.find_room(channel_id).await else {
            return;
        };
        let ended = {
            let mut room = room.lock().await;
            let Some(peer) = room.peers.get_mut(&peer_id) else {
                return;
            };
            if peer.live_claim.as_ref() != Some(expected) {
                return;
            }
            peer.live_claim = None;
            peer.withdrawn_live = Some(expected.nonce);
            peer.video_kinds
                .insert(expected.track_id.clone(), String::new());
            let _ = peer.out.send(ServerFrame::error("forbidden"));
            let id = format!("{}:{}", peer_id.0, expected.track_id);
            room.pubs
                .get(&id)
                .map(|publication| (id, publication.life.clone()))
        };
        if let Some((_, life)) = &ended {
            life.stop.send_replace(true);
        }
        if let Some(redis) = &self.redis {
            crate::live::release(redis, expected.nonce, peer_id.0).await;
        }
        if let Some((id, life)) = ended {
            self.remove_publication(channel_id, &id, &life).await;
        }
    }

    async fn find_room(&self, channel_id: Uuid) -> Option<Arc<Mutex<Room>>> {
        self.rooms.read().await.get(&channel_id).cloned()
    }

    async fn build_pc(
        &self,
        ice_addr: &str,
    ) -> webrtc::error::Result<(
        Arc<dyn PeerConnection>,
        mpsc::UnboundedReceiver<PcEvent>,
        watch::Receiver<u64>,
    )> {
        let mut media = MediaEngine::default();
        register_sfu_codecs(&mut media)?;
        let registry =
            register_default_interceptors(webrtc::peer_connection::Registry::new(), &mut media)?;
        let registry = registry.with(feedback::KeyframeFeedback::new);

        let mut settings = SettingEngine::default();
        // ICE-lite only emits host candidates. STUN/TURN URLs on this PC
        // make webrtc-rs fail with "agent does not need URL with selected
        // candidate types". Browsers get those URLs from the API ticket.
        settings.set_lite(true);
        if let Some(ip) = &self.advertised_ip {
            settings.set_nat_1to1_ips(vec![ip.clone()], RTCIceCandidateType::Host);
        }

        let config = RTCConfigurationBuilder::new().build();

        let (tx, rx) = mpsc::unbounded_channel();
        let (gather_tx, gather_rx) = watch::channel(0);
        let pc = PeerConnectionBuilder::new()
            .with_configuration(config)
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_setting_engine(settings)
            .with_handler(Arc::new(Handler {
                tx,
                gathered: gather_tx,
            }))
            .with_udp_addrs(vec![ice_addr.to_owned()])
            .build()
            .await?;
        Ok((Arc::new(pc), rx, gather_rx))
    }

    async fn drive(
        self: Arc<Self>,
        peer_id: PeerId,
        channel_id: Uuid,
        pc: Arc<dyn PeerConnection>,
        events: &mut mpsc::UnboundedReceiver<PcEvent>,
    ) {
        while let Some(event) = events.recv().await {
            match event {
                PcEvent::Ice(init) => {
                    let mid = init.sdp_mid.filter(|m| !m.is_empty());
                    if let Some(out) = self.out_of(peer_id, channel_id).await {
                        let _ = out.send(ServerFrame::Ice {
                            ice: init.candidate,
                            mid,
                        });
                    }
                }
                PcEvent::Track(track) => {
                    if let Err(err) = self.publish(peer_id, channel_id, track).await {
                        if matches!(err, SfuError::Forbidden)
                            && let Some(out) = self.out_of(peer_id, channel_id).await
                        {
                            let _ = out.send(ServerFrame::error("forbidden"));
                        }
                        warn!(peer = %peer_id.0, error = %err, "publish failed");
                    }
                }
                PcEvent::IceFailed => {
                    self.stats.ice_fails.fetch_add(1, Ordering::Relaxed);
                }
                PcEvent::Closed => break,
            }
        }
        let _ = pc.close().await;
        self.leave(peer_id, channel_id).await;
    }

    async fn out_of(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
    ) -> Option<mpsc::UnboundedSender<ServerFrame>> {
        let room = self.find_room(channel_id).await?;
        let room = room.lock().await;
        room.peers.get(&peer_id).map(|p| p.out.clone())
    }

    async fn publish(
        self: &Arc<Self>,
        publisher: PeerId,
        channel_id: Uuid,
        track: Arc<dyn TrackRemote>,
    ) -> Result<(), SfuError> {
        let ssrc = *track.ssrcs().await.first().ok_or(SfuError::BadAnnounce)?;
        let codec = track.codec(ssrc).await.ok_or(SfuError::BadAnnounce)?;
        let kind = track.kind().await;
        let track_id = track.track_id().await;
        let room = self
            .find_room(channel_id)
            .await
            .ok_or(SfuError::NotInRoom)?;
        let pub_id = format!("{}:{track_id}", publisher.0);
        let (packets, _) = broadcast::channel(RTP_Q);
        let (stop, mut stopped) = watch::channel(false);
        let (done_tx, done) = watch::channel(false);
        let life = Arc::new(PublicationLife { stop, done });
        let (kf_tx, kf_rx) = mpsc::unbounded_channel();
        let publication = {
            let mut room = room.lock().await;
            let peer = room.peers.get_mut(&publisher).ok_or(SfuError::NotInRoom)?;
            if peer.watch_user.is_some() {
                return Err(SfuError::Forbidden);
            }
            let tag = if kind == RtpCodecKind::Video {
                match peer.video_kinds.get(&track_id).cloned() {
                    Some(tag) => tag,
                    None => {
                        warn!(
                            track_identity_empty = track_id.is_empty(),
                            announced_video_count = peer.video_kinds.len(),
                            "publisher track has no announced MSID binding"
                        );
                        return Err(SfuError::BadAnnounce);
                    }
                }
            } else {
                "a".into()
            };
            if tag.is_empty() {
                return Ok(());
            }
            if tag == "l" && !peer.go_live {
                return Err(SfuError::Forbidden);
            }
            if peer.remote_tracks.len() >= 64 && !peer.remote_tracks.contains_key(&track_id) {
                return Err(SfuError::BadAnnounce);
            }
            if tag == "l"
                && let Some(redis) = &self.redis
            {
                let live = peer
                    .live_claim
                    .as_ref()
                    .filter(|live| live.track_id == track_id)
                    .ok_or(SfuError::Forbidden)?;
                let authority = peer.authority.as_ref().ok_or(SfuError::Forbidden)?;
                let checked_at = Instant::now();
                let Some(ttl) =
                    crate::live::validate_and_acquire(redis, authority, live.nonce, publisher.0)
                        .await
                        .unwrap_or(None)
                else {
                    return Err(SfuError::Forbidden);
                };
                *live.deadline.lock().unwrap() = checked_at + ttl;
            }
            let live_deadline = (tag == "l")
                .then(|| peer.live_claim.as_ref().map(|live| live.deadline.clone()))
                .flatten();
            peer.remote_tracks.insert(track_id.clone(), track.clone());
            let user_id = peer.user_id;
            if room.pubs.contains_key(&pub_id) {
                return Ok(());
            }
            let publication = Published {
                id: pub_id.clone(),
                publisher,
                track_id: track_id.clone(),
                stream_id: format!("{user_id}:{tag}"),
                kind,
                codec: codec.clone(),
                packets: packets.clone(),
                keyframe: (kind == RtpCodecKind::Video).then_some(kf_tx),
                life: life.clone(),
                live_deadline,
            };
            room.pubs.insert(pub_id.clone(), publication.clone());
            publication
        };
        let sfu = self.clone();
        let read_life = life.clone();
        let read_id = pub_id.clone();
        let live_deadline = publication.live_deadline.clone();
        tokio::spawn(async move {
            let mut rtp = PublisherRtpStats::new(codec.clock_rate);
            let mut keyframes = kf_rx;
            loop {
                if live_expired(&live_deadline) {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = stopped.changed() => break,
                    _ = tokio::time::sleep_until(live_wakeup(&live_deadline)), if live_deadline.is_some() => {},
                    evt = track.poll() => {
                        if live_expired(&live_deadline) { break; }
                        if !handle_publisher_event(&packets, &sfu.stats, &mut rtp, evt) { break; }
                    }
                    Some(_) = keyframes.recv(), if kind == RtpCodecKind::Video => request_keyframe(&track, ssrc).await,
                }
            }
            done_tx.send_replace(true);
            let mut ended_live = None;
            if !*read_life.stop.borrow()
                && let Some(room) = sfu.find_room(channel_id).await
            {
                let mut room = room.lock().await;
                if let Some(peer) = room.peers.get_mut(&publisher) {
                    ended_live = peer
                        .live_claim
                        .as_ref()
                        .filter(|live| {
                            live.track_id == track_id
                                && live_deadline
                                    .as_ref()
                                    .is_some_and(|deadline| Arc::ptr_eq(deadline, &live.deadline))
                        })
                        .cloned();
                    peer.remote_tracks.remove(&track_id);
                    if let Some(kind) = peer.video_kinds.get_mut(&track_id) {
                        kind.clear();
                    }
                }
            }
            if let Some(live) = ended_live {
                sfu.stop_live_binding(publisher, channel_id, &live).await;
            }
            sfu.remove_publication(channel_id, &read_id, &read_life)
                .await;
        });
        self.attach_publication(channel_id, &publication).await;
        Ok(())
    }

    async fn remove_publication(
        &self,
        channel_id: Uuid,
        pub_id: &str,
        life: &Arc<PublicationLife>,
    ) {
        let Some(room) = self.find_room(channel_id).await else {
            return;
        };
        let subscribers = {
            let mut room = room.lock().await;
            if !room
                .pubs
                .get(pub_id)
                .is_some_and(|p| Arc::ptr_eq(&p.life, life))
            {
                return;
            }
            room.pubs.remove(pub_id);
            room.peers
                .values()
                .map(|p| {
                    (
                        p.pc.clone(),
                        p.out.clone(),
                        p.gathered.clone(),
                        p.sdp.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        life.stop.send_replace(true);
        for (pc, out, gathered, sdp) in subscribers {
            self.detach_subscription(&pc, &out, &gathered, &sdp, pub_id)
                .await;
        }
        let mut done = life.done.clone();
        let _ = done.wait_for(|done| *done).await;
    }

    async fn detach_subscription(
        &self,
        pc: &Arc<dyn PeerConnection>,
        out: &mpsc::UnboundedSender<ServerFrame>,
        gathered: &watch::Receiver<u64>,
        sdp: &Arc<Mutex<PeerSdp>>,
        id: &str,
    ) {
        let mut gate = sdp.lock().await;
        if let Some(SubscriptionState::Active(mut sub)) = gate.subscriptions.remove(id) {
            sub.task.abort();
            let _ = (&mut sub.task).await;
            if let Err(err) = pc.remove_track(&sub.sender).await {
                warn!(error = %err, "remove subscriber sender failed");
            }
            gate.dirty = true;
        }
        self.negotiate_locked(pc, out, gathered, &mut gate).await;
    }

    async fn attach_publication(&self, channel_id: Uuid, publication: &Published) {
        let Some(room) = self.find_room(channel_id).await else {
            return;
        };
        let jobs = {
            let room = room.lock().await;
            room.peers
                .iter()
                .filter(|(id, p)| **id != publication.publisher && p.receives(publication))
                .map(|(_, p)| Forward {
                    pc: p.pc.clone(),
                    out: p.out.clone(),
                    gathered: p.gathered.clone(),
                    sdp: p.sdp.clone(),
                    publication: publication.clone(),
                })
                .collect::<Vec<_>>()
        };
        for job in jobs {
            self.forward_to(job).await;
        }
    }

    async fn attach_existing_pubs(&self, subscriber: PeerId, channel_id: Uuid) {
        let Some(room) = self.find_room(channel_id).await else {
            return;
        };
        let jobs = {
            let room = room.lock().await;
            let Some(peer) = room.peers.get(&subscriber) else {
                return;
            };
            room.pubs
                .values()
                .filter(|p| p.publisher != subscriber && peer.receives(p))
                .map(|p| Forward {
                    pc: peer.pc.clone(),
                    out: peer.out.clone(),
                    gathered: peer.gathered.clone(),
                    sdp: peer.sdp.clone(),
                    publication: p.clone(),
                })
                .collect::<Vec<_>>()
        };
        for job in jobs {
            self.forward_to(job).await;
        }
    }

    async fn forward_to(&self, job: Forward) {
        let mut gate = job.sdp.lock().await;
        if gate.closed || *gate.closing.borrow() || *job.publication.life.stop.borrow() {
            return;
        }
        gate.subscriptions
            .entry(job.publication.id.clone())
            .or_insert(SubscriptionState::Pending(job.publication));
        self.negotiate_locked(&job.pc, &job.out, &job.gathered, &mut gate)
            .await;
    }

    /// Every PC mutation, including rollback, runs while the same SDP lock is held.
    async fn negotiate_locked(
        &self,
        pc: &Arc<dyn PeerConnection>,
        out: &mpsc::UnboundedSender<ServerFrame>,
        gathered: &watch::Receiver<u64>,
        gate: &mut PeerSdp,
    ) {
        if gate.closed || *gate.closing.borrow() || !gate.negotiated || gate.have_local_offer {
            return;
        }
        let queued = gate
            .subscriptions
            .iter()
            .filter_map(|(id, state)| {
                matches!(state, SubscriptionState::Pending(_)).then_some(id.clone())
            })
            .collect::<Vec<_>>();
        for id in queued {
            let Some(SubscriptionState::Pending(publication)) = gate.subscriptions.remove(&id)
            else {
                continue;
            };
            if *publication.life.stop.borrow() {
                continue;
            }
            let ssrc = rand::random::<u32>();
            let local = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
                publication.stream_id.clone(),
                format!("{}-{ssrc}", publication.stream_id),
                format!("gelabber-{id}"),
                publication.kind,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(ssrc),
                        ..Default::default()
                    },
                    codec: publication.codec.clone(),
                    ..Default::default()
                }],
            )));
            let sender = match pc.add_track(local.clone() as Arc<dyn TrackLocal>).await {
                Ok(sender) => sender,
                Err(err) => {
                    warn!(error = %err, "add subscriber track failed");
                    continue;
                }
            };
            limit_forward_codec(pc, &sender, &publication.codec).await;
            let (payload_type, binding) = watch::channel(None);
            let codec = publication.codec.clone();
            let task = spawn_forwarder(
                local,
                ssrc,
                publication,
                binding,
                gate.closing.subscribe(),
                self.stats.clone(),
            );
            gate.subscriptions.insert(
                id.clone(),
                SubscriptionState::Active(Subscription {
                    sender,
                    task,
                    codec,
                    payload_type,
                }),
            );
            gate.offered.insert(id);
            gate.dirty = true;
        }
        if !gate.dirty {
            return;
        }
        gate.have_local_offer = true;
        let result = async {
            let offer = pc.create_offer(None).await?;
            let before = *gathered.borrow();
            pc.set_local_description(offer).await?;
            let local = local_sdp_after_gather(pc, gathered, before, &gate.closing)
                .await
                .ok_or(webrtc::error::Error::ErrUnknownType)?;
            out.send(ServerFrame::Offer { sdp: local })
                .map_err(|_| webrtc::error::Error::ErrUnknownType)?;
            Ok::<(), webrtc::error::Error>(())
        }
        .await;
        if let Err(err) = result {
            warn!(error = %err, "subscriber offer failed");
            self.rollback_locked(pc, gate).await;
        } else {
            gate.dirty = false;
        }
    }

    async fn rollback_locked(&self, pc: &Arc<dyn PeerConnection>, gate: &mut PeerSdp) {
        if pc.pending_local_description().await.is_some() {
            match RTCSessionDescription::rollback(None) {
                Ok(rollback) => {
                    if let Err(err) = pc.set_local_description(rollback).await {
                        warn!(error = %err, "subscriber rollback failed");
                        // Do not unlock negotiation as stable when rollback failed.
                        return;
                    }
                }
                Err(_) => return,
            }
        }
        for id in std::mem::take(&mut gate.offered) {
            if let Some(SubscriptionState::Active(mut sub)) = gate.subscriptions.remove(&id) {
                sub.task.abort();
                let _ = (&mut sub.task).await;
                let _ = pc.remove_track(&sub.sender).await;
            }
        }
        gate.have_local_offer = false;
        gate.pending_ice.clear();
        // Keep removals of already bound sources dirty; the next offer must carry them.
    }

    async fn fail_local_offer(
        &self,
        pc: &Arc<dyn PeerConnection>,
        out: &mpsc::UnboundedSender<ServerFrame>,
        gathered: &watch::Receiver<u64>,
        sdp: &Arc<Mutex<PeerSdp>>,
        only_if_outstanding: bool,
    ) {
        let mut gate = sdp.lock().await;
        if gate.closed || *gate.closing.borrow() || (only_if_outstanding && !gate.have_local_offer)
        {
            return;
        }
        self.rollback_locked(pc, &mut gate).await;
        self.negotiate_locked(pc, out, gathered, &mut gate).await;
    }
}

fn ice_init(ice: String, mid: Option<String>) -> RTCIceCandidateInit {
    let mid = mid.filter(|m| !m.is_empty());
    RTCIceCandidateInit {
        candidate: ice,
        sdp_mid: mid.clone(),
        sdp_mline_index: if mid.is_some() { None } else { Some(0) },
        username_fragment: None,
        url: None,
    }
}

async fn flush_ice(
    pc: &Arc<dyn PeerConnection>,
    peer_id: PeerId,
    queued: Vec<(String, Option<String>)>,
) {
    for (ice, mid) in queued {
        if let Err(err) = pc.add_ice_candidate(ice_init(ice, mid)).await {
            warn!(peer = %peer_id.0, error = %err, "buffered ice dropped");
        }
    }
}

async fn limit_forward_codec(
    pc: &Arc<dyn PeerConnection>,
    sender: &Arc<dyn RtpSender>,
    codec: &RTCRtpCodec,
) {
    limit_forward_codec_with_pt(pc, sender, codec, 0).await;
}

async fn limit_forward_codec_with_pt(
    pc: &Arc<dyn PeerConnection>,
    sender: &Arc<dyn RtpSender>,
    codec: &RTCRtpCodec,
    payload_type: u8,
) {
    let preference = RTCRtpCodecParameters {
        rtp_codec: codec.clone(),
        payload_type,
    };
    for transceiver in pc.get_transceivers().await {
        if !transceiver
            .sender()
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.id() == sender.id())
        {
            continue;
        }
        if let Err(err) = transceiver
            .set_codec_preferences(vec![preference.clone()])
            .await
        {
            warn!(
                error = %err,
                mime = %codec.mime_type,
                "forward codec preference skipped"
            );
        }
    }
}

async fn sender_mid(pc: &Arc<dyn PeerConnection>, sender: &Arc<dyn RtpSender>) -> Option<String> {
    for transceiver in pc.get_transceivers().await {
        if transceiver
            .sender()
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.id() == sender.id())
        {
            return transceiver.mid().await.ok().flatten();
        }
    }
    None
}

/// Resolve only the sender's MID and the publication codec in the actual SDP.
/// Sender get_parameters() can expose preferences rather than the negotiated
/// binding after a reoffer in the pinned library. TrackLocalContext is private.
fn negotiated_payload_type(sdp: &str, mid: &str, codec: &RTCRtpCodec) -> Option<u8> {
    let mut parsed = RTCSessionDescription::answer(sdp.to_owned())
        .ok()?
        .unmarshal()
        .ok()?;
    parsed.media_descriptions.retain(|media| {
        media.media_name.port.value != 0
            && media
                .attributes
                .iter()
                .any(|a| a.key == "mid" && a.value.as_deref() == Some(mid))
            && !media.attributes.iter().any(|a| a.key == "inactive")
    });
    let [media] = parsed.media_descriptions.as_slice() else {
        return None;
    };
    let mut candidates = Vec::new();
    for format in &media.media_name.formats {
        let Ok(pt) = format.parse::<u8>() else {
            continue;
        };
        let Ok(bound) = parsed.get_codec_for_payload_type(pt) else {
            continue;
        };
        let mime = format!("{}/{}", media.media_name.media, bound.name);
        let channels = bound.encoding_parameters.parse::<u16>().unwrap_or(0);
        if mime.eq_ignore_ascii_case(&codec.mime_type)
            && bound.clock_rate == codec.clock_rate
            && channels == codec.channels
        {
            candidates.push((pt, bound.fmtp));
        }
    }
    let normalize = |fmtp: &str| {
        let mut params = fmtp
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        params.sort_unstable();
        params.join(";")
    };
    let exact = candidates
        .iter()
        .filter(|(_, fmtp)| normalize(fmtp) == normalize(&codec.sdp_fmtp_line))
        .collect::<Vec<_>>();
    if let [matched] = exact.as_slice() {
        return Some(matched.0);
    }
    // Opus fmtp is optional receiver tuning, not a different bitstream profile.
    if codec.mime_type.eq_ignore_ascii_case(MIME_TYPE_OPUS) && candidates.len() == 1 {
        return Some(candidates[0].0);
    }
    None
}

fn remap_answer_payload(
    answer: RTCSessionDescription,
    mid: &str,
    from: u8,
    to: u8,
) -> Result<RTCSessionDescription, webrtc::error::Error> {
    let mut parsed = answer.unmarshal()?;
    for media in &mut parsed.media_descriptions {
        if !media
            .attributes
            .iter()
            .any(|a| a.key == "mid" && a.value.as_deref() == Some(mid))
        {
            continue;
        }
        for format in &mut media.media_name.formats {
            if format == &from.to_string() {
                *format = to.to_string();
            }
        }
        for attribute in &mut media.attributes {
            if matches!(attribute.key.as_str(), "rtpmap" | "fmtp" | "rtcp-fb")
                && let Some(value) = &mut attribute.value
                && let Some((pt, rest)) = value.split_once(' ')
                && pt == from.to_string()
            {
                *value = format!("{to} {rest}");
            }
        }
    }
    RTCSessionDescription::answer(parsed.marshal())
}

async fn refresh_subscriber_bindings(
    pc: &Arc<dyn PeerConnection>,
    gate: &PeerSdp,
    answer: Option<&RTCSessionDescription>,
) {
    for state in gate.subscriptions.values() {
        let SubscriptionState::Active(sub) = state else {
            continue;
        };
        let pt = match (answer, sender_mid(pc, &sub.sender).await) {
            (Some(answer), Some(mid)) => negotiated_payload_type(&answer.sdp, &mid, &sub.codec),
            _ => None,
        };
        // The pinned core can overwrite an existing sender's codec PT with
        // another leg's mapping while setting a local answer. Reconcile that
        // sender to its own accepted MID/codec before allowing queued RTP.
        let pt = if let Some(pt) = pt {
            limit_forward_codec_with_pt(pc, &sub.sender, &sub.codec, pt).await;
            sub.sender
                .get_parameters()
                .await
                .ok()
                .and_then(|parameters| {
                    parameters
                        .rtp_parameters
                        .codecs
                        .iter()
                        .any(|codec| {
                            codec.payload_type == pt
                                && codec
                                    .rtp_codec
                                    .mime_type
                                    .eq_ignore_ascii_case(&sub.codec.mime_type)
                        })
                        .then_some(pt)
                })
        } else {
            None
        };
        sub.payload_type.send_replace(pt);
    }
}

/// Sending video MSIDs in SDP order, excluding rejected/recvonly/inactive sections.
fn video_sources(sdp: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let sections: Vec<Vec<_>> = sdp
        .split("\nm=")
        .skip(1)
        .map(|section| section.lines().map(str::trim).collect())
        .collect();
    let bundles: Vec<Vec<_>> = sdp
        .lines()
        .filter_map(|line| line.trim().strip_prefix("a=group:BUNDLE "))
        .map(|group| group.split_whitespace().collect())
        .collect();
    for section in sdp.split("\nm=").skip(1) {
        let mut lines = section.lines().map(str::trim);
        let Some(media) = lines.next() else {
            continue;
        };
        if !media.starts_with("video ") {
            continue;
        }
        let lines = lines.collect::<Vec<_>>();
        if media.split_whitespace().nth(1) == Some("0") {
            let mid = lines.iter().find_map(|line| line.strip_prefix("a=mid:"));
            let bundled = lines.contains(&"a=bundle-only")
                && mid.is_some_and(|mid| {
                    bundles.iter().any(|group| {
                        group.contains(&mid)
                            && group.first().is_some_and(|master| {
                                sections.iter().any(|section| {
                                    section
                                        .iter()
                                        .any(|line| line.strip_prefix("a=mid:") == Some(*master))
                                        && section
                                            .first()
                                            .and_then(|media| media.split_whitespace().nth(1))
                                            != Some("0")
                                })
                            })
                    })
                });
            if !bundled {
                continue;
            }
        }
        if lines
            .iter()
            .any(|l| matches!(*l, "a=recvonly" | "a=inactive"))
        {
            continue;
        }
        for line in lines {
            let track = if let Some(msid) = line.strip_prefix("a=msid:") {
                msid.split_whitespace().nth(1)
            } else if line.starts_with("a=ssrc:") {
                line.split_once(" msid:")
                    .and_then(|(_, msid)| msid.split_whitespace().nth(1))
            } else {
                None
            };
            if let Some(id) = track
                && !sources.iter().any(|s| s == id)
            {
                sources.push(id.into());
            }
        }
    }
    sources
}

fn spawn_forwarder(
    local: Arc<TrackLocalStaticRTP>,
    ssrc: u32,
    publication: Published,
    mut binding: watch::Receiver<Option<u8>>,
    mut closing: watch::Receiver<bool>,
    stats: Arc<SfuStats>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut packets = publication.packets.subscribe();
        let mut stopped = publication.life.stop.subscribe();
        let mut rtcp_open = true;
        loop {
            if *stopped.borrow() || *closing.borrow() || live_expired(&publication.live_deadline) {
                break;
            }
            tokio::select! {
                biased;
                _ = stopped.changed() => break,
                _ = closing.changed() => break,
                _ = tokio::time::sleep_until(live_wakeup(&publication.live_deadline)), if publication.live_deadline.is_some() => {},
                result = binding.changed() => {
                    if result.is_err() { break; }
                    if binding.borrow().is_some() {
                        rtcp_open = true;
                        if let Some(kf) = &publication.keyframe { let _ = kf.send(()); }
                    }
                }
                event = local.poll(), if binding.borrow().is_some() && rtcp_open && publication.keyframe.is_some() => {
                    match event {
                        Some(TrackLocalEvent::OnRtcpPacket(pkts)) if asks_keyframe(&pkts) => {
                            debug!(source = %publication.stream_id, "subscriber keyframe feedback");
                            if let Some(kf) = &publication.keyframe { let _ = kf.send(()); }
                        }
                        None => rtcp_open = false,
                        _ => {}
                    }
                }
                packet = packets.recv() => {
                    match packet {
                        Ok(packet) => {
                            if live_expired(&publication.live_deadline) { break; }
                            // PT belongs to the negotiated subscriber leg, not
                            // the incoming publisher packet. Never enqueue RTP
                            // before the corresponding SDP answer was accepted.
                            let payload_type = *binding.borrow();
                            let Some(payload_type) = payload_type else { continue; };
                            let n = packet.payload.len() as u64;
                            let mut packet = prepare_forwarded_rtp(packet, ssrc);
                            packet.header.payload_type = payload_type;
                            if local.write_rtp(packet).await.is_ok() {
                                stats.forwarded_bytes.fetch_add(n, Ordering::Relaxed);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {},
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    })
}

fn handle_publisher_event(
    packets: &broadcast::Sender<rtp::Packet>,
    stats: &SfuStats,
    rtp: &mut PublisherRtpStats,
    evt: Option<TrackRemoteEvent>,
) -> bool {
    match evt {
        Some(TrackRemoteEvent::OnRtpPacket(packet)) => {
            rtp.observe(&packet, stats);
            let _ = packets.send(packet);
            true
        }
        Some(TrackRemoteEvent::OnEnded | TrackRemoteEvent::OnError) | None => false,
        Some(_) => true,
    }
}

/// How far behind the highest sequence a packet can still fill a gap.
/// Gaps that age out of this window are counted as loss. Reordering inside
/// the window is not.
const RTP_REORDER_WINDOW: u16 = 64;

#[derive(Debug)]
struct PublisherRtpStats {
    clock_rate: f64,
    /// Highest sequence observed. Late packets do not move this backward.
    max_seq: Option<u16>,
    /// Sequences behind `max_seq` that have not arrived yet, still inside
    /// [`RTP_REORDER_WINDOW`].
    missing: HashSet<u16>,
    last_arrival: Option<Instant>,
    last_timestamp: Option<u32>,
    jitter_seconds: f64,
}

impl PublisherRtpStats {
    fn new(clock_rate: u32) -> Self {
        Self {
            clock_rate: clock_rate as f64,
            max_seq: None,
            missing: HashSet::new(),
            last_arrival: None,
            last_timestamp: None,
            jitter_seconds: 0.0,
        }
    }

    fn observe(&mut self, packet: &rtp::Packet, stats: &SfuStats) {
        stats.rtp_packets_total.fetch_add(1, Ordering::Relaxed);
        self.observe_sequence(packet.header.sequence_number, stats);
        self.observe_jitter(packet.header.timestamp, stats);
    }

    /// Wrap-safe high-water mark. A packet behind the high-water mark fills
    /// a provisional gap; it is loss only after it leaves the reorder window.
    fn observe_sequence(&mut self, seq: u16, stats: &SfuStats) {
        let Some(max_seq) = self.max_seq else {
            self.max_seq = Some(seq);
            return;
        };
        if seq == max_seq {
            return;
        }
        let ahead = seq.wrapping_sub(max_seq);
        if ahead > 0 && ahead < 0x8000 {
            let gap = ahead - 1;
            if gap > RTP_REORDER_WINDOW {
                let immediate = u64::from(gap - RTP_REORDER_WINDOW);
                stats.rtp_lost_total.fetch_add(immediate, Ordering::Relaxed);
                self.expire_missing(seq, stats);
                for behind in 1..=RTP_REORDER_WINDOW {
                    self.missing.insert(seq.wrapping_sub(behind));
                }
            } else {
                let mut cursor = max_seq.wrapping_add(1);
                while cursor != seq {
                    self.missing.insert(cursor);
                    cursor = cursor.wrapping_add(1);
                }
                self.expire_missing(seq, stats);
            }
            self.max_seq = Some(seq);
            return;
        }
        let behind = max_seq.wrapping_sub(seq);
        if behind == 0 || behind > RTP_REORDER_WINDOW {
            return;
        }
        self.missing.remove(&seq);
    }

    fn expire_missing(&mut self, max_seq: u16, stats: &SfuStats) {
        let mut lost = 0u64;
        self.missing.retain(|seq| {
            let behind = max_seq.wrapping_sub(*seq);
            if behind > RTP_REORDER_WINDOW {
                lost += 1;
                false
            } else {
                true
            }
        });
        if lost > 0 {
            stats.rtp_lost_total.fetch_add(lost, Ordering::Relaxed);
        }
    }

    /// RFC 3550 interarrival jitter. Timestamp deltas stay signed so a late
    /// packet is a small negative step, including across the 32-bit wrap.
    fn observe_jitter(&mut self, ts: u32, stats: &SfuStats) {
        if self.clock_rate <= 0.0 {
            return;
        }
        let now = Instant::now();
        if let (Some(last_arrival), Some(last_timestamp)) = (self.last_arrival, self.last_timestamp)
        {
            let arrival_delta = now.saturating_duration_since(last_arrival).as_secs_f64();
            let rtp_ticks = ts.wrapping_sub(last_timestamp) as i32;
            let rtp_delta = f64::from(rtp_ticks) / self.clock_rate;
            let d = (arrival_delta - rtp_delta).abs();
            self.jitter_seconds += (d - self.jitter_seconds) / 16.0;
            let jitter_micros = (self.jitter_seconds * 1_000_000.0).round().max(0.0) as u64;
            stats
                .rtp_jitter_micros
                .store(jitter_micros, Ordering::Relaxed);
        }
        self.last_arrival = Some(now);
        self.last_timestamp = Some(ts);
    }
}

async fn request_keyframe(track: &Arc<dyn TrackRemote>, media_ssrc: u32) {
    debug!(media_ssrc, "request publisher keyframe");
    let pli = PictureLossIndication {
        sender_ssrc: 0,
        media_ssrc,
    };
    if track.write_rtcp(vec![Box::new(pli)]).await.is_err() {
        debug!(media_ssrc, "pli to publisher failed");
    }
}

fn asks_keyframe(packets: &[Box<dyn rtcp::Packet>]) -> bool {
    packets.iter().any(|packet| {
        packet
            .as_any()
            .downcast_ref::<PictureLossIndication>()
            .is_some()
            || packet.as_any().downcast_ref::<FullIntraRequest>().is_some()
    })
}

/// Register SFU codecs: Opus-only audio, full default video set from rtc 0.20.5.
///
/// webrtc-rs 0.20 `set_codec_preferences_from_remote_description` walks the
/// remote offer codecs with `.rev()` and pushes matches, which *reverses*
/// preference order. Chrome offers `111 … 8` (Opus first); the SFU answer
/// became `m=audio … 8 0 9 111` (PCMA first) and stats showed audio/PCMA
/// ~64 kbps — root cause of #53. Omitting PCMA/PCMU/G722 makes the reversed
/// list still Opus-only. Video registrations mirror `MediaEngine::register_default_codecs`
/// (VP8/VP9/H264 variants/AV1/H265 + RTX) so camera/screenshare parity is kept.
fn register_sfu_codecs(media: &mut MediaEngine) -> webrtc::error::Result<()> {
    media.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1;usedtx=1".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: 111,
        },
        RtpCodecKind::Audio,
    )?;

    let video_rtcp_feedback = vec![
        RTCPFeedback {
            typ: "goog-remb".to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "ccm".to_owned(),
            parameter: "fir".to_owned(),
        },
        RTCPFeedback {
            typ: "nack".to_owned(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "nack".to_owned(),
            parameter: "pli".to_owned(),
        },
    ];

    // Mirror rtc 0.20.5 MediaEngine::register_default_codecs video + RTX PTs.
    let rtx = |payload_type: u8, apt: u8| RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_RTX.to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: format!("apt={apt}"),
            rtcp_feedback: vec![],
        },
        payload_type,
    };

    for codec in [
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_VP8.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: String::new(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 96,
        },
        rtx(97, 96),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_VP9.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "profile-id=0".to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 98,
        },
        rtx(99, 98),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_VP9.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "profile-id=1".to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 100,
        },
        rtx(101, 100),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"
                        .to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 102,
        },
        rtx(103, 102),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f"
                        .to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 127,
        },
        rtx(104, 127),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                        .to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 125,
        },
        rtx(105, 125),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f"
                        .to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 108,
        },
        rtx(109, 108),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f"
                        .to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 127,
        },
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032"
                        .to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 123,
        },
        rtx(124, 123),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_AV1.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "profile-id=0".to_owned(),
                rtcp_feedback: video_rtcp_feedback.clone(),
            },
            payload_type: 41,
        },
        rtx(106, 41),
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_HEVC.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: String::new(),
                rtcp_feedback: video_rtcp_feedback,
            },
            payload_type: 126,
        },
        rtx(107, 126),
    ] {
        media.register_codec(codec, RtpCodecKind::Video)?;
    }
    Ok(())
}

/// Chrome always attaches `mid` / transport-cc / audio-level using the
/// *publisher* extmap ids. webrtc 0.20 `write_rtp` rejects a packet if any
/// extension id is not negotiated on this sender — which they are not, on
/// the subscriber leg. Strip them; SSRC is rewritten for the new track.
fn prepare_forwarded_rtp(mut packet: rtp::Packet, ssrc: u32) -> rtp::Packet {
    packet.header.ssrc = ssrc;
    packet.header.csrc.clear();
    for id in packet.header.get_extension_ids() {
        let _ = packet.header.del_extension(id);
    }
    if packet.header.extensions.is_empty() {
        packet.header.extension = false;
        packet.header.extensions_padding = 0;
    }
    packet
}

fn saturating_dec(atom: &AtomicU64) {
    let mut current = atom.load(Ordering::Relaxed);
    while current > 0 {
        match atom.compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return,
            Err(seen) => current = seen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MIME_TYPE_OPUS, RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters,
        RtpCodecKind, Sfu, prepare_forwarded_rtp,
    };
    use rtc::rtp::packet::Packet;
    use std::sync::Arc;

    #[test]
    fn bundle_only_video_uses_its_live_bundle_transport() {
        let sdp = "v=0\r\na=group:BUNDLE audio camera screen removed\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:audio\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:camera\r\na=bundle-only\r\na=sendrecv\r\na=msid:camera-stream camera-track\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:screen\r\na=bundle-only\r\na=sendonly\r\na=msid:screen-stream screen-track\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:removed\r\na=msid:removed-stream removed-track\r\n";
        assert_eq!(super::video_sources(sdp), ["camera-track", "screen-track"]);
        assert!(
            super::video_sources(
                &sdp.replace("a=group:BUNDLE audio camera screen removed\r\n", "")
            )
            .is_empty()
        );
        assert!(super::video_sources(&sdp.replace("m=audio 9", "m=audio 0")).is_empty());
        assert_eq!(
            super::video_sources(&sdp.replace("a=sendrecv", "a=recvonly")),
            ["screen-track"]
        );
    }

    #[test]
    fn payload_binding_uses_exact_mid_and_codec_profile() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 98 99\r\na=mid:first\r\na=rtpmap:96 VP8/90000\r\na=rtpmap:98 VP9/90000\r\na=fmtp:98 profile-id=0\r\na=rtpmap:99 VP9/90000\r\na=fmtp:99 profile-id=2\r\nm=video 9 UDP/TLS/RTP/SAVPF 41 42\r\na=mid:second\r\na=rtpmap:41 VP9/90000\r\na=fmtp:41 profile-id=2\r\na=rtpmap:42 VP9/90000\r\na=fmtp:42 profile-id=0\r\n";
        let codec = RTCRtpCodec {
            mime_type: "video/VP9".into(),
            clock_rate: 90000,
            sdp_fmtp_line: "profile-id=2".into(),
            ..Default::default()
        };
        assert_eq!(
            super::negotiated_payload_type(sdp, "first", &codec),
            Some(99)
        );
        assert_eq!(
            super::negotiated_payload_type(sdp, "second", &codec),
            Some(41)
        );
        assert_eq!(super::negotiated_payload_type(sdp, "missing", &codec), None);
        let wrong = RTCRtpCodec {
            sdp_fmtp_line: "profile-id=3".into(),
            ..codec.clone()
        };
        assert_eq!(super::negotiated_payload_type(sdp, "first", &wrong), None);
        assert_eq!(
            super::negotiated_payload_type(
                &sdp.replace("m=video 9", "m=video 0"),
                "second",
                &codec
            ),
            None
        );
        let remapped = super::remap_answer_payload(
            webrtc::peer_connection::RTCSessionDescription::answer(sdp.to_owned()).unwrap(),
            "second",
            41,
            43,
        )
        .unwrap();
        assert_eq!(
            super::negotiated_payload_type(&remapped.sdp, "first", &codec),
            Some(99)
        );
        assert_eq!(
            super::negotiated_payload_type(&remapped.sdp, "second", &codec),
            Some(43)
        );
        assert!(remapped.sdp.contains("a=fmtp:43 profile-id=2"));
        assert!(remapped.sdp.contains("a=rtpmap:42 VP9/90000"));
        let reoffer = sdp.replace("41", "43");
        assert_eq!(
            super::negotiated_payload_type(&reoffer, "second", &codec),
            Some(43)
        );
    }

    struct NoopHandler;

    #[async_trait::async_trait]
    impl webrtc::peer_connection::PeerConnectionEventHandler for NoopHandler {}

    /// Regression for #53: Chrome-like offer lists Opus then G.711; SFU answer
    /// must still prefer Opus (first PT), not PCMA/PCMU/G722.
    #[tokio::test]
    async fn sfu_answer_prefers_opus_when_offer_lists_pcma() {
        use crate::config::Config;
        use crate::protocol::ServerFrame;
        use crate::ticket::TicketClaim;
        use rtc::media_stream::MediaStreamTrack;
        use tokio::sync::mpsc;
        use uuid::Uuid;
        use webrtc::media_stream::track_local::TrackLocal;
        use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
        use webrtc::peer_connection::{
            MediaEngine, PeerConnection, PeerConnectionBuilder, RTCSessionDescription,
            register_default_interceptors,
        };

        let config = Config::from_source(|key| match key {
            "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
            "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
            "TURN_URLS" => Some("stun:127.0.0.1:3478".to_owned()),
            "TURN_USERNAME" => Some("gelabber".to_owned()),
            "TURN_PASSWORD" => Some("gelabberturn".to_owned()),
            _ => None,
        })
        .unwrap();
        let sfu = Arc::new(Sfu::new(&config));
        let (out, mut rx) = mpsc::unbounded_channel();
        let channel = Uuid::from_u128(53);
        let peer = sfu
            .join(
                TicketClaim {
                    u: Uuid::from_u128(1),
                    s: Uuid::from_u128(9),
                    c: channel,
                    g: true,
                },
                out,
            )
            .await
            .expect("join");

        // Client MediaEngine includes PCMA/PCMU/G722 (Chrome-like).
        let mut media = MediaEngine::default();
        media.register_default_codecs().unwrap();
        let registry =
            register_default_interceptors(webrtc::peer_connection::Registry::new(), &mut media)
                .unwrap();
        let pc = PeerConnectionBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(NoopHandler))
            .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
            .build()
            .await
            .unwrap();
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            "stream".into(),
            "audio".into(),
            "mic".into(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(0x1111_0001),
                    ..Default::default()
                },
                codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: 48000,
                    channels: 2,
                    sdp_fmtp_line: String::new(),
                    rtcp_feedback: vec![],
                },
                ..Default::default()
            }],
        )));
        pc.add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
            .await
            .unwrap();
        let offer = pc.create_offer(None).await.unwrap();
        // Track encodings are Opus-only, so inject Chrome-like G.711 PTs into the
        // offer SDP (order: Opus first, then PCMA/PCMU/G722) — the #53 shape.
        let offer_sdp = inject_g711_after_opus(&offer.sdp);
        let offer_audio = offer_sdp
            .lines()
            .find(|l| l.starts_with("m=audio"))
            .expect("client m=audio");
        assert!(
            offer_audio.contains(" 8") && offer_audio.contains(" 0"),
            "precondition: Chrome-like offer must list G.711: {offer_audio}"
        );

        pc.set_local_description(offer).await.unwrap();
        sfu.apply_remote(peer, channel, offer_sdp, true)
            .await
            .expect("apply offer");

        let answer_sdp = loop {
            match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
                Ok(Some(ServerFrame::Answer { sdp })) => break sdp,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("sfu closed before answer"),
                Err(_) => panic!("timeout waiting for SFU answer"),
            }
        };
        let audio_line = answer_sdp
            .lines()
            .find(|l| l.starts_with("m=audio"))
            .expect("answer m=audio");
        let pts: Vec<&str> = audio_line.split_whitespace().skip(3).collect();
        assert_eq!(
            pts.first().copied(),
            Some("111"),
            "Opus must be first in SFU answer: {audio_line}"
        );
        assert!(
            !pts.iter().any(|p| *p == "8" || *p == "0" || *p == "9"),
            "G.711/G.722 must not appear in SFU answer: {audio_line}"
        );

        pc.set_remote_description(RTCSessionDescription::answer(answer_sdp).unwrap())
            .await
            .expect("client accepts SFU answer");
    }

    /// Chrome offers Opus then static G.711/G.722; webrtc-rs track encodings with
    /// Opus-only omit them from create_offer, so splice them in for the regression.
    fn inject_g711_after_opus(sdp: &str) -> String {
        let mut out = Vec::new();
        let mut injected = false;
        for line in sdp.lines() {
            if let Some(rest) = line.strip_prefix("m=audio ") {
                let mut parts: Vec<&str> = rest.split_whitespace().collect();
                // m=audio <port> <proto> <pts...>
                if parts.len() >= 3 {
                    let mut pts: Vec<&str> = parts[3..].to_vec();
                    for pt in ["8", "0", "9"] {
                        if !pts.contains(&pt) {
                            pts.push(pt);
                        }
                    }
                    parts.truncate(3);
                    parts.extend(pts);
                    out.push(format!("m=audio {}", parts.join(" ")));
                    injected = true;
                    continue;
                }
            }
            out.push(line.to_owned());
            if injected && line.starts_with("a=rtpmap:111") {
                out.push("a=rtpmap:8 PCMA/8000".to_owned());
                out.push("a=rtpmap:0 PCMU/8000".to_owned());
                out.push("a=rtpmap:9 G722/8000".to_owned());
                injected = false;
            }
        }
        out.join("\n") + "\n"
    }

    #[test]
    fn strips_publisher_header_extensions() {
        let mut packet = Packet::default();
        packet.header.version = 2;
        packet.header.ssrc = 0x1111_0001;
        packet.header.payload_type = 111;
        packet
            .header
            .set_extension(1, bytes::Bytes::from_static(b"0"))
            .expect("mid");
        packet
            .header
            .set_extension(3, bytes::Bytes::from_static(&[0x01, 0x02]))
            .expect("transport-cc");
        assert!(packet.header.extension);
        assert!(!packet.header.get_extension_ids().is_empty());

        let out = prepare_forwarded_rtp(packet, 0x2222_0002);
        assert_eq!(out.header.ssrc, 0x2222_0002);
        assert_eq!(out.header.payload_type, 111);
        assert!(!out.header.extension);
        assert!(out.header.get_extension_ids().is_empty());
        assert!(out.header.csrc.is_empty());
    }

    #[tokio::test]
    async fn ice_ports_return_instead_of_wrapping() {
        use crate::config::Config;

        let config = Config::from_source(|key| match key {
            "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
            "MEDIA_ICE_BIND" => Some("127.0.0.1:40000".to_owned()),
            "MEDIA_ICE_PORT_MAX" => Some("40001".to_owned()),
            _ => None,
        })
        .unwrap();
        let sfu = Sfu::new(&config);
        let first = sfu.ice_ports.take().await.expect("port");
        let second = sfu.ice_ports.take().await.expect("port");
        assert!(sfu.ice_ports.take().await.is_none(), "pool must not wrap");
        sfu.ice_ports.release(&first).await;
        assert_eq!(sfu.ice_ports.take().await.as_deref(), Some(first.as_str()));
        let _ = second;
    }

    fn rtp_packet(seq: u16, timestamp: u32) -> rtc::rtp::packet::Packet {
        let mut packet = rtc::rtp::packet::Packet::default();
        packet.header.sequence_number = seq;
        packet.header.timestamp = timestamp;
        packet
    }

    #[test]
    fn reordering_does_not_count_as_loss() {
        use std::sync::atomic::Ordering;

        let stats = super::SfuStats::default();
        let mut rtp = super::PublisherRtpStats::new(48_000);
        // 100, 102, 101, 103 is fully delivered. The old high-water reset
        // counted two losses.
        for (seq, ts) in [(100u16, 100u32), (102, 102), (101, 101), (103, 103)] {
            rtp.observe(&rtp_packet(seq, ts), &stats);
        }
        assert_eq!(stats.rtp_lost_total.load(Ordering::Relaxed), 0);
        assert_eq!(stats.rtp_packets_total.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn sequence_wrap_and_aged_gap() {
        use std::sync::atomic::Ordering;

        let stats = super::SfuStats::default();
        let mut rtp = super::PublisherRtpStats::new(48_000);
        for seq in [65534u16, 0, 65535, 1] {
            rtp.observe(&rtp_packet(seq, 0), &stats);
        }
        assert_eq!(stats.rtp_lost_total.load(Ordering::Relaxed), 0);

        let mut rtp = super::PublisherRtpStats::new(48_000);
        rtp.observe(&rtp_packet(100, 0), &stats);
        rtp.observe(&rtp_packet(102, 0), &stats);
        let aged = 102u16
            .wrapping_add(super::RTP_REORDER_WINDOW)
            .wrapping_add(1);
        rtp.observe(&rtp_packet(aged, 0), &stats);
        assert_eq!(stats.rtp_lost_total.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn late_audio_timestamp_does_not_explode_jitter() {
        use std::sync::atomic::Ordering;

        let stats = super::SfuStats::default();
        let mut rtp = super::PublisherRtpStats::new(48_000);
        // One 20 ms frame late: 960 ticks at 48 kHz is -0.02 s, not ~89478 s.
        rtp.observe(&rtp_packet(1, 1920), &stats);
        rtp.observe(&rtp_packet(2, 960), &stats);
        let micros = stats.rtp_jitter_micros.load(Ordering::Relaxed);
        assert!(
            micros < 1_000_000,
            "late packet jitter blew up to {micros} µs"
        );

        // A real timestamp wrap of one frame stays a small positive step.
        let mut rtp = super::PublisherRtpStats::new(48_000);
        let last = u32::MAX - 10;
        rtp.observe(&rtp_packet(1, last), &stats);
        rtp.observe(&rtp_packet(2, last.wrapping_add(960)), &stats);
        let wrapped = stats.rtp_jitter_micros.load(Ordering::Relaxed);
        assert!(
            wrapped < 1_000_000,
            "timestamp wrap jitter blew up to {wrapped} µs"
        );
    }
}

#[cfg(test)]
#[path = "sfu_lifecycle_tests.rs"]
mod lifecycle_tests;
