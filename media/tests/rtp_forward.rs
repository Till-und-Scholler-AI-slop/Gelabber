//! Two webrtc-rs "browsers" through the SFU: RTP from A is forwarded to B.
//! webrtc 0.20.5 only puts `ice-ufrag` on the SDP after ICE gathering.

use std::sync::Arc;
use std::time::Duration;

use gelabber_media::config::Config;
use gelabber_media::protocol::ServerFrame;
use gelabber_media::sfu::Sfu;
use gelabber_media::ticket::TicketClaim;
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp::packet::Packet;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCIceCandidateInit, RTCIceGatheringState, RTCPeerConnectionState, RTCSessionDescription,
    register_default_interceptors,
};

struct ClientHandler {
    gathered: watch::Sender<u64>,
    connected: mpsc::UnboundedSender<()>,
    packets: mpsc::UnboundedSender<Packet>,
    ice: mpsc::UnboundedSender<RTCIceCandidateInit>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for ClientHandler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let next = *self.gathered.borrow() + 1;
            let _ = self.gathered.send(next);
        }
    }

    async fn on_ice_candidate(&self, event: webrtc::peer_connection::RTCPeerConnectionIceEvent) {
        if event.candidate.address.is_empty() {
            return;
        }
        if let Ok(init) = event.candidate.to_json() {
            let _ = self.ice.send(init);
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if state == RTCPeerConnectionState::Connected {
            let _ = self.connected.send(());
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let tx = self.packets.clone();
        tokio::spawn(async move {
            while let Some(evt) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(packet) = evt {
                    let _ = tx.send(packet);
                }
            }
        });
    }
}

fn claim(user: u128, channel: u128) -> TicketClaim {
    TicketClaim {
        u: Uuid::from_u128(user),
        s: Uuid::from_u128(9),
        c: Uuid::from_u128(channel),
    }
}

struct Client {
    pc: Arc<dyn PeerConnection>,
    gathered: watch::Receiver<u64>,
    ice_rx: mpsc::UnboundedReceiver<RTCIceCandidateInit>,
}

async fn client_pc(
    connected: mpsc::UnboundedSender<()>,
    packets: mpsc::UnboundedSender<Packet>,
) -> Client {
    let mut media = MediaEngine::default();
    media.register_default_codecs().unwrap();
    let registry =
        register_default_interceptors(webrtc::peer_connection::Registry::new(), &mut media)
            .unwrap();
    let (gather_tx, gather_rx) = watch::channel(0);
    let (ice_tx, ice_rx) = mpsc::unbounded_channel();
    let pc = PeerConnectionBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(Arc::new(ClientHandler {
            gathered: gather_tx,
            connected,
            packets,
            ice: ice_tx,
        }))
        .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
        .build()
        .await
        .unwrap();
    Client {
        pc: Arc::new(pc),
        gathered: gather_rx,
        ice_rx,
    }
}

fn vp8_track(ssrc: u32) -> Arc<TrackLocalStaticRTP> {
    Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        format!("stream-{ssrc}"),
        format!("video-{ssrc}"),
        format!("cam-{ssrc}"),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
            codec: RTCRtpCodec {
                mime_type: "video/VP8".into(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: String::new(),
                rtcp_feedback: vec![],
            },
            ..Default::default()
        }],
    )))
}

fn opus_track(ssrc: u32) -> Arc<TrackLocalStaticRTP> {
    Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        format!("stream-{ssrc}"),
        format!("audio-{ssrc}"),
        format!("mic-{ssrc}"),
        RtpCodecKind::Audio,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
            codec: RTCRtpCodec {
                mime_type: "audio/opus".into(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: String::new(),
                rtcp_feedback: vec![],
            },
            ..Default::default()
        }],
    )))
}

async fn wait_gather(gathered: &watch::Receiver<u64>, before: u64) {
    let mut rx = gathered.clone();
    if *rx.borrow() <= before {
        let _ = tokio::time::timeout(Duration::from_secs(2), rx.wait_for(|n| *n > before)).await;
    }
}

