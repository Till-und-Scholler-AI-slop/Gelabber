use super::*;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use tauri::async_runtime::block_on;

/// Every name the web client knows.
const KNOWN_FEATURES: &[&str] = &[
    "screen",
    "camera",
    "app-audio",
    "app-audio-excludes-self",
    "video-frames",
    "vp9-svc",
    "h264-hardware",
    "av1-hardware",
    "av1-hardware-decode",
];

/// What the web client reads: the fields of v0.5 stay, and `features` is
/// what this platform's build can do today, in known names. The web
/// client offers and hides functions by this list, so a change to
/// `FEATURES` is repeated here on purpose.
#[test]
fn media_info_reports_the_platform_features() {
    let info = pollster::block_on(media_info());
    assert_eq!(info["abi"], gelabber_media_core::ABI_VERSION);
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(info["platform"], std::env::consts::OS);
    let today: &[&str] = if cfg!(any(target_os = "linux", windows)) {
        &[
            "screen",
            "camera",
            "app-audio",
            "app-audio-excludes-self",
            "video-frames",
            "vp9-svc",
        ]
    } else {
        &[]
    };
    // The hardware codecs depend on the machine, not on the build; they
    // come last: H264's encoder, AV1's encoder, AV1's decoder.
    let mut features: Vec<Value> = info["features"].as_array().unwrap().clone();
    for (codec, side, feature) in [
        ("av1", "hardwareDecoder", "av1-hardware-decode"),
        ("av1", "hardwareEncoder", "av1-hardware"),
        ("h264", "hardwareEncoder", "h264-hardware"),
    ] {
        let hardware = info["videoCodecs"][codec][side] == true;
        assert_eq!(features.last() == Some(&json!(feature)), hardware, "{info}");
        if hardware {
            features.pop();
        }
    }
    assert_eq!(
        features,
        serde_json::json!(today).as_array().unwrap().clone()
    );
    for codec in ["h264", "av1"] {
        if cfg!(any(target_os = "linux", windows)) {
            assert!(info["videoCodecs"][codec].is_object(), "{info}");
        } else {
            assert!(info["videoCodecs"][codec].is_null(), "{info}");
        }
    }
    for feature in FEATURES {
        assert!(
            KNOWN_FEATURES.contains(feature),
            "unknown feature {feature}"
        );
    }
}

#[test]
fn a_view_request_is_whole_pixels_within_reason() {
    let limits = |max_width, max_height, max_fps| VideoSinkLimits {
        max_width,
        max_height,
        max_fps,
    };
    assert_eq!(request(None, None, None), Ok(limits(1280, 720, 0)));
    // A canvas of 1176 CSS pixels at 1.6 device pixels each.
    assert_eq!(
        request(Some(1881.6), Some(1058.4), Some(29.97)),
        Ok(limits(1882, 1059, 30))
    );
    // A canvas that is not laid out yet, and nonsense.
    assert_eq!(
        request(Some(0.0), Some(0.0), Some(0.0)),
        Ok(limits(2, 2, 1))
    );
    assert_eq!(
        request(Some(1e9), Some(4320.0), Some(1e9)),
        Ok(limits(16384, 4320, 240))
    );
    assert!(request(Some(f64::NAN), Some(720.0), None).is_err());
    assert!(request(Some(1280.0), Some(-1.0), None).is_err());
    assert!(request(None, None, Some(f64::INFINITY)).is_err());
}

/// Size and sequence number of the view's next frame.
fn next_frame(frames: &Frames, view: u64, after: Option<u32>) -> ((u32, u32), u32) {
    let (tx, rx) = mpsc::channel();
    let view = frames.view(view).unwrap();
    view.pull(after, Box::new(move |answer| tx.send(answer).unwrap()));
    let packet = rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    let field = |at: usize| u32::from_le_bytes(packet[at..at + 4].try_into().unwrap());
    let (width, height) = (field(8), field(12));
    let chroma = (width.div_ceil(2) * height.div_ceil(2)) as usize;
    let planes = (width * height) as usize + 2 * chroma;
    assert_eq!(packet.len(), crate::frames::HEADER_LEN + planes);
    ((width, height), field(16))
}

