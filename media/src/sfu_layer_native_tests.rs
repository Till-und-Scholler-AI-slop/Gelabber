//! Opt-in native browser acceptance against the locked SFU stack. Dedicated
//! in-process SFU and loopback signaling; no Redis/API or outside user sessions.
use super::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
#[derive(Clone)]
struct Probe {
    peer: PeerId,
    channel: Uuid,
    user: Uuid,
    frames: Arc<Mutex<mpsc::UnboundedReceiver<ServerFrame>>>,
}
#[derive(Clone)]
struct Fixture {
    sfu: Arc<Sfu>,
    peers: Arc<Mutex<Vec<Probe>>>,
}
async fn connect(f: &Fixture, channel: Uuid, body: serde_json::Value) -> serde_json::Value {
    let sdp = body["sdp"].as_str().unwrap().to_owned();
    let (out, mut frames) = mpsc::unbounded_channel();
    let user = Uuid::new_v4();
    let peer = f
        .sfu
        .join_inner(
            TicketClaim {
                u: user,
                s: Uuid::new_v4(),
                c: channel,
                g: true,
            },
            None,
            None,
            body["version"].as_u64().unwrap_or(3) as u8,
            out,
        )
        .await
        .unwrap();
    announce_sources(f, peer, channel, &body).await;
    f.sfu.apply_remote(peer, channel, sdp, true).await.unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = frames.recv().await {
            if let ServerFrame::Answer { sdp } = frame {
                return sdp;
            }
        }
        panic!("SFU answer channel ended")
    })
    .await
    .unwrap();
    let index = {
        let mut peers = f.peers.lock().await;
        peers.push(Probe {
            peer,
            channel,
            user,
            frames: Arc::new(Mutex::new(frames)),
        });
        peers.len() - 1
    };
    serde_json::json!({"sdp":answer,"probe":index,"user":user})
}
async fn announce_sources(f: &Fixture, peer: PeerId, channel: Uuid, body: &serde_json::Value) {
    if let Some(sources) = body["sources"].as_array() {
        for source in sources {
            f.sfu
                .announce_track(
                    peer,
                    channel,
                    source["kind"].as_str().unwrap(),
                    Some(source["track"].as_str().unwrap()),
                )
                .await
                .unwrap();
        }
    }
}
async fn watch_source(
    State(f): State<Fixture>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let peers = f.peers.lock().await;
    let probe = peers[index].clone();
    let user = peers[body["publisher"].as_u64().unwrap() as usize].user;
    drop(peers);
    f.sfu
        .set_watch(
            probe.peer,
            probe.channel,
            user,
            body["kind"].as_str().unwrap(),
            body["on"].as_bool().unwrap(),
        )
        .await
        .unwrap();
    Json(serde_json::json!({"ok":true}))
}
async fn debug_peer(State(f): State<Fixture>, Path(index): Path<usize>) -> Json<serde_json::Value> {
    let probe = f.peers.lock().await[index].clone();
    let room = f.sfu.find_room(probe.channel).await.unwrap();
    let (pc, gate) = {
        let room = room.lock().await;
        let peer = &room.peers[&probe.peer];
        (peer.pc.clone(), peer.sdp.clone())
    };
    let gate = gate.lock().await;
    let mut bindings = Vec::new();
    for state in gate.subscriptions.values() {
        if let SubscriptionState::Active(sub) = state {
            let params = sub.sender.get_parameters().await.unwrap();
            bindings.push(serde_json::json!({"source":sub.source,"mid":sender_mid(&pc,&sub.sender).await,"binding":*sub.payload_type.borrow(),"ssrc":params.encodings.first().and_then(|encoding|encoding.rtp_coding_parameters.ssrc),"codec":sub.codec.mime_type,"fmtp":sub.codec.sdp_fmtp_line,"parameters":params.rtp_parameters.codecs.iter().map(|c|serde_json::json!({"pt":c.payload_type,"codec":c.rtp_codec.mime_type,"fmtp":c.rtp_codec.sdp_fmtp_line})).collect::<Vec<_>>()}));
        }
    }
    let mut mids = Vec::new();
    let mut receivers = Vec::new();
    for t in pc.get_transceivers().await {
        mids.push(serde_json::json!({"mid":t.mid().await.unwrap(),"direction":format!("{:?}",t.direction().await.unwrap()),"sender":t.sender().await.unwrap().is_some()}));
        if let Some(receiver) = t.receiver().await.unwrap() {
            let track = receiver.track();
            let mut codings = Vec::new();
            for ssrc in track.ssrcs().await {
                codings.push(serde_json::json!({"ssrc":ssrc,"rid":track.rid(ssrc).await,"codec":track.codec(ssrc).await.map(|c|c.mime_type)}));
            }
            let params = receiver.get_parameters().await.ok();
            receivers.push(serde_json::json!({"mid":t.mid().await.unwrap(),"stream":track.stream_id().await,"track":track.track_id().await,"known":codings,"parameters":params.map(|params|params.rtp_parameters.codecs.iter().map(|codec|serde_json::json!({"pt":codec.payload_type,"codec":codec.rtp_codec.mime_type})).collect::<Vec<_>>())}));
        }
    }
    Json(
        serde_json::json!({"ridRecovery":gate.rid_recovery.observed(),"ridBoundSeeds":gate.rid_recovery.bound_seeds(),"haveLocalOffer":gate.have_local_offer,"receivers":receivers,"bindings":bindings,"transceivers":mids,"reserved":gate.publication_slots.len(),"active":gate.subscriptions.values().filter(|s|matches!(s,SubscriptionState::Active(_))).count()}),
    )
}
async fn offer(
    State(f): State<Fixture>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(connect(&f, Uuid::new_v4(), body).await)
}
async fn subscriber(
    State(f): State<Fixture>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let channel = f.peers.lock().await[index].channel;
    Json(connect(&f, channel, body).await)
}
async fn answer(
    State(f): State<Fixture>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let probe = f.peers.lock().await[index].clone();
    f.sfu
        .apply_remote(
            probe.peer,
            probe.channel,
            body["sdp"].as_str().unwrap().into(),
            false,
        )
        .await
        .unwrap();
    Json(serde_json::json!({"ok":true}))
}
async fn renegotiate(
    State(f): State<Fixture>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let probe = f.peers.lock().await[index].clone();
    // Hold before applying the offer: the concurrent poll route must not
    // consume this client-offer answer before its request receives it.
    let mut frames = probe.frames.lock().await;
    announce_sources(&f, probe.peer, probe.channel, &body).await;
    f.sfu
        .apply_remote(
            probe.peer,
            probe.channel,
            body["sdp"].as_str().unwrap().into(),
            true,
        )
        .await
        .unwrap();
    let sdp = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = frames.recv().await {
            match frame {
                ServerFrame::Answer { sdp } => return sdp,
                ServerFrame::Ice { .. } => {}
                other => panic!("unexpected client-offer response {other:?}"),
            }
        }
        panic!("source restart answer channel ended")
    })
    .await
    .unwrap();
    Json(serde_json::json!({"sdp":sdp}))
}
async fn retract(
    State(f): State<Fixture>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let probe = f.peers.lock().await[index].clone();
    f.sfu
        .retract_track(
            probe.peer,
            probe.channel,
            body["kind"].as_str().unwrap(),
            body["track"].as_str(),
        )
        .await
        .unwrap();
    Json(serde_json::json!({"ok":true}))
}
async fn poll(State(f): State<Fixture>, Path(index): Path<usize>) -> Json<serde_json::Value> {
    let probe = f.peers.lock().await[index].clone();
    let mut frames = probe.frames.lock().await;
    let mut result = Vec::new();
    while let Ok(frame) = frames.try_recv() {
        result.push(frame);
    }
    Json(serde_json::json!(result))
}
async fn layer(
    State(f): State<Fixture>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let peers = f.peers.lock().await;
    let probe = peers[index].clone();
    let user = peers[body["publisher"].as_u64().unwrap() as usize].user;
    drop(peers);
    f.sfu
        .set_viewer_layer(
            probe.peer,
            probe.channel,
            user,
            body["kind"].as_str().unwrap_or("v"),
            body["height"].as_u64().unwrap() as u16,
            body["congested"].as_bool().unwrap_or(false),
        )
        .await
        .unwrap();
    Json(serde_json::json!({"ok":true}))
}
async fn observed(State(f): State<Fixture>, Path(index): Path<usize>) -> Json<serde_json::Value> {
    let probe = f.peers.lock().await[index].clone();
    let room = f.sfu.find_room(probe.channel).await.unwrap();
    let tracks = room.lock().await.peers[&probe.peer]
        .remote_tracks
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut layers = Vec::new();
    for track in tracks {
        if track.kind().await != RtpCodecKind::Video {
            continue;
        }
        for ssrc in track.ssrcs().await {
            layers.push(serde_json::json!({"rid":track.rid(ssrc).await,"codec":track.codec(ssrc).await.map(|c|c.mime_type)}));
        }
    }
    let publications = room
        .lock()
        .await
        .pubs
        .values()
        .filter(|p| p.publisher == probe.peer)
        .map(|p| serde_json::json!({"source":p.stream_id,"track":p.track_id,"layered":p.layered,"received":p.received_packets.load(Ordering::Relaxed),"listeners":p.packets.receiver_count()}))
        .collect::<Vec<_>>();
    Json(serde_json::json!({"layers":layers,"publications":publications}))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "local native Chromium/Firefox two-viewer test; npm ci in web, run --ignored --exact"]
