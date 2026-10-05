//! Evaluation-only control adapter. No production ticket/ACL semantics are claimed.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use mediasoup::{prelude::*, worker::WorkerLogLevel};
use serde_json::{Value, json};
use std::{collections::HashMap, net::Ipv4Addr, sync::Arc};
use tokio::sync::Mutex;

type Failure = (StatusCode, Json<Value>);
type Reply = Result<Json<Value>, Failure>;
fn failure(message: impl ToString) -> Failure {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": message.to_string()})),
    )
}
fn field<'a>(request: &'a Value, key: &str) -> Result<&'a str, Failure> {
    request[key]
        .as_str()
        .ok_or_else(|| failure(format!("missing {key}")))
}
#[derive(Default)]
struct Peer {
    transports: HashMap<String, WebRtcTransport>,
    producers: HashMap<String, Producer>,
    consumers: HashMap<String, Consumer>,
}
#[derive(Clone)]
struct App {
    router: mediasoup::router::Router,
    _worker: Worker,
    token: Arc<String>,
    announced: Arc<String>,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
}
async fn health(State(app): State<App>) -> Reply {
    if app._worker.closed() {
        return Err(failure("worker closed"));
    }
    Ok(Json(
        json!({"backend":"mediasoup-rust", "crate":"0.29.0", "worker":"mediasoup-sys 0.19.0", "production_feature_acceptance":false}),
    ))
}
async fn rpc(State(app): State<App>, headers: HeaderMap, Json(request): Json<Value>) -> Reply {
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        != Some(&format!("Bearer {}", app.token))
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        ));
    }
    let operation = field(&request, "op")?;
    if operation == "capabilities" {
        return Ok(Json(json!(app.router.rtp_capabilities())));
    }
    if operation == "reset" {
        app.peers.lock().await.clear();
        return Ok(Json(json!({"ok":true})));
    }
    if operation == "summary" {
        let peers = app.peers.lock().await;
        return Ok(Json(
            json!({"peers":peers.len(), "transports":peers.values().map(|p|p.transports.len()).sum::<usize>(), "producers":peers.values().map(|p|p.producers.len()).sum::<usize>(), "consumers":peers.values().map(|p|p.consumers.len()).sum::<usize>()}),
        ));
    }
    let peer = field(&request, "peer")?.to_owned();
    if peer.is_empty()
        || peer.len() > 64
        || !peer.bytes().all(|v| v.is_ascii_alphanumeric() || v == b'-')
    {
        return Err(failure("invalid peer"));
    }
    // Serialize control mutations so concurrent leave/create cannot race limits.
    let mut peers = app.peers.lock().await;
    if operation == "join" {
        if peers.contains_key(&peer) {
            return Err(failure("duplicate peer"));
        }
        if peers.len() >= 64 {
            return Err(failure("evaluation peer limit"));
        }
        peers.insert(peer, Peer::default());
        return Ok(Json(json!({"ok":true})));
    }
    if operation == "leave" {
        peers.remove(&peer);
        return Ok(Json(json!({"ok":true})));
    }
    if !peers.contains_key(&peer) {
        return Err(failure("unknown peer"));
    }
    if operation == "transport" {
        if peers[&peer].transports.len() >= 2 {
            return Err(failure("evaluation transport limit"));
        }
        let listen = ListenInfo {
            protocol: Protocol::Udp,
            ip: Ipv4Addr::UNSPECIFIED.into(),
            announced_address: Some((*app.announced).clone()),
            expose_internal_ip: false,
            port: None,
            port_range: Some(10000..=10199),
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        };
        let mut options = WebRtcTransportOptions::new(WebRtcTransportListenInfos::new(listen));
        options.initial_available_outgoing_bitrate = 6_000_000;
        let transport = app
            .router
            .create_webrtc_transport(options)
            .await
            .map_err(failure)?;
        let response = json!({"id":transport.id(), "iceParameters":transport.ice_parameters(), "iceCandidates":transport.ice_candidates(), "dtlsParameters":transport.dtls_parameters()});
        peers
            .get_mut(&peer)
            .ok_or_else(|| failure("peer left"))?
            .transports
            .insert(transport.id().to_string(), transport);
        return Ok(Json(response));
    }
    if operation == "resume" {
        let consumer = peers[&peer]
            .consumers
            .get(field(&request, "consumerId")?)
            .cloned()
            .ok_or_else(|| failure("unknown consumer"))?;
        consumer.resume().await.map_err(failure)?;
        return Ok(Json(json!({"ok":true})));
    }
    let transport = peers[&peer]
        .transports
        .get(field(&request, "transportId")?)
        .cloned()
        .ok_or_else(|| failure("unknown transport"))?;
    match operation {
        "connect" => {
            transport
                .connect(
                    serde_json::from_value::<WebRtcTransportRemoteParameters>(request.clone())
                        .map_err(failure)?,
                )
                .await
                .map_err(failure)?;
            Ok(Json(json!({"ok":true})))
        }
        "produce" => {
            if peers[&peer].producers.len() >= 3 {
                return Err(failure("evaluation producer limit"));
            }
            let kind =
                serde_json::from_value::<MediaKind>(request["kind"].clone()).map_err(failure)?;
            let parameters =
                serde_json::from_value::<RtpParameters>(request["rtpParameters"].clone())
                    .map_err(failure)?;
            let producer = transport
                .produce(ProducerOptions::new(kind, parameters))
                .await
                .map_err(failure)?;
            let response = json!({"id":producer.id()});
            peers
                .get_mut(&peer)
                .ok_or_else(|| failure("peer left"))?
                .producers
                .insert(producer.id().to_string(), producer);
            Ok(Json(response))
        }
        "consume" => {
            if peers[&peer].consumers.len() >= 128 {
                return Err(failure("evaluation consumer limit"));
            }
            let producer_id = field(&request, "producerId")?.parse().map_err(failure)?;
            let capabilities =
                serde_json::from_value::<RtpCapabilities>(request["rtpCapabilities"].clone())
                    .map_err(failure)?;
            let mut options = ConsumerOptions::new(producer_id, capabilities);
            options.paused = true;
            let consumer = transport.consume(options).await.map_err(failure)?;
            let response = json!({"id":consumer.id(), "producerId":consumer.producer_id(), "kind":consumer.kind(), "rtpParameters":consumer.rtp_parameters()});
            peers
                .get_mut(&peer)
                .ok_or_else(|| failure("peer left"))?
                .consumers
                .insert(consumer.id().to_string(), consumer);
            Ok(Json(response))
        }
        "stats" => Ok(Json(json!(transport.get_stats().await.map_err(failure)?))),
        _ => Err(failure("unknown operation")),
    }
}
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("BENCH_TOKEN")?;
    if token.len() < 32 {
        return Err("BENCH_TOKEN must contain at least 32 bytes".into());
    }
    let worker = WorkerManager::new()
        .create_worker({
            let mut s = WorkerSettings::default();
            s.log_level = WorkerLogLevel::Error;
            s
        })
        .await?;
    let codecs = serde_json::from_value(json!([
        {"kind":"audio","mimeType":"audio/opus","clockRate":48000,"channels":2},
        {"kind":"video","mimeType":"video/VP8","clockRate":90000,"rtcpFeedback":[{"type":"nack"},{"type":"nack","parameter":"pli"},{"type":"ccm","parameter":"fir"},{"type":"goog-remb"},{"type":"transport-cc"}]}
    ]))?;
    let router = worker.create_router(RouterOptions::new(codecs)).await?;
    let state = App {
        router,
        _worker: worker,
        token: Arc::new(token),
        announced: Arc::new(
            std::env::var("BENCH_ADVERTISED_IP").unwrap_or_else(|_| "127.0.0.1".into()),
        ),
        peers: Arc::default(),
    };
    let routes = Router::new()
        .route("/health", get(health))
        .route("/ready", get(health))
        .route("/rpc", post(rpc))
        .layer(DefaultBodyLimit::max(128 * 1024))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(
        std::env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:8091".into()),
    )
    .await?;
    println!(
        "mediasoup-rust benchmark adapter listening on {}",
        listener.local_addr()?
    );
    axum::serve(listener, routes)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