/// The whole way from the core to a page: a local source without a
/// producer, its frames scaled by the core to what the views ask for.
#[test]
fn views_of_a_local_source_get_its_frames_at_their_size() {
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 640, 360, 30).unwrap();
    let frames = Frames::default();
    let origin = Origin::Source(1);
    let tap = |source: &Source| {
        let source = source.clone();
        move || Ok(Box::new(SourceTap(source)) as Box<dyn Tap>)
    };
    let small = request(Some(319.5), Some(400.0), None).unwrap();
    let view = frames.open(origin, small, tap(&source)).unwrap();
    let (size, first) = next_frame(&frames, view, None);
    assert_eq!((size, first), ((320, 180), 1));

    // The view grows: frames follow, never beyond the source's size.
    let large = request(Some(4000.0), Some(4000.0), None).unwrap();
    frames.configure(view, large).unwrap();
    let mut last = first;
    loop {
        let (size, seq) = next_frame(&frames, view, Some(last));
        assert!(seq > last);
        last = seq;
        if size == (640, 360) {
            break;
        }
        assert_eq!(size, (320, 180));
    }

    // A second view of the same source shares the sink: the larger
    // request decides the size, its own rate limit holds.
    let slow = request(Some(160.0), Some(90.0), Some(10.0)).unwrap();
    let second = frames.open(origin, slow, tap(&source)).unwrap();
    let (size, mut seen) = next_frame(&frames, second, None);
    assert_eq!(size, (640, 360));
    let started = Instant::now();
    let from = seen;
    while started.elapsed() < Duration::from_secs(1) {
        seen = next_frame(&frames, second, Some(seen)).1;
    }
    assert!((8..=12).contains(&(seen - from)), "{} frames", seen - from);
    // Alone it gets its own size.
    frames.close(view);
    loop {
        let (size, seq) = next_frame(&frames, second, Some(seen));
        seen = seq;
        if size == (160, 90) {
            break;
        }
    }

    // The page closes the source while a producer still holds it: the
    // view ends and the source is free of its sink.
    let producer_holds = source.clone();
    drop(source);
    let (tx, rx) = mpsc::channel();
    let waiting = frames.view(second).unwrap();
    waiting.pull(None, Box::new(move |answer| tx.send(answer).unwrap()));
    frames.close_origin(origin);
    let ended = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    // (The next frame may have won the race against the close.)
    assert!(ended.is_ok() || ended == Err("view closed"));
    assert!(frames.view(second).is_err());
    let (calls_tx, calls) = mpsc::channel();
    producer_holds
        .set_video_sink(Some(Box::new(move |_| {
            let _ = calls_tx.send(());
        })))
        .unwrap();
    calls.recv_timeout(Duration::from_secs(5)).unwrap();

    // An audio source has no picture to show.
    let microphone = Source::microphone(&engine, &json!({})).unwrap();
    let error = frames
        .open(Origin::Source(2), small, tap(&microphone))
        .unwrap_err();
    assert!(error.contains("video source"), "{error}");
}

/// The page's source is gone from the registry by the time a page load
/// ends its views, so a view the old page opens during the load (its
/// command had the source already) is ended and frees the source.
#[test]
fn a_page_load_forgets_a_source_before_it_ends_its_views() {
    /// Says whether the page still holds `origin` when its feed ends.
    struct Witness {
        media: Arc<Media>,
        origin: Origin,
        held: mpsc::Sender<bool>,
    }
    impl Tap for Witness {
        fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
            if sink.is_none() {
                self.held.send(self.media.holds(self.origin)).unwrap();
            }
            Ok(())
        }

        fn set_limits(&mut self, _: VideoSinkLimits) -> Result<()> {
            Ok(())
        }
    }

    let media = Arc::new(Media::default());
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 640, 360, 30).unwrap();
    let handle = held(&media, source);
    let origin = Origin::Source(handle);
    let (held, held_at_the_end) = mpsc::channel();
    let witness = Witness {
        media: media.clone(),
        origin,
        held,
    };
    let tap = || Ok(Box::new(witness) as Box<dyn Tap>);
    let open = media.frames.open(origin, DEFAULT_REQUEST, tap).unwrap();
    // A `media_view_open` of the old page that has looked its source up.
    let page = media.page();
    let (_, in_flight) = media.view_target(None, Some(handle), None, false).unwrap();
    let (in_flight, let_go) = watched(in_flight);

    media.reset();
    assert_eq!(held_at_the_end.try_recv(), Ok(false));
    assert!(media.frames.view(open).is_err());
    assert!(!media.holds(origin));

    // The command goes on after the load: the view it opens is ended
    // again, and nothing keeps the source.
    let late = block_on(open_view(&media, page, origin, DEFAULT_REQUEST, in_flight));
    assert_eq!(late, Err("the stream of the view was closed".into()));
    let_go.try_recv().expect("the feed of the late view ended");
}

