//! Real RTC regression: compound PLI and multi-target FIR reach every source.
use rtc::media_stream::MediaStreamTrack;
use rtc::rtcp;
use rtc::rtcp::payload_feedbacks::{
    full_intra_request::{FirEntry, FullIntraRequest},
    picture_loss_indication::PictureLossIndication,
};
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::{TrackLocal, TrackLocalEvent};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCIceGatheringState, RTCPeerConnectionState, Registry, register_default_interceptors,
};
#[path = "../src/sfu_feedback.rs"]
mod feedback;
fn asks_keyframe(packets: &[Box<dyn rtcp::Packet>]) -> bool {
    packets
        .iter()
        .any(|p| p.as_any().is::<PictureLossIndication>() || p.as_any().is::<FullIntraRequest>())
}
struct Handler {
    gather: watch::Sender<bool>,
    connected: watch::Sender<bool>,
    tracks: mpsc::UnboundedSender<Arc<dyn TrackRemote>>,
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gather.send_replace(true);
        }
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if state == RTCPeerConnectionState::Connected {
            self.connected.send_replace(true);
        }
    }
    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let _ = self.tracks.send(track);
    }
}
async fn peer() -> (
    Arc<dyn PeerConnection>,
    watch::Receiver<bool>,
    watch::Receiver<bool>,
    mpsc::UnboundedReceiver<Arc<dyn TrackRemote>>,
) {
    let mut media = MediaEngine::default();
    media.register_default_codecs().unwrap();
    let registry = register_default_interceptors(Registry::new(), &mut media)
        .unwrap()
        .with(feedback::KeyframeFeedback::new);
    let (gather, gathered) = watch::channel(false);
    let (connected, connection) = watch::channel(false);
    let (tracks, remote) = mpsc::unbounded_channel();
    let pc = PeerConnectionBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
        .with_handler(Arc::new(Handler {
            gather,
            connected,
            tracks,
        }))
        .build()
        .await
        .unwrap();
    (Arc::new(pc), gathered, connection, remote)
}
#[tokio::test]
async fn compound_pli_and_multi_target_fir_reach_each_source() {
    let (sender, mut send_gather, mut send_connect, _) = peer().await;
    let (receiver, mut recv_gather, mut recv_connect, mut remote) = peer().await;
    let mut locals = Vec::new();
    let counts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    for i in 0..2 {
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            format!("stream-{i}"),
            format!("track-{i}"),
            "".into(),
            RtpCodecKind::Video,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(0x11223300 + i as u32),
                    ..Default::default()
                },
                codec: RTCRtpCodec {
                    mime_type: "video/VP8".into(),
                    clock_rate: 90000,
                    ..Default::default()
                },
                ..Default::default()
            }],
        )));
        sender.add_track(track.clone()).await.unwrap();
        let poll = track.clone();
        let counts = counts.clone();
        tokio::spawn(async move {
            while let Some(TrackLocalEvent::OnRtcpPacket(packets)) = poll.poll().await {
                if asks_keyframe(&packets) {
                    assert_eq!(packets.len(), 1);
                    assert_eq!(packets[0].destination_ssrc(), vec![0x11223300 + i as u32]);
                    if let Some(fir) = packets[0].as_any().downcast_ref::<FullIntraRequest>() {
                        assert_eq!(fir.sender_ssrc, 0x99);
                        assert_eq!(fir.media_ssrc, 0);
                        assert_eq!(fir.fir.len(), 1);
                        assert_eq!(fir.fir[0].sequence_number, if i == 0 { 7 } else { 9 });
                    }
                    counts[i].fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        locals.push(track);
    }
    let offer = sender.create_offer(None).await.unwrap();
    sender.set_local_description(offer).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), send_gather.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    receiver
        .set_remote_description(sender.local_description().await.unwrap())
        .await
        .unwrap();
    let answer = receiver.create_answer(None).await.unwrap();
    receiver.set_local_description(answer).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), recv_gather.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    sender
        .set_remote_description(receiver.local_description().await.unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), send_connect.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), recv_connect.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    for (i, track) in locals.iter().enumerate() {
        let packet = rtc::rtp::Packet {
            header: rtc::rtp::Header {
                version: 2,
                payload_type: 96,
                sequence_number: 1,
                timestamp: 3000,
                ssrc: 0x11223300 + i as u32,
                ..Default::default()
            },
            payload: vec![0xAA; 100].into(),
        };
        track.write_rtp(packet).await.unwrap();
    }
    let first = tokio::time::timeout(Duration::from_secs(3), remote.recv())
        .await
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(3), remote.recv())
        .await
        .unwrap()
        .unwrap();
    for track in [&first, &second] {
        let poll = track.clone();
        tokio::spawn(async move {
            while let Some(event) = poll.poll().await {
                if matches!(event, TrackRemoteEvent::OnEnded) {
                    break;
                }
            }
        });
    }
    let batch: Vec<Box<dyn rtcp::Packet>> = vec![
        Box::new(PictureLossIndication {
            sender_ssrc: 0,
            media_ssrc: 0x11223300,
        }),
        Box::new(PictureLossIndication {
            sender_ssrc: 0,
            media_ssrc: 0x11223301,
        }),
    ];
    for _ in 0..3 {
        first.write_rtcp(batch.clone()).await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while counts.iter().any(|count| count.load(Ordering::SeqCst) < 3) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("compound PLI must reach both local sources");
    assert_eq!(
        [
            counts[0].load(Ordering::SeqCst),
            counts[1].load(Ordering::SeqCst)
        ],
        [3, 3]
    );
    first
        .write_rtcp(vec![Box::new(FullIntraRequest {
            sender_ssrc: 0x99,
            media_ssrc: 0,
            fir: vec![
                FirEntry {
                    ssrc: 0x11223300,
                    sequence_number: 7,
                },
                FirEntry {
                    ssrc: 0x11223301,
                    sequence_number: 9,
                },
            ],
        })])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while counts.iter().any(|count| count.load(Ordering::SeqCst) < 4) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("every entry in multi-target FIR must reach its source");
    assert_eq!(
        [
            counts[0].load(Ordering::SeqCst),
            counts[1].load(Ordering::SeqCst)
        ],
        [4, 4]
    );
    sender.close().await.unwrap();
    receiver.close().await.unwrap();
}
