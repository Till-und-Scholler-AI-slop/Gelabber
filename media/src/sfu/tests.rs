use super::*;
use super::{rpc::note_consumer_failure, stop::stop_batch};
use gelabber_shared::ticket::{TicketAuthorization, TicketClaim};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn first_engine(sfu: &Sfu) -> Arc<Engine> {
    sfu.engines.lock().unwrap()[0].clone()
}

fn receiver(codecs: Value) -> RtpCapabilities {
    serde_json::from_value(json!({ "codecs": codecs, "headerExtensions": [] })).unwrap()
}

#[test]
fn av1_and_vp9_come_as_one_stream() {
    let sent = |mime: &str, layers: usize| -> RtpParameters {
        let encodings: Vec<Value> = (0..layers)
            .map(|layer| json!({ "ssrc": 1000 + layer, "rid": format!("r{layer}") }))
            .collect();
        serde_json::from_value(json!({
            "codecs": [{"mimeType": mime, "payloadType": 45, "clockRate": 90000}],
            "encodings": encodings,
            "headerExtensions": [],
            "rtcp": { "reducedSize": true },
        }))
        .unwrap()
    };
    for mime in ["video/AV1", "video/VP9"] {
        assert!(validate_parameters(SourceKind::Camera, &sent(mime, 1)).is_ok());
        assert!(matches!(
            validate_parameters(SourceKind::Screen, &sent(mime, 2)),
            Err(SfuError::UnsupportedCodec)
        ));
    }
    for mime in ["video/VP8", "video/H264"] {
        assert!(validate_parameters(SourceKind::Camera, &sent(mime, 2)).is_ok());
    }
}

#[test]
fn video_codecs_are_those_every_receiver_decodes() {
    let h264 = |profile: &str, mode: u8| {
        json!({"kind":"video","mimeType":"video/H264","clockRate":90000,"preferredPayloadType":102,
            "parameters":{"profile-level-id":profile,"packetization-mode":mode}})
    };
    let vp8 =
        json!({"kind":"video","mimeType":"video/VP8","clockRate":90000,"preferredPayloadType":96});
    let av1 =
        json!({"kind":"video","mimeType":"video/AV1","clockRate":90000,"preferredPayloadType":45});
    let opus = json!({"kind":"audio","mimeType":"audio/opus","clockRate":48000,"channels":2,"preferredPayloadType":111});
    let none: [RtpCapabilities; 0] = [];
    assert_eq!(
        common_video_codecs(none.iter()),
        ["video/vp8", "video/vp9", "video/h264", "video/av1"]
    );
    let chrome = receiver(json!([opus, vp8, h264("42e01f", 1), av1]));
    let desktop = receiver(json!([opus, vp8, h264("42e01f", 1)]));
    assert_eq!(
        common_video_codecs([&chrome].into_iter()),
        ["video/vp8", "video/h264", "video/av1"]
    );
    assert_eq!(
        common_video_codecs([&chrome, &desktop].into_iter()),
        ["video/vp8", "video/h264"]
    );
    // Firefox without its H264 plugin; decoders of plain Baseline, High
    // or mode 0 only.
    let firefox = receiver(json!([opus, vp8, av1]));
    let high_only = receiver(json!([
        opus,
        vp8,
        h264("42001f", 1),
        h264("640032", 1),
        h264("42e01f", 0)
    ]));
    assert_eq!(
        common_video_codecs([&chrome, &firefox].into_iter()),
        ["video/vp8", "video/av1"]
    );
    assert_eq!(common_video_codecs([&high_only].into_iter()), ["video/vp8"]);
    // A room starts with the set of no receivers: peers that state no
    // capabilities (old clients, tests) are never sent a codecs frame.
    assert!(RoomData::default().update_video_codecs(None).is_none());
}

#[test]
fn consumer_failures_stop_retrying_per_producer() {
    let mut failed = HashMap::new();
    let live = |id: &str| id != "gone";
    failed.insert("gone".to_owned(), 3);
    for _ in 1..CONSUMER_FAILURE_LIMIT {
        assert!(note_consumer_failure(&mut failed, live, "a".to_owned()));
    }
    assert!(!note_consumer_failure(&mut failed, live, "a".to_owned()));
    assert!(!failed.contains_key("gone"));
    // Another producer, e.g. the same source republished, has its own budget.
    assert!(note_consumer_failure(&mut failed, live, "b".to_owned()));
    assert!(!note_consumer_failure(&mut failed, live, "a".to_owned()));
    assert_eq!(failed["a"], CONSUMER_FAILURE_LIMIT + 1);
}

