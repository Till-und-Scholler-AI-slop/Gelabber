//! Native core against the server's media engine (mediasoup 0.29) on loopback:
//! ICE-lite, DTLS, SRTP, encoders and decoders, with the router codecs and
//! transport options of media/src/sfu.rs.
//!
//! Proves the spike: libmediasoupclient's parameters are accepted by the same
//! mediasoup version the Gelabber media gateway runs, and media flows both ways.

use gelabber_media_core::{Audio, Device, Direction, Engine, Source, Transport, TransportEvent};
use mediasoup::prelude::*;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

/// Copy of `router_codecs` in media/src/sfu.rs (v0.4.0).
fn router_codecs() -> Vec<RtpCodecCapability> {
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
    serde_json::from_value(Value::Array(codecs)).unwrap()
}

/// Same checks as `validate_parameters` in media/src/sfu.rs.
fn validate_like_server(kind: &str, rtp: &RtpParameters) {
    assert!(!rtp.codecs.is_empty() && rtp.codecs.len() <= 16, "codec count");
    assert!(rtp.encodings.len() <= 2, "at most two encodings");
    assert!(rtp.header_extensions.len() <= 32, "header extension count");
    let first = serde_json::to_value(&rtp.codecs[0]).unwrap();
    let mime = first["mimeType"].as_str().unwrap().to_ascii_lowercase();
    if kind == "video" {
        assert!(
            matches!(mime.as_str(), "video/vp8" | "video/vp9" | "video/h264" | "video/av1"),
            "video codec {mime}"
        );
    } else {
        assert_eq!(mime, "audio/opus");
        assert!(rtp.encodings.len() <= 1);
    }
}

struct Server {
    router: Router,
    webrtc: WebRtcServer,
    _worker: Worker,
}

impl Server {
    async fn start() -> Self {
        let manager = WorkerManager::new();
        let worker = manager.create_worker(WorkerSettings::default()).await.unwrap();
        let router = worker
            .create_router(RouterOptions::new(router_codecs()))
            .await
            .unwrap();
        let listen = ListenInfo {
            protocol: Protocol::Udp,
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            announced_address: None,
            expose_internal_ip: false,
            port: None,
            port_range: Some(20000..=30000),
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        };
        let webrtc = worker
            .create_webrtc_server(WebRtcServerOptions::new(WebRtcServerListenInfos::new(listen)))
            .await
            .unwrap();
        Self {
            router,
            webrtc,
            _worker: worker,
        }
    }

    /// Mirrors `ClientFrame::CreateTransport` in media/src/sfu.rs.
    async fn transport(&self) -> (WebRtcTransport, Value) {
        let mut options = WebRtcTransportOptions::new_with_server(self.webrtc.clone());
        options.enable_tcp = false;
        options.prefer_udp = true;
        let transport = self.router.create_webrtc_transport(options).await.unwrap();
        let params = json!({
            "id": transport.id(),
            "iceParameters": transport.ice_parameters(),
            "iceCandidates": transport.ice_candidates(),
            "dtlsParameters": transport.dtls_parameters(),
        });
        (transport, params)
    }
}

/// Answers native transport events the way the media gateway does.
fn serve_events(
    runtime: Handle,
    native: Transport,
    server: WebRtcTransport,
    events: std::sync::mpsc::Receiver<TransportEvent>,
    producers: Arc<Mutex<Vec<Producer>>>,
) {
    std::thread::spawn(move || {
        for event in events {
            match event {
                TransportEvent::Connect {
                    request,
                    dtls_parameters,
                } => {
                    let dtls_parameters: DtlsParameters =
                        serde_json::from_value(dtls_parameters).unwrap();
                    let result = runtime.block_on(
                        server.connect(WebRtcTransportRemoteParameters { dtls_parameters }),
                    );
                    native
                        .respond(request, result.map(|()| json!({})).map_err(|e| e.to_string()))
                        .unwrap();
                }
                TransportEvent::Produce {
                    request,
                    kind,
                    rtp_parameters,
                    ..
                } => {
                    let rtp: RtpParameters = serde_json::from_value(rtp_parameters).unwrap();
                    validate_like_server(&kind, &rtp);
                    let media = if kind == "audio" {
                        MediaKind::Audio
                    } else {
                        MediaKind::Video
                    };
                    let result = runtime.block_on(server.produce(ProducerOptions::new(media, rtp)));
                    let answer = result
                        .map(|producer| {
                            let id = producer.id().to_string();
                            producers.lock().unwrap().push(producer);
                            json!({ "id": id })
                        })
                        .map_err(|e| e.to_string());
                    native.respond(request, answer).unwrap();
                }
                TransportEvent::ConnectionState(state) => {
                    eprintln!("native {:?} transport: {state}", native.direction());
                }
            }
        }
    });
}

