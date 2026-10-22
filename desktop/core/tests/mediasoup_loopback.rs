//! Native core against the server's media engine (mediasoup 0.29) on loopback:
//! ICE-lite, DTLS, SRTP, encoders and decoders, with the router codecs and
//! transport options of media/src/sfu.rs.
//!
//! Proves the spike: libmediasoupclient's parameters are accepted by the same
//! mediasoup version the Gelabber media gateway runs, and media flows both ways.
//!
//! Opus, VP8 and AV1 always run, AV1 as one stream (mediasoup forwards it
//! as one stream or SVC, never simulcast). H264 runs where the core has it
//! (on Windows only with Media Foundation's codecs); GELABBER_EXPECT_H264=1
//! makes a core without it fail, and so do GELABBER_EXPECT_ENCODER and
//! GELABBER_EXPECT_DECODER, which name the H264 encoder and decoder to find.
//! GELABBER_EXPECT_AV1_ENCODER and _DECODER name AV1's: libaom and dav1d,
//! or a system codec in front of them (GELABBER_AV1_ENCODER, _DECODER).
//! GELABBER_TEST_LISTEN_IP puts the server on another address of the machine
//! than 127.0.0.1 (see `listen_ip_from`).

mod common;

use common::{
    Server, blocking, check_decoder, check_encoder, h264_demanded_by, h264_under_test, layers_of,
    listen_ip, listen_ip_from, outbound_rtp, run_host_test, simulcast_sizes, wait_for,
};
use gelabber_media_core::{Audio, Device, Direction, Engine, Source, Transport, VideoFrame};
use mediasoup::prelude::*;
// Trait methods (id, produce, consume); the name is taken by the native transport.
use mediasoup::prelude::Transport as _;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_client_round_trips_media_through_mediasoup() {
    if std::env::var_os("GELABBER_MEDIA_LOG").is_some() {
        gelabber_media_core::set_log_level(gelabber_media_core::LogLevel::Info);
    }
    let server = Server::start().await;
    round_trip(&server).await;
    server.close().await;
}