#[tokio::test]
async fn stop_batch_deadline_includes_queued_unpaused_resources() {
    let started = Arc::new(AtomicUsize::new(0));
    let paused = Arc::new(AtomicUsize::new(0));
    let jobs = (0..=STOP_CONCURRENCY).map(|index| {
        let started = started.clone();
        let paused = paused.clone();
        async move {
            started.fetch_add(1, Ordering::SeqCst);
            if index < STOP_CONCURRENCY {
                std::future::pending::<()>().await;
            }
            paused.fetch_add(1, Ordering::SeqCst);
        }
    });
    assert!(stop_batch(jobs).await.is_err());
    assert_eq!(started.load(Ordering::SeqCst), STOP_CONCURRENCY);
    assert_eq!(paused.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stop_batch_waits_for_concurrent_pause_acknowledgements() {
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let paused = Arc::new(AtomicUsize::new(0));
    let jobs = (0..2).map(|_| {
        let barrier = barrier.clone();
        let paused = paused.clone();
        async move {
            barrier.wait().await;
            paused.fetch_add(1, Ordering::SeqCst);
        }
    });
    assert!(stop_batch(jobs).await.is_ok());
    assert_eq!(paused.load(Ordering::SeqCst), 2);
}

async fn stopping_fixture() -> (
    Arc<Sfu>,
    Arc<Peer>,
    Arc<Publication>,
    mpsc::Receiver<ServerFrame>,
) {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(std::env::var(key).expect("isolated REDIS_URL required")),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        _ => None,
    })
    .unwrap();
    let sfu = Sfu::with_redis(
        &config,
        redis::Client::open(config.redis_url.as_str()).unwrap(),
    )
    .await
    .unwrap();
    let engine = first_engine(&sfu);
    let router = engine
        .native(engine.worker.clone(), |worker| async move {
            worker
                .create_router(RouterOptions::new(router_codecs().unwrap()))
                .await
        })
        .await
        .unwrap();
    let mut options = WebRtcTransportOptions::new_with_server(engine.server.clone());
    options.app_data = AppData::new(AtomicU64::new(0));
    let transport = engine
        .native(router.clone(), move |router| async move {
            router.create_webrtc_transport(options).await
        })
        .await
        .unwrap();
    let rtp = serde_json::from_value(json!({
        "codecs":[{"mimeType":"audio/opus","payloadType":111,"clockRate":48000,
            "channels":2,"parameters":{},"rtcpFeedback":[]}],
        "headerExtensions":[],"encodings":[{"ssrc":1234}],
        "rtcp":{"cname":"stop-order-test","reducedSize":true}
    }))
    .unwrap();
    let producer = engine
        .native(transport.clone(), move |transport| async move {
            transport
                .produce(ProducerOptions::new(MediaKind::Audio, rtp))
                .await
        })
        .await
        .unwrap();
    assert!(!producer.paused());
    let (out, rx) = mpsc::channel(16);
    // Deliberately absent authority keys trigger the real denial path.
    // The native resources are installed directly to hold their gate before
    // starting revocation; this fixture exercises stop ordering, not join.
    let channel = Uuid::new_v4();
    let peer = Arc::new(Peer {
        id: PeerId(Uuid::new_v4()),
        authority: AuthorizedTicketClaim {
            claim: TicketClaim {
                u: Uuid::new_v4(),
                s: Uuid::new_v4(),
                c: channel,
                g: false,
            },
            auth: TicketAuthorization {
                session: Uuid::new_v4().simple().to_string().repeat(2),
                expires_at: 9_007_199_254_740_991,
                member: Uuid::new_v4(),
                channel: Uuid::new_v4(),
            },
        },
        watch_user: None,
        grant: Grant::new(),
        out,
        rpc_gate: Mutex::new(()),
        lease_gate: Mutex::new(()),
        attach_scheduled: AtomicBool::new(false),
        attach_dirty: AtomicBool::new(false),
        retry_scheduled: AtomicBool::new(false),
        attach_failures: AtomicUsize::new(0),
    });
    let publication = Arc::new(Publication {
        producer: NativeProducer::new(producer, transport.clone(), engine.lost.clone()),
        peer: peer.clone(),
        kind: SourceKind::Mic,
        epoch: Uuid::new_v4(),
        parent: None,
        live: None,
        grant: Grant::new(),
        gate: Mutex::new(()),
        height: 0,
        layers: 1,
        packet_samples: StdMutex::new(HashMap::new()),
    });
    let data = RoomData {
        peers: HashMap::from([(
            peer.id,
            PeerData {
                peer: peer.clone(),
                capabilities: None,
                transports: HashMap::from([(true, transport)]),
                publications: HashMap::from([(SourceKind::Mic, publication.clone())]),
                consumers: HashMap::new(),
                pending_consumers: HashSet::new(),
                failed_consumers: HashMap::new(),
                watches: HashMap::new(),
                withdrawn_live: VecDeque::new(),
                retired_producers: VecDeque::new(),
                retired_transports: VecDeque::new(),
            },
        )]),
        publications: HashMap::from([(publication.id(), publication.clone())]),
        ..RoomData::default()
    };
    let mut room = Room::new(engine.clone(), Some(router));
    *room.data.get_mut() = data;
    sfu.rooms.lock().await.insert(channel, Arc::new(room));
    engine.peers.store(1, Ordering::Relaxed);
    for counter in [
        &sfu.counters.rooms,
        &sfu.counters.peers,
        &sfu.counters.producers,
        &sfu.counters.transports,
    ] {
        counter.store(1, Ordering::Relaxed);
    }
    (sfu, peer, publication, rx)
}