/// Registers a source as a command of the present page does.
fn held(media: &Media, source: Source) -> u64 {
    media.insert(media.page(), &media.sources, source).unwrap()
}

/// A camera takes its time to open, and the page is reloaded
/// meanwhile: the source is nobody's when the command gets to register
/// it. It stayed in the registry, capturing, until the next page load.
#[test]
fn what_a_command_makes_for_a_page_that_left_is_freed_not_registered() {
    let media = Media::default();
    let engine = Engine::new(Audio::Dummy).unwrap();
    // The command starts and opens its source.
    let page = media.page();
    let source = Source::test_pattern(&engine, 320, 180, 15).unwrap();
    let (sink, freed_on) = quiet_sink();
    source.set_video_sink(Some(sink)).unwrap();
    media.reset();
    let late = media.insert(page, &media.sources, source);
    assert_eq!(late, Err(PAGE_GONE.into()));
    assert!(media.sources.lock().unwrap().is_empty());
    // A sink is freed with its source, here off the command's thread.
    let freed_on = freed_on.recv_timeout(LIMIT).expect("the source is freed");
    assert_ne!(freed_on, std::thread::current().id());

    // A command of the page that is there now registers as ever.
    let source = Source::test_pattern(&engine, 320, 180, 15).unwrap();
    let handle = held(&media, source);
    assert!(media.holds(Origin::Source(handle)));
}

/// A view of the test pattern opens while the page is replaced. The
/// pattern is nobody's to close, so no registry tells the view that its
/// page left: the page count does.
#[test]
fn a_pattern_view_opened_for_a_page_that_left_is_ended() {
    let media = Media::default();
    let origin = Origin::Pattern(320, 180, 30);
    // In place of the pattern, whose thread the test in frames.rs counts.
    let pattern = Noted::default();
    let page = media.page();
    media.reset();
    let late = open_view(&media, page, origin, DEFAULT_REQUEST, pattern.tap());
    assert_eq!(block_on(late), Err(PAGE_GONE.into()));
    assert_eq!(pattern.sinks(), [true, false]);

    // The page that is there now gets its view.
    let pattern = Noted::default();
    let page = media.page();
    let opened = open_view(&media, page, origin, DEFAULT_REQUEST, pattern.tap());
    let view = block_on(opened).unwrap();
    assert!(media.frames.view(view).is_ok());
    assert_eq!(pattern.sinks(), [true]);
}

/// The microphone test is the engine's, not an object with a handle:
/// a start that a command of the old page gets to after the page load
/// would run with no page to end it.
#[test]
fn the_microphone_test_is_not_switched_for_a_page_that_left() {
    let media = Media::default();
    let switched = std::cell::Cell::new(0);
    let switch = |page| {
        switch_monitor(&media.page, &media.monitor, page, || {
            switched.set(switched.get() + 1);
            Ok(())
        })
    };
    let page = media.page();
    assert_eq!(switch(page), Ok(()));
    assert_eq!(switched.get(), 1);
    media.reset();
    assert_eq!(switch(page), Err(PAGE_GONE.into()));
    assert_eq!(switched.get(), 1);
    assert_eq!(switch(media.page()), Ok(()));
    assert_eq!(switched.get(), 2);
}

/// A page load ends the microphone test after a switch that is under
/// way: a start that was let through before the load must not come out
/// behind the load's stop.
#[test]
fn a_page_load_ends_the_microphone_test_after_a_switch_under_way() {
    let media = Arc::new(Media::default());
    // The page load only ends a test of an engine that exists.
    let engine = Engine::new(Audio::Dummy).unwrap();
    assert!(media.engine.set(engine).is_ok());
    let (begun, in_the_switch) = mpsc::channel();
    let (go_on, waits) = mpsc::channel::<()>();
    let (switching, page) = (media.clone(), media.page());
    let switch = std::thread::spawn(move || {
        switch_monitor(&switching.page, &switching.monitor, page, || {
            begun.send(()).unwrap();
            waits.recv().map_err(err)
        })
    });
    in_the_switch.recv_timeout(LIMIT).unwrap();
    let (loaded, load) = mpsc::channel();
    let loading = media.clone();
    std::thread::spawn(move || {
        loading.reset();
        let _ = loaded.send(());
    });
    // The load waits for the switch: not being through is no timing
    // matter, only being through would be.
    assert!(load.recv_timeout(Duration::from_millis(300)).is_err());
    go_on.send(()).unwrap();
    assert_eq!(switch.join().unwrap(), Ok(()));
    load.recv_timeout(LIMIT).expect("the page load goes on");
}