async fn apply_sfu_frames(
    client: &Arc<dyn PeerConnection>,
    sfu: &Arc<Sfu>,
    peer: gelabber_media::sfu::PeerId,
    channel: Uuid,
    rx: &mut mpsc::UnboundedReceiver<ServerFrame>,
    deadline: Duration,
) {
    let start = tokio::time::Instant::now();
    while start.elapsed() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Some(ServerFrame::Answer { sdp })) => {
                client
                    .set_remote_description(RTCSessionDescription::answer(sdp).unwrap())
                    .await
                    .unwrap();
            }
            Ok(Some(ServerFrame::Offer { sdp })) => {
                client
                    .set_remote_description(RTCSessionDescription::offer(sdp).unwrap())
                    .await
                    .unwrap();
                let answer = client.create_answer(None).await.unwrap();
                client.set_local_description(answer.clone()).await.unwrap();
                sfu.apply_remote(peer, channel, answer.sdp, false)
                    .await
                    .unwrap();
            }
            Ok(Some(ServerFrame::Ice { ice, mid })) => {
                let _ = client
                    .add_ice_candidate(RTCIceCandidateInit {
                        candidate: ice,
                        sdp_mid: mid,
                        sdp_mline_index: Some(0),
                        username_fragment: None,
                        url: None,
                    })
                    .await;
            }
            Ok(Some(_)) | Err(_) => {}
            Ok(None) => break,
        }
    }
}

async fn flush_client_ice(
    ice_rx: &mut mpsc::UnboundedReceiver<RTCIceCandidateInit>,
    sfu: &Arc<Sfu>,
    peer: gelabber_media::sfu::PeerId,
    channel: Uuid,
) {
    while let Ok(init) = ice_rx.try_recv() {
        let _ = sfu
            .add_ice(peer, channel, init.candidate, init.sdp_mid)
            .await;
    }
}

async fn pump_offer(
    client: &Client,
    sfu: &Arc<Sfu>,
    peer: gelabber_media::sfu::PeerId,
    channel: Uuid,
    rx: &mut mpsc::UnboundedReceiver<ServerFrame>,
) {
    let before = *client.gathered.borrow();
    let offer = client.pc.create_offer(None).await.unwrap();
    client.pc.set_local_description(offer).await.unwrap();
    wait_gather(&client.gathered, before).await;
    let local = client.pc.local_description().await.unwrap();
    assert!(
        local.sdp.contains("ice-ufrag"),
        "offer must carry ICE credentials after gather: {}",
        local.sdp
    );
    sfu.apply_remote(peer, channel, local.sdp, true)
        .await
        .unwrap();
    apply_sfu_frames(&client.pc, sfu, peer, channel, rx, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn forwards_rtp_between_two_peers() {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        "TURN_URLS" => Some("stun:127.0.0.1:3478,turn:127.0.0.1:3478".to_owned()),
        "TURN_USERNAME" => Some("gelabber".to_owned()),
        "TURN_PASSWORD" => Some("gelabberturn".to_owned()),
        _ => None,
    })
    .unwrap();
    let sfu = Arc::new(Sfu::new(&config));
    let channel = Uuid::from_u128(3);

    let (a_out, mut a_rx) = mpsc::unbounded_channel();
    let (b_out, mut b_rx) = mpsc::unbounded_channel();
    let a_id = sfu.join(claim(1, 3), a_out).await.expect("join a");
    let b_id = sfu.join(claim(2, 3), b_out).await.expect("join b");

    let (a_conn_tx, mut a_conn_rx) = mpsc::unbounded_channel();
    let (b_conn_tx, mut b_conn_rx) = mpsc::unbounded_channel();
    let (a_pkt_tx, _a_pkt_rx) = mpsc::unbounded_channel();
    let (b_pkt_tx, mut b_pkt_rx) = mpsc::unbounded_channel();

    let mut a = client_pc(a_conn_tx, a_pkt_tx).await;
    let mut b = client_pc(b_conn_tx, b_pkt_tx).await;
    let track = opus_track(0x1111_0001);
    let sender =
        a.pc.add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
            .await
            .unwrap();
    b.pc.add_track(Arc::clone(&opus_track(0x2222_0001)) as Arc<dyn TrackLocal>)
        .await
        .unwrap();

    pump_offer(&a, &sfu, a_id, channel, &mut a_rx).await;
    pump_offer(&b, &sfu, b_id, channel, &mut b_rx).await;
    flush_client_ice(&mut a.ice_rx, &sfu, a_id, channel).await;
    flush_client_ice(&mut b.ice_rx, &sfu, b_id, channel).await;
    apply_sfu_frames(
        &a.pc,
        &sfu,
        a_id,
        channel,
        &mut a_rx,
        Duration::from_millis(400),
    )
    .await;
    apply_sfu_frames(
        &b.pc,
        &sfu,
        b_id,
        channel,
        &mut b_rx,
        Duration::from_millis(400),
    )
    .await;

    let _ = tokio::time::timeout(Duration::from_secs(8), a_conn_rx.recv()).await;
    let _ = tokio::time::timeout(Duration::from_secs(8), b_conn_rx.recv()).await;

    let pt = sender
        .get_parameters()
        .await
        .ok()
        .and_then(|p| p.rtp_parameters.codecs.first().map(|c| c.payload_type))
        .unwrap_or(111);

    let mut pkt = Packet::default();
    pkt.header.version = 2;
    pkt.header.ssrc = 0x1111_0001;
    pkt.header.payload_type = pt;
    pkt.payload = bytes::Bytes::from_static(&[0xF8, 0xFF, 0xFE]);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut seq = 1u16;
    let mut got = None;
    while tokio::time::Instant::now() < deadline && got.is_none() {
        pkt.header.sequence_number = seq;
        pkt.header.timestamp = u32::from(seq) * 960;
        seq = seq.wrapping_add(1);
        let _ = track.write_rtp(pkt.clone()).await;
        flush_client_ice(&mut a.ice_rx, &sfu, a_id, channel).await;
        flush_client_ice(&mut b.ice_rx, &sfu, b_id, channel).await;
        apply_sfu_frames(
            &a.pc,
            &sfu,
            a_id,
            channel,
            &mut a_rx,
            Duration::from_millis(20),
        )
        .await;
        apply_sfu_frames(
            &b.pc,
            &sfu,
            b_id,
            channel,
            &mut b_rx,
            Duration::from_millis(20),
        )
        .await;
        if let Ok(Some(packet)) =
            tokio::time::timeout(Duration::from_millis(20), b_pkt_rx.recv()).await
        {
            got = Some(packet);
        }
    }
    assert!(got.is_some(), "subscriber should receive forwarded RTP");
}

