//! Two webrtc-rs "browsers" through the SFU: RTP from A is forwarded to B.

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
use tokio::sync::mpsc;
use uuid::Uuid;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCPeerConnectionState, RTCSessionDescription, register_default_interceptors,
};

struct ClientHandler {
    connected: mpsc::UnboundedSender<()>,
    packets: mpsc::UnboundedSender<Packet>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for ClientHandler {
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

async fn client_pc(
    connected: mpsc::UnboundedSender<()>,
    packets: mpsc::UnboundedSender<Packet>,
) -> Arc<dyn PeerConnection> {
    let mut media = MediaEngine::default();
    media.register_default_codecs().unwrap();
    let registry =
        register_default_interceptors(webrtc::peer_connection::Registry::new(), &mut media)
            .unwrap();
    let pc = PeerConnectionBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(Arc::new(ClientHandler { connected, packets }))
        .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
        .build()
        .await
        .unwrap();
    Arc::new(pc)
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

async fn pump_offer(
    client: &Arc<dyn PeerConnection>,
    sfu: &Arc<Sfu>,
    peer: gelabber_media::sfu::PeerId,
    channel: Uuid,
    rx: &mut mpsc::UnboundedReceiver<ServerFrame>,
) {
    let offer = client.create_offer(None).await.unwrap();
    client.set_local_description(offer).await.unwrap();
    let local = client.local_description().await.unwrap();
    sfu.apply_remote(peer, channel, local.sdp, true)
        .await
        .unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Some(ServerFrame::Answer { sdp }) => return sdp,
                Some(_) => continue,
                None => panic!("sfu closed"),
            }
        }
    })
    .await
    .expect("sfu answer");
    client
        .set_remote_description(RTCSessionDescription::answer(answer).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn forwards_rtp_between_two_peers() {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
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

    let a = client_pc(a_conn_tx, a_pkt_tx).await;
    let b = client_pc(b_conn_tx, b_pkt_tx).await;
    let track = opus_track(0x1111_0001);
    a.add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
        .await
        .unwrap();

    pump_offer(&a, &sfu, a_id, channel, &mut a_rx).await;
    pump_offer(&b, &sfu, b_id, channel, &mut b_rx).await;

    // B may get an SFU renegotiation offer once A's track is forwarded.
    if let Ok(Some(ServerFrame::Offer { sdp })) =
        tokio::time::timeout(Duration::from_secs(3), b_rx.recv()).await
    {
        b.set_remote_description(RTCSessionDescription::offer(sdp).unwrap())
            .await
            .unwrap();
        let answer = b.create_answer(None).await.unwrap();
        b.set_local_description(answer.clone()).await.unwrap();
        sfu.apply_remote(b_id, channel, answer.sdp, false)
            .await
            .unwrap();
    }

    let _ = tokio::time::timeout(Duration::from_secs(8), a_conn_rx.recv()).await;
    let _ = tokio::time::timeout(Duration::from_secs(8), b_conn_rx.recv()).await;

    let mut pkt = Packet::default();
    pkt.header.ssrc = 0x1111_0001;
    pkt.header.sequence_number = 1;
    pkt.payload = bytes::Bytes::from_static(&[0xF8, 0xFF, 0xFE]);
    for seq in 1..=40u16 {
        pkt.header.sequence_number = seq;
        let _ = track.write_rtp(pkt.clone()).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let got = tokio::time::timeout(Duration::from_secs(8), b_pkt_rx.recv()).await;
    assert!(got.is_ok(), "subscriber should receive forwarded RTP");
}
