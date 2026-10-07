//! Voice through the platform audio device module: a PulseAudio
//! (pipewire-pulse) monitor source as the microphone, the web client's three
//! processing modes, Opus through mediasoup 0.29, and playout of the consumer
//! on a null sink.
//!
//! Needs the session from desktop/native/scripts/fake-desktop-session.sh,
//! which plays white noise into the microphone. Skipped unless
//! GELABBER_TEST_AUDIO=1 so a developer machine's real devices stay alone.

mod common;

use common::{Server, blocking, serve_events, wait_for};
use gelabber_media_core::{
    Audio, Consumer, Device, Direction, Engine, Producer, Source, Transport,
};
use mediasoup::prelude::Transport as _;
use mediasoup::prelude::*;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

async fn produce(send: &Transport, source: &Source, options: Value) -> Producer {
    let (send, source) = (send.clone(), source.clone());
    blocking(move || send.produce(&source, &options))
        .await
        .unwrap()
}

/// Polls the microphone meters until `ready` accepts a window that was
/// measured after this call (the meters keep their last values while capture
/// processing is idle).
async fn levels_when(engine: &Engine, what: &str, ready: impl Fn(&Value) -> bool) -> Value {
    let blocks = |l: &Value| l["blocks"].as_u64().unwrap_or(0);
    let start = Instant::now();
    let mut levels = engine.audio_levels().unwrap();
    // One meter window is 8 blocks of 10 ms; skip the one in progress.
    let fresh_after = blocks(&levels) + 16;
    let accept = |l: &Value| blocks(l) >= fresh_after && ready(l);
    while !accept(&levels) && start.elapsed() < Duration::from_secs(15) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        levels = engine.audio_levels().unwrap();
    }
    eprintln!("{what}: {levels}");
    assert!(
        blocks(&levels) >= fresh_after,
        "{what}: capture processing stalled: {levels}"
    );
    assert!(ready(&levels), "{what}: {levels}");
    levels
}

