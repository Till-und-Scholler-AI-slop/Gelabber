//! Native core against the server's media engine (mediasoup 0.29) on loopback:
//! ICE-lite, DTLS, SRTP, encoders and decoders, with the router codecs and
//! transport options of media/src/sfu.rs.
//!
//! Proves the spike: libmediasoupclient's parameters are accepted by the same
//! mediasoup version the Gelabber media gateway runs, and media flows both ways.

mod common;

use common::{Server, blocking, serve_events, wait_for};
use gelabber_media_core::{Audio, Device, Direction, Engine, Source, Transport};
use mediasoup::prelude::*;
// Trait methods (id, produce, consume); the name is taken by the native transport.
use mediasoup::prelude::Transport as _;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

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
    assert!(
        device
            .can_produce(gelabber_media_core::MediaKind::Audio)
            .unwrap()
    );
    assert!(
        device
            .can_produce(gelabber_media_core::MediaKind::Video)
            .unwrap()
    );
    let recv_caps = device.rtp_capabilities().unwrap();
    let recv_codecs: Vec<String> = recv_caps["codecs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["mimeType"].as_str().unwrap().to_ascii_lowercase())
        .collect();
    eprintln!("native receive codecs: {recv_codecs:?}");
    for mime in ["audio/opus", "video/vp8", "video/h264"] {
        assert!(
            recv_codecs.iter().any(|c| c == mime),
            "native can receive {mime}"
        );
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
        blocking(move || {
            send.produce(
                &mic,
                &json!({"codecOptions": {"opusStereo": false, "opusDtx": true}}),
            )
        })
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
        wait_for(
            &format!("{name} RTP at the server"),
            Duration::from_secs(20),
            || {
                let producer = producer.clone();
                async move {
                    let stats = producer.get_stats().await.unwrap_or_default();
                    stats.iter().any(|s| s.byte_count > 0 && s.score > 0)
                }
            },
        )
        .await;
    }
    // Both simulcast layers must come up. The 720p layer only starts once
    // libwebrtc's bandwidth estimate and CPU budget allow it, so poll; on
    // failure print the sender's own per-layer view (qualityLimitationReason,
    // frameWidth, active) next to the server's.
    for (name, producer, native) in [("H264", &h264_server, &h264), ("VP8", &vp8_server, &vp8)] {
        let start = Instant::now();
        let mut live = 0;
        while start.elapsed() < Duration::from_secs(20) {
            let stats = producer.get_stats().await.unwrap();
            live = stats.iter().filter(|s| s.byte_count > 0).count();
            if live == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let stats = producer.get_stats().await.unwrap();
        eprintln!(
            "{name}: {live} live layer(s) after {:?}: {:?}",
            start.elapsed(),
            stats
                .iter()
                .map(|s| (s.ssrc, s.rid.clone(), s.byte_count, s.bitrate, s.score))
                .collect::<Vec<_>>()
        );
        if live != 2 {
            let outbound: Vec<Value> = native
                .stats()
                .unwrap()
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
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
                .collect();
            eprintln!("{name} native outbound-rtp: {}", Value::Array(outbound));
        }
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
    // Wait for decoded frames at the native sinks; on failure report both
    // ends (server consumer egress, libwebrtc inbound-rtp) for every codec
    // before failing.
    let mut missing = Vec::new();
    for (server_consumer, native) in &consumers {
        let mime =
            serde_json::to_value(&server_consumer.rtp_parameters().codecs[0]).unwrap()["mimeType"]
                .clone();
        let start = Instant::now();
        let mut frames = 0;
        while start.elapsed() < Duration::from_secs(20) {
            frames = native.stats().unwrap()["framesReceived"]
                .as_u64()
                .unwrap_or(0);
            if frames > 10 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let stats = native.stats().unwrap();
        eprintln!(
            "native consumer {mime}: {frames} frames, {}x{} after {:?}",
            stats["width"],
            stats["height"],
            start.elapsed()
        );
        if frames <= 10 {
            let inbound: Vec<Value> = stats["rtc"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s["type"] == "inbound-rtp" || s["type"] == "transport")
                .collect();
            eprintln!(
                "{mime} native inbound-rtp/transport: {}",
                Value::Array(inbound)
            );
            let server_stats = server_consumer.get_stats().await.unwrap();
            eprintln!("{mime} server consumer stats: {server_stats:?}");
            eprintln!(
                "{mime} server consumer rtpParameters: {}",
                serde_json::to_string(server_consumer.rtp_parameters()).unwrap()
            );
            missing.push(mime);
        }
    }
    assert!(missing.is_empty(), "no decoded frames for {missing:?}");

    // Native objects close in dependency order.
    drop(consumers);
    drop((audio, h264, vp8));
    drop((send, recv));
}