/// A second member joined to the fixture's room with its authority keys in
/// Redis, a receive transport and the router's capabilities, ready to consume.
struct Viewer {
    id: PeerId,
    events: mpsc::Receiver<ServerFrame>,
    transport: WebRtcTransport,
    keys: Vec<String>,
}
impl Viewer {
    async fn join(sfu: &Arc<Sfu>, owner: &Peer, room: &Room, capacity: usize) -> Self {
        use crate::redis_connection::ConnectionSource;
        use gelabber_shared::ticket;
        let channel = owner.authority.claim.c;
        let mut authority = owner.authority.clone();
        authority.claim.u = Uuid::new_v4();
        authority.auth.member = Uuid::new_v4();
        authority.auth.session = Uuid::new_v4().simple().to_string().repeat(2);
        let keys = [
            (
                ticket::member_authority_key(authority.claim.s, authority.claim.u),
                authority.auth.member.to_string(),
            ),
            (
                ticket::channel_authority_key(channel),
                authority.auth.channel.to_string(),
            ),
            (
                ticket::session_authority_key(&authority.auth.session),
                authority.claim.u.to_string(),
            ),
        ];
        let mut redis = sfu.redis.connection().await.unwrap();
        for (key, value) in &keys {
            let _: () = redis::cmd("SET")
                .arg(key)
                .arg(value)
                .arg("EX")
                .arg(30)
                .query_async(&mut redis)
                .await
                .unwrap();
        }
        let (out, events) = mpsc::channel(capacity);
        let id = sfu
            .join_authorized_watch_version(authority, None, MEDIA_PROTOCOL_VERSION, out)
            .await
            .unwrap();
        let router = room.router.get().unwrap();
        let mut options = WebRtcTransportOptions::new_with_server(room.engine.server.clone());
        options.app_data = AppData::new(AtomicU64::new(0));
        let transport = room
            .engine
            .native(router.clone(), move |router| async move {
                router.create_webrtc_transport(options).await
            })
            .await
            .unwrap();
        {
            let mut data = room.data.lock().await;
            let peer = data.peers.get_mut(&id).unwrap();
            peer.transports.insert(false, transport.clone());
            peer.capabilities = Some(
                serde_json::from_value(serde_json::to_value(router.rtp_capabilities()).unwrap())
                    .unwrap(),
            );
        }
        sfu.counters.transports.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            events,
            transport,
            keys: keys.into_iter().map(|(key, _)| key).collect(),
        }
    }
    async fn revoke(&self, sfu: &Sfu) {
        use crate::redis_connection::ConnectionSource;
        let mut redis = sfu.redis.connection().await.unwrap();
        for key in &self.keys {
            let _: () = redis::cmd("DEL")
                .arg(key)
                .query_async(&mut redis)
                .await
                .unwrap();
        }
    }
}

