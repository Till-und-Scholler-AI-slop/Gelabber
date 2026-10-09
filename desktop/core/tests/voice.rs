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

/// The file the program `name` runs (pw-play is a link to pw-cat).
fn program_of(name: &str) -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .find_map(|dir| std::fs::canonicalize(dir.join(name)).ok())
        .unwrap_or_else(|| panic!("{name} on PATH"))
}

/// Its file name, which is what the sound server knows the program by.
fn binary_of(name: &str) -> String {
    let program = program_of(name);
    program.file_name().unwrap().to_str().unwrap().to_owned()
}

/// pactl in the test session.
async fn pactl(args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    blocking(move || {
        let output = std::process::Command::new("pactl")
            .env("LC_ALL", "C")
            .args(&args)
            .output()
            .expect("pactl");
        assert!(
            output.status.success(),
            "pactl {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    })
    .await
}

/// Indices of the playback streams ("sink inputs") of the applications
/// called `name`, once there are `count` of them.
async fn sink_inputs_of(name: &str, count: usize) -> Vec<String> {
    let wanted = format!("application.name = \"{name}\"");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let (mut index, mut found) = (None, Vec::new());
        for line in pactl(&["list", "sink-inputs"]).await.lines() {
            if let Some(number) = line.strip_prefix("Sink Input #") {
                index = Some(number.trim().to_owned());
            } else if line.contains(&wanted) {
                found.push(index.clone().expect("sink input before its properties"));
            }
        }
        if found.len() >= count {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("no {count} sink input(s) of {name}");
}

/// Index of the playback stream of the application `name`, once it plays.
async fn sink_input_of(name: &str) -> String {
    sink_inputs_of(name, 1).await.remove(0)
}

/// Waits for the playback stream ("sink input") of a virtual device: one
/// that carries what the core tells such a stream by.
async fn playback_of_a_virtual_device() {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if pactl(&["list", "sink-inputs"])
            .await
            .lines()
            .any(|line| line.trim_start().starts_with("node.link-group = "))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("no sink input with a node.link-group: the loopback has no playback stream");
}

/// For `sh -c`: paplay, or the copy of it `$1`, as the application `$0` with
/// the session's noise file, read as raw stereo.
const NOISE_PLAYER: &str = r#""${1:-paplay}" --playback --raw --rate=48000 --channels=2 \
    --format=s16le --device=gelabber-speakers --client-name="$0" "$XDG_RUNTIME_DIR/noise.wav""#;

/// A sound player that ends with the test.
struct Player(std::process::Child);

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts the noise player as the application `name`. Its shell is gone
/// before it plays, so it is no process of this test. Returns its pid.
fn start_player(name: &str) -> String {
    start_player_of(name, None)
}

/// The same from `program`, a copy of paplay under another file name: to
/// the sound server another binary.
fn start_player_of(name: &str, program: Option<&std::path::Path>) -> String {
    let mut shell = std::process::Command::new("sh");
    shell.args([
        "-c",
        &format!("{NOISE_PLAYER} >/dev/null 2>&1 & echo $!"),
        name,
    ]);
    if let Some(program) = program {
        shell.arg(program);
    }
    let started = shell.output().expect("paplay");
    String::from_utf8_lossy(&started.stdout).trim().to_owned()
}

/// Ends a player from [`start_player`], which has to be playing still.
fn stop_player(pid: &str) {
    assert!(
        std::process::Command::new("kill")
            .arg(pid)
            .status()
            .expect("kill")
            .success(),
        "player {pid} was running"
    );
}

/// The application list in a fixed order.
fn by_id(apps: &Value) -> Vec<Value> {
    let mut apps = apps.as_array().expect("application list").clone();
    apps.sort_by_key(|app| app["id"].as_str().unwrap_or_default().to_owned());
    apps
}

/// Playback streams an application-sound source captures once it settled.
async fn captured_streams(source: &Source, expected: u64) -> u64 {
    let streams = || source.state().unwrap()["streams"].as_u64().unwrap_or(0);
    let start = Instant::now();
    while streams() < expected && start.elapsed() < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // A stream too many shows up with the others, in the first listing.
    tokio::time::sleep(Duration::from_secs(1)).await;
    streams()
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

    // Source audio. The noise player (pw-play, another process) is an
    // application. This process is not, although it is playing the consumer
    // out through the device module all along, and neither is a player it
    // started, which stands for the app's webview helpers.
    let samples_played = || {
        consumer.stats().unwrap()["samplesPlayed"]
            .as_u64()
            .unwrap_or(0)
    };
    let played_before = samples_played();
    let helper = Player(
        std::process::Command::new("sh")
            .args([
                "-c",
                &format!("exec {NOISE_PLAYER}"),
                "gelabber-test-helper",
            ])
            .spawn()
            .expect("paplay"),
    );
    sink_input_of("gelabber-test-helper").await;
    let apps = engine.audio_apps().unwrap();
    eprintln!("audio apps: {apps}");
    let listed = apps.as_array().expect("application list");
    assert_eq!(listed.len(), 1, "the noise player and nothing else: {apps}");
    let player = &listed[0];
    assert_eq!(player["name"], "pw-play", "{apps}");
    assert_eq!(player["streams"], 1, "{apps}");
    // The id is the binary, whatever the application calls itself.
    let player_id = player["id"].as_str().expect("application id").to_owned();
    assert_eq!(player_id, binary_of("pw-play"), "{apps}");

    // "" is every application: the noise player's stream alone.
    let everything = Source::app_audio(&engine, &json!({"app": ""})).unwrap();
    assert_eq!(
        captured_streams(&everything, 1).await,
        1,
        "the noise player and not this process: {}",
        everything.state().unwrap()
    );
    assert!(
        samples_played() > played_before,
        "this process played sound in the meantime"
    );
    drop(everything);
    drop(helper);

    // The call played out through a virtual device: a sink whose sound a
    // loopback plays on to the speakers, the way an echo canceller, an
    // equaliser or a combined sink is built. The loopback's playback stream
    // carries the call and is the sound server's, not this process's. It is
    // no application: neither listed nor captured.
    let virtual_sink = pactl(&[
        "load-module",
        "module-null-sink",
        "sink_name=gelabber-virtual",
        "sink_properties=device.description=Gelabber-Virtual",
    ])
    .await;
    let loopback = pactl(&[
        "load-module",
        "module-loopback",
        "source=gelabber-virtual.monitor",
        "sink=gelabber-speakers",
    ])
    .await;
    playback_of_a_virtual_device().await;
    engine
        .configure_audio(&json!({"output": "Gelabber-Virtual"}))
        .unwrap();
    let played_before = samples_played();
    let everything = Source::app_audio(&engine, &json!({"app": ""})).unwrap();
    assert_eq!(
        captured_streams(&everything, 1).await,
        1,
        "the noise player and not the virtual device's output: {}",
        everything.state().unwrap()
    );
    assert!(
        samples_played() > played_before,
        "this process played sound through the virtual device in the meantime"
    );
    let apps = engine.audio_apps().unwrap();
    assert_eq!(
        by_id(&apps),
        std::slice::from_ref(player),
        "the noise player and no virtual device"
    );
    drop(everything);
    engine
        .configure_audio(&json!({"output": speakers_id}))
        .unwrap();
    for module in [loopback, virtual_sink] {
        pactl(&["unload-module", module.trim()]).await;
    }

    // The same player on its own (its shell is gone before it plays) is an
    // application. It is chosen by its name here: the id up to 0.5.2, which
    // a client may have stored. Its sound goes on a track of its own next
    // to the microphone.
    let player_pid = start_player("gelabber-test-player");
    let player_stream = sink_input_of("gelabber-test-player").await;
    let app_audio = Source::app_audio(&engine, &json!({"app": "gelabber-test-player"})).unwrap();
    assert_eq!(captured_streams(&app_audio, 1).await, 1);
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

    // The player changes to an output with another channel layout. Its ports
    // are replaced and the links to the capture stream go with them; the
    // session manager does not put them back.
    pactl(&[
        "load-module",
        "module-null-sink",
        "sink_name=gelabber-surround",
        "channels=6",
        "channel_map=front-left,front-right,front-center,lfe,rear-left,rear-right",
    ])
    .await;
    pactl(&["move-sink-input", &player_stream, "gelabber-surround"]).await;
    // Longer than a level from before the change lasts.
    tokio::time::sleep(Duration::from_secs(2)).await;
    wait_for(
        "application sound after the player changed outputs",
        Duration::from_secs(10),
        || {
            let consumer = hearing;
            async move {
                consumer.stats().unwrap()["audioLevel"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0
            }
        },
    )
    .await;
    eprintln!(
        "after the player changed outputs: {} / consumer {}",
        app_audio.state().unwrap(),
        app_consumer.stats().unwrap()["audioLevel"]
    );
    assert_eq!(app_audio.state().unwrap()["streams"], 1);
    drop(app_consumer);
    drop(app_producer);
    drop(app_audio);
    stop_player(&player_pid);

    // Two applications that run one binary, as on a shared Electron or Wine.
    // The binary's entry is both; each is listed by the name it gave itself
    // as well.
    let pair = ["gelabber-test-one", "gelabber-test-two"];
    let pair_pids = pair.map(start_player);
    for name in pair {
        sink_input_of(name).await;
    }
    let shared = binary_of("paplay");
    let apps = engine.audio_apps().unwrap();
    eprintln!("audio apps, two on one binary: {apps}");
    assert_eq!(
        by_id(&apps),
        by_id(&json!([
            {"id": pair[0], "name": pair[0], "streams": 1},
            {"id": pair[1], "name": pair[1], "streams": 1},
            {"id": shared, "name": shared, "streams": 2},
            {"id": player_id, "name": "pw-play", "streams": 1},
        ]))
    );

    // An application chosen by its id. Up to 0.5.2 this process was listed as
    // well, under the name libwebrtc gives its playout. One of the two by
    // its name, both by their binary.
    for (app, expected) in [
        (player_id.as_str(), 1),
        ("WEBRTC VoiceEngine", 0),
        (pair[1], 1),
        (shared.as_str(), 2),
    ] {
        let chosen = Source::app_audio(&engine, &json!({"app": app})).unwrap();
        assert_eq!(
            captured_streams(&chosen, expected).await,
            expected,
            "streams captured for {app:?}"
        );
    }

    // One of the two alone is the binary's entry, under its name.
    stop_player(&pair_pids[1]);
    let alone = by_id(&json!([
        {"id": shared, "name": pair[0], "streams": 1},
        {"id": player_id, "name": "pw-play", "streams": 1},
    ]));
    let (listing, alone) = (&engine, &alone);
    wait_for(
        "one application left on the binary",
        Duration::from_secs(10),
        || async move { by_id(&listing.audio_apps().unwrap()) == *alone },
    )
    .await;
    stop_player(&pair_pids[0]);

    // Two programs on different binaries that call themselves the same, as
    // programs with an Electron of their own all play as "Chromium". Each
    // binary's entry says which it is, and the name is an entry too: it
    // chooses both, as it did when it was the id (up to 0.5.2).
    let same = "gelabber-test-same";
    let twin = "gelabber-test-twin";
    let twin_program =
        std::path::Path::new(&std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"))
            .join(twin);
    std::fs::copy(program_of("paplay"), &twin_program).expect("a copy of paplay");
    let same_pids = [
        start_player(same),
        start_player_of(same, Some(&twin_program)),
    ];
    sink_inputs_of(same, 2).await;
    let told_apart = by_id(&json!([
        {"id": same, "name": same, "streams": 2},
        {"id": twin, "name": format!("{same} ({twin})"), "streams": 1},
        {"id": shared, "name": format!("{same} ({shared})"), "streams": 1},
        {"id": player_id, "name": "pw-play", "streams": 1},
    ]));
    let (listing, told_apart) = (&engine, &told_apart);
    wait_for(
        "two binaries under one name, told apart",
        Duration::from_secs(10),
        || async move {
            let apps = listing.audio_apps().unwrap();
            eprintln!("audio apps, one name on two binaries: {apps}");
            by_id(&apps) == *told_apart
        },
    )
    .await;
    for (app, expected) in [(same, 2), (twin, 1), (shared.as_str(), 1)] {
        let chosen = Source::app_audio(&engine, &json!({"app": app})).unwrap();
        assert_eq!(
            captured_streams(&chosen, expected).await,
            expected,
            "streams captured for {app:?}"
        );
    }
    // With a second program on the first binary, that binary's entry goes by
    // its id, and the name's entry is the one a binary already has for each
    // of its names. The other binary still says which it is.
    let other = "gelabber-test-other";
    let other_pid = start_player(other);
    sink_input_of(other).await;
    let told_apart = by_id(&json!([
        {"id": other, "name": other, "streams": 1},
        {"id": same, "name": same, "streams": 2},
        {"id": twin, "name": format!("{same} ({twin})"), "streams": 1},
        {"id": shared, "name": shared, "streams": 2},
        {"id": player_id, "name": "pw-play", "streams": 1},
    ]));
    let (listing, told_apart) = (&engine, &told_apart);
    wait_for(
        "a name of one binary's programs that another binary carries too",
        Duration::from_secs(10),
        || async move {
            let apps = listing.audio_apps().unwrap();
            eprintln!("audio apps, a shared binary's name on another binary: {apps}");
            by_id(&apps) == *told_apart
        },
    )
    .await;
    for pid in same_pids.iter().chain([&other_pid]) {
        stop_player(pid);
    }

    // Ending the test leaves the call's capture running.
    engine.monitor_audio(None).unwrap();
    levels_when(&engine, "after the microphone test", |l| {
        level(l, "input") >= 10.0
    })
    .await;
    drop(original_producer);
}