/// Reports the thread it is dropped on.
struct Dropped(mpsc::Sender<std::thread::ThreadId>);

impl Drop for Dropped {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}

/// A sink that takes frames and does nothing, and the word when it is
/// dropped: with the window it fed, or with the source it was set on.
fn quiet_sink() -> (VideoSink, mpsc::Receiver<std::thread::ThreadId>) {
    let (dropped, gone) = mpsc::channel();
    let dropped = Dropped(dropped);
    let sink = move |_: &gelabber_media_core::VideoFrame<'_>| {
        let _ = &dropped;
    };
    (Box::new(sink), gone)
}

/// A feed's tap that says when the feed lets go of it, and with it of
/// the consumer or source behind it.
struct Watched {
    tap: Box<dyn Tap>,
    _dropped: Dropped,
}

impl Tap for Watched {
    fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
        self.tap.set_sink(sink)
    }

    fn set_limits(&mut self, limits: VideoSinkLimits) -> Result<()> {
        self.tap.set_limits(limits)
    }
}

fn watched(make: MakeTap) -> (MakeTap, mpsc::Receiver<std::thread::ThreadId>) {
    let (dropped, let_go) = mpsc::channel();
    let _dropped = Dropped(dropped);
    let make = move || {
        Ok(Box::new(Watched {
            tap: make()?,
            _dropped,
        }) as Box<dyn Tap>)
    };
    (Box::new(make), let_go)
}

/// A tap in place of a consumer's: notes what the feed asks of it.
#[derive(Clone, Default)]
struct Noted(Arc<Mutex<Asked>>);

#[derive(Default)]
struct Asked {
    /// A sink set (true) or taken off (false), in order.
    sinks: Vec<bool>,
    limits: Vec<VideoSinkLimits>,
}

impl Tap for Noted {
    fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
        self.0.lock().unwrap().sinks.push(sink.is_some());
        Ok(())
    }

    fn set_limits(&mut self, limits: VideoSinkLimits) -> Result<()> {
        self.0.lock().unwrap().limits.push(limits);
        Ok(())
    }
}

impl Noted {
    fn tap(&self) -> MakeTap {
        let tap = self.clone();
        Box::new(move || Ok(Box::new(tap) as Box<dyn Tap>))
    }

    fn sinks(&self) -> Vec<bool> {
        self.0.lock().unwrap().sinks.clone()
    }

    fn limits(&self) -> VideoSinkLimits {
        *self.0.lock().unwrap().limits.last().unwrap()
    }
}

/// A consumer or a source of the page, exactly one; a test pattern only
/// in an app that was started for tests.
#[test]
fn a_view_shows_a_consumer_or_a_source_and_a_pattern_only_in_tests() {
    let media = Media::default();
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 640, 360, 30).unwrap();
    let source = held(&media, source);
    let pattern = || {
        Some(PatternOptions {
            width: 640,
            height: 360,
            fps: 30,
        })
    };
    let target = |consumer, source, pattern, patterns| {
        let target = media.view_target(consumer, source, pattern, patterns);
        target.map(|(origin, _)| origin)
    };
    let neither = Err("a view shows either a consumer or a source".to_string());
    assert_eq!(
        target(None, Some(source), None, false),
        Ok(Origin::Source(source))
    );
    assert_eq!(
        target(None, None, pattern(), true),
        Ok(Origin::Pattern(640, 360, 30))
    );
    assert_eq!(target(None, None, pattern(), false), neither);
    assert_eq!(target(None, None, None, true), neither);
    assert_eq!(target(Some(1), Some(source), None, false), neither);
    assert_eq!(target(None, Some(source), pattern(), true), neither);
    assert_eq!(
        target(Some(987654), None, None, false),
        Err("unknown consumer 987654".into())
    );
    assert_eq!(
        target(None, Some(987654), None, false),
        Err("unknown source 987654".into())
    );
}