/// Adds `count` audio publications of the fixture's owner to its room, each
/// on its own native producer, as a full voice room has.
async fn publish_many(owner: &Arc<Peer>, room: &Room, count: u32) -> Vec<Arc<Publication>> {
    let transport = room.data.lock().await.peers[&owner.id].transports[&true].clone();
    let mut publications = Vec::new();
    for index in 0..count {
        let rtp = serde_json::from_value(json!({
            "codecs":[{"mimeType":"audio/opus","payloadType":111,"clockRate":48000,
                "channels":2,"parameters":{},"rtcpFeedback":[]}],
            "headerExtensions":[],"encodings":[{"ssrc":10_000 + index}],
            "rtcp":{"cname":format!("attach-{index}"),"reducedSize":true}
        }))
        .unwrap();
        let producer = room
            .engine
            .native(transport.clone(), move |transport| async move {
                transport
                    .produce(ProducerOptions::new(MediaKind::Audio, rtp))
                    .await
            })
            .await
            .unwrap();
        publications.push(Arc::new(Publication {
            producer: NativeProducer::new(producer, transport.clone(), room.engine.lost.clone()),
            peer: owner.clone(),
            kind: SourceKind::Mic,
            epoch: Uuid::new_v4(),
            parent: None,
            live: None,
            grant: Grant::new(),
            gate: Mutex::new(()),
            height: 0,
            layers: 1,
            packet_samples: StdMutex::new(HashMap::new()),
        }));
    }
    let mut data = room.data.lock().await;
    for publication in &publications {
        data.publications
            .insert(publication.id(), publication.clone());
    }
    publications
}

/// Joining a room with 40 publications: every one is consumed and announced
/// in one attach pass. Prints the pass time to compare join latency.
#[tokio::test]
async fn attach_pass_announces_every_publication_of_a_full_room() {
    const PUBLICATIONS: u32 = 40;
    let (sfu, owner, _publication, _owner_events) = stopping_fixture().await;
    let channel = owner.authority.claim.c;
    let room = sfu.room(channel).await.unwrap();
    publish_many(&owner, &room, PUBLICATIONS - 1).await;
    let mut viewer = Viewer::join(&sfu, &owner, &room, 64).await;
    let started = Instant::now();
    sfu.attach_existing(channel, viewer.id).await;
    let elapsed = started.elapsed();
    // Written past libtest's capture so the CI log keeps the number.
    let _ = std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("attach pass for {PUBLICATIONS} publications took {elapsed:?}\n").as_bytes(),
    );
    let mut announced = HashSet::new();
    while let Ok(frame) = viewer.events.try_recv() {
        if let ServerFrame::Consumer { producer_id, .. } = frame {
            announced.insert(producer_id);
        }
    }
    assert_eq!(announced.len(), PUBLICATIONS as usize);
    {
        let data = room.data.lock().await;
        let peer = &data.peers[&viewer.id];
        assert_eq!(peer.consumers.len(), PUBLICATIONS as usize);
        assert!(peer.pending_consumers.is_empty());
    }
    assert_eq!(
        viewer.transport.dump().await.unwrap().consumer_ids.len(),
        PUBLICATIONS as usize
    );
    sfu.leave(viewer.id, channel).await;
    viewer.revoke(&sfu).await;
}

