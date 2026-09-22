//! Single-node SFU: a room is a voice channel. RTP from a publisher is
//! written onto `TrackLocalStaticRTP`s of every other peer. No mesh, no
//! recording, no second node.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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

use gelabber_shared::ticket::deny_key;

use crate::config::Config;
use crate::error::SfuError;
use crate::protocol::ServerFrame;
use crate::ticket::TicketClaim;

const RTP_Q: usize = 512;

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
) -> Option<String> {
    let mut rx = gathered.clone();
    if *rx.borrow() <= before {
        let _ = tokio::time::timeout(Duration::from_secs(3), rx.wait_for(|n| *n > before)).await;
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

struct Published {
    id: String,
    stream_id: String,
    kind: RtpCodecKind,
    codec: RTCRtpCodec,
    packets: broadcast::Sender<rtp::Packet>,
    keyframe: Option<mpsc::UnboundedSender<()>>,
}

struct PendingPub {
    pub_id: String,
    stream_id: String,
    kind: RtpCodecKind,
    codec: RTCRtpCodec,
    packets: broadcast::Sender<rtp::Packet>,
    keyframe: Option<mpsc::UnboundedSender<()>>,
}

struct PeerSdp {
    /// First remote offer has been answered — safe to renegotiate.
    negotiated: bool,
    /// Local offer in flight; skip further `create_offer` until the answer.
    have_local_offer: bool,
    pending: Vec<PendingPub>,
    /// Trickle candidates that arrived while our renegotiation offer was still unanswered.
    /// Applying them immediately fails (unknown mid) and used to toast-storm the viewer.
    pending_ice: Vec<(String, Option<String>)>,
}

impl PeerSdp {
    fn new() -> Self {
        Self {
            negotiated: false,
            have_local_offer: false,
            pending: Vec::new(),
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
    /// Host UDP address taken from [`IcePorts`], returned on leave.
    ice_addr: String,
    pc: Arc<dyn PeerConnection>,
    out: mpsc::UnboundedSender<ServerFrame>,
    gathered: watch::Receiver<u64>,
    sdp: Arc<Mutex<PeerSdp>>,
    /// Camera (`v`) / screen (`s`) / live (`l`) tags for the next inbound tracks.
    next_kind: VecDeque<String>,
}

struct Room {
    peers: HashMap<PeerId, Peer>,
    pubs: HashMap<String, Published>,
}

struct Forward {
    pc: Arc<dyn PeerConnection>,
    out: mpsc::UnboundedSender<ServerFrame>,
    gathered: watch::Receiver<u64>,
    sdp: Arc<Mutex<PeerSdp>>,
    pub_id: String,
    stream_id: String,
    kind: RtpCodecKind,
    codec: RTCRtpCodec,
    packets: broadcast::Sender<rtp::Packet>,
    keyframe: Option<mpsc::UnboundedSender<()>>,
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
        if self.revoked(claim.s, claim.u).await {
            return Err(SfuError::Revoked);
        }
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

        let room = self.room(claim.c).await;
        {
            let mut room = room.lock().await;
            let first = room.peers.is_empty();
            room.peers.insert(
                peer_id,
                Peer {
                    id: peer_id,
                    user_id: claim.u,
                    channel_id: claim.c,
                    go_live: claim.g,
                    ice_addr: ice_addr.clone(),
                    pc: pc.clone(),
                    out: out.clone(),
                    gathered,
                    sdp: Arc::new(Mutex::new(PeerSdp::new())),
                    next_kind: VecDeque::new(),
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
        self.watch_revoke(peer_id, claim.c, claim.s, claim.u, out);

        Ok(peer_id)
    }

    pub async fn apply_remote(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        sdp: String,
        as_offer: bool,
    ) -> Result<(), SfuError> {
        let desc = if as_offer {
            RTCSessionDescription::offer(sdp).map_err(SfuError::negotiation)?
        } else {
            RTCSessionDescription::answer(sdp).map_err(SfuError::negotiation)?
        };
        let room = self.room(channel_id).await;
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
        if as_offer && gate.have_local_offer {
            // Impolite: our offer is in flight. The polite client answers it.
            return Ok(());
        }

        if let Err(err) = pc.set_remote_description(desc).await {
            drop(gate);
            // A rejected answer leaves this peer in have-local-offer. Clearing
            // the gate flag alone keeps every later publication, including a
            // new joiner's audio, queued forever.
            if !as_offer {
                self.fail_local_offer(&pc, &out, &gathered, &sdp, false)
                    .await;
            }
            return Err(SfuError::negotiation(err));
        }

        if as_offer {
            let answer = pc
                .create_answer(None)
                .await
                .map_err(SfuError::negotiation)?;
            let before = *gathered.borrow();
            pc.set_local_description(answer)
                .await
                .map_err(SfuError::negotiation)?;
            if let Some(local) = local_sdp_after_gather(&pc, &gathered, before).await {
                let _ = out.send(ServerFrame::Answer { sdp: local });
            }
            gate.negotiated = true;
            gate.have_local_offer = false;
            let pending = std::mem::take(&mut gate.pending);
            let queued_ice = std::mem::take(&mut gate.pending_ice);
            drop(gate);
            flush_ice(&pc, peer_id, queued_ice).await;
            self.attach_existing_pubs(peer_id, channel_id).await;
            self.flush_pending(pc, out, gathered, sdp, pending).await;
        } else {
            gate.have_local_offer = false;
            let pending = std::mem::take(&mut gate.pending);
            let queued_ice = std::mem::take(&mut gate.pending_ice);
            drop(gate);
            flush_ice(&pc, peer_id, queued_ice).await;
            self.flush_pending(pc, out, gathered, sdp, pending).await;
        }
        Ok(())
    }

    /// Tag the next inbound track(s) as camera (`v`), screen (`s`), or
    /// Go Live (`l`) so subscribers can attach the forwarded stream.
    pub async fn announce(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        kind: &str,
    ) -> Result<(), SfuError> {
        if kind != "v" && kind != "s" && kind != "l" {
            return Err(SfuError::BadAnnounce);
        }
        let room = self.room(channel_id).await;
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&peer_id).ok_or(SfuError::NotInRoom)?;
        if kind == "l" && !peer.go_live {
            return Err(SfuError::Forbidden);
        }
        peer.next_kind.push_back(kind.to_owned());
        Ok(())
    }

    /// Drop one camera/screen/live tag that never became a track. A rejected
    /// publisher offer must not label the next successful video as the failed one.
    pub async fn retract(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        kind: &str,
    ) -> Result<(), SfuError> {
        if kind != "v" && kind != "s" && kind != "l" {
            return Err(SfuError::BadAnnounce);
        }
        let room = self.room(channel_id).await;
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&peer_id).ok_or(SfuError::NotInRoom)?;
        if let Some(index) = peer.next_kind.iter().position(|item| item == kind) {
            peer.next_kind.remove(index);
            info!(peer = %peer_id.0, kind, "retracted unpublished kind");
        }
        Ok(())
    }

    /// Subscriber could not complete our offer. Roll signaling back to stable
    /// and forward anything that queued behind that offer.
    pub async fn abort_offer(&self, peer_id: PeerId, channel_id: Uuid) -> Result<(), SfuError> {
        let room = self.room(channel_id).await;
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
        let room = self.room(channel_id).await;
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
        let room = self.room(channel_id).await;
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
        let room = self.room(channel_id).await;
        let peer = {
            let mut room = room.lock().await;
            room.pubs
                .retain(|id, _| !id.starts_with(&peer_id.0.to_string()));
            room.peers.remove(&peer_id)
        };
        if let Some(peer) = peer {
            saturating_dec(&self.stats.peers);
            self.ice_ports.release(&peer.ice_addr).await;
            let _ = peer.pc.close().await;
            info!(peer = %peer_id.0, channel = %channel_id, "sfu leave");
        }
        let empty = {
            let room = room.lock().await;
            room.peers.is_empty()
        };
        if empty && self.rooms.write().await.remove(&channel_id).is_some() {
            saturating_dec(&self.stats.rooms);
        }
    }

    async fn revoked(&self, server_id: Uuid, user_id: Uuid) -> bool {
        let Some(redis) = &self.redis else {
            return false;
        };
        let key = deny_key(server_id, user_id);
        let mut conn = match redis.get_multiplexed_async_connection().await {
            Ok(conn) => conn,
            Err(err) => {
                warn!(error = %err, "revoke check skipped; redis unavailable");
                return false;
            }
        };
        match redis::cmd("EXISTS")
            .arg(key)
            .query_async::<i64>(&mut conn)
            .await
        {
            Ok(n) => n > 0,
            Err(err) => {
                warn!(error = %err, "revoke check failed");
                false
            }
        }
    }

    fn watch_revoke(
        self: &Arc<Self>,
        peer_id: PeerId,
        channel_id: Uuid,
        server_id: Uuid,
        user_id: Uuid,
        out: mpsc::UnboundedSender<ServerFrame>,
    ) {
        if self.redis.is_none() {
            return;
        }
        let sfu = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if !sfu.has_peer(peer_id, channel_id).await {
                    break;
                }
                if sfu.revoked(server_id, user_id).await {
                    warn!(
                        %user_id,
                        %server_id,
                        peer = %peer_id.0,
                        "closing sfu peer after revoke"
                    );
                    let _ = out.send(ServerFrame::error("unauthorized"));
                    sfu.leave(peer_id, channel_id).await;
                    break;
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

    async fn room(&self, channel_id: Uuid) -> Arc<Mutex<Room>> {
        let mut rooms = self.rooms.write().await;
        rooms
            .entry(channel_id)
            .or_insert_with(|| {
                Arc::new(Mutex::new(Room {
                    peers: HashMap::new(),
                    pubs: HashMap::new(),
                }))
            })
            .clone()
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
        let room = self.room(channel_id).await;
        let room = room.lock().await;
        room.peers.get(&peer_id).map(|p| p.out.clone())
    }

    async fn publish(
        &self,
        publisher: PeerId,
        channel_id: Uuid,
        track: Arc<dyn TrackRemote>,
    ) -> Result<(), SfuError> {
        let ssrcs = track.ssrcs().await;
        let ssrc = *ssrcs.first().ok_or(SfuError::BadAnnounce)?;
        let codec = track.codec(ssrc).await.ok_or(SfuError::BadAnnounce)?;
        let kind = track.kind().await;
        let track_id = track.track_id().await;
        let (user_id, kind_tag) = {
            let room = self.room(channel_id).await;
            let mut room = room.lock().await;
            let peer = room.peers.get_mut(&publisher).ok_or(SfuError::NotInRoom)?;
            // Camera / screen / live announces tag the next *video* track.
            // A re-fired mic after renegotiation must not consume `l` / `s`.
            let tag = match kind {
                RtpCodecKind::Video => peer
                    .next_kind
                    .pop_front()
                    .unwrap_or_else(|| "v".to_string()),
                _ => "a".to_string(),
            };
            (peer.user_id, tag)
        };
        let stream_id = format!("{user_id}:{kind_tag}");
        let pub_id = format!("{}:{track_id}", publisher.0);
        let clock_rate = codec.clock_rate;
        let (packets, _) = broadcast::channel(RTP_Q);
        let keyframe = if kind == RtpCodecKind::Video {
            let (tx, rx) = mpsc::unbounded_channel();
            spawn_publisher_readout(
                Arc::clone(&track),
                ssrc,
                packets.clone(),
                Some(rx),
                Arc::clone(&self.stats),
                clock_rate,
            );
            Some(tx)
        } else {
            spawn_publisher_readout(
                Arc::clone(&track),
                ssrc,
                packets.clone(),
                None,
                Arc::clone(&self.stats),
                clock_rate,
            );
            None
        };

        {
            let room = self.room(channel_id).await;
            let mut room = room.lock().await;
            room.pubs.insert(
                pub_id.clone(),
                Published {
                    id: pub_id.clone(),
                    stream_id: stream_id.clone(),
                    kind,
                    codec: codec.clone(),
                    packets: packets.clone(),
                    keyframe: keyframe.clone(),
                },
            );
        }

        debug!(pub_id, mime = %codec.mime_type, stream_id, "publisher track");

        let subscribers = {
            let room = self.room(channel_id).await;
            let room = room.lock().await;
            room.peers
                .iter()
                .filter(|(id, _)| **id != publisher)
                .map(|(id, p)| {
                    (
                        *id,
                        p.pc.clone(),
                        p.out.clone(),
                        p.gathered.clone(),
                        p.sdp.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        for (_peer_id, pc, out, gathered, sdp) in subscribers {
            self.forward_to(Forward {
                pc,
                out,
                gathered,
                sdp,
                pub_id: pub_id.clone(),
                stream_id: stream_id.clone(),
                kind,
                codec: codec.clone(),
                packets: packets.clone(),
                keyframe: keyframe.clone(),
            })
            .await;
        }
        Ok(())
    }

    async fn attach_existing_pubs(&self, subscriber: PeerId, channel_id: Uuid) {
        let (pc, out, gathered, sdp, pubs) = {
            let room = self.room(channel_id).await;
            let room = room.lock().await;
            let Some(peer) = room.peers.get(&subscriber) else {
                return;
            };
            let pubs = room
                .pubs
                .values()
                .filter(|p| !p.id.starts_with(&subscriber.0.to_string()))
                .map(|p| {
                    (
                        p.id.clone(),
                        p.stream_id.clone(),
                        p.kind,
                        p.codec.clone(),
                        p.packets.clone(),
                        p.keyframe.clone(),
                    )
                })
                .collect::<Vec<_>>();
            (
                peer.pc.clone(),
                peer.out.clone(),
                peer.gathered.clone(),
                peer.sdp.clone(),
                pubs,
            )
        };
        for (id, stream_id, kind, codec, packets, keyframe) in pubs {
            self.forward_to(Forward {
                pc: pc.clone(),
                out: out.clone(),
                gathered: gathered.clone(),
                sdp: sdp.clone(),
                pub_id: id,
                stream_id,
                kind,
                codec,
                packets,
                keyframe,
            })
            .await;
        }
    }

    async fn flush_pending(
        &self,
        pc: Arc<dyn PeerConnection>,
        out: mpsc::UnboundedSender<ServerFrame>,
        gathered: watch::Receiver<u64>,
        sdp: Arc<Mutex<PeerSdp>>,
        pending: Vec<PendingPub>,
    ) {
        for pub_ in pending {
            self.forward_to(Forward {
                pc: pc.clone(),
                out: out.clone(),
                gathered: gathered.clone(),
                sdp: sdp.clone(),
                pub_id: pub_.pub_id,
                stream_id: pub_.stream_id,
                kind: pub_.kind,
                codec: pub_.codec,
                packets: pub_.packets,
                keyframe: pub_.keyframe,
            })
            .await;
        }
    }

    async fn forward_to(&self, job: Forward) {
        let Forward {
            pc,
            out,
            gathered,
            sdp,
            pub_id,
            stream_id,
            kind,
            codec,
            packets,
            keyframe,
        } = job;
        {
            let mut gate = sdp.lock().await;
            if !gate.negotiated || gate.have_local_offer {
                gate.pending.push(PendingPub {
                    pub_id,
                    stream_id,
                    kind,
                    codec,
                    packets,
                    keyframe,
                });
                return;
            }
            gate.have_local_offer = true;
        }
        let ssrc = rand::random::<u32>();
        let local = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            stream_id.clone(),
            format!("{stream_id}-{ssrc}"),
            format!("gelabber-{pub_id}"),
            kind,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: codec.clone(),
                ..Default::default()
            }],
        )));
        let before = pc
            .get_transceivers()
            .await
            .iter()
            .map(|transceiver| transceiver.id())
            .collect::<Vec<_>>();
        if let Err(err) = pc
            .add_track(Arc::clone(&local) as Arc<dyn TrackLocal>)
            .await
        {
            warn!(pub_id, error = %err, "add_track failed");
            Box::pin(self.fail_local_offer(&pc, &out, &gathered, &sdp, false)).await;
            return;
        }
        // Offer only the codec we will actually forward. The full video list
        // (every H264 profile, AV1, HEVC, RTX) made the viewer's answer huge
        // and could negotiate a payload type this forwarder does not write.
        limit_forward_codec(&pc, &before, &codec).await;

        let mut rx = packets.subscribe();
        let forwarded = Arc::clone(&self.stats);
        let local_rtp = Arc::clone(&local);
        tokio::spawn(async move {
            let mut bound = false;
            let mut warned = false;
            loop {
                match rx.recv().await {
                    Ok(packet) => {
                        let n = packet.payload.len() as u64;
                        let packet = prepare_forwarded_rtp(packet, ssrc);
                        // Unbound until the subscriber answers. Skip; do not
                        // kill the forwarder. PLI only after the first
                        // successful write — an earlier IDR is dropped.
                        match local_rtp.write_rtp(packet).await {
                            Ok(()) => {
                                forwarded.forwarded_bytes.fetch_add(n, Ordering::Relaxed);
                                if !bound {
                                    bound = true;
                                    if let Some(kf) = keyframe.as_ref() {
                                        let _ = kf.send(());
                                        let local_rtcp = Arc::clone(&local_rtp);
                                        let relay = kf.clone();
                                        tokio::spawn(async move {
                                            // Started after bind: None is a
                                            // closed RTCP channel, not pre-bind.
                                            while let Some(evt) = local_rtcp.poll().await {
                                                if let TrackLocalEvent::OnRtcpPacket(pkts) = evt
                                                    && asks_keyframe(&pkts)
                                                {
                                                    let _ = relay.send(());
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                            Err(err) => {
                                if !warned {
                                    debug!(error = %err, "forward write_rtp skipped");
                                    warned = true;
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        match pc.create_offer(None).await {
            Ok(offer) => {
                let before = *gathered.borrow();
                if pc.set_local_description(offer).await.is_ok()
                    && let Some(local) = local_sdp_after_gather(&pc, &gathered, before).await
                {
                    info!(
                        pub_id,
                        bytes = local.len(),
                        mime = %codec.mime_type,
                        stream_id,
                        "subscriber renegotiation offer"
                    );
                    let _ = out.send(ServerFrame::Offer { sdp: local });
                } else {
                    warn!(pub_id, "renegotiation offer was not sent");
                    Box::pin(self.fail_local_offer(&pc, &out, &gathered, &sdp, false)).await;
                }
            }
            Err(err) => {
                warn!(error = %err, "renegotiation offer failed");
                Box::pin(self.fail_local_offer(&pc, &out, &gathered, &sdp, false)).await;
            }
        }
    }

    /// Drop an outstanding local offer and send whatever queued behind it.
    /// Resetting `have_local_offer` alone leaves the peer connection in
    /// `have-local-offer`, so the next `create_offer` never leaves the SFU.
    async fn fail_local_offer(
        &self,
        pc: &Arc<dyn PeerConnection>,
        out: &mpsc::UnboundedSender<ServerFrame>,
        gathered: &watch::Receiver<u64>,
        sdp: &Arc<Mutex<PeerSdp>>,
        only_if_outstanding: bool,
    ) {
        let pending = {
            let mut gate = sdp.lock().await;
            if only_if_outstanding && !gate.have_local_offer {
                return;
            }
            gate.have_local_offer = false;
            // Those candidates named the m-line this offer is abandoning.
            gate.pending_ice.clear();
            std::mem::take(&mut gate.pending)
        };
        if pc.pending_local_description().await.is_some() {
            match RTCSessionDescription::rollback(None) {
                Ok(rollback) => {
                    if let Err(err) = pc.set_local_description(rollback).await {
                        warn!(
                            error = %err,
                            queued = pending.len(),
                            "subscriber offer rollback failed"
                        );
                    } else {
                        info!(queued = pending.len(), "subscriber offer aborted");
                    }
                }
                Err(err) => {
                    warn!(error = %err, "subscriber offer rollback skipped");
                }
            }
        } else if !pending.is_empty() {
            info!(queued = pending.len(), "subscriber pending flushed");
        }
        self.flush_pending(
            pc.clone(),
            out.clone(),
            gathered.clone(),
            sdp.clone(),
            pending,
        )
        .await;
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

async fn limit_forward_codec(pc: &Arc<dyn PeerConnection>, before: &[usize], codec: &RTCRtpCodec) {
    let preference = RTCRtpCodecParameters {
        rtp_codec: codec.clone(),
        ..Default::default()
    };
    for transceiver in pc.get_transceivers().await {
        if before.contains(&transceiver.id()) {
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

fn spawn_publisher_readout(
    track: Arc<dyn TrackRemote>,
    ssrc: u32,
    packets: broadcast::Sender<rtp::Packet>,
    mut keyframes: Option<mpsc::UnboundedReceiver<()>>,
    stats: Arc<SfuStats>,
    clock_rate: u32,
) {
    tokio::spawn(async move {
        let mut rtp = PublisherRtpStats::new(clock_rate);
        loop {
            if let Some(kf) = keyframes.as_mut() {
                tokio::select! {
                    evt = track.poll() => {
                        if !handle_publisher_event(&packets, &stats, &mut rtp, evt) {
                            break;
                        }
                    }
                    req = kf.recv() => {
                        if req.is_none() {
                            keyframes = None;
                            continue;
                        }
                        request_keyframe(&track, ssrc).await;
                    }
                }
            } else {
                let evt = track.poll().await;
                if !handle_publisher_event(&packets, &stats, &mut rtp, evt) {
                    break;
                }
            }
        }
    });
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
            ..Default::default()
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
        ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
                        if !pts.iter().any(|p| *p == pt) {
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
            .set_extension(1, bytes::Bytes::from_static(&[b'0']))
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
