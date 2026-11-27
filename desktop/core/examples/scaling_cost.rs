//! What a self view of a large source costs on its capture thread, with the
//! picture handed on as it is and scaled down to 720p and 360p. A
//! measurement, not a test: it prints and asserts nothing, so it is not
//! part of the test run (GEL-382). Linux only (`/proc/thread-self`); run it
//! in release:
//!
//! `cargo run --release --manifest-path desktop/Cargo.toml -p gelabber-media-core --example scaling_cost`

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gelabber_media_core::{Audio, Engine, Source, VideoFrame, VideoSinkLimits};

/// CPU time this thread ran.
fn thread_cpu() -> Duration {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").unwrap_or_default();
    Duration::from_nanos(
        stat.split_whitespace()
            .next()
            .and_then(|ns| ns.parse().ok())
            .unwrap_or(0),
    )
}

fn limits(max_width: u32, max_height: u32) -> VideoSinkLimits {
    VideoSinkLimits {
        max_width,
        max_height,
        max_fps: 0,
    }
}

/// The capture thread's CPU time between the first and the last frame: the
/// pattern itself, the scaling and the copy a sink makes.
#[derive(Default)]
struct Cost {
    frames: u32,
    size: (u32, u32),
    first: Duration,
    last: Duration,
    copy: Vec<u8>,
}

fn main() {
    let engine = Engine::new(Audio::Dummy).expect("engine");
    for (width, height, fps) in [(3840u32, 2160u32, 30u32), (1920, 1080, 60)] {
        for wanted in [limits(0, 0), limits(1280, 720), limits(640, 360)] {
            let source = Source::test_pattern(&engine, width, height, fps).expect("source");
            source.set_video_sink_limits(wanted).expect("limits");
            let cost = Arc::new(Mutex::new(Cost::default()));
            let shared = cost.clone();
            source
                .set_video_sink(Some(Box::new(move |frame: &VideoFrame<'_>| {
                    let mut cost = shared.lock().expect("cost");
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
                .expect("sink");
            let start = Instant::now();
            while cost.lock().expect("cost").frames < 91 {
                if start.elapsed() > Duration::from_secs(30) {
                    eprintln!("{width}x{height}@{fps}: no 91 frames in 30 s");
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            source.set_video_sink(None).expect("sink");
            let cost = cost.lock().expect("cost");
            if cost.frames > 1 {
                println!(
                    "{width}x{height}@{fps} -> {}x{}: {:.2?} of the capture thread per frame",
                    cost.size.0,
                    cost.size.1,
                    (cost.last - cost.first) / (cost.frames - 1),
                );
            }
        }
    }
}