/// Authority that lapses while a pass creates its consumers: none of them
/// is announced or kept, and every native consumer is closed again.
#[tokio::test]
async fn attach_pass_announces_nothing_after_revocation() {
    const PUBLICATIONS: usize = 12;
    let (sfu, owner, publication, _owner_events) = stopping_fixture().await;
    let channel = owner.authority.claim.c;
    let room = sfu.room(channel).await.unwrap();
    let mut pubs = publish_many(&owner, &room, PUBLICATIONS as u32 - 1).await;
    pubs.push(publication);
    let mut viewer = Viewer::join(&sfu, &owner, &room, 64).await;
    let created = Arc::new(AtomicUsize::new(0));
    let client = redis::Client::open(std::env::var("REDIS_URL").unwrap()).unwrap();
    let probe = (
        created.clone(),
        client,
        viewer.keys.clone(),
        room.engine.clone(),
    );
    let results = sfu
        .attach_batch(&room, viewer.id, pubs, move |transport, options| {
            let (created, client, keys, engine) = probe.clone();
            async move {
                engine
                    .native(transport, move |transport| async move {
                        let consumer = transport.consume(options).await;
                        // The last creation of the pass revokes before it
                        // returns, so the pass's single check must see the
                        // revocation.
                        if created.fetch_add(1, Ordering::SeqCst) + 1 == PUBLICATIONS {
                            let mut redis =
                                client.get_multiplexed_async_connection().await.unwrap();
                            let _: () = redis::cmd("DEL")
                                .arg(&keys)
                                .query_async(&mut redis)
                                .await
                                .unwrap();
                        }
                        consumer
                    })
                    .await
            }
        })
        .await;
    assert_eq!(created.load(Ordering::SeqCst), PUBLICATIONS);
    assert_eq!(results.len(), PUBLICATIONS);
    assert!(
        results.iter().all(|r| matches!(r, Err(SfuError::Revoked))),
        "{results:?}"
    );
    while let Ok(frame) = viewer.events.try_recv() {
        assert!(!matches!(frame, ServerFrame::Consumer { .. }));
    }
    // The authority watcher may already have removed the viewer.
    if let Some(peer) = room.data.lock().await.peers.get(&viewer.id) {
        assert!(peer.consumers.is_empty());
        assert!(peer.pending_consumers.is_empty());
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while viewer
            .transport
            .dump()
            .await
            .is_ok_and(|dump| !dump.consumer_ids.is_empty())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("revoked consumers must be closed natively");
    sfu.leave(viewer.id, channel).await;
}

#[tokio::test]
async fn native_consume_timeout_retries_without_room_activity_and_retires_late_consumer() {
    let (sfu, owner, publication, _owner_events) = stopping_fixture().await;
    let channel = owner.authority.claim.c;
    let room = sfu.room(channel).await.unwrap();
    let mut joined = Viewer::join(&sfu, &owner, &room, 16).await;
    let (viewer, transport) = (joined.id, joined.transport.clone());
    let (complete, pending) = tokio::sync::oneshot::channel();
    let (late_created, late_ready) = tokio::sync::oneshot::channel();
    let probe = StdMutex::new(Some((pending, late_created)));
    let engine = room.engine.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        sfu.attach_batch(
            &room,
            viewer,
            vec![publication.clone()],
            move |transport, options| {
                let (pending, late_created) = probe.lock().unwrap().take().unwrap();
                let engine = engine.clone();
                async move {
                    engine
                        .native(transport, move |transport| async move {
                            pending.await.unwrap();
                            let result = transport.consume(options).await;
                            assert!(result.is_ok(), "probe must create a real late consumer");
                            late_created.send(()).unwrap();
                            result
                        })
                        .await
                }
            },
        ),
    )
    .await
    .expect("attach must release its pending reservation after timeout");
    assert!(matches!(result[..], [Err(SfuError::Unavailable)]));
    let event = tokio::time::timeout(Duration::from_secs(3), joined.events.recv())
        .await
        .unwrap()
        .unwrap();
    let ServerFrame::Consumer {
        producer_id,
        consumer_id,
        ..
    } = event
    else {
        panic!("retry did not announce a consumer");
    };
    assert_eq!(producer_id, publication.id());
    {
        let data = room.data.lock().await;
        let peer = data.peers.get(&viewer).unwrap();
        assert_eq!(peer.consumers.len(), 1);
        assert!(peer.pending_consumers.is_empty());
    }
    complete.send(()).unwrap();
    late_ready.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let dump = transport.dump().await.unwrap();
            if dump.consumer_ids.len() == 1 && dump.consumer_ids[0].to_string() == consumer_id {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("late unannounced consumer must be closed");
    sfu.leave(viewer, channel).await;
    joined.revoke(&sfu).await;
}

async fn assert_terminal_after_native_stop(revoke: bool) {
    let (sfu, peer, publication, mut rx) = stopping_fixture().await;
    let gate = publication.gate.lock().await;
    let shutdown = if revoke {
        Sfu::watch_authority(
            Arc::downgrade(&sfu),
            peer.authority.claim.c,
            Arc::downgrade(&peer),
        );
        None
    } else {
        let engine = sfu.clone();
        Some(tokio::spawn(async move { engine.shutdown().await }))
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while peer.grant.valid() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "terminal event preceded the confirmed native stop"
    );
    assert!(!publication.producer.paused());
    assert!(sfu.peer_present(peer.id, peer.authority.claim.c).await);
    drop(gate);
    let expected = if revoke { "unauthorized" } else { "gone" };
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(ServerFrame::Err { e, .. }) = rx.recv().await {
                assert_eq!(e, expected);
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(publication.producer.paused());
    assert!(!sfu.peer_present(peer.id, peer.authority.claim.c).await);
    if let Some(task) = shutdown {
        task.await.unwrap();
    }
}

#[tokio::test]
async fn shutdown_terminal_event_follows_confirmed_native_stop() {
    assert_terminal_after_native_stop(false).await;
}

#[tokio::test]
async fn revocation_terminal_event_follows_confirmed_native_stop() {
    assert_terminal_after_native_stop(true).await;
}

#[tokio::test]
async fn revoked_rpc_waits_for_confirmed_native_stop() {
    let (sfu, peer, publication, _rx) = stopping_fixture().await;
    let gate = publication.gate.lock().await;
    peer.grant.stop();
    let rpc = sfu.rpc(
        peer.id,
        peer.authority.claim.c,
        ClientFrame::Capabilities {
            id: 1,
            rtp: json!({}),
        },
    );
    tokio::pin!(rpc);
    // The operation gate represents an in-flight native operation. A revoked
    // RPC must not finish while that operation still prevents confirmed stop.
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut rpc)
            .await
            .is_err()
    );
    assert!(!publication.producer.paused());
    assert!(sfu.peer_present(peer.id, peer.authority.claim.c).await);
    drop(gate);
    assert!(matches!(rpc.await, Err(SfuError::Revoked)));
    assert!(publication.producer.paused());
    assert!(!sfu.peer_present(peer.id, peer.authority.claim.c).await);
}