fn level(levels: &Value, key: &str) -> f64 {
    levels[key].as_f64().unwrap_or(0.0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voice_modes_reach_a_consumer() {
    if std::env::var_os("GELABBER_TEST_AUDIO").is_none() {
        eprintln!("skipped: set GELABBER_TEST_AUDIO=1 inside fake-desktop-session.sh");
        return;
    }
    if std::env::var_os("GELABBER_MEDIA_LOG").is_some() {
        gelabber_media_core::set_log_level(gelabber_media_core::LogLevel::Info);
    }
    let mic_id = std::env::var("GELABBER_TEST_MIC").expect("GELABBER_TEST_MIC");
    let speakers_id = std::env::var("GELABBER_TEST_SPEAKERS").expect("GELABBER_TEST_SPEAKERS");

    let server = Server::start().await;
    let runtime = Handle::current();
    let engine = Engine::new(Audio::Default).unwrap();

    let devices = engine.audio_devices().unwrap();
    eprintln!("audio devices: {devices}");
    let has = |list: &str, id: &str| {
        devices[list]
            .as_array()
            .is_some_and(|items| items.iter().any(|d| d["id"] == id))
    };
    assert!(has("inputs", &mic_id), "microphone {mic_id} listed");
    assert!(
        has("outputs", &speakers_id),
        "speakers {speakers_id} listed"
    );
    assert!(has("inputs", ""), "system default input listed");
    engine
        .configure_audio(&json!({"input": mic_id, "output": speakers_id}))
        .unwrap();
    let selected = engine.audio_devices().unwrap();
    assert_eq!(selected["input"], mic_id.as_str());
    assert_eq!(selected["output"], speakers_id.as_str());
    assert!(
        engine
            .configure_audio(&json!({"input": "no-such-device"}))
            .is_err()
    );

    // The microphone test: capture without a call. It keeps running below,
    // so the producers join capture that is already on.
    engine
        .monitor_audio(Some(&json!({"processingMode": "browser"})))
        .unwrap();
    levels_when(&engine, "microphone test", |_| true).await;

    let device = Device::new(&engine).unwrap();
    device
        .load(&serde_json::to_value(server.router.rtp_capabilities()).unwrap())
        .unwrap();
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
    let (server_recv, recv_params) = server.transport().await;
    let (recv, recv_events) = Transport::new(&device, Direction::Recv, &recv_params).unwrap();
    serve_events(
        runtime,
        recv.clone(),
        server_recv.clone(),
        recv_events,
        Arc::new(Mutex::new(Vec::new())),
    );

    // "browser" without noise suppression or AGC: the noise passes as is.
    let plain = Source::microphone(
        &engine,
        &json!({"processingMode": "browser", "noiseSuppression": false,
                "autoGainControl": false, "echoCancellation": false}),
    )
    .unwrap();
    let browser_producer =
        produce(&send, &plain, json!({"codecOptions": {"opusDtx": false}})).await;
    let server_producer = server_producers.lock().unwrap().last().cloned().unwrap();
    assert_eq!(server_producer.kind(), MediaKind::Audio);
    wait_for("Opus at the server", Duration::from_secs(20), || {
        let producer = server_producer.clone();
        async move {
            let stats = producer.get_stats().await.unwrap_or_default();
            stats.iter().any(|s| s.byte_count > 0)
        }
    })
    .await;

    let caps: RtpCapabilities = serde_json::from_value(device.rtp_capabilities().unwrap()).unwrap();
    let mut options = ConsumerOptions::new(server_producer.id(), caps);
    options.paused = true;
    let server_consumer = server_recv.consume(options).await.unwrap();
    let announcement = json!({
        "id": server_consumer.id(),
        "producerId": server_producer.id(),
        "kind": "audio",
        "rtpParameters": server_consumer.rtp_parameters(),
    });
    let consumer: Consumer = {
        let recv = recv.clone();
        blocking(move || recv.consume(&announcement)).await.unwrap()
    };
    server_consumer.resume().await.unwrap();
    let playing = &consumer;
    wait_for("remote audio played out", Duration::from_secs(20), || {
        let consumer = playing;
        async move {
            let stats = consumer.stats().unwrap();
            stats["samplesPlayed"].as_u64().unwrap_or(0) > 48_000
                && stats["audioLevel"].as_u64().unwrap_or(0) > 0
        }
    })
    .await;
    let played = consumer.stats().unwrap();
    eprintln!(
        "consumer: audioLevel {} samplesPlayed {}",
        played["audioLevel"], played["samplesPlayed"]
    );
    consumer.set_volume(0.5).unwrap();

    let browser = levels_when(&engine, "browser mode", |l| level(l, "input") >= 10.0).await;
    assert_eq!(browser["denoised"], false);
    assert!(level(&browser, "processed") >= level(&browser, "input") * 0.7);

    engine.configure_audio(&json!({"inputGain": 2.0})).unwrap();
    levels_when(&engine, "browser mode, gain 2", |l| {
        level(l, "input") >= 10.0 && level(l, "processed") >= level(l, "input") * 1.6
    })
    .await;
    engine.configure_audio(&json!({"inputGain": 1.0})).unwrap();

    // Each new mode is produced before the old producer closes, so capture
    // keeps running (as when the client swaps tracks). The test microphone
    // (a remapped null-sink monitor) records only zeros after a quick
    // capture restart.

    // "enhanced": RNNoise takes the noise out.
    let enhanced = Source::microphone(
        &engine,
        &json!({"processingMode": "enhanced", "echoCancellation": false}),
    )
    .unwrap();
    let enhanced_producer =
        produce(&send, &enhanced, json!({"codecOptions": {"opusDtx": true}})).await;
    let denoised = levels_when(&engine, "enhanced mode", |l| {
        l["denoised"] == true && level(l, "input") >= 10.0
    })
    .await;
    assert!(
        level(&denoised, "processed") * 3.0 <= level(&denoised, "input"),
        "RNNoise suppresses white noise: {denoised}"
    );
    drop(browser_producer);

    // "original": stereo Opus, no processing.
    let original = Source::microphone(
        &engine,
        &json!({"processingMode": "original", "echoCancellation": false}),
    )
    .unwrap();
    let original_producer = produce(
        &send,
        &original,
        json!({"codecOptions": {"opusStereo": true, "opusDtx": false}}),
    )
    .await;
    drop(enhanced_producer);
    eprintln!(
        "original rtpParameters: {}",
        original_producer.rtp_parameters().unwrap()["codecs"]
    );
    levels_when(&engine, "original mode", |l| {
        l["denoised"] == false
            && level(l, "input") >= 10.0
            && level(l, "processed") >= level(l, "input") * 0.7
    })
    .await;

    // Source audio: the noise player (pw-play, another process) as an
    // application, on its own track next to the microphone.
    let apps = engine.audio_apps().unwrap();
    eprintln!("audio apps: {apps}");
    assert!(
        apps.as_array()
            .is_some_and(|list| list.iter().any(|app| app["id"] == "pw-play")),
        "pw-play listed: {apps}"
    );
    let app_audio = Source::app_audio(&engine, &json!({"app": "pw-play"})).unwrap();
    let capturing = &app_audio;
    wait_for(
        "application stream captured",
        Duration::from_secs(10),
        || {
            let source = capturing;
            async move { source.state().unwrap()["streams"].as_u64().unwrap_or(0) >= 1 }
        },
    )
    .await;
    let app_producer = produce(
        &send,
        &app_audio,
        json!({"codecOptions": {"opusStereo": true, "opusDtx": false}}),
    )
    .await;
    let server_app = server_producers.lock().unwrap().last().cloned().unwrap();
    assert_ne!(server_app.id(), server_producer.id());
    let caps: RtpCapabilities = serde_json::from_value(device.rtp_capabilities().unwrap()).unwrap();
    let mut options = ConsumerOptions::new(server_app.id(), caps);
    options.paused = true;
    let server_app_consumer = server_recv.consume(options).await.unwrap();
    let announcement = json!({
        "id": server_app_consumer.id(),
        "producerId": server_app.id(),
        "kind": "audio",
        "rtpParameters": server_app_consumer.rtp_parameters(),
    });
    let app_consumer: Consumer = {
        let recv = recv.clone();
        blocking(move || recv.consume(&announcement)).await.unwrap()
    };
    server_app_consumer.resume().await.unwrap();
    let hearing = &app_consumer;
    wait_for(
        "application sound played out",
        Duration::from_secs(20),
        || {
            let consumer = hearing;
            async move {
                let stats = consumer.stats().unwrap();
                stats["samplesPlayed"].as_u64().unwrap_or(0) > 48_000
                    && stats["audioLevel"].as_u64().unwrap_or(0) > 0
            }
        },
    )
    .await;
    eprintln!(
        "application sound: {} / consumer {}",
        app_audio.state().unwrap(),
        app_consumer.stats().unwrap()["audioLevel"]
    );
    drop(app_consumer);
    drop(app_producer);
    drop(app_audio);

    // Ending the test leaves the call's capture running.
    engine.monitor_audio(None).unwrap();
    levels_when(&engine, "after the microphone test", |l| {
        level(l, "input") >= 10.0
    })
    .await;
    drop(original_producer);
}
