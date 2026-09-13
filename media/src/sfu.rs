//! Single-node SFU: a room is a voice channel. RTP from a publisher is
//! written onto `TrackLocalStaticRTP`s of every other peer. No mesh, no
//! recording, no second node.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use rtc::media_stream::MediaStreamTrack;
use rtc::rtp;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, watch};
use tracing::{debug, info, warn};
use uuid::Uuid;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceCandidateType, RTCIceGatheringState,
    RTCIceServer, RTCSessionDescription, SettingEngine, register_default_interceptors,
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
}

struct PendingPub {
    pub_id: String,
    stream_id: String,
    kind: RtpCodecKind,
    codec: RTCRtpCodec,
    packets: broadcast::Sender<rtp::Packet>,
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
}

pub struct Sfu {
    ice_servers: Vec<RTCIceServer>,
    ice_ports: IcePorts,
    advertised_ip: Option<String>,
    rooms: RwLock<HashMap<Uuid, Arc<Mutex<Room>>>>,
}

impl Sfu {
    pub fn new(config: &Config) -> Self {
        Self {
            ice_servers: config
                .ice_servers
                .iter()
                .map(super::ice::IceServer::to_rtc)
                .collect(),
            ice_ports: IcePorts::from_config(config),
            advertised_ip: config.advertised_ip.clone(),
            rooms: RwLock::new(HashMap::new()),
        }
    }

    pub fn room_count(&self) -> usize {
        // test helper; cheap snapshot
        self.rooms.try_read().map(|g| g.len()).unwrap_or(0)
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
            let _ = peer.pc.close().await;
            info!(peer = %peer_id.0, channel = %channel_id, "sfu leave");
        }
        let empty = {
            let room = room.lock().await;
            room.peers.is_empty()
        };
        if empty {
            self.rooms.write().await.remove(&channel_id);
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
        media.register_default_codecs()?;
        let registry =
            register_default_interceptors(webrtc::peer_connection::Registry::new(), &mut media)?;

        let mut settings = SettingEngine::default();
        settings.set_lite(true);
        if let Some(ip) = &self.advertised_ip {
            settings.set_nat_1to1_ips(vec![ip.clone()], RTCIceCandidateType::Host);
        }

        let config = RTCConfigurationBuilder::new()
            .with_ice_servers(self.ice_servers.clone())
            .build();

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
            let tag = match peer.next_kind.pop_front() {
                Some(tag) => tag,
                None => match kind {
                    RtpCodecKind::Video => "v".to_string(),
                    _ => "a".to_string(),
                },
            };
            (peer.user_id, tag)
        };
        let stream_id = format!("{user_id}:{kind_tag}");
        let pub_id = format!("{}:{track_id}", publisher.0);
        let (packets, _) = broadcast::channel(RTP_Q);

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
            })
            .await;
        }

        tokio::spawn(async move {
            while let Some(evt) = track.poll().await {
                match evt {
                    TrackRemoteEvent::OnRtpPacket(packet) => {
                        let _ = packets.send(packet);
                    }
                    TrackRemoteEvent::OnEnded | TrackRemoteEvent::OnError => break,
                    _ => {}
                }
            }
        });
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
        for (id, stream_id, kind, codec, packets) in pubs {
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
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(mut packet) => {
                        packet.header.ssrc = ssrc;
                        if local.write_rtp(packet).await.is_err() {
                            break;
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