#[tokio::test]
async fn slow_resource_gate_closes_only_its_native_producer() {
    let (sfu, _peer, publication, _rx) = stopping_fixture().await;
    let transport = publication.producer.transport();
    // Reuse the complete parameters accepted by the native fixture. Only
    // the second source's SSRC differs; no partial SDK JSON schema here.
    let mut rtp = publication
        .producer
        .resource
        .with(|producer| producer.rtp_parameters().clone())
        .unwrap();
    rtp.encodings[0].ssrc = Some(5678);
    let engine = first_engine(&sfu);
    let other = engine
        .native(transport.clone(), move |transport| async move {
            transport
                .produce(ProducerOptions::new(MediaKind::Audio, rtp))
                .await
        })
        .await
        .unwrap();
    let _gate = publication.gate.lock().await;
    tokio::time::timeout(
        Duration::from_secs(2),
        sfu.confirmed_stop_resources(std::slice::from_ref(&publication), &[]),
    )
    .await
    .unwrap();
    assert!(publication.producer.closed());
    let dump = transport.dump().await.unwrap();
    assert!(!dump.producer_ids.contains(&publication.producer.id()));
    assert!(dump.producer_ids.contains(&other.id()));
    assert!(!other.paused());
    assert!(sfu.ready());
}

async fn pool(workers: usize) -> Arc<Sfu> {
    let workers = workers.to_string();
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(std::env::var(key).expect("isolated REDIS_URL required")),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        "MEDIA_WORKERS" => Some(workers.clone()),
        _ => None,
    })
    .unwrap();
    Sfu::with_redis(
        &config,
        redis::Client::open(config.redis_url.as_str()).unwrap(),
    )
    .await
    .unwrap()
}

