//! Screen capture through xdg-desktop-portal and PipeWire, encoded as H264,
//! sent through mediasoup 0.29 and decoded by a native consumer; then shared
//! the way the web client does it, as VP8 in two simulcast layers.
//!
//! Needs a Wayland session with PipeWire and a portal that answers without a
//! dialog; CI runs it in desktop/native/scripts/fake-desktop-session.sh (real
//! portal frontend, test ScreenCast backend). Skipped unless
//! GELABBER_TEST_SCREEN=1 so a developer desktop never opens a picker.

mod common;

use common::{Server, blocking, check_encoder, layers_of, simulcast_sizes, wait_for};
use gelabber_media_core::{Audio, Device, Direction, Engine, Source, Transport};
use mediasoup::prelude::Transport as _;
use mediasoup::prelude::*;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screen_capture_reaches_a_consumer() {
    if std::env::var_os("GELABBER_TEST_SCREEN").is_none() {
        eprintln!("skipped: set GELABBER_TEST_SCREEN=1 inside a Wayland session with a portal");
        return;
    }
    if std::env::var_os("GELABBER_MEDIA_LOG").is_some() {
        gelabber_media_core::set_log_level(gelabber_media_core::LogLevel::Info);
    }
    let server = Server::start().await;
    share_a_screen(&server).await;
    server.close().await;
}

/// The call of the test above, dropped on return.
async fn share_a_screen(server: &Server) {
    let engine = Engine::new(Audio::Dummy).unwrap();
    let device = Device::new(&engine).unwrap();
    device
        .load(&serde_json::to_value(server.router.rtp_capabilities()).unwrap())
        .unwrap();

    let screen = Source::screen(&engine, &json!({"type": "screen", "fps": 30})).unwrap();
    let start = Instant::now();
    let mut state = screen.state().unwrap();
    while state["state"] == "pending" && start.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        state = screen.state().unwrap();
    }
    eprintln!("screen source after {:?}: {state}", start.elapsed());
    assert_eq!(state["state"], "live", "portal picked a source");
    wait_for("captured frames", Duration::from_secs(20), || {
        let screen = screen.clone();
        async move { screen.state().unwrap()["frames"].as_u64().unwrap_or(0) > 5 }
    })
    .await;

    let server_producers = Arc::new(Mutex::new(Vec::new()));
    let (server_send, send_params) = server.transport().await;
    let (send, send_events) = Transport::new(&device, Direction::Send, &send_params).unwrap();
    server.serve(
        send.clone(),
        server_send.clone(),
        send_events,
        server_producers.clone(),
    );
    let producer = {
        let (send, screen) = (send.clone(), screen.clone());
        blocking(move || send.produce(&screen, &json!({"codec": "video/H264"})))
            .await
            .unwrap()
    };
    let server_producer = server_producers.lock().unwrap()[0].clone();
    wait_for("screen RTP at the server", Duration::from_secs(20), || {
        let producer = server_producer.clone();
        async move {
            let stats = producer.get_stats().await.unwrap_or_default();
            stats.iter().any(|s| s.byte_count > 0)
        }
    })
    .await;

    let (server_recv, recv_params) = server.transport().await;
    let (recv, recv_events) = Transport::new(&device, Direction::Recv, &recv_params).unwrap();
    server.serve(
        recv.clone(),
        server_recv.clone(),
        recv_events,
        Arc::new(Mutex::new(Vec::new())),
    );
    let caps: RtpCapabilities = serde_json::from_value(device.rtp_capabilities().unwrap()).unwrap();
    let mut options = ConsumerOptions::new(server_producer.id(), caps);
    options.paused = true;
    let server_consumer = server_recv.consume(options).await.unwrap();
    let announcement = json!({
        "id": server_consumer.id(),
        "producerId": server_producer.id(),
        "kind": "video",
        "rtpParameters": server_consumer.rtp_parameters(),
    });
    let consumer = {
        let recv = recv.clone();
        blocking(move || recv.consume(&announcement)).await.unwrap()
    };
    server_consumer.resume().await.unwrap();

    let start = Instant::now();
    let mut stats = consumer.stats().unwrap();
    while stats["framesReceived"].as_u64().unwrap_or(0) <= 10
        && start.elapsed() < Duration::from_secs(20)
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
        stats = consumer.stats().unwrap();
    }
    let sender = producer.stats().unwrap();
    let outbound: Vec<Value> = sender
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s["type"] == "outbound-rtp")
        .map(|s| {
            json!({
                "frameWidth": s["frameWidth"], "frameHeight": s["frameHeight"],
                "framesEncoded": s["framesEncoded"], "framesPerSecond": s["framesPerSecond"],
                "encoderImplementation": s["encoderImplementation"],
                "qualityLimitationReason": s["qualityLimitationReason"],
            })
        })
        .collect();
    eprintln!("screen sender: {}", Value::Array(outbound));
    eprintln!(
        "screen consumer: {} frames, {}x{} after {:?}; source {}",
        stats["framesReceived"],
        stats["width"],
        stats["height"],
        start.elapsed(),
        screen.state().unwrap()
    );
    assert!(
        stats["framesReceived"].as_u64().unwrap_or(0) > 10,
        "decoded screen frames"
    );
    assert!(stats["width"].as_u64().unwrap_or(0) >= 320);
    check_encoder("screen", &sender);
    drop(consumer);
    drop(producer);

    // A share from the web client: VP8 with a layer at a quarter and one in
    // full. The session's monitor is 1366 wide, which no quarter comes out
    // of; libvpx would encode neither layer of it.
    let captured = screen.state().unwrap();
    let (width, height) = (
        captured["width"].as_u64().unwrap(),
        captured["height"].as_u64().unwrap(),
    );
    let shared = {
        let (send, screen) = (send.clone(), screen.clone());
        // The start bitrate carries both layers at once; the page leaves
        // that to the bandwidth estimate.
        let options = json!({
            "codec": "video/VP8",
            "encodings": [{ "scaleResolutionDownBy": 4 }, { "scaleResolutionDownBy": 1 }],
            "codecOptions": { "videoGoogleStartBitrate": 3000 },
        });
        blocking(move || send.produce(&screen, &options))
            .await
            .unwrap()
    };
    let shared_at_server = server_producers.lock().unwrap()[1].clone();
    assert_eq!(
        simulcast_sizes("screen as VP8", &shared, &shared_at_server).await,
        layers_of(width, height, 4),
        "layers of the {width}x{height} screen"
    );

    drop(shared);
    drop((send, recv));
    drop(screen);
}