async fn native_browser_rid_feasibility() {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".into()),
        "MEDIA_ICE_BIND" => Some("0.0.0.0:0".into()),
        _ => None,
    })
    .unwrap();
    let f = Fixture {
        sfu: Arc::new(Sfu::new(&config)),
        peers: Arc::new(Mutex::new(Vec::new())),
    };
    let app = Router::new()
        .route(
            "/",
            get(|| async {
                axum::response::Html("<!doctype html><title>Owned layer fixture</title>")
            }),
        )
        .route("/offer", post(offer))
        .route("/subscriber/{index}", post(subscriber))
        .route("/answer/{index}", post(answer))
        .route("/renegotiate/{index}", post(renegotiate))
        .route("/retract/{index}", post(retract))
        .route("/poll/{index}", get(poll))
        .route("/layer/{index}", post(layer))
        .route("/observed/{index}", get(observed))
        .route("/watch/{index}", post(watch_source))
        .route("/debug/{index}", get(debug_peer))
        .with_state(f.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        if std::env::var_os("GELABBER_LAYER_LIFECYCLE_ONLY").is_some() {
            "../web/scripts/layers/native-lifecycle-probe.mjs"
        } else {
            "../web/scripts/layers/native-rid-probe.mjs"
        },
    );
    let result = tokio::task::spawn_blocking(move || {
        std::process::Command::new("node")
            .arg(script)
            .arg(url)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    for probe in f.peers.lock().await.clone() {
        f.sfu.leave(probe.peer, probe.channel).await;
    }
    server.abort();
    assert!(
        result.status.success(),
        "native layer acceptance failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("{}", String::from_utf8_lossy(&result.stdout));
}