/// An app started without the variable refuses a test pattern; nothing
/// in `cargo test` sets it (smoke.sh and smoke.ps1 do, for the built app).
#[test]
fn without_the_variable_a_test_pattern_view_is_refused() {
    assert_eq!(std::env::var_os(TEST_PATTERN_ENV), None);
    assert!(!test_patterns_allowed());
    let media = Media::default();
    let pattern = PatternOptions {
        width: 640,
        height: 360,
        fps: 30,
    };
    let target = media.view_target(None, None, Some(pattern), test_patterns_allowed());
    assert_eq!(
        target.map(|(origin, _)| origin),
        Err("a view shows either a consumer or a source".to_string())
    );
}

/// The page closes a source while a view shows it: the view ends, its
/// waiting frame request fails, and nothing keeps the source capturing.
#[test]
fn closing_a_source_ends_the_views_of_it() {
    let media = Media::default();
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 640, 360, 30).unwrap();
    let handle = held(&media, source);
    let (origin, tap) = media.view_target(None, Some(handle), None, false).unwrap();
    let (tap, let_go) = watched(tap);
    let page = media.page();
    let view = block_on(open_view(&media, page, origin, DEFAULT_REQUEST, tap)).unwrap();
    let (_, seen) = next_frame(&media.frames, view, None);
    let (answer, waiting) = mpsc::channel();
    let pulled = media.frames.view(view).unwrap();
    pulled.pull(
        Some(seen),
        Box::new(move |frame| answer.send(frame).unwrap()),
    );

    media.close_source(handle)();
    assert!(!media.holds(origin));
    assert!(media.frames.view(view).is_err());
    // (The next frame may have won the race against the close.)
    let ended = waiting.recv_timeout(LIMIT).unwrap();
    assert!(ended.is_ok() || ended == Err("view closed"));
    let_go.try_recv().expect("the feed of the view ended");
    // Closing it again, or a source that never was, is fine.
    media.close_source(handle)();
    media.close_source(987654)();
}

/// The page closes a source in the moment a view of it opens: the
/// command had looked the source up, so its feed would keep the source
/// capturing with nothing left to end it.
#[test]
fn a_view_of_a_source_closed_while_it_opened_is_ended() {
    let media = Media::default();
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 640, 360, 30).unwrap();
    let handle = held(&media, source);
    let (origin, tap) = media.view_target(None, Some(handle), None, false).unwrap();
    let (tap, let_go) = watched(tap);
    media.close_source(handle)();
    let opened = block_on(open_view(
        &media,
        media.page(),
        origin,
        DEFAULT_REQUEST,
        tap,
    ));
    assert_eq!(opened, Err("the stream of the view was closed".into()));
    let_go.try_recv().expect("the feed of the view ended");
}

/// The page closes a consumer that it shows in the page and in a viewer
/// window: both end.
#[test]
fn closing_a_consumer_ends_its_views_and_its_window() {
    let media = Media::default();
    let consumer = Noted::default();
    let origin = Origin::Consumer(7);
    let request = DEFAULT_REQUEST;
    let view = media.frames.open(origin, request, consumer.tap()).unwrap();
    let (sink, window_gone) = quiet_sink();
    let window = media.frames.set_window(origin, sink, consumer.tap());
    window.unwrap();
    assert_eq!(consumer.sinks(), [true]);

    media.close_consumer(7)();
    assert!(media.frames.view(view).is_err());
    assert_eq!(consumer.sinks(), [true, false]);
    window_gone
        .try_recv()
        .expect("the window's sink is dropped");
}

/// A viewer window closes: the consumer's views in the page stay and
/// get the stream at their own size again, not the window's.
#[test]
fn closing_a_viewer_window_leaves_the_views_in_the_page() {
    let media = Media::default();
    let consumer = Noted::default();
    let origin = Origin::Consumer(7);
    let small = request(Some(320.0), Some(180.0), Some(15.0)).unwrap();
    let view = media.frames.open(origin, small, consumer.tap()).unwrap();
    let (sink, window_gone) = quiet_sink();
    let window = media.frames.set_window(origin, sink, consumer.tap());
    window.unwrap();
    // The window shows the stream as it is.
    assert_eq!(consumer.limits(), VideoSinkLimits::default());

    media.close_viewer(7)();
    window_gone
        .try_recv()
        .expect("the window's sink is dropped");
    assert_eq!(consumer.limits(), small);
    assert!(media.frames.view(view).is_ok());
    assert_eq!(consumer.sinks(), [true]);
    media.close_viewer(7)();
}

