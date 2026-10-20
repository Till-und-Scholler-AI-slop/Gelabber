//! Issue #197: a closed producer or consumer releases its transceiver at
//! once, not only when its transport closes. Unpatched, libmediasoupclient
//! keeps every RtpTransceiver a transport ever had in its mid map, so a long
//! call that toggles screen share and camera or sees people come and go only
//! grows (patches/libmediasoupclient in desktop/native). Only the first,
//! BUNDLE-tagged one per transport stays, which is bounded.

mod common;

use common::{Server, blocking};
use gelabber_media_core::{Audio, Device, Direction, Engine, Source, Transport, VideoFrame};
use mediasoup::prelude::*;
// Trait methods (id, consume); the name is taken by the native transport.
use mediasoup::prelude::Transport as _;
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Toggles of one video source, each a producer and a consumer of it.
const CYCLES: usize = 12;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_producers_and_consumers_release_their_transceivers() {
    let server = Server::start().await;
    toggle(&server).await;
    server.close().await;
}

async fn toggle(server: &Server) {
    let engine = Engine::new(Audio::Dummy).unwrap();
    let device = Device::new(&engine).unwrap();
    let caps = serde_json::to_value(server.router.rtp_capabilities()).unwrap();
    device.load(&caps).unwrap();
    let client_caps: RtpCapabilities =
        serde_json::from_value(device.rtp_capabilities().unwrap()).unwrap();

    let server_producers = Arc::new(Mutex::new(Vec::new()));
    let (server_send, send_params) = server.transport().await;
    let (send, send_events) = Transport::new(&device, Direction::Send, &send_params).unwrap();
    server.serve(
        send.clone(),
        server_send,
        send_events,
        server_producers.clone(),
    );
    let (server_recv, recv_params) = server.transport().await;
    let (recv, recv_events) = Transport::new(&device, Direction::Recv, &recv_params).unwrap();
    server.serve(
        recv.clone(),
        server_recv.clone(),
        recv_events,
        Arc::new(Mutex::new(Vec::new())),
    );

    // (cycle, consumer's receiver released, producer's sender released)
    let mut held = Vec::new();
    for cycle in 0..=CYCLES {
        let source = Source::test_pattern(&engine, 320, 180, 15).unwrap();
        let producer = {
            let send = send.clone();
            blocking(move || send.produce(&source, &json!({"codec": "video/VP8"})))
                .await
                .unwrap()
        };
        let server_producer = server_producers
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id().to_string() == producer.id())
            .cloned()
            .unwrap();
        let server_consumer = server_recv
            .consume(ConsumerOptions::new(
                server_producer.id(),
                client_caps.clone(),
            ))
            .await
            .unwrap();
        let announcement = json!({
            "id": server_consumer.id(),
            "producerId": server_producer.id(),
            "kind": "video",
            "rtpParameters": server_consumer.rtp_parameters(),
        });
        let mut consumer = {
            let recv = recv.clone();
            blocking(move || recv.consume(&announcement)).await.unwrap()
        };

        if cycle == CYCLES {
            // Media still flows after the closed mids, through a sink: stats
            // would cache the transceiver in libwebrtc's stats collector.
            let frames = Arc::new(AtomicU64::new(0));
            let counted = frames.clone();
            consumer
                .set_video_sink(Some(Box::new(move |_: &VideoFrame<'_>| {
                    counted.fetch_add(1, Ordering::Relaxed);
                })))
                .unwrap();
            let start = Instant::now();
            while frames.load(Ordering::Relaxed) < 5 && start.elapsed() < Duration::from_secs(20) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let frames = frames.load(Ordering::Relaxed);
            eprintln!("cycle {cycle}: {frames} frames after {:?}", start.elapsed());
            assert!(
                frames >= 5,
                "frames after {CYCLES} closed producers and consumers"
            );
        }

        let (consumer_released, producer_released) = blocking(move || {
            (
                consumer.close_released().unwrap(),
                producer.close_released().unwrap(),
            )
        })
        .await;
        drop(server_consumer);
        if !(consumer_released && producer_released) {
            held.push((cycle, consumer_released, producer_released));
        }
    }
    // The first m-section of a transport carries the BUNDLE transport, so
    // libmediasoupclient only disables it instead of closing it: libwebrtc
    // keeps that one transceiver per transport. Every later one goes.
    assert_eq!(
        held,
        [(0, false, false)],
        "(cycle, receiver released, sender released) still held by the core"
    );

    drop((send, recv));
}