/// A room with one peer and a receive transport on `engine`, counted like a join.
async fn seat(
    sfu: &Arc<Sfu>,
    engine: &Arc<Engine>,
) -> (Uuid, Arc<Peer>, mpsc::Receiver<ServerFrame>) {
    let router = engine
        .native(engine.worker.clone(), |worker| async move {
            worker
                .create_router(RouterOptions::new(router_codecs().unwrap()))
                .await
        })
        .await
        .unwrap();
    let mut options = WebRtcTransportOptions::new_with_server(engine.server.clone());
    options.app_data = AppData::new(AtomicU64::new(0));
    let transport = engine
        .native(router.clone(), move |router| async move {
            router.create_webrtc_transport(options).await
        })
        .await
        .unwrap();
    let channel = Uuid::new_v4();
    let (out, rx) = mpsc::channel(16);
    let peer = Arc::new(Peer {
        id: PeerId(Uuid::new_v4()),
        authority: AuthorizedTicketClaim {
            claim: TicketClaim {
                u: Uuid::new_v4(),
                s: Uuid::new_v4(),
                c: channel,
                g: false,
            },
            auth: TicketAuthorization {
                session: Uuid::new_v4().simple().to_string().repeat(2),
                expires_at: 9_007_199_254_740_991,
                member: Uuid::new_v4(),
                channel: Uuid::new_v4(),
            },
        },
        watch_user: None,
        grant: Grant::new(),
        out,
        rpc_gate: Mutex::new(()),
        lease_gate: Mutex::new(()),
        attach_scheduled: AtomicBool::new(false),
        attach_dirty: AtomicBool::new(false),
        retry_scheduled: AtomicBool::new(false),
        attach_failures: AtomicUsize::new(0),
    });
    let mut room = Room::new(engine.clone(), Some(router));
    room.data.get_mut().peers.insert(
        peer.id,
        PeerData {
            peer: peer.clone(),
            capabilities: None,
            transports: HashMap::from([(false, transport)]),
            publications: HashMap::new(),
            consumers: HashMap::new(),
            pending_consumers: HashSet::new(),
            failed_consumers: HashMap::new(),
            watches: HashMap::new(),
            withdrawn_live: VecDeque::new(),
            retired_producers: VecDeque::new(),
            retired_transports: VecDeque::new(),
        },
    );
    sfu.rooms.lock().await.insert(channel, Arc::new(room));
    for counter in [
        &sfu.counters.rooms,
        &sfu.counters.peers,
        &sfu.counters.transports,
    ] {
        counter.fetch_add(1, Ordering::Relaxed);
    }
    engine.peers.fetch_add(1, Ordering::Relaxed);
    (channel, peer, rx)
}

#[tokio::test]
async fn new_rooms_go_to_the_least_loaded_live_worker() {
    let sfu = pool(3).await;
    let engines = sfu.engines.lock().unwrap().clone();
    assert_eq!(sfu.workers(), WorkerStatus { live: 3, total: 3 });
    // Each worker has its own WebRtcServer and so its own UDP port.
    let mut ports = HashSet::new();
    for engine in &engines {
        let (channel, _, _) = seat(&sfu, engine).await;
        let room = sfu.rooms.lock().await.remove(&channel).unwrap();
        let data = room.data.lock().await;
        let transport = data.peers.values().next().unwrap().transports[&false].clone();
        ports.insert(transport.ice_candidates()[0].port);
        drop(data);
        let mut data = room.data.lock().await;
        data.peers.clear();
        for counter in [
            &sfu.counters.rooms,
            &sfu.counters.peers,
            &sfu.counters.transports,
        ] {
            counter.fetch_sub(1, Ordering::Relaxed);
        }
        engine.peers.fetch_sub(1, Ordering::Relaxed);
    }
    assert_eq!(ports.len(), 3);
    assert_eq!(sfu.pick_engine().unwrap().slot, 0);
    let (_a, _pa, _ra) = seat(&sfu, &engines[0]).await;
    assert_eq!(sfu.pick_engine().unwrap().slot, 1);
    let (_b, _pb, _rb) = seat(&sfu, &engines[1]).await;
    let (_c, _pc, _rc) = seat(&sfu, &engines[1]).await;
    assert_eq!(sfu.pick_engine().unwrap().slot, 2);
    engines[2].lost.store(true, Ordering::SeqCst);
    assert_eq!(sfu.pick_engine().unwrap().slot, 0);
    assert_eq!(sfu.workers(), WorkerStatus { live: 2, total: 3 });
    assert!(sfu.ready());
    let metrics = sfu.metrics_text();
    assert!(metrics.contains("gelabber_media_workers 3\n"), "{metrics}");
    assert!(
        metrics.contains("gelabber_media_workers_live 2\n"),
        "{metrics}"
    );
    assert!(
        metrics.contains("gelabber_media_worker_peers{worker=\"1\"} 2\n"),
        "{metrics}"
    );
}

