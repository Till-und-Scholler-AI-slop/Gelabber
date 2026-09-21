//! An answer rejected in the media WebSocket before `apply_remote` must
//! still abort the outstanding SFU offer. The next publish then gets out.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use gelabber_media::ws::{MAX_FRAME, MAX_SDP};
use gelabber_media::{AppState, Config};
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp::packet::Packet;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCIceGatheringState, RTCPeerConnectionState, RTCSessionDescription,
    register_default_interceptors,
};

struct Handler {
    gathered: watch::Sender<u64>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let next = *self.gathered.borrow() + 1;
            let _ = self.gathered.send(next);
        }
    }

    async fn on_ice_candidate(&self, _event: webrtc::peer_connection::RTCPeerConnectionIceEvent) {}

    async fn on_connection_state_change(&self, _state: RTCPeerConnectionState) {}
}

struct Client {
    pc: Arc<dyn PeerConnection>,
    gathered: watch::Receiver<u64>,
}

async fn client_pc() -> Client {
    let mut media = MediaEngine::default();
    media.register_default_codecs().unwrap();
    let registry =
        register_default_interceptors(webrtc::peer_connection::Registry::new(), &mut media)
            .unwrap();
    let (gather_tx, gather_rx) = watch::channel(0);
    let pc = PeerConnectionBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(Arc::new(Handler {
            gathered: gather_tx,
        }))
        .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
        .build()
        .await
        .unwrap();
    Client {
        pc: Arc::new(pc),
        gathered: gather_rx,
    }
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

async fn serve() -> (std::net::SocketAddr, redis::Client) {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let redis = redis::Client::open(redis_url.as_str()).expect("redis url");
    redis
        .get_multiplexed_async_connection()
        .await
        .expect("redis reachable");
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(redis_url.clone()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        "TURN_URLS" => Some("stun:127.0.0.1:3478,turn:127.0.0.1:3478".into()),
        "TURN_USERNAME" => Some("gelabber".into()),
        "TURN_PASSWORD" => Some("gelabberturn".into()),
        _ => None,
    })
    .expect("config");
    let state = AppState::from_config(&config).expect("state");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, gelabber_media::app(state))
            .await
            .expect("serve");
    });
    (addr, redis)
}

async fn mint(redis: &redis::Client, code: &str, user: u128, channel: u128) {
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let claim = json!({
        "u": Uuid::from_u128(user),
        "s": Uuid::from_u128(9),
        "c": Uuid::from_u128(channel),
    });
    let _: () = redis::cmd("SET")
        .arg(format!("gb:mt:{code}"))
        .arg(claim.to_string())
        .arg("EX")
        .arg(60)
        .query_async(&mut conn)
        .await
        .unwrap();
}

async fn connect(
    addr: std::net::SocketAddr,
    code: &str,
) -> (
    mpsc::UnboundedReceiver<Value>,
    mpsc::UnboundedSender<String>,
) {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .expect("ws");
    ws.send(Message::Text(
        format!(r#"{{"op":"j","tk":"{code}"}}"#).into(),
    ))
    .await
    .unwrap();
    let (mut sink, mut stream) = ws.split();
    let (in_tx, in_rx) = mpsc::unbounded_channel();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                incoming = stream.next() => {
                    match incoming {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                                if in_tx.send(value).is_err() {
                                    break;
                                }
                            }
                        }
                        Some(Ok(_)) => {}
                        _ => break,
                    }
                }
                outgoing = out_rx.recv() => {
                    let Some(text) = outgoing else { break };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    (in_rx, out_tx)
}

async fn expect_ok(rx: &mut mpsc::UnboundedReceiver<Value>) {
    let frame = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("join frame")
        .expect("open");
    assert_eq!(frame["op"], "ok", "{frame}");
}

async fn wait_gather(gathered: &watch::Receiver<u64>, before: u64) {
    let mut rx = gathered.clone();
    if *rx.borrow() <= before {
        let _ = tokio::time::timeout(Duration::from_secs(2), rx.wait_for(|n| *n > before)).await;
    }
}

async fn send_offer(client: &Client, tx: &mpsc::UnboundedSender<String>) {
    let before = *client.gathered.borrow();
    let offer = client.pc.create_offer(None).await.unwrap();
    client.pc.set_local_description(offer).await.unwrap();
    wait_gather(&client.gathered, before).await;
    let local = client.pc.local_description().await.unwrap();
    tx.send(json!({ "op": "o", "sdp": local.sdp }).to_string())
        .unwrap();
}

async fn take_matching(
    rx: &mut mpsc::UnboundedReceiver<Value>,
    deadline: Duration,
    pred: impl Fn(&Value) -> bool,
) -> Option<Value> {
    let start = tokio::time::Instant::now();
    while start.elapsed() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Some(frame)) if pred(&frame) => return Some(frame),
            Ok(Some(_)) | Err(_) => {}
            Ok(None) => return None,
        }
    }
    None
}