/// The call of the test above. All of it is dropped on return, which the
/// server needs to close.
async fn round_trip(server: &Server) {
    let engine = Engine::new(Audio::Dummy).unwrap();

    // Cameras: CI runners have none, so opening the default one must fail
    // cleanly; with a camera it must open and close.
    let cameras = engine.video_devices().unwrap();
    eprintln!("cameras: {cameras}");
    let camera = Source::camera(&engine, &json!({"width": 640, "height": 360}));
    match cameras.as_array().map(Vec::len) {
        Some(0) => assert!(camera.is_err(), "no camera, no source"),
        _ => eprintln!("camera: {:?}", camera.map(|c| c.state().unwrap())),
    }

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
    for mime in ["audio/opus", "video/vp8", "video/av1"] {
        assert!(
            recv_codecs.iter().any(|c| c == mime),
            "native can receive {mime}"
        );
    }
    eprintln!(
        "system video codecs: {}",
        gelabber_media_core::video_codecs().unwrap()
    );
    let with_h264 = h264_under_test(&recv_caps);
    if !with_h264 {
        eprintln!("no H264 in this core: Opus and VP8 only");
    }
    // The server parses this exact value in its `capabilities` handler.
    let _: RtpCapabilities = serde_json::from_value(recv_caps.clone()).unwrap();

    // Send side.
    let server_producers = Arc::new(Mutex::new(Vec::new()));
    let (server_send, send_params) = server.transport().await;
    let (send, send_events) = Transport::new(&device, Direction::Send, &send_params).unwrap();
    server.serve(
        send.clone(),
        server_send.clone(),
        send_events,
        server_producers.clone(),
    );

    let mic = Source::microphone(&engine, &json!({})).unwrap();

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
    // Per codec: its name, the producer's options and the test pattern's
    // size. AV1 like the web client sends it: one stream, at a size libaom
    // keeps up with on a runner, and capped so that it leaves the bandwidth
    // estimate room for the 720p layers of the others.
    let mut codecs = Vec::new();
    if with_h264 {
        codecs.push((
            "H264",
            json!({"codec": "video/H264", "encodings": layers, "codecOptions": {"videoGoogleStartBitrate": 1000}}),
            (1280, 720),
        ));
    }
    codecs.push((
        "VP8",
        json!({"codec": "video/VP8", "encodings": layers}),
        (1280, 720),
    ));
    codecs.push((
        "AV1",
        json!({"codec": "video/AV1", "encodings": [{ "maxBitrate": 300_000 }]}),
        (640, 360),
    ));
    // Per codec: its name and the native producer of its own test pattern.
    let mut produced = Vec::new();
    for (name, options, (width, height)) in codecs {
        let source = Source::test_pattern(&engine, width, height, 30).unwrap();
        let send = send.clone();
        let producer = blocking(move || send.produce(&source, &options))
            .await
            .unwrap();
        if name != "VP8" {
            eprintln!(
                "{name} rtpParameters: {}",
                producer.rtp_parameters().unwrap()
            );
        }
        produced.push((name, producer));
    }
    assert_eq!(server_producers.lock().unwrap().len(), 1 + produced.len());

    let server_producer = |id: String| {
        server_producers
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id().to_string() == id)
            .cloned()
            .unwrap()
    };
    // Per codec: name, the server's producer, the native producer.
    let mut video: Vec<_> = produced
        .into_iter()
        .map(|(name, native)| (name, server_producer(native.id().to_owned()), native))
        .collect();
    let audio_server = server_producer(audio.id().to_owned());
    assert_eq!(audio_server.kind(), MediaKind::Audio);

    for (name, producer, _) in &video {
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
    // Both simulcast layers must come up (AV1: its one stream). The 720p
    // layer only starts once libwebrtc's bandwidth estimate and CPU budget
    // allow it, so poll; on failure print the sender's own per-layer view
    // (qualityLimitationReason, frameWidth, active) next to the server's.
    for (name, producer, native) in &video {
        let expected = if *name == "AV1" { 1 } else { 2 };
        let start = Instant::now();
        let mut live = 0;
        while start.elapsed() < Duration::from_secs(20) {
            let stats = producer.get_stats().await.unwrap();
            live = stats.iter().filter(|s| s.byte_count > 0).count();
            if live == expected {
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
        if live != expected {
            eprintln!("{name} native outbound-rtp: {}", outbound_rtp(native));
        }
        assert_eq!(live, expected, "{name} sends all its layers");
    }
    for (name, _, native) in &video {
        match *name {
            "H264" => check_encoder(name, &native.stats().unwrap(), "GELABBER_EXPECT_ENCODER"),
            "AV1" => check_encoder(
                name,
                &native.stats().unwrap(),
                "GELABBER_EXPECT_AV1_ENCODER",
            ),
            _ => {}
        }
    }

    // Receive side: the server consumes for the native client, paused until
    // the client is ready (like `consumerReady`).
    let (server_recv, recv_params) = server.transport().await;
    let (recv, recv_events) = Transport::new(&device, Direction::Recv, &recv_params).unwrap();
    server.serve(
        recv.clone(),
        server_recv.clone(),
        recv_events,
        Arc::new(Mutex::new(Vec::new())),
    );
    let client_caps: RtpCapabilities = serde_json::from_value(recv_caps).unwrap();
    let mut consumers = Vec::new();
    for (name, producer, _) in &video {
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
        consumers.push((*name, server_consumer, native));
    }
    // Wait for decoded frames at the native sinks; on failure report both
    // ends (server consumer egress, libwebrtc inbound-rtp) for every codec
    // before failing.
    let mut missing = Vec::new();
    for (_, server_consumer, native) in &consumers {
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
    for (name, _, native) in &consumers {
        let stats = native.stats().unwrap();
        match *name {
            "H264" => check_decoder(name, &stats, "GELABBER_EXPECT_DECODER", "h264", true),
            // A system decoder named here is in front of dav1d; "dav1d" is
            // the one behind it.
            "AV1" => {
                let element = std::env::var("GELABBER_EXPECT_AV1_DECODER")
                    .is_ok_and(|expected| expected.starts_with("GStreamer"));
                check_decoder(name, &stats, "GELABBER_EXPECT_AV1_DECODER", "av1", element)
            }
            _ => {}
        }
    }

    // Frames for the viewer: I420 planes of the decoded size. A failed
    // assertion in the sink is caught at the FFI boundary and shows as
    // missing frames.
    #[derive(Default)]
    struct Seen {
        frames: u64,
        size: (u32, u32),
        varied: bool,
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let sink_seen = seen.clone();
    let (_, _, viewer) = consumers
        .iter_mut()
        .find(|(name, ..)| *name == "VP8")
        .unwrap();
    viewer
        .set_video_sink(Some(Box::new(move |frame: &VideoFrame<'_>| {
            let (w, h) = (frame.width as usize, frame.height as usize);
            let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
            assert!(frame.stride_y >= w && frame.stride_u >= cw && frame.stride_v >= cw);
            // Up to the end of the last row (gm_video_frame).
            assert!(frame.y.len() >= frame.stride_y * (h - 1) + w);
            assert!(frame.u.len() >= frame.stride_u * (ch - 1) + cw);
            assert!(frame.v.len() >= frame.stride_v * (ch - 1) + cw);
            let row = &frame.y[..w];
            let mut seen = sink_seen.lock().unwrap();
            seen.frames += 1;
            seen.size = (frame.width, frame.height);
            seen.varied |= row.iter().min() != row.iter().max();
        })))
        .unwrap();
    wait_for("frames at the video sink", Duration::from_secs(10), || {
        let seen = seen.clone();
        async move { seen.lock().unwrap().frames > 10 }
    })
    .await;
    viewer.set_video_sink(None).unwrap();
    let after_removal = {
        let seen = seen.lock().unwrap();
        eprintln!(
            "video sink: {} frames, {:?}, varied {}",
            seen.frames, seen.size, seen.varied
        );
        assert!(seen.size.0 >= 320 && seen.size.1 >= 180, "{:?}", seen.size);
        assert!(seen.varied, "test pattern rows are not flat");
        seen.frames
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(seen.lock().unwrap().frames, after_removal, "sink removed");

    // Sender controls the web client uses: encodings with bitrate caps and
    // priority, replaceTrack, and track enabled.
    let (_, _, vp8) = video.iter_mut().find(|(name, ..)| *name == "VP8").unwrap();
    let params = vp8.parameters().unwrap();
    assert_eq!(
        params["encodings"].as_array().map(Vec::len),
        Some(2),
        "{params}"
    );
    vp8.set_parameters(&json!({"encodings": [
        {"maxBitrate": 150_000},
        {"maxBitrate": 1_200_000, "maxFramerate": 15, "priority": "high", "networkPriority": "high"},
    ]}))
    .unwrap();
    let params = vp8.parameters().unwrap();
    eprintln!("VP8 sender parameters: {params}");
    assert_eq!(params["encodings"][0]["maxBitrate"], 150_000);
    assert_eq!(params["encodings"][1]["maxBitrate"], 1_200_000);
    assert_eq!(params["encodings"][1]["priority"], "high");
    assert_eq!(params["encodings"][1]["networkPriority"], "high");
    assert!(
        vp8.set_parameters(&json!({"encodings": [{"priority": "urgent"}]}))
            .is_err()
    );
    let replacement = Source::test_pattern(&engine, 640, 360, 15).unwrap();
    vp8.replace_source(&replacement).unwrap();
    assert!(vp8.replace_source(&mic).is_err(), "kind mismatch refused");
    replacement.set_enabled(false).unwrap();
    replacement.set_enabled(true).unwrap();

    assert_no_tcp_candidates(server, &device, &mic).await;

    // Native objects close in dependency order.
    drop(consumers);
    drop((audio, video));
    drop((send, recv));
}

/// Simulcast from pictures whose sides do not divide by the layers' scale
/// factors: a 1366x768 screen, a window, a camera picture libwebrtc stepped
/// down, with the web client's layers (a quarter and the whole). libvpx
/// encodes the layers only when each has exactly the picture's aspect and
/// sends nothing otherwise, so the picture loses the columns and rows that
/// do not divide. Layers at an eighth, which no source is prepared for, get
/// there through what the encoder asks of its source.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simulcast_layers_fit_a_picture_of_any_size() {
    if std::env::var_os("GELABBER_MEDIA_LOG").is_some() {
        gelabber_media_core::set_log_level(gelabber_media_core::LogLevel::Info);
    }
    let server = Server::start().await;
    layers_by_size(&server).await;
    server.close().await;
}

/// The call of the test above, dropped on return.
async fn layers_by_size(server: &Server) {
    let engine = Engine::new(Audio::Dummy).unwrap();
    let device = Device::new(&engine).unwrap();
    let caps = serde_json::to_value(server.router.rtp_capabilities()).unwrap();
    device.load(&caps).unwrap();

    let server_producers = Arc::new(Mutex::new(Vec::new()));
    let (server_send, send_params) = server.transport().await;
    let (send, send_events) = Transport::new(&device, Direction::Send, &send_params).unwrap();
    server.serve(
        send.clone(),
        server_send,
        send_events,
        server_producers.clone(),
    );
    // Audio first: the harness, unlike the gateway, passes an empty CNAME
    // on, which mediasoup refuses for a transport's first producer.
    let mic = Source::microphone(&engine, &json!({})).unwrap();
    let audio = {
        let (send, mic) = (send.clone(), mic.clone());
        blocking(move || send.produce(&mic, &json!({})))
            .await
            .unwrap()
    };

    // Width, height and the scale factor of the small layer.
    for (width, height, factor) in [
        (1366u32, 768u32, 4u64),
        (1894, 1012, 4),
        (480, 270, 4),
        (1366, 768, 8),
    ] {
        let name = format!("{width}x{height} over {factor} and 1");
        // Ten pictures a second, so that a slow machine does not step the
        // picture down before its size is read.
        let source = Source::test_pattern(&engine, width, height, 10).unwrap();
        // With a start bitrate that carries both layers: what is under
        // test is their sizes, not how soon the bandwidth estimate of a new
        // transport lets the large one start.
        let options = json!({
            "codec": "video/VP8",
            "encodings": [{ "scaleResolutionDownBy": factor }, { "scaleResolutionDownBy": 1 }],
            "codecOptions": { "videoGoogleStartBitrate": 3000 },
        });
        let native = {
            let send = send.clone();
            blocking(move || send.produce(&source, &options))
                .await
                .unwrap()
        };
        let at_server = server_producers
            .lock()
            .unwrap()
            .iter()
            .find(|producer| producer.id().to_string() == native.id())
            .cloned()
            .unwrap();
        assert_eq!(
            simulcast_sizes(&name, &native, &at_server).await,
            layers_of(width.into(), height.into(), factor),
            "{name}"
        );
    }

    drop(audio);
    drop(send);
}

/// Application sound on Windows (WASAPI process loopback), as far as a
/// machine without sound devices takes it. The list is an array of
/// applications, without this process. A binary that plays nothing is no
/// error: the capture starts, mixes silence on the system clock and waits
/// for the binary to play (no loopback client yet). "Every application"
/// either captures or says why with Windows' reason, not with the stub's
/// "not available" of 0.6.0-alpha.1. Both captures end cleanly when dropped,
/// and the engine goes on. Nothing else in CI calls these on Windows.
#[test]
fn windows_application_sound_starts_and_ends() {
    if !cfg!(windows) {
        eprintln!("skipped: the Linux voice test (GELABBER_TEST_AUDIO) covers PipeWire");
        return;
    }
    let engine = Engine::new(Audio::Dummy).unwrap();
    let apps = engine.audio_apps().unwrap();
    eprintln!("applications playing: {apps}");
    for app in apps.as_array().expect("a list") {
        assert!(app["id"].is_string() && app["name"].is_string(), "{app}");
        assert!(app["streams"].as_u64().unwrap_or(0) > 0, "{app}");
    }

    let waiting = Source::app_audio(&engine, &json!({"app": "no-such-program.exe"}))
        .expect("a binary that plays nothing is no error");
    let first = waiting.state().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let later = waiting.state().unwrap();
    eprintln!("waiting for no-such-program.exe: {first} -> {later}");
    assert_eq!(later["state"], "live");
    assert_eq!(later["streams"], 0);
    assert!(
        later["frames"].as_u64().unwrap() >= first["frames"].as_u64().unwrap() + 20,
        "the mix goes on in 10 ms blocks"
    );
    drop(waiting);

    match Source::app_audio(&engine, &json!({"app": ""})) {
        Ok(every) => {
            std::thread::sleep(Duration::from_millis(300));
            let state = every.state().unwrap();
            eprintln!("every application: {state}");
            assert_eq!(state["state"], "live");
            assert_eq!(state["streams"], 1, "one client for all but this process");
        }
        Err(error) => {
            eprintln!("every application: {error}");
            let error = error.to_string();
            assert!(!error.contains("not available on Windows"), "{error}");
            assert!(error.contains("application sound"), "{error}");
        }
    }
    Source::test_pattern(&engine, 640, 360, 30).unwrap();
}

/// AV1 temporal units as the core's system encoders hand them on:
/// desktop/native/tests/av1_obu_test.cc runs the OBU parser without an
/// encoder, on every platform.
#[test]
fn av1_key_frames_carry_a_sequence_header() {
    run_host_test("av1_obu_test", "AV1 OBUs");
}

/// A run that names the H264 encoder to find must not pass on a core without
/// H264 by skipping the H264 half.
#[test]
fn an_expected_encoder_demands_h264() {
    assert_eq!(h264_demanded_by(None, None), None);
    assert_eq!(h264_demanded_by(Some("0"), Some("")), None);
    assert_eq!(
        h264_demanded_by(Some("1"), None),
        Some("GELABBER_EXPECT_H264=1")
    );
    assert_eq!(
        h264_demanded_by(None, Some("GStreamer x264enc")),
        Some("GELABBER_EXPECT_ENCODER or _DECODER is set")
    );
}

/// The server stays on 127.0.0.1 unless GELABBER_TEST_LISTEN_IP names an
/// address; a variable left empty counts as unset.
#[test]
fn the_server_listens_on_loopback_unless_told_otherwise() {
    let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    let other: std::net::IpAddr = "192.0.2.7".parse().unwrap();
    assert_eq!(listen_ip_from(None), loopback);
    assert_eq!(listen_ip_from(Some("")), loopback);
    assert_eq!(listen_ip_from(Some("192.0.2.7")), other);
}

fn local_candidates(transport: &Transport) -> Vec<Value> {
    transport
        .stats()
        .unwrap()
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s["type"] == "local-candidate")
        .collect()
}

/// The media server has no ICE-TCP, so the core gathers no TCP candidates:
/// each would be a listening socket, which is what the Windows firewall asks
/// about. libwebrtc only gets to its TCP gathering phase while ICE is still
/// unconnected, so this transport's remote candidates point at a local socket
/// that never answers.
async fn assert_no_tcp_candidates(server: &Server, device: &Device, mic: &Source) {
    let silent = std::net::UdpSocket::bind((listen_ip(), 0)).unwrap();
    let port = silent.local_addr().unwrap().port();
    let (server_transport, mut params) = server.transport().await;
    for candidate in params["iceCandidates"].as_array_mut().unwrap() {
        candidate["port"] = json!(port);
    }
    let (transport, events) = Transport::new(device, Direction::Send, &params).unwrap();
    server.serve(
        transport.clone(),
        server_transport,
        events,
        Arc::new(Mutex::new(Vec::new())),
    );
    let producer = {
        let (transport, mic) = (transport.clone(), mic.clone());
        blocking(move || transport.produce(&mic, &json!({})))
            .await
            .unwrap()
    };
    wait_for("local ICE candidates", Duration::from_secs(10), || {
        let transport = transport.clone();
        async move { !local_candidates(&transport).is_empty() }
    })
    .await;
    // The gathering phases (UDP, relay, TCP) start 50 ms apart.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let candidates = local_candidates(&transport);
    eprintln!(
        "local candidates without a connection: {:?}",
        candidates
            .iter()
            .map(|c| (c["protocol"].clone(), c["candidateType"].clone()))
            .collect::<Vec<_>>()
    );
    for candidate in &candidates {
        assert_eq!(candidate["protocol"], "udp", "{candidate}");
    }
    drop(producer);
}