#[tokio::test]
async fn lost_worker_closes_only_its_rooms_and_is_replaced() {
    let sfu = pool(2).await;
    let engines = sfu.engines.lock().unwrap().clone();
    let (lost_channel, lost_peer, mut lost_rx) = seat(&sfu, &engines[0]).await;
    let (kept_channel, kept_peer, mut kept_rx) = seat(&sfu, &engines[1]).await;
    sfu.engine_lost(&engines[0], "test: worker lost").await;

    let frame = lost_rx.try_recv().expect("peer of the lost worker is told");
    assert!(
        matches!(&frame, ServerFrame::Err { e, .. } if e == "gone"),
        "{frame:?}"
    );
    assert!(!lost_peer.grant.valid());
    assert!(!sfu.peer_present(lost_peer.id, lost_channel).await);
    assert!(sfu.room(lost_channel).await.is_err());

    assert!(
        kept_rx.try_recv().is_err(),
        "other worker's peers are untouched"
    );
    assert!(kept_peer.grant.valid());
    assert!(sfu.peer_present(kept_peer.id, kept_channel).await);
    assert!(engines[1].alive());
    assert!(sfu.ready());
    assert_eq!(sfu.room_count(), 1);
    assert_eq!(sfu.counters.peers.load(Ordering::Relaxed), 1);
    assert_eq!(sfu.counters.transports.load(Ordering::Relaxed), 1);
    assert_eq!(sfu.workers(), WorkerStatus { live: 1, total: 2 });
    // New rooms avoid the lost worker until its slot is replaced.
    assert_eq!(sfu.pick_engine().unwrap().slot, 1);

    tokio::time::timeout(Duration::from_secs(10), async {
        while sfu.workers().live < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("lost slot gets a new worker");
    let replaced = first_engine(&sfu);
    assert!(!Arc::ptr_eq(&replaced, &engines[0]));
    assert!(replaced.alive());
    assert_eq!(sfu.counters.worker_failures.load(Ordering::Relaxed), 1);
    assert_eq!(sfu.counters.worker_restarts.load(Ordering::Relaxed), 1);
    assert_eq!(sfu.pick_engine().unwrap().slot, 0);
}

#[tokio::test]
async fn worker_exit_seen_by_a_heartbeat_tick_first_still_closes_its_rooms() {
    let sfu = pool(2).await;
    let engines = sfu.engines.lock().unwrap().clone();
    let (channel, peer, mut rx) = seat(&sfu, &engines[0]).await;
    // `on_dead`'s order: `lost` first, the notification afterwards. A tick
    // in between must not end the supervisor without handling the loss.
    engines[0].lost.store(true, Ordering::SeqCst);
    tokio::time::sleep(HEARTBEAT_INTERVAL * 2).await;
    engines[0].exited.notify_one();
    let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("the supervisor handles the loss")
        .unwrap();
    assert!(
        matches!(&frame, ServerFrame::Err { e, .. } if e == "gone"),
        "{frame:?}"
    );
    assert!(!sfu.peer_present(peer.id, channel).await);
    // Handled once although both the tick and the notification saw it.
    tokio::time::sleep(HEARTBEAT_INTERVAL * 2).await;
    assert_eq!(sfu.counters.worker_failures.load(Ordering::Relaxed), 1);
    assert!(engines[0].down_for() > Duration::ZERO);
    let metrics = sfu.metrics_text();
    assert!(
        metrics.contains("gelabber_media_worker_down_seconds{worker=\"1\"} 0\n"),
        "{metrics}"
    );
}

#[tokio::test]
async fn rooms_of_a_lost_worker_refuse_rpcs_while_the_pool_stays_ready() {
    let sfu = pool(2).await;
    let engine = first_engine(&sfu);
    let (channel, _peer, _rx) = seat(&sfu, &engine).await;
    let room = sfu.room(channel).await.unwrap();
    engine.lost.store(true, Ordering::SeqCst);
    assert!(!sfu.room_ready(&room));
    assert!(sfu.ready());
}

#[test]
fn fail_closed_terminates_child_process_with_exit_one() {
    const CHILD: &str = "GELABBER_TEST_FAIL_CLOSED_CHILD";
    if std::env::var_os(CHILD).is_some() {
        fail_closed("test child executes the real termination function");
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("sfu::tests::fail_closed_terminates_child_process_with_exit_one")
        .env(CHILD, "1")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1));
}
