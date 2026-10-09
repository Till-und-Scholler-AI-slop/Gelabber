//! Video sinks for views inside the app: a sink on a local source without a
//! transport or a producer (the self view), the limits that scale frames
//! down and cap their rate, and sinks going away while frames are delivered.
//! The test pattern stands in for camera and screen; all three are
//! `LocalVideoSource`s behind the same libwebrtc video track.

mod common;

use common::{Server, blocking, serve_events};
use gelabber_media_core::{
    Audio, Device, Direction, Engine, Source, Transport, VideoFrame, VideoSink, VideoSinkLimits,
};
use mediasoup::prelude::*;
// Trait methods (id, produce, consume); the name is taken by the native transport.
use mediasoup::prelude::Transport as _;
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

#[derive(Default)]
struct Seen {
    frames: u64,
    size: (u32, u32),
    source: (u32, u32),
    rotation: u32,
    first: Option<Instant>,
    last: Option<Instant>,
    threads: Vec<String>,
    black: u64,
    varied: bool,
    last_first_pixel: Option<u8>,
    /// How far a frame's chroma was from the pattern's at worst: one U value
    /// per frame and V 128 everywhere.
    chroma_error: u8,
    /// A frame whose planes do not hold what its size says.
    broken: Option<String>,
}

impl Seen {
    /// Rate at which the sink was called.
    fn fps(&self) -> f64 {
        let span = self.last.unwrap().duration_since(self.first.unwrap());
        (self.frames - 1) as f64 / span.as_secs_f64()
    }
}

fn thread_tag() -> String {
    let name = std::fs::read_to_string("/proc/thread-self/comm").unwrap_or_default();
    format!("{:?}/{}", std::thread::current().id(), name.trim())
}

fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < limit, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A plane's rows without the stride padding; `None` when the slice is short.
fn rows(plane: &[u8], stride: usize, width: usize, rows: usize) -> Option<Vec<&[u8]>> {
    (0..rows)
        .map(|row| plane.get(row * stride..row * stride + width))
        .collect()
}

fn recorder(seen: Arc<Mutex<Seen>>) -> VideoSink {
    Box::new(move |frame: &VideoFrame<'_>| {
        let now = Instant::now();
        let mut seen = seen.lock().unwrap();
        seen.first.get_or_insert(now);
        seen.last = Some(now);
        seen.frames += 1;
        seen.size = (frame.width, frame.height);
        seen.source = (frame.source_width, frame.source_height);
        seen.rotation = frame.rotation;
        let tag = thread_tag();
        if !seen.threads.contains(&tag) {
            seen.threads.push(tag);
        }
        let (width, height) = (frame.width as usize, frame.height as usize);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let planes = (
            rows(frame.y, frame.stride_y, width, height),
            rows(frame.u, frame.stride_u, chroma_width, chroma_height),
            rows(frame.v, frame.stride_v, chroma_width, chroma_height),
        );
        let (Some(y), Some(u), Some(v)) = planes else {
            seen.broken = Some(format!("short planes at {width}x{height}"));
            return;
        };
        let luma_black = y.iter().all(|row| row.iter().all(|&value| value == 0));
        if luma_black && u[0][0] == 128 && v[0][0] == 128 {
            seen.black += 1;
            return;
        }
        let spread = |plane: &[&[u8]], value: u8| {
            let samples = plane.iter().flat_map(|row| row.iter());
            samples.map(|&p| p.abs_diff(value)).max().unwrap_or(0)
        };
        seen.chroma_error = seen
            .chroma_error
            .max(spread(&u, u[0][0]))
            .max(spread(&v, 128));
        let first = y[0][0];
        if seen.last_first_pixel.is_some_and(|last| last != first) {
            seen.varied = true;
        }
        seen.last_first_pixel = Some(first);
    })
}

/// Frames at a fresh recorder until `enough` holds; the sink stays set.
fn watch(source: &Source, what: &str, enough: impl Fn(&Seen) -> bool) -> Arc<Mutex<Seen>> {
    let seen = Arc::new(Mutex::new(Seen::default()));
    source.set_video_sink(Some(recorder(seen.clone()))).unwrap();
    wait_for(what, Duration::from_secs(10), || {
        enough(&seen.lock().unwrap())
    });
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.broken, None);
        // libyuv's box filter rounds down: a flat plane may come out one lower.
        assert!(
            seen.chroma_error <= 1,
            "chroma off by {}",
            seen.chroma_error
        );
    }
    seen
}

