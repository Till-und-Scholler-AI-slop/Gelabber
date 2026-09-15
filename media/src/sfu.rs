//! Single-node SFU: a room is a voice channel. RTP from a publisher is
//! written onto `TrackLocalStaticRTP`s of every other peer. No mesh, no
//! recording, no second node.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::Duration;

use rtc::media_stream::MediaStreamTrack;
use rtc::rtcp;
use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp;
use rtc::peer_connection::configuration::media_engine::{
    MIME_TYPE_H264, MIME_TYPE_OPUS, MIME_TYPE_VP8,
};
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RTCPFeedback, RtpCodecKind,
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

use crate::config::Config;
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
struct IcePorts {
    ip: std::net::IpAddr,
    min: u16,
    max: u16,
    next: AtomicU16,
}

impl IcePorts {
    fn from_config(config: &Config) -> Self {
        let addr: SocketAddr = config
            .ice_bind
            .parse()
            .unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap());
        let min = addr.port();
        let max = config.ice_port_max.unwrap_or(min).max(min);
        Self {
            ip: addr.ip(),
            min,
            max,
            next: AtomicU16::new(0),
        }
    }

    fn take(&self) -> String {
        if self.min == 0 {
            return SocketAddr::new(self.ip, 0).to_string();
        }
        let span = u32::from(self.max - self.min) + 1;
        let i = u32::from(self.next.fetch_add(1, Ordering::Relaxed));
        let port = self.min + (i % span) as u16;
        SocketAddr::new(self.ip, port).to_string()
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
}

impl PeerSdp {
    fn new() -> Self {
        Self {
            negotiated: false,
            have_local_offer: false,
            pending: Vec::new(),
        }
    }
}

struct Peer {
    #[allow(dead_code)]
    id: PeerId,
    user_id: Uuid,
    #[allow(dead_code)]
    channel_id: Uuid,
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
    rooms: RwLock<HashMap<Uuid, Arc<Mutex<Room>>>>,
    stats: Arc<SfuStats>,
}

#[derive(Default)]
struct SfuStats {
    rooms: AtomicU64,
    peers: AtomicU64,
    forwarded_bytes: AtomicU64,
    ice_fails: AtomicU64,
}

impl Sfu {
    pub fn new(config: &Config) -> Self {
        Self {
            ice_ports: IcePorts::from_config(config),
            advertised_ip: config.advertised_ip.clone(),
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
             gelabber_media_ice_fails_total {ice_fails}\n"
        )
    }

    pub async fn join(
        self: &Arc<Self>,
        claim: TicketClaim,
        out: mpsc::UnboundedSender<ServerFrame>,
    ) -> Result<PeerId, String> {
        let peer_id = PeerId(Uuid::new_v4());
        let (pc, mut events, gathered) = self
            .build_pc()
            .await
            .map_err(|err| format!("peer connection: {err}"))?;

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
            "sfu join"
        );

        let sfu = Arc::clone(self);
        tokio::spawn(async move {
            sfu.drive(peer_id, claim.c, pc, &mut events).await;
        });