/// A viewer window opens for a consumer the page closed in the same
/// moment: its feed ends again, for nothing else would end it.
#[test]
fn a_window_of_a_consumer_closed_while_it_opened_is_ended() {
    let media = Media::default();
    let consumer = Noted::default();
    let (sink, window_gone) = quiet_sink();
    let opened = block_on(open_window(&media, 7, sink, consumer.tap()));
    assert_eq!(opened, Err("unknown consumer 7".into()));
    assert_eq!(consumer.sinks(), [true, false]);
    window_gone
        .try_recv()
        .expect("the window's sink is dropped");
}

/// How long a test waits for something that has to happen.
const LIMIT: Duration = Duration::from_secs(10);

/// Runs `job` on a thread of its own and gives up after [`LIMIT`]: what a
/// page load or a closing transport does must not wait for the page.
fn within<T: Send + 'static>(what: &str, job: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(job());
    });
    result
        .recv_timeout(LIMIT)
        .unwrap_or_else(|_| panic!("{what} did not return"))
}

/// What a router offers, enough to load a device without a server.
fn router_capabilities() -> Value {
    let feedback = json!([
        {"type": "nack", "parameter": ""},
        {"type": "nack", "parameter": "pli"},
        {"type": "ccm", "parameter": "fir"},
        {"type": "goog-remb", "parameter": ""},
        {"type": "transport-cc", "parameter": ""},
    ]);
    let extension = |kind: &str, uri: &str, id: u32| {
        json!({
            "kind": kind,
            "uri": uri,
            "preferredId": id,
            "preferredEncrypt": false,
            "direction": "sendrecv",
        })
    };
    let mid = "urn:ietf:params:rtp-hdrext:sdes:mid";
    let send_time = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
    json!({
        "codecs": [
            {
                "kind": "audio",
                "mimeType": "audio/opus",
                "clockRate": 48000,
                "channels": 2,
                "preferredPayloadType": 100,
                "parameters": {},
                "rtcpFeedback": [
                    {"type": "nack", "parameter": ""},
                    {"type": "transport-cc", "parameter": ""},
                ],
            },
            {
                "kind": "video",
                "mimeType": "video/VP8",
                "clockRate": 90000,
                "preferredPayloadType": 101,
                "parameters": {},
                "rtcpFeedback": feedback,
            },
            {
                "kind": "video",
                "mimeType": "video/rtx",
                "clockRate": 90000,
                "preferredPayloadType": 102,
                "parameters": {"apt": 101},
                "rtcpFeedback": [],
            },
        ],
        "headerExtensions": [
            extension("audio", mid, 1),
            extension("video", mid, 1),
            extension("audio", send_time, 4),
            extension("video", send_time, 4),
        ],
    })
}

/// A transport no server stands behind, without an address to try.
fn transport_options() -> Value {
    json!({
        "id": "11111111-1111-4111-8111-111111111111",
        "iceParameters": {
            "usernameFragment": "abcdabcdabcdabcd",
            "password": "abcdefghijklmnopqrstuvwxyz012345",
            "iceLite": true,
        },
        "iceCandidates": [],
        "dtlsParameters": {
            "role": "auto",
            "fingerprints": [{
                "algorithm": "sha-256",
                "value": "82:5A:68:3D:36:C3:0A:DE:AF:E7:32:43:D2:88:83:57:\
                          AC:2D:65:E5:80:C4:B6:FB:AF:1A:A0:21:9F:6D:0C:AD",
            }],
        },
    })
}

/// What the server announces for somebody's microphone.
fn consumer_announcement(number: u32) -> Value {
    json!({
        "id": format!("33333333-3333-4333-8333-{number:012}"),
        "producerId": format!("44444444-4444-4444-8444-{number:012}"),
        "kind": "audio",
        "rtpParameters": {
            "mid": number.to_string(),
            "codecs": [{
                "mimeType": "audio/opus",
                "payloadType": 100,
                "clockRate": 48000,
                "channels": 2,
                "parameters": {},
                "rtcpFeedback": [],
            }],
            "headerExtensions": [],
            "encodings": [{"ssrc": 1000 + number}],
            "rtcp": {"cname": "gelabber", "reducedSize": true, "mux": true},
        },
    })
}