#[test]
fn a_local_source_feeds_a_sink_without_a_producer() {
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 640, 360, 30).unwrap();
    eprintln!("test thread {}", thread_tag());

    let seen = watch(&source, "frames", |seen| seen.frames > 30);
    {
        let seen = seen.lock().unwrap();
        eprintln!(
            "sink: {} frames, {:?}, rotation {}, {:.1} fps, threads {:?}",
            seen.frames,
            seen.size,
            seen.rotation,
            seen.fps(),
            seen.threads
        );
        assert_eq!(seen.size, (640, 360));
        assert_eq!(seen.source, (640, 360));
        assert_eq!(seen.rotation, 0);
        assert!(seen.varied, "the pattern moves");
        assert_eq!(seen.black, 0);
        assert!(
            (25.0..=35.0).contains(&seen.fps()),
            "source rate, got {}",
            seen.fps()
        );
        assert_eq!(seen.threads.len(), 1, "one capture thread");
        assert_ne!(seen.threads[0], thread_tag());
    }

    // A disabled source shows black, like a disabled MediaStreamTrack.
    source.set_enabled(false).unwrap();
    wait_for("black frames", Duration::from_secs(5), || {
        seen.lock().unwrap().black > 5
    });
    source.set_enabled(true).unwrap();
    let black = seen.lock().unwrap().black;
    std::thread::sleep(Duration::from_millis(300));
    let after = seen.lock().unwrap().black;
    assert!(after <= black + 1, "picture again after enabling");

    // Replacing the sink: the old one is not called again.
    let first_total = {
        let second = Arc::new(Mutex::new(Seen::default()));
        source
            .set_video_sink(Some(recorder(second.clone())))
            .unwrap();
        let total = seen.lock().unwrap().frames;
        wait_for("frames at the second sink", Duration::from_secs(5), || {
            second.lock().unwrap().frames > 10
        });
        total
    };
    assert_eq!(
        seen.lock().unwrap().frames,
        first_total,
        "first sink replaced"
    );

    // Removing it: nothing more arrives, and removing twice is fine.
    let second = watch(&source, "frames again", |seen| seen.frames > 5);
    let started = Instant::now();
    source.set_video_sink(None).unwrap();
    eprintln!("sink removed in {:?}", started.elapsed());
    let total = second.lock().unwrap().frames;
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(second.lock().unwrap().frames, total, "sink removed");
    source.set_video_sink(None).unwrap();

    // A clone shares the source (producers hold one): same sink slot.
    let third = watch(&source.clone(), "frames through a clone", |seen| {
        seen.frames > 5
    });

    // Dropping the source with a sink still set frees it cleanly.
    drop(source);
    let total = third.lock().unwrap().frames;
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(third.lock().unwrap().frames, total, "source gone");

    // Audio sources have no picture.
    let mic = Source::microphone(&engine, &json!({})).unwrap();
    let error = mic
        .set_video_sink(Some(Box::new(|_: &VideoFrame<'_>| {})))
        .unwrap_err();
    eprintln!("microphone: {error}");
    assert!(error.to_string().contains("video source"));
    assert!(
        mic.set_video_sink_limits(VideoSinkLimits::default())
            .is_err()
    );
    // Removing is a no-op there, so closing any source may clear its sink.
    mic.set_video_sink(None).unwrap();
}

fn limits(max_width: u32, max_height: u32, max_fps: u32) -> VideoSinkLimits {
    VideoSinkLimits {
        max_width,
        max_height,
        max_fps,
    }
}

#[test]
fn sink_limits_scale_down_and_cap_the_rate() {
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 640, 360, 30).unwrap();

    // Limits set before the sink hold from its first frame. The picture
    // shrinks to fit, aspect kept, to even dimensions, and is never enlarged.
    for (wanted, expected) in [
        (limits(320, 180, 0), (320, 180)),
        (limits(300, 300, 0), (300, 168)),
        (limits(0, 101, 0), (178, 100)),
        (limits(333, 0, 0), (332, 186)),
        (limits(640, 360, 0), (640, 360)),
        (limits(4000, 4000, 0), (640, 360)),
        (limits(1, 1, 0), (2, 2)),
    ] {
        source.set_video_sink(None).unwrap();
        source.set_video_sink_limits(wanted).unwrap();
        let seen = watch(&source, "scaled frames", |seen| seen.frames >= 5);
        let seen = seen.lock().unwrap();
        eprintln!("{wanted:?}: {:?} from {:?}", seen.size, seen.source);
        assert_eq!(seen.size, expected, "{wanted:?}");
        assert_eq!(seen.source, (640, 360));
        // (Two pixels average the moving pattern away.)
        assert!(seen.varied || expected == (2, 2), "the picture still moves");
    }

    // Limits change under a running sink, and lifting them restores the source.
    source.set_video_sink_limits(limits(160, 90, 0)).unwrap();
    let seen = watch(&source, "frames", |seen| seen.frames >= 3);
    source.set_video_sink_limits(limits(320, 180, 0)).unwrap();
    wait_for("the new size", Duration::from_secs(5), || {
        seen.lock().unwrap().size == (320, 180)
    });
    source
        .set_video_sink_limits(VideoSinkLimits::default())
        .unwrap();
    wait_for("the source size", Duration::from_secs(5), || {
        seen.lock().unwrap().size == (640, 360)
    });

    // A rate limit drops frames evenly; one at or above the source's rate
    // drops none.
    for (max_fps, expected) in [(10, 8.5..=11.0), (30, 25.0..=31.5), (1000, 25.0..=31.5)] {
        source.set_video_sink(None).unwrap();
        source
            .set_video_sink_limits(limits(320, 180, max_fps))
            .unwrap();
        let started = Instant::now();
        let seen = watch(&source, "paced frames", |_| {
            started.elapsed() > Duration::from_secs(2)
        });
        let seen = seen.lock().unwrap();
        eprintln!(
            "max {max_fps} fps: {} frames, {:.1} fps",
            seen.frames,
            seen.fps()
        );
        assert!(
            expected.contains(&seen.fps()),
            "max {max_fps} fps, got {}",
            seen.fps()
        );
        assert_eq!(seen.size, (320, 180));
    }
}

#[test]
fn removing_a_sink_waits_for_the_frame_in_delivery() {
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 320, 180, 30).unwrap();
    let inside = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicU64::new(0));
    for _ in 0..5 {
        let (sink_inside, sink_calls) = (inside.clone(), calls.clone());
        source
            .set_video_sink(Some(Box::new(move |_: &VideoFrame<'_>| {
                sink_inside.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(25));
                sink_calls.fetch_add(1, Ordering::SeqCst);
                sink_inside.store(false, Ordering::SeqCst);
            })))
            .unwrap();
        // Remove it while a call runs: the call ends first, none follows.
        wait_for("a call in flight", Duration::from_secs(5), || {
            inside.load(Ordering::SeqCst)
        });
        source.set_video_sink(None).unwrap();
        assert!(!inside.load(Ordering::SeqCst), "removal waited");
        let total = calls.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(calls.load(Ordering::SeqCst), total, "no call after removal");
    }
    // The same when the source itself goes with a call in flight.
    let sink_inside = inside.clone();
    source
        .set_video_sink(Some(Box::new(move |_: &VideoFrame<'_>| {
            sink_inside.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(25));
            sink_inside.store(false, Ordering::SeqCst);
        })))
        .unwrap();
    wait_for("a call in flight", Duration::from_secs(5), || {
        inside.load(Ordering::SeqCst)
    });
    drop(source);
    assert!(!inside.load(Ordering::SeqCst), "freeing waited");
}