#[tokio::test]
async fn concurrent_first_offers_do_not_glare() {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        "TURN_URLS" => Some("stun:127.0.0.1:3478,turn:127.0.0.1:3478".to_owned()),
        "TURN_USERNAME" => Some("gelabber".to_owned()),
        "TURN_PASSWORD" => Some("gelabberturn".to_owned()),
        _ => None,
    })
    .unwrap();
    let sfu = Arc::new(Sfu::new(&config));
    let channel = Uuid::from_u128(3);

    let (a_out, mut a_rx) = mpsc::unbounded_channel();
    let (b_out, mut b_rx) = mpsc::unbounded_channel();
    let a_id = sfu.join(claim(1, 3), a_out).await.expect("join a");
    let b_id = sfu.join(claim(2, 3), b_out).await.expect("join b");

    let (a_conn_tx, _a_conn_rx) = mpsc::unbounded_channel();
    let (b_conn_tx, _b_conn_rx) = mpsc::unbounded_channel();
    let (a_pkt_tx, _a_pkt_rx) = mpsc::unbounded_channel();
    let (b_pkt_tx, _b_pkt_rx) = mpsc::unbounded_channel();

    let a = client_pc(a_conn_tx, a_pkt_tx).await;
    let b = client_pc(b_conn_tx, b_pkt_tx).await;
    a.pc.add_track(Arc::clone(&opus_track(0x1111_0001)) as Arc<dyn TrackLocal>)
        .await
        .unwrap();
    b.pc.add_track(Arc::clone(&opus_track(0x2222_0001)) as Arc<dyn TrackLocal>)
        .await
        .unwrap();

    let (a_res, b_res) = tokio::join!(
        async {
            pump_offer(&a, &sfu, a_id, channel, &mut a_rx).await;
        },
        async {
            pump_offer(&b, &sfu, b_id, channel, &mut b_rx).await;
        },
    );
    let _ = (a_res, b_res);
    assert!(
        a.pc.remote_description().await.is_some(),
        "A should have an SFU answer"
    );
    assert!(
        b.pc.remote_description().await.is_some(),
        "B should have an SFU answer"
    );
}