/// A transport of a page in a call without a server. Its events arrive
/// here where the page's channel would get them; the test plays the page.
struct Call {
    media: Arc<Media>,
    engine: Engine,
    transport: u64,
    events: mpsc::Receiver<Value>,
}

impl Call {
    fn new(direction: &str) -> Self {
        Self::with_page(direction, |_, _| {})
    }

    /// `page` sees every event as the page would, on the events thread
    /// and before the next event is looked at.
    fn with_page(direction: &str, page: impl Fn(&Media, &Value) + Send + 'static) -> Self {
        let media = Arc::new(Media::default());
        let engine = Engine::new(Audio::Dummy).unwrap();
        let device = Device::new(&engine).unwrap();
        device.load(&router_capabilities()).unwrap();
        let device = media.insert(media.page(), &media.devices, device).unwrap();
        let (to_test, events) = mpsc::channel();
        let seen_by = media.clone();
        let send = move |message: Value| {
            page(&seen_by, &message);
            to_test.send(message).is_ok()
        };
        let (transport, id) = media
            .open_transport(media.page(), device, direction, &transport_options(), send)
            .unwrap();
        assert_eq!(id, "11111111-1111-4111-8111-111111111111");
        Self {
            media,
            engine,
            transport,
            events,
        }
    }

    fn transport(&self) -> Arc<PageTransport> {
        get(&self.media.transports, self.transport, "transport").unwrap()
    }

    /// The transport's next request to the page: its type and number.
    fn request(&self) -> (String, u64) {
        loop {
            let event = self.events.recv_timeout(LIMIT).expect("a request");
            if let Some(request) = event["request"].as_u64() {
                return (event["type"].as_str().unwrap().to_owned(), request);
            }
        }
    }

    /// The page's `media_transport_respond`.
    fn answer(&self, request: u64, result: Value) {
        self.transport().answer(request, Ok(result)).unwrap();
    }

    fn produce(&self, source: &Source) -> mpsc::Receiver<Result<Producer>> {
        produce(self.transport(), source)
    }

    /// What `media_consume` runs on the blocking pool.
    fn consume(&self, number: u32) -> mpsc::Receiver<Result<Consumer>> {
        let transport = self.transport();
        let (done, outcome) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(transport.consume(&consumer_announcement(number)));
        });
        outcome
    }

    /// Whether the transport is freed, which ends its events. Nothing
    /// holds it any more then: no producer, no consumer, no call.
    fn freed(&self) -> bool {
        let deadline = Instant::now() + LIMIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return true,
                Err(mpsc::RecvTimeoutError::Timeout) => return false,
            }
        }
    }
}

/// What `media_produce` runs on the blocking pool, on the transport it
/// looked up; the outcome arrives when the call ends.
fn produce(transport: Arc<PageTransport>, source: &Source) -> mpsc::Receiver<Result<Producer>> {
    let source = source.clone();
    let (done, outcome) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(transport.produce(&source, &json!({})));
    });
    outcome
}

/// The error a call ended with.
fn failure<T>(outcome: &mpsc::Receiver<Result<T>>, what: &str) -> String {
    match outcome.recv_timeout(LIMIT) {
        Ok(Err(error)) => error,
        Ok(Ok(_)) => panic!("{what} succeeded"),
        Err(_) => panic!("{what} still waits for the page"),
    }
}

/// In a call, the page starts another producer and is reloaded before it
/// answered the `produce` request. The call in the core waits for that
/// answer with the transport's lock, which closing the microphone's
/// producer needs: the page load froze the app for good.
#[test]
fn a_page_load_ends_a_produce_that_waits_for_the_page() {
    let call = Call::new("send");
    let microphone = Source::microphone(&call.engine, &json!({})).unwrap();
    let produced = call.produce(&microphone);
    let (kind, request) = call.request();
    assert_eq!(kind, "connect");
    call.answer(request, json!({}));
    let (kind, request) = call.request();
    assert_eq!(kind, "produce");
    call.answer(
        request,
        json!({"id": "22222222-2222-4222-8222-222222222222"}),
    );
    let producer = produced.recv_timeout(LIMIT).unwrap().unwrap();
    assert_eq!(producer.id(), "22222222-2222-4222-8222-222222222222");
    let producer = Arc::new(Mutex::new(producer));
    let registered = call
        .media
        .insert(call.media.page(), &call.media.producers, producer);
    registered.unwrap();

    let camera = Source::test_pattern(&call.engine, 320, 180, 15).unwrap();
    let pending = call.produce(&camera);
    assert_eq!(call.request().0, "produce");
    let media = call.media.clone();
    within("the page load", move || media.reset());
    let error = failure(&pending, "the produce");
    assert!(error.contains(PAGE_GONE), "{error}");
    // The microphone's producer could close.
    assert!(call.freed());
    assert!(call.media.producers.lock().unwrap().is_empty());
    assert!(call.media.transports.lock().unwrap().is_empty());
}