/// The limits of a consumer's sink, on decoded remote video through
/// mediasoup, and freeing a consumer whose sink is still set.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_consumer_sink_keeps_the_same_limits() {
    let server = Server::start().await;
    let runtime = Handle::current();
    let engine = Engine::new(Audio::Dummy).unwrap();
    let device = Device::new(&engine).unwrap();
    let caps = serde_json::to_value(server.router.rtp_capabilities()).unwrap();
    device.load(&caps).unwrap();

    let producers = Arc::new(Mutex::new(Vec::new()));
    let (server_send, send_params) = server.transport().await;
    let (send, send_events) = Transport::new(&device, Direction::Send, &send_params).unwrap();
    serve_events(
        runtime.clone(),
        send.clone(),
        server_send,
        send_events,
        producers.clone(),
    );
    // Audio first: the harness, unlike the gateway, passes an empty CNAME
    // on, which mediasoup refuses for a transport's first producer.
    let mic = Source::microphone(&engine, &json!({})).unwrap();
    let pattern = Source::test_pattern(&engine, 640, 360, 30).unwrap();
    let (_audio, video) = {
        let (send, mic, pattern) = (send.clone(), mic.clone(), pattern.clone());
        blocking(move || {
            let audio = send.produce(&mic, &json!({})).unwrap();
            let video = send
                .produce(&pattern, &json!({"codec": "video/VP8"}))
                .unwrap();
            (audio, video)
        })
        .await
    };
    let server_producer = producers
        .lock()
        .unwrap()
        .iter()
        .find(|producer| producer.id().to_string() == video.id())
        .cloned()
        .unwrap();

    let (server_recv, recv_params) = server.transport().await;
    let (recv, recv_events) = Transport::new(&device, Direction::Recv, &recv_params).unwrap();
    serve_events(
        runtime.clone(),
        recv.clone(),
        server_recv.clone(),
        recv_events,
        Arc::new(Mutex::new(Vec::new())),
    );
    let client_caps: RtpCapabilities =
        serde_json::from_value(device.rtp_capabilities().unwrap()).unwrap();
    let mut options = ConsumerOptions::new(server_producer.id(), client_caps);
    options.paused = true;
    let server_consumer = server_recv.consume(options).await.unwrap();
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
    server_consumer.resume().await.unwrap();

    let start = Instant::now();
    while consumer.stats().unwrap()["framesReceived"]
        .as_u64()
        .unwrap_or(0)
        <= 10
    {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "no decoded frames"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Small enough to scale whatever size the encoder has reached by now.
    let seen = Arc::new(Mutex::new(Seen::default()));
    consumer
        .set_video_sink_limits(limits(128, 128, 10))
        .unwrap();
    consumer
        .set_video_sink(Some(recorder(seen.clone())))
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    {
        let seen = seen.lock().unwrap();
        eprintln!(
            "consumer sink: {} frames, {:?} from {:?}, {:.1} fps, chroma within {}, threads {:?}",
            seen.frames,
            seen.size,
            seen.source,
            seen.fps(),
            seen.chroma_error,
            seen.threads
        );
        assert_eq!(seen.broken, None);
        // The pattern, or a step down from it. A local source keeps both
        // sides of what it hands on a multiple of 4, so three quarters of
        // 640x360 are 480x264.
        let (width, height) = seen.source;
        assert!(
            [(640, 360), (480, 264), (320, 180)].contains(&seen.source),
            "decoded size {:?}",
            seen.source
        );
        assert_eq!(seen.size, (128, (height * 128 / width) & !1));
        // At most the limit; how much less is up to the decoder's pace.
        assert!((5.0..=11.0).contains(&seen.fps()), "got {}", seen.fps());
    }

    // Freeing the consumer with its sink still set: no call afterwards.
    blocking(move || drop(consumer)).await;
    let total = seen.lock().unwrap().frames;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(seen.lock().unwrap().frames, total, "consumer gone");

    drop(video);
    drop((send, recv));
}

