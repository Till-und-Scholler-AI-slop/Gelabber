//! Shared by the integration tests: a mediasoup 0.29 router configured like
//! the media gateway (media/src/sfu.rs) and the event loop that answers native
//! transport events the way the gateway does.
#![allow(dead_code)]

use gelabber_media_core::{Transport, TransportEvent};
use mediasoup::prelude::*;
// Trait methods (id, produce, consume); the name is taken by the native transport.
use mediasoup::prelude::Transport as _;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

/// Copy of `router_codecs` in media/src/sfu.rs (v0.4.0).
pub fn router_codecs() -> Vec<RtpCodecCapability> {
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
pub fn validate_like_server(kind: &str, rtp: &RtpParameters) {
    assert!(
        !rtp.codecs.is_empty() && rtp.codecs.len() <= 16,
        "codec count"
    );
    assert!(rtp.encodings.len() <= 2, "at most two encodings");
    assert!(rtp.header_extensions.len() <= 32, "header extension count");
    let first = serde_json::to_value(&rtp.codecs[0]).unwrap();
    let mime = first["mimeType"].as_str().unwrap().to_ascii_lowercase();
    if kind == "video" {
        assert!(
            matches!(
                mime.as_str(),
                "video/vp8" | "video/vp9" | "video/h264" | "video/av1"
            ),
            "video codec {mime}"
        );
    } else {
        assert_eq!(mime, "audio/opus");
        assert!(rtp.encodings.len() <= 1);
    }
}

/// Where the test's mediasoup listens, given the value of
/// GELABBER_TEST_LISTEN_IP: 127.0.0.1 unless it names another address of this
/// machine. libwebrtc gathers no loopback candidate and sends from the
/// machine's own address. Linux carries that to 127.0.0.1; a system that
/// keeps the two apart (reported of Windows, whose CI job tries it and sets
/// the variable) needs the server on the machine's address.
pub fn listen_ip_from(configured: Option<&str>) -> IpAddr {
    match configured {
        Some(ip) if !ip.is_empty() => ip
            .parse()
            .unwrap_or_else(|_| panic!("GELABBER_TEST_LISTEN_IP is no IP address: {ip}")),
        _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
    }
}

/// The address for this run, from its environment.
pub fn listen_ip() -> IpAddr {
    listen_ip_from(std::env::var("GELABBER_TEST_LISTEN_IP").ok().as_deref())
}

pub struct Server {
    pub router: Router,
    webrtc: WebRtcServer,
    _worker: Worker,
}

impl Server {
    pub async fn start() -> Self {
        let manager = WorkerManager::new();
        let worker = manager
            .create_worker(WorkerSettings::default())
            .await
            .unwrap();
        let router = worker
            .create_router(RouterOptions::new(router_codecs()))
            .await
            .unwrap();
        let listen = ListenInfo {
            protocol: Protocol::Udp,
            ip: listen_ip(),
            announced_address: None,
            expose_internal_ip: false,
            port: None,
            port_range: Some(20000..=30000),
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        };
        let webrtc = worker
            .create_webrtc_server(WebRtcServerOptions::new(WebRtcServerListenInfos::new(
                listen,
            )))
            .await
            .unwrap();
        Self {
            router,
            webrtc,
            _worker: worker,
        }
    }

    /// Mirrors `ClientFrame::CreateTransport` in media/src/sfu.rs.
    pub async fn transport(&self) -> (WebRtcTransport, Value) {
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
pub fn serve_events(
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
                        .respond(
                            request,
                            result.map(|()| json!({})).map_err(|e| e.to_string()),
                        )
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

pub async fn wait_for<F, Fut>(what: &str, timeout: Duration, mut check: F)
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

pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

/// The sender's own view of each layer it encodes.
pub fn outbound_rtp(producer: &gelabber_media_core::Producer) -> Value {
    let layers = producer.stats().unwrap();
    let layers = layers.as_array().into_iter().flatten();
    layers
        .filter(|s| s["type"] == "outbound-rtp")
        .map(|s| {
            json!({
                "rid": s["rid"], "active": s["active"],
                "frameWidth": s["frameWidth"], "frameHeight": s["frameHeight"],
                "framesEncoded": s["framesEncoded"], "bytesSent": s["bytesSent"],
                "targetBitrate": s["targetBitrate"],
                "qualityLimitationReason": s["qualityLimitationReason"],
                "encoderImplementation": s["encoderImplementation"],
            })
        })
        .collect()
}

/// Waits until RTP of two simulcast layers reached the server's producer and
/// returns the sizes the native producer encodes them at, the small layer
/// first. Fails with the sender's view of its layers when they do not come.
pub async fn simulcast_sizes(
    name: &str,
    native: &gelabber_media_core::Producer,
    at_server: &Producer,
) -> Vec<(u64, u64)> {
    let start = Instant::now();
    let (mut live, mut sizes) = (0, Vec::new());
    while (live, sizes.len()) != (2, 2) && start.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let stats = at_server.get_stats().await.unwrap_or_default();
        live = stats.iter().filter(|s| s.byte_count > 0).count();
        sizes = outbound_rtp(native)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|layer| {
                Some((
                    layer["frameWidth"].as_u64()?,
                    layer["frameHeight"].as_u64()?,
                ))
            })
            .collect();
    }
    eprintln!(
        "{name}: {live} live layer(s) after {:?}: {}",
        start.elapsed(),
        outbound_rtp(native)
    );
    assert_eq!(live, 2, "{name} sends both simulcast layers");
    sizes.sort_unstable();
    sizes
}

/// The two layers of a `width` x `height` picture with the small one at
/// 1/`factor`, the small one first: the picture keeps of each side what
/// divides by the factor, and the small layer is that divided, to the pixel.
/// (A local source keeps multiples of 4 by itself; the factors here are 4
/// and 8.)
pub fn layers_of(width: u64, height: u64, factor: u64) -> Vec<(u64, u64)> {
    let whole = (width - width % factor, height - height % factor);
    vec![(whole.0 / factor, whole.1 / factor), whole]
}

/// Whether the native device's RTP capabilities list `mime` (e.g.
/// "video/h264").
pub fn can_receive(capabilities: &Value, mime: &str) -> bool {
    capabilities["codecs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|codec| codec["mimeType"].as_str())
        .any(|found| found.eq_ignore_ascii_case(mime))
}

/// What in the environment says the core must have H264, given the values of
/// GELABBER_EXPECT_H264 and GELABBER_EXPECT_ENCODER: the first set to 1, or
/// the second naming the H264 encoder a test is to find.
pub fn h264_demanded_by(
    expect_h264: Option<&str>,
    expect_encoder: Option<&str>,
) -> Option<&'static str> {
    if expect_h264 == Some("1") {
        Some("GELABBER_EXPECT_H264=1")
    } else if expect_encoder.is_some_and(|name| !name.is_empty()) {
        Some("GELABBER_EXPECT_ENCODER is set")
    } else {
        None
    }
}

/// Whether the H264 half of a test runs. The Windows core has no H264, so a
/// device without it only skips that half; GELABBER_EXPECT_H264=1 (set where
/// the core is built with H264, as on Linux) turns a missing codec into a
/// failure instead, and so does GELABBER_EXPECT_ENCODER: its check needs an
/// H264 producer.
pub fn h264_under_test(capabilities: &Value) -> bool {
    let available = can_receive(capabilities, "video/h264");
    let (expect_h264, expect_encoder) = (
        std::env::var("GELABBER_EXPECT_H264").ok(),
        std::env::var("GELABBER_EXPECT_ENCODER").ok(),
    );
    if let Some(demand) = h264_demanded_by(expect_h264.as_deref(), expect_encoder.as_deref()) {
        assert!(available, "{demand}, but the native device has no H264");
    }
    available
}

/// With GELABBER_EXPECT_ENCODER set, some outbound-rtp entry's
/// `encoderImplementation` must contain it (e.g. "GStreamer x264enc").
pub fn check_encoder(name: &str, stats: &Value) {
    let Ok(expected) = std::env::var("GELABBER_EXPECT_ENCODER") else {
        return;
    };
    let found: Vec<String> = stats
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["type"] == "outbound-rtp")
        .filter_map(|s| s["encoderImplementation"].as_str().map(str::to_owned))
        .collect();
    eprintln!("{name} encoder: {found:?}");
    assert!(
        found.iter().any(|i| i.contains(&expected)),
        "{name} encoder {found:?}, expected {expected}"
    );
}