/// The page answered `connect` and was replaced before the `produce`
/// request that follows reached it: that one was not open at the page
/// load, and still nobody is left to answer it.
#[test]
fn a_request_after_the_page_load_is_refused() {
    let call = Call::with_page("send", |media, event| {
        if event["type"] == "connect" {
            let transports = media.transports.lock().unwrap();
            let transport = transports.values().next().cloned().unwrap();
            drop(transports);
            let request = event["request"].as_u64().unwrap();
            transport.answer(request, Ok(json!({}))).unwrap();
            media.reset();
        }
    });
    let camera = Source::test_pattern(&call.engine, 320, 180, 15).unwrap();
    let pending = call.produce(&camera);
    let error = failure(&pending, "the produce");
    assert!(error.contains(PAGE_GONE), "{error}");
    assert!(call.freed());
}

/// Joining a call with several people in it: the consumers are made at
/// once. The first waits for the page's answer to `connect`, the others
/// for the transport's lock, and each of them asks to connect again when
/// its turn comes, because the transport still is not connected.
#[test]
fn a_page_load_ends_the_calls_queued_behind_one_that_waits() {
    let call = Call::new("recv");
    let first = call.consume(1);
    assert_eq!(call.request().0, "connect");
    let queued = [call.consume(2), call.consume(3)];
    let media = call.media.clone();
    within("the page load", move || media.reset());
    failure(&first, "the first consume");
    for consume in &queued {
        failure(consume, "a queued consume");
    }
    assert!(call.freed());
}

/// The page closes a transport while a produce on it waits for the
/// page: `media_transport_respond` no longer finds the transport, so
/// the page's answer would never arrive.
#[test]
fn closing_a_transport_ends_a_produce_that_waits_for_the_page() {
    let call = Call::new("send");
    let camera = Source::test_pattern(&call.engine, 320, 180, 15).unwrap();
    let pending = call.produce(&camera);
    assert_eq!(call.request().0, "connect");
    // Another produce of the page has looked the transport up.
    let looked_up = call.transport();
    let media = call.media.clone();
    let transport = call.transport;
    within("closing the transport", move || {
        media.close_transport(transport)
    });
    failure(&pending, "the produce");
    // It asks to connect after the close.
    failure(&produce(looked_up, &camera), "a produce after the close");
    assert!(call.freed());
}

/// A page load frees what the page left on another thread than its own,
/// the UI's: closing in the core may take a while.
#[test]
fn a_page_load_frees_the_leftovers_off_its_thread() {
    let media = Media::default();
    let engine = Engine::new(Audio::Dummy).unwrap();
    let source = Source::test_pattern(&engine, 320, 180, 15).unwrap();
    // A sink is freed with its source.
    let (sink, freed_on) = quiet_sink();
    source.set_video_sink(Some(sink)).unwrap();
    held(&media, source);
    media.reset();
    let freed_on = freed_on.recv_timeout(LIMIT).expect("the source is freed");
    assert_ne!(freed_on, std::thread::current().id());
}

/// The server origin's permission set grants exactly the media commands.
#[test]
fn media_permission_set_matches_the_commands() {
    let set = include_str!("../../permissions/media.toml");
    let granted: Vec<&str> = set
        .lines()
        .filter_map(|line| line.trim().strip_prefix("\"allow-"))
        .filter_map(|rest| rest.strip_suffix("\","))
        .collect();
    let expected: Vec<String> = MEDIA_COMMANDS.iter().map(|c| c.replace('_', "-")).collect();
    assert_eq!(granted, expected);
    let main = include_str!("../main.rs");
    for command in MEDIA_COMMANDS {
        assert!(
            main.contains(&format!("media::{command},")),
            "{command} missing from generate_handler!"
        );
    }
}