fn thread_cpu() -> Duration {
    // Nanoseconds this thread ran.
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").unwrap_or_default();
    Duration::from_nanos(
        stat.split_whitespace()
            .next()
            .and_then(|ns| ns.parse().ok())
            .unwrap_or(0),
    )
}

/// What a self view of a large source costs on its capture thread, with the
/// picture handed on as it is and scaled down:
/// `cargo test --release --test source_preview -- --ignored --nocapture cost`.
#[test]
#[ignore = "timing; run in release"]
fn cost_of_scaling_on_the_capture_thread() {
    let engine = Engine::new(Audio::Dummy).unwrap();
    for (width, height, fps) in [(3840u32, 2160u32, 30u32), (1920, 1080, 60)] {
        for wanted in [limits(0, 0, 0), limits(1280, 720, 0), limits(640, 360, 0)] {
            let source = Source::test_pattern(&engine, width, height, fps).unwrap();
            source.set_video_sink_limits(wanted).unwrap();
            // CPU time of the capture thread between the first and the last
            // call: the pattern itself, the scaling and this copy.
            #[derive(Default)]
            struct Cost {
                frames: u32,
                size: (u32, u32),
                first: Duration,
                last: Duration,
                copy: Vec<u8>,
            }
            let cost = Arc::new(Mutex::new(Cost::default()));
            let shared = cost.clone();
            source
                .set_video_sink(Some(Box::new(move |frame: &VideoFrame<'_>| {
                    let mut cost = shared.lock().unwrap();
                    cost.copy.clear();
                    cost.copy.extend_from_slice(frame.y);
                    cost.copy.extend_from_slice(frame.u);
                    cost.copy.extend_from_slice(frame.v);
                    let now = thread_cpu();
                    if cost.frames == 0 {
                        cost.first = now;
                    }
                    cost.last = now;
                    cost.frames += 1;
                    cost.size = (frame.width, frame.height);
                })))
                .unwrap();
            wait_for("timed frames", Duration::from_secs(30), || {
                cost.lock().unwrap().frames >= 91
            });
            source.set_video_sink(None).unwrap();
            let cost = cost.lock().unwrap();
            eprintln!(
                "{width}x{height}@{fps} -> {}x{}: {:.2?} of the capture thread per frame",
                cost.size.0,
                cost.size.1,
                (cost.last - cost.first) / (cost.frames - 1),
            );
        }
    }
}