async fn apply_answer(client: &Client, rx: &mut mpsc::UnboundedReceiver<Value>) {
    let frame = take_matching(rx, Duration::from_secs(5), |frame| frame["op"] == "a")
        .await
        .expect("sfu answer");
    let sdp = frame["sdp"].as_str().unwrap().to_owned();
    client
        .pc
        .set_remote_description(RTCSessionDescription::answer(sdp).unwrap())
        .await
        .unwrap();
}

/// Oversized subscriber answer is refused in the WebSocket, then a later
/// publish must still be offered.
#[tokio::test]
async fn oversized_answer_on_ws_lets_the_next_publish_through() {
    let (addr, redis) = serve().await;
    let channel = 7u128;
    mint(&redis, "abcdefghjkmn", 1, channel).await;
    mint(&redis, "bbcdefghjkmn", 2, channel).await;
    let (mut a_rx, a_tx) = connect(addr, "abcdefghjkmn").await;
    let (mut b_rx, b_tx) = connect(addr, "bbcdefghjkmn").await;
    expect_ok(&mut a_rx).await;
    expect_ok(&mut b_rx).await;

    let a = client_pc().await;
    let b = client_pc().await;
    a.pc.add_track(Arc::clone(&opus_track(0x1111_0001)) as Arc<dyn TrackLocal>)
        .await
        .unwrap();
    b.pc.add_track(Arc::clone(&opus_track(0x2222_0001)) as Arc<dyn TrackLocal>)
        .await
        .unwrap();
    send_offer(&a, &a_tx).await;
    apply_answer(&a, &mut a_rx).await;
    send_offer(&b, &b_tx).await;
    apply_answer(&b, &mut b_rx).await;

    let video = vp8_track(0x3333_0001);
    a.pc.add_track(Arc::clone(&video) as Arc<dyn TrackLocal>)
        .await
        .unwrap();
    send_offer(&a, &a_tx).await;
    apply_answer(&a, &mut a_rx).await;

    let mut pkt = Packet::default();
    pkt.header.version = 2;
    pkt.header.ssrc = 0x3333_0001;
    pkt.header.payload_type = 96;
    pkt.payload = bytes::Bytes::from_static(&[0x10, 0x00, 0x00]);
    let mut seq = 1u16;
    let first = loop {
        pkt.header.sequence_number = seq;
        pkt.header.timestamp = u32::from(seq) * 3000;
        seq = seq.wrapping_add(1);
        let _ = video.write_rtp(pkt.clone()).await;
        if let Some(frame) = take_matching(&mut b_rx, Duration::from_millis(80), |frame| {
            frame["op"] == "o"
        })
        .await
        {
            break frame;
        }
        if seq > 80 {
            panic!("subscriber never received the outstanding video offer");
        }
    };
    let offer = first["sdp"].as_str().unwrap();
    assert!(
        offer.contains("VP8/90000"),
        "outstanding offer must be real video: {offer}"
    );

    // Larger than MAX_SDP, still under MAX_FRAME, so ws.rs rejects it as an
    // answer before apply_remote.
    let pad = "x".repeat(MAX_SDP + 32);
    assert!(pad.len() > MAX_SDP && pad.len() + 32 < MAX_FRAME);
    b_tx.send(json!({ "op": "a", "sdp": format!("v=0\r\n{pad}") }).to_string())
        .unwrap();
    let err = take_matching(&mut b_rx, Duration::from_secs(3), |frame| {
        frame["op"] == "err"
    })
    .await
    .expect("oversized answer is negotiation_failed");
    assert_eq!(err["e"], "negotiation_failed", "{err}");

    let again = vp8_track(0x4444_0001);
    a.pc.add_track(Arc::clone(&again) as Arc<dyn TrackLocal>)
        .await
        .unwrap();
    send_offer(&a, &a_tx).await;
    apply_answer(&a, &mut a_rx).await;
    let mut seq = 1u16;
    pkt.header.ssrc = 0x4444_0001;
    let second = loop {
        pkt.header.sequence_number = seq;
        pkt.header.timestamp = u32::from(seq) * 3000;
        seq = seq.wrapping_add(1);
        let _ = again.write_rtp(pkt.clone()).await;
        let _ = video.write_rtp(pkt.clone()).await;
        if let Some(frame) = take_matching(&mut b_rx, Duration::from_millis(80), |frame| {
            frame["op"] == "o"
        })
        .await
        {
            break frame;
        }
        if seq > 80 {
            panic!("next publish produced no offer after the oversized answer");
        }
    };
    let offer = second["sdp"].as_str().unwrap();
    assert!(
        offer.contains("VP8/90000"),
        "offer after the aborted answer must carry the new video: {offer}"
    );
    b.pc.set_remote_description(RTCSessionDescription::offer(offer.to_owned()).unwrap())
        .await
        .expect("subscriber accepts the offer that follows the oversized answer");
}