        Ok(peer_id)
    }

    pub async fn apply_remote(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        sdp: String,
        as_offer: bool,
    ) -> Result<(), String> {
        let desc = if as_offer {
            RTCSessionDescription::offer(sdp).map_err(|err| err.to_string())?
        } else {
            RTCSessionDescription::answer(sdp).map_err(|err| err.to_string())?
        };
        let room = self.room(channel_id).await;
        let (pc, out, gathered, sdp) = {
            let room = room.lock().await;
            let peer = room.peers.get(&peer_id).ok_or("not in room")?;
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

        pc.set_remote_description(desc)
            .await
            .map_err(|err| err.to_string())?;

        if as_offer {
            let answer = pc
                .create_answer(None)
                .await
                .map_err(|err| err.to_string())?;
            let before = *gathered.borrow();
            pc.set_local_description(answer)
                .await
                .map_err(|err| err.to_string())?;
            if let Some(sdp) = local_sdp_after_gather(&pc, &gathered, before).await {
                let _ = out.send(ServerFrame::Answer { sdp });
            }
            gate.negotiated = true;
            gate.have_local_offer = false;
            let pending = std::mem::take(&mut gate.pending);
            drop(gate);
            self.attach_existing_pubs(peer_id, channel_id).await;
            self.flush_pending(pc, out, gathered, sdp, pending).await;
        } else {
            gate.have_local_offer = false;
            let pending = std::mem::take(&mut gate.pending);
            drop(gate);
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
    ) -> Result<(), String> {
        if kind != "v" && kind != "s" && kind != "l" {
            return Err("bad_request".into());
        }
        let room = self.room(channel_id).await;
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&peer_id).ok_or("not in room")?;
        peer.next_kind.push_back(kind.to_owned());
        Ok(())
    }

    pub async fn add_ice(
        &self,
        peer_id: PeerId,
        channel_id: Uuid,
        ice: String,
        mid: Option<String>,
    ) -> Result<(), String> {
        let room = self.room(channel_id).await;
        let pc = {
            let room = room.lock().await;
            room.peers.get(&peer_id).ok_or("not in room")?.pc.clone()
        };
        let mid = mid.filter(|m| !m.is_empty());
        pc.add_ice_candidate(RTCIceCandidateInit {
            candidate: ice,
            sdp_mid: mid.clone(),
            sdp_mline_index: if mid.is_some() { None } else { Some(0) },
            username_fragment: None,
            url: None,
        })
        .await
        .map_err(|err| err.to_string())
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
            .with_udp_addrs(vec![self.ice_ports.take()])
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
    ) -> Result<(), String> {
        let ssrcs = track.ssrcs().await;
        let ssrc = *ssrcs.first().ok_or("track has no ssrc")?;
        let codec = track.codec(ssrc).await.ok_or("track has no codec")?;
        let kind = track.kind().await;
        let track_id = track.track_id().await;
        let (user_id, kind_tag) = {
            let room = self.room(channel_id).await;
            let mut room = room.lock().await;
            let peer = room.peers.get_mut(&publisher).ok_or("not in room")?;
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
        let (packets, _) = broadcast::channel(RTP_Q);
        let keyframe = if kind == RtpCodecKind::Video {
            let (tx, rx) = mpsc::unbounded_channel();
            spawn_publisher_readout(Arc::clone(&track), ssrc, packets.clone(), Some(rx));
            Some(tx)
        } else {
            spawn_publisher_readout(Arc::clone(&track), ssrc, packets.clone(), None);
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
        if let Err(err) = pc
            .add_track(Arc::clone(&local) as Arc<dyn TrackLocal>)
            .await
        {
            warn!(error = %err, "add_track failed");
            sdp.lock().await.have_local_offer = false;
            return;
        }

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
                    let _ = out.send(ServerFrame::Offer { sdp: local });
                } else {
                    sdp.lock().await.have_local_offer = false;
                }
            }
            Err(err) => {
                warn!(error = %err, "renegotiation offer failed");
                sdp.lock().await.have_local_offer = false;
            }
        }
    }
}

fn spawn_publisher_readout(
    track: Arc<dyn TrackRemote>,
    ssrc: u32,
    packets: broadcast::Sender<rtp::Packet>,
    mut keyframes: Option<mpsc::UnboundedReceiver<()>>,
) {
    tokio::spawn(async move {
        loop {
            if let Some(kf) = keyframes.as_mut() {
                tokio::select! {
                    evt = track.poll() => {
                        if !handle_publisher_event(&packets, evt) {
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
                if !handle_publisher_event(&packets, evt) {
                    break;
                }
            }
        }
    });
}

fn handle_publisher_event(
    packets: &broadcast::Sender<rtp::Packet>,
    evt: Option<TrackRemoteEvent>,
) -> bool {
    match evt {
        Some(TrackRemoteEvent::OnRtpPacket(packet)) => {
            let _ = packets.send(packet);
            true
        }
        Some(TrackRemoteEvent::OnEnded | TrackRemoteEvent::OnError) | None => false,
        Some(_) => true,
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

/// Register SFU codecs: Opus-only audio, then common video (VP8/H264 + RTX).
///
/// webrtc-rs 0.20 `set_codec_preferences_from_remote_description` walks the
/// remote offer codecs with `.rev()` and pushes matches, which *reverses*
/// preference order. Chrome offers `111 … 8` (Opus first); the SFU answer
/// became `m=audio … 8 0 9 111` (PCMA first) and stats showed audio/PCMA
/// ~64 kbps — root cause of #53. Omitting PCMA/PCMU/G722 makes the reversed
/// list still Opus-only.
fn register_sfu_codecs(media: &mut MediaEngine) -> webrtc::error::Result<()> {
    media.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
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

    let rtx = |payload_type: u8, apt: u8| RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: "video/rtx".to_owned(),
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
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                        .to_owned(),
                rtcp_feedback: video_rtcp_feedback,
            },
            payload_type: 125,
            ..Default::default()
        },
        rtx(105, 125),
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
        MIME_TYPE_OPUS, RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
        Sfu, prepare_forwarded_rtp,
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
            MediaEngine, PeerConnectionBuilder, RTCSessionDescription,
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
                },
                out,
            )
            .await
            .expect("join");

        // Client MediaEngine includes PCMA/PCMU/G722 (Chrome-like).
        let mut media = MediaEngine::default();
        media.register_default_codecs().unwrap();
        let registry = register_default_interceptors(
            webrtc::peer_connection::Registry::new(),
            &mut media,
        )
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

        let _ = pc
            .set_remote_description(RTCSessionDescription::answer(answer_sdp).unwrap())
            .await;
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
}