async fn wait_for<F, Fut>(what: &str, timeout: Duration, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_client_round_trips_media_through_mediasoup() {
    if std::env::var_os("GELABBER_MEDIA_LOG").is_some() {
        gelabber_media_core::set_log_level(gelabber_media_core::LogLevel::Info);
    }
    let server = Server::start().await;
    let runtime = Handle::current();

    let engine = Engine::new(Audio::Dummy).unwrap();
    let device = Device::new(&engine).unwrap();
    let caps = serde_json::to_value(server.router.rtp_capabilities()).unwrap();
    device.load(&caps).unwrap();
    assert!(device.can_produce(gelabber_media_core::MediaKind::Audio).unwrap());
    assert!(device.can_produce(gelabber_media_core::MediaKind::Video).unwrap());
    let recv_caps = device.rtp_capabilities().unwrap();
    let recv_codecs: Vec<String> = recv_caps["codecs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["mimeType"].as_str().unwrap().to_ascii_lowercase())
        .collect();
    eprintln!("native receive codecs: {recv_codecs:?}");
    for mime in ["audio/opus", "video/vp8", "video/h264"] {
        assert!(recv_codecs.iter().any(|c| c == mime), "native can receive {mime}");
    }
    // The server parses this exact value in its `capabilities` handler.
    let _: RtpCapabilities = serde_json::from_value(recv_caps.clone()).unwrap();

    // Send side.
    let server_producers = Arc::new(Mutex::new(Vec::new()));
    let (server_send, send_params) = server.transport().await;
    let (send, send_events) = Transport::new(&device, Direction::Send, &send_params).unwrap();
    serve_events(
        runtime.clone(),
        send.clone(),
        server_send.clone(),
        send_events,
        server_producers.clone(),
    );

    let mic = Source::microphone(&engine).unwrap();
    let h264_source = Source::test_pattern(&engine, 1280, 720, 30).unwrap();
    let vp8_source = Source::test_pattern(&engine, 1280, 720, 30).unwrap();

    let audio = {
        let (send, mic) = (send.clone(), mic.clone());
        blocking(move || send.produce(&mic, &json!({"codecOptions": {"opusStereo": false, "opusDtx": true}})))
            .await
            .unwrap()
    };
    // Two simulcast layers like the web client (scaleResolutionDownBy 4 and 1).
    let layers = json!([{ "scaleResolutionDownBy": 4 }, { "scaleResolutionDownBy": 1 }]);
    let h264 = {
        let (send, source, layers) = (send.clone(), h264_source.clone(), layers.clone());
        blocking(move || {
            send.produce(
                &source,
                &json!({"codec": "video/H264", "encodings": layers, "codecOptions": {"videoGoogleStartBitrate": 1000}}),
            )
        })
        .await
        .unwrap()
    };
    let vp8 = {
        let (send, source) = (send.clone(), vp8_source.clone());
        blocking(move || send.produce(&source, &json!({"codec": "video/VP8", "encodings": layers})))
            .await
            .unwrap()
    };
    eprintln!("H264 rtpParameters: {}", h264.rtp_parameters().unwrap());
    assert_eq!(server_producers.lock().unwrap().len(), 3);

    let server_producer = |id: String| {
        server_producers
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id().to_string() == id)
            .cloned()
            .unwrap()
    };
    let h264_server = server_producer(h264.id().to_owned());
    let vp8_server = server_producer(vp8.id().to_owned());
    let audio_server = server_producer(audio.id().to_owned());
    assert_eq!(audio_server.kind(), MediaKind::Audio);

    for (name, producer) in [("H264", &h264_server), ("VP8", &vp8_server)] {
        wait_for(&format!("{name} RTP at the server"), Duration::from_secs(20), || {
            let producer = producer.clone();
            async move {
                let stats = producer.get_stats().await.unwrap_or_default();
                stats.iter().any(|s| s.byte_count > 0 && s.score > 0)
            }
        })
        .await;
    }
    // Simulcast layer report (both must eventually send; H264 simulcast relies on
    // libwebrtc's simulcast adapter around OpenH264).
    tokio::time::sleep(Duration::from_secs(3)).await;
    for (name, producer) in [("H264", &h264_server), ("VP8", &vp8_server)] {
        let stats = producer.get_stats().await.unwrap();
        let live = stats.iter().filter(|s| s.byte_count > 0).count();
        eprintln!(
            "{name}: {live} live layer(s): {:?}",
            stats
                .iter()
                .map(|s| (s.ssrc, s.byte_count, s.bitrate, s.score))
                .collect::<Vec<_>>()
        );
        assert_eq!(live, 2, "{name} sends both simulcast layers");
    }

    // Receive side: the server consumes for the native client, paused until
    // the client is ready (like `consumerReady`).
    let (server_recv, recv_params) = server.transport().await;
    let (recv, recv_events) = Transport::new(&device, Direction::Recv, &recv_params).unwrap();
    serve_events(
        runtime.clone(),
        recv.clone(),
        server_recv.clone(),
        recv_events,
        Arc::new(Mutex::new(Vec::new())),
    );
    let client_caps: RtpCapabilities = serde_json::from_value(recv_caps).unwrap();
    let mut consumers = Vec::new();
    for producer in [&h264_server, &vp8_server] {
        let mut options = ConsumerOptions::new(producer.id(), client_caps.clone());
        options.paused = true;
        let server_consumer = server_recv.consume(options).await.unwrap();
        let announcement = json!({
            "id": server_consumer.id(),
            "producerId": producer.id(),
            "kind": "video",
            "rtpParameters": server_consumer.rtp_parameters(),
        });
        let native = {
            let recv = recv.clone();
            blocking(move || recv.consume(&announcement)).await.unwrap()
        };
        server_consumer.resume().await.unwrap();
        consumers.push((server_consumer, native));
    }
    for (server_consumer, native) in &consumers {
        let mime = serde_json::to_value(&server_consumer.rtp_parameters().codecs[0]).unwrap()
            ["mimeType"]
            .clone();
        wait_for(
            &format!("decoded {mime} frames at the native client"),
            Duration::from_secs(20),
            || {
                let stats = native.stats().unwrap();
                async move { stats["framesReceived"].as_u64().unwrap_or(0) > 10 }
            },
        )
        .await;
        let stats = native.stats().unwrap();
        eprintln!(
            "native consumer {mime}: {} frames, {}x{}",
            stats["framesReceived"], stats["width"], stats["height"]
        );
    }

    // Native objects close in dependency order.
    drop(consumers);
    drop((audio, h264, vp8));
    drop((send, recv));
}