fn test_config() -> Config {
    Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        "TURN_URLS" => Some("stun:127.0.0.1:3478,turn:127.0.0.1:3478".to_owned()),
        "TURN_USERNAME" => Some("gelabber".to_owned()),
        "TURN_PASSWORD" => Some("gelabberturn".to_owned()),
        _ => None,
    })
    .unwrap()
}

/// Go Live / screen share: the subscriber offer names only the forwarded
/// codec, and ICE that arrives before the answer is not an error.
#[tokio::test]
async fn video_renegotiation_keeps_subscriber_and_buffers_early_ice() {
    let sfu = Arc::new(Sfu::new(&test_config()));
    let channel = Uuid::from_u128(4);

    let (a_out, mut a_rx) = mpsc::unbounded_channel();
    let (b_out, mut b_rx) = mpsc::unbounded_channel();
    let a_id = sfu.join(claim(1, 4), a_out).await.expect("join a");
    let b_id = sfu.join(claim(2, 4), b_out).await.expect("join b");

    let (a_conn_tx, _a_conn_rx) = mpsc::unbounded_channel();
    let (b_conn_tx, _b_conn_rx) = mpsc::unbounded_channel();
    let (a_pkt_tx, _a_pkt_rx) = mpsc::unbounded_channel();
    let (b_pkt_tx, _b_pkt_rx) = mpsc::unbounded_channel();

    let mut a = client_pc(a_conn_tx, a_pkt_tx).await;
    let b = client_pc(b_conn_tx, b_pkt_tx).await;
    let audio = opus_track(0x1111_0001);
    a.pc.add_track(Arc::clone(&audio) as Arc<dyn TrackLocal>)
        .await
        .unwrap();
    b.pc.add_track(Arc::clone(&opus_track(0x2222_0001)) as Arc<dyn TrackLocal>)
        .await
        .unwrap();
    pump_offer(&a, &sfu, a_id, channel, &mut a_rx).await;
    pump_offer(&b, &sfu, b_id, channel, &mut b_rx).await;

    let video = vp8_track(0x3333_0001);
    let sender =
        a.pc.add_track(Arc::clone(&video) as Arc<dyn TrackLocal>)
            .await
            .unwrap();
    pump_offer(&a, &sfu, a_id, channel, &mut a_rx).await;

    let pt = sender
        .get_parameters()
        .await
        .ok()
        .and_then(|p| p.rtp_parameters.codecs.first().map(|c| c.payload_type))
        .unwrap_or(96);
    let mut pkt = Packet::default();
    pkt.header.version = 2;
    pkt.header.ssrc = 0x3333_0001;
    pkt.header.payload_type = pt;
    pkt.payload = bytes::Bytes::from_static(&[0x10, 0x00, 0x00]);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut offer = None;
    let mut seq = 1u16;
    while tokio::time::Instant::now() < deadline && offer.is_none() {
        pkt.header.sequence_number = seq;
        pkt.header.timestamp = u32::from(seq) * 3000;
        seq = seq.wrapping_add(1);
        let _ = video.write_rtp(pkt.clone()).await;
        flush_client_ice(&mut a.ice_rx, &sfu, a_id, channel).await;
        match tokio::time::timeout(Duration::from_millis(40), b_rx.recv()).await {
            Ok(Some(ServerFrame::Offer { sdp })) => offer = Some(sdp),
            Ok(Some(_)) | Err(_) => {}
            Ok(None) => break,
        }
    }
    let offer = offer.expect("subscriber should be offered the forwarded video");
    assert!(
        offer.contains("VP8/90000"),
        "forwarded offer must carry VP8: {offer}"
    );
    assert!(
        !offer.contains("H265/90000") && !offer.contains("AV1/90000"),
        "forwarded offer must not list every registered video codec: {offer}"
    );
    assert!(
        offer.len() < 48 * 1024,
        "single-codec offer should stay well under the old 48 KiB cap, got {}",
        offer.len()
    );

    sfu.add_ice(
        b_id,
        channel,
        "candidate:1 1 udp 2122260223 192.0.2.1 9 typ host".into(),
        Some("1".into()),
    )
    .await
    .expect("ICE before the answer is buffered, not a hard failure");

    b.pc.set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .await
        .expect("subscriber accepts the slim offer");
    let answer = b.pc.create_answer(None).await.unwrap();
    b.pc.set_local_description(answer.clone()).await.unwrap();
    sfu.apply_remote(b_id, channel, answer.sdp, false)
        .await
        .expect("subscriber answer applies");
}
