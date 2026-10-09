//! Tauri commands over the native media core, for the server's web client.
//!
//! The web client keeps all signaling (media WebSocket, tickets, ACL, source
//! epochs) and drives these commands the way it drives `mediasoup-client`:
//! load a device, create transports, answer their `connect`/`produce`
//! requests through the server, produce local sources, consume announced
//! producers. Objects are handles (numbers) into a per-app registry; a page
//! load drops every object the previous page left behind.
use crate::frames::{DEFAULT_REQUEST, Frames, Origin, Tap, TestPattern};
use crate::viewer::{Viewer, ViewerEvent};
use gelabber_media_core::{
    Audio, Consumer, Device, Direction, Engine, MediaKind, Producer, Source, Transport,
    TransportEvent, VideoSink, VideoSinkLimits,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tauri::{
    State,
    async_runtime::{channel, spawn_blocking},
    ipc::{Channel, Response},
};

#[cfg(test)]
include!("commands.rs");

type Shared<T> = Arc<Mutex<T>>;

#[derive(Default)]
pub struct Media {
    engine: OnceLock<Engine>,
    next: AtomicU64,
    devices: Mutex<HashMap<u64, Device>>,
    transports: Mutex<HashMap<u64, Transport>>,
    sources: Mutex<HashMap<u64, Source>>,
    producers: Mutex<HashMap<u64, Shared<Producer>>>,
    consumers: Mutex<HashMap<u64, Shared<Consumer>>>,
    /// Video shown in the page and in viewer windows (frames.rs).
    frames: Frames,
}

type Result<T> = std::result::Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn get<T: Clone>(map: &Mutex<HashMap<u64, T>>, handle: u64, what: &str) -> Result<T> {
    map.lock()
        .unwrap()
        .get(&handle)
        .cloned()
        .ok_or_else(|| format!("unknown {what} {handle}"))
}

impl Media {
    fn engine(&self) -> Result<&Engine> {
        if let Some(engine) = self.engine.get() {
            return Ok(engine);
        }
        // GELABBER_AUDIO=dummy: no audio devices at all, for smoke tests on
        // machines without audio endpoints (CI runners).
        let audio = match std::env::var("GELABBER_AUDIO").as_deref() {
            Ok("dummy") => Audio::Dummy,
            _ => Audio::Default,
        };
        let engine = Engine::new(audio).map_err(err)?;
        // A racing first call loses its engine; both are equivalent.
        Ok(self.engine.get_or_init(|| engine))
    }

    fn insert<T>(&self, map: &Mutex<HashMap<u64, T>>, value: T) -> u64 {
        let handle = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        map.lock().unwrap().insert(handle, value);
        handle
    }

    /// Whether the page still holds the consumer or source a feed shows.
    fn holds(&self, origin: Origin) -> bool {
        match origin {
            Origin::Consumer(handle) => self.consumers.lock().unwrap().contains_key(&handle),
            Origin::Source(handle) => self.sources.lock().unwrap().contains_key(&handle),
            Origin::Pattern(..) => true,
        }
    }

    /// Drops every object of a page that went away. The registries are
    /// emptied before anything is freed: the old page lives on until the
    /// navigation commits, and a view it opens meanwhile has to find its
    /// consumer or source gone (`closed_meanwhile`), or a camera would go on
    /// capturing with no handle left to close it. Then the views end, which
    /// hold the sinks of consumers and sources; then producers and consumers,
    /// which close on their transports.
    pub fn reset(&self) {
        fn take<T>(map: &Mutex<HashMap<u64, T>>) -> HashMap<u64, T> {
            std::mem::take(&mut *map.lock().unwrap())
        }
        let consumers = take(&self.consumers);
        let producers = take(&self.producers);
        let sources = take(&self.sources);
        let transports = take(&self.transports);
        let devices = take(&self.devices);
        self.frames.reset();
        drop(consumers);
        drop(producers);
        drop(sources);
        drop(transports);
        drop(devices);
        if let Some(viewer) = Viewer::running() {
            viewer.close_all();
        }
        if let Some(engine) = self.engine.get() {
            // A microphone test the page left open.
            let _ = engine.monitor_audio(None);
        }
    }
}

/// Runs a native call that may block (produce/consume wait for the server
/// round trips the page answers) off the async runtime.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    spawn_blocking(job).await.map_err(err)?
}

/// What this build can do besides voice and watching streams
/// (`media_info.features`). For a capture that is missing the web client
/// keeps the button in its place, greyed out, and says why; only the
/// controls for application sound are hidden. The names are a contract with
/// it (`KNOWN_FEATURES` in the tests):
/// - "screen": screen and window capture (`media_source_screen`)
/// - "camera": `media_video_devices`, `media_source_camera`
/// - "app-audio": sound of other applications (`media_audio_apps`,
///   `media_source_app_audio`)
/// - "app-audio-excludes-self": sharing every application's sound leaves
///   out this app's own, so a call is not sent back into itself
/// - "video-frames": decoded frames for the page to draw inside the app
///   window
///
/// The only place that decides the list. An app before v0.6 reports none.
const FEATURES: &[&str] = if cfg!(target_os = "linux") {
    &[
        "screen",
        "camera",
        "app-audio",
        "app-audio-excludes-self",
        "video-frames",
    ]
} else if cfg!(windows) {
    &["camera", "video-frames"]
} else {
    &[]
};

#[tauri::command]
pub async fn media_info() -> Value {
    json!({
        "abi": gelabber_media_core::ABI_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "features": FEATURES,
    })
}

#[tauri::command]
pub async fn media_audio_devices(media: State<'_, Media>) -> Result<Value> {
    media.engine()?.audio_devices().map_err(err)
}

#[tauri::command]
pub async fn media_audio_configure(media: State<'_, Media>, options: Value) -> Result<()> {
    media.engine()?.configure_audio(&options).map_err(err)
}

#[tauri::command]
pub async fn media_audio_levels(media: State<'_, Media>) -> Result<Value> {
    media.engine()?.audio_levels().map_err(err)
}

/// The microphone test: `options` (`{"processingMode","inputGain"}`) starts
/// it, `null` ends it.
#[tauri::command]
pub async fn media_audio_monitor(media: State<'_, Media>, options: Option<Value>) -> Result<()> {
    let engine = media.engine()?.clone();
    blocking(move || engine.monitor_audio(options.as_ref()).map_err(err)).await
}

#[tauri::command]
pub async fn media_device_load(media: State<'_, Media>, capabilities: Value) -> Result<Value> {
    let device = Device::new(media.engine()?).map_err(err)?;
    device.load(&capabilities).map_err(err)?;
    let mut result = json!({
        "rtpCapabilities": device.rtp_capabilities().map_err(err)?,
        "canProduce": {
            "audio": device.can_produce(MediaKind::Audio).map_err(err)?,
            "video": device.can_produce(MediaKind::Video).map_err(err)?,
        },
    });
    result["device"] = media.insert(&media.devices, device).into();
    Ok(result)
}

#[tauri::command]
pub async fn media_device_close(media: State<'_, Media>, device: u64) -> Result<()> {
    media.devices.lock().unwrap().remove(&device);
    Ok(())
}

/// `events` receives `{"type":"connect","request","dtlsParameters"}`,
/// `{"type":"produce","request","kind","rtpParameters","appData"}` and
/// `{"type":"connectionstatechange","state"}`; answer the first two with
/// `media_transport_respond`.
#[tauri::command]
pub async fn media_transport_create(
    media: State<'_, Media>,
    device: u64,
    direction: String,
    options: Value,
    events: Channel<Value>,
) -> Result<Value> {
    let device = get(&media.devices, device, "device")?;
    let direction = match direction.as_str() {
        "send" => Direction::Send,
        "recv" => Direction::Recv,
        other => return Err(format!("unknown direction {other}")),
    };
    let (transport, receiver) = Transport::new(&device, direction, &options).map_err(err)?;
    let id = transport.id().to_owned();
    std::thread::Builder::new()
        .name("gelabber-transport-events".into())
        .spawn(move || {
            // Ends when the transport is freed and drops its sender.
            for event in receiver {
                let message = match event {
                    TransportEvent::Connect {
                        request,
                        dtls_parameters,
                    } => json!({
                        "type": "connect",
                        "request": request,
                        "dtlsParameters": dtls_parameters,
                    }),
                    TransportEvent::Produce {
                        request,
                        kind,
                        rtp_parameters,
                        app_data,
                    } => json!({
                        "type": "produce",
                        "request": request,
                        "kind": kind,
                        "rtpParameters": rtp_parameters,
                        "appData": app_data,
                    }),
                    TransportEvent::ConnectionState(state) => json!({
                        "type": "connectionstatechange",
                        "state": state,
                    }),
                };
                if events.send(message).is_err() {
                    break;
                }
            }
        })
        .map_err(err)?;
    let handle = media.insert(&media.transports, transport);
    Ok(json!({ "transport": handle, "id": id }))
}

/// Exactly one of `result`/`error`.
#[tauri::command]
pub async fn media_transport_respond(
    media: State<'_, Media>,
    transport: u64,
    request: u64,
    result: Option<Value>,
    error: Option<String>,
) -> Result<()> {
    let transport = get(&media.transports, transport, "transport")?;
    let answer = match (result, error) {
        (Some(value), None) => Ok(value),
        (None, Some(message)) => Err(message),
        _ => return Err("respond with either result or error".into()),
    };
    transport.respond(request, answer).map_err(err)
}

#[tauri::command]
pub async fn media_transport_restart_ice(
    media: State<'_, Media>,
    transport: u64,
    ice_parameters: Value,
) -> Result<()> {
    let transport = get(&media.transports, transport, "transport")?;
    blocking(move || transport.restart_ice(&ice_parameters).map_err(err)).await
}

#[tauri::command]
pub async fn media_transport_stats(media: State<'_, Media>, transport: u64) -> Result<Value> {
    let transport = get(&media.transports, transport, "transport")?;
    blocking(move || transport.stats().map_err(err)).await
}

#[tauri::command]
pub async fn media_transport_close(media: State<'_, Media>, transport: u64) -> Result<()> {
    let removed = media.transports.lock().unwrap().remove(&transport);
    // Producers and consumers keep the transport alive until they close.
    drop(removed);
    Ok(())
}

#[tauri::command]
pub async fn media_source_microphone(media: State<'_, Media>, options: Value) -> Result<u64> {
    let source = Source::microphone(media.engine()?, &options).map_err(err)?;
    Ok(media.insert(&media.sources, source))
}

/// Opens the desktop's own screen/window picker; poll `media_source_state`.
#[tauri::command]
pub async fn media_source_screen(media: State<'_, Media>, options: Value) -> Result<u64> {
    let source = Source::screen(media.engine()?, &options).map_err(err)?;
    Ok(media.insert(&media.sources, source))
}

/// Applications playing sound: `[{"id","name","streams"}]`.
#[tauri::command]
pub async fn media_audio_apps(media: State<'_, Media>) -> Result<Value> {
    let engine = media.engine()?.clone();
    blocking(move || engine.audio_apps().map_err(err)).await
}

/// Source audio: `{"app"?}`; without an id every application but this one.
#[tauri::command]
pub async fn media_source_app_audio(media: State<'_, Media>, options: Value) -> Result<u64> {
    let engine = media.engine()?.clone();
    let source = blocking(move || Source::app_audio(&engine, &options).map_err(err)).await?;
    Ok(media.insert(&media.sources, source))
}

/// Cameras: `[{"id","name"}]`.
#[tauri::command]
pub async fn media_video_devices(media: State<'_, Media>) -> Result<Value> {
    let engine = media.engine()?.clone();
    blocking(move || engine.video_devices().map_err(err)).await
}

/// `{"device"?, "width"?, "height"?, "fps"?}`; fails when the camera is
/// missing or busy.
#[tauri::command]
pub async fn media_source_camera(media: State<'_, Media>, options: Value) -> Result<u64> {
    let engine = media.engine()?.clone();
    let source = blocking(move || Source::camera(&engine, &options).map_err(err)).await?;
    Ok(media.insert(&media.sources, source))
}

#[tauri::command]
pub async fn media_source_state(media: State<'_, Media>, source: u64) -> Result<Value> {
    get(&media.sources, source, "source")?.state().map_err(err)
}

#[tauri::command]
pub async fn media_source_set_enabled(
    media: State<'_, Media>,
    source: u64,
    enabled: bool,
) -> Result<()> {
    get(&media.sources, source, "source")?
        .set_enabled(enabled)
        .map_err(err)
}

/// Producers keep their source running until they close too; the page's
/// views of it end here.
#[tauri::command]
pub async fn media_source_close(media: State<'_, Media>, source: u64) -> Result<()> {
    // Out of the registry first: a view opening right now then finds the
    // source gone, or is closed with the others.
    let removed = media.sources.lock().unwrap().remove(&source);
    let frames = media.frames.clone();
    blocking(move || {
        frames.close_origin(Origin::Source(source));
        drop(removed);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn media_produce(
    media: State<'_, Media>,
    transport: u64,
    source: u64,
    options: Value,
) -> Result<Value> {
    let transport = get(&media.transports, transport, "transport")?;
    let source = get(&media.sources, source, "source")?;
    let producer = blocking(move || transport.produce(&source, &options).map_err(err)).await?;
    let mut result = json!({
        "id": producer.id(),
        "rtpParameters": producer.rtp_parameters().map_err(err)?,
    });
    result["producer"] = media
        .insert(&media.producers, Arc::new(Mutex::new(producer)))
        .into();
    Ok(result)
}

fn producer(media: &Media, handle: u64) -> Result<Shared<Producer>> {
    get(&media.producers, handle, "producer")
}

#[tauri::command]
pub async fn media_producer_pause(
    media: State<'_, Media>,
    producer: u64,
    paused: bool,
) -> Result<()> {
    let producer = self::producer(&media, producer)?;
    let producer = producer.lock().unwrap();
    producer.set_paused(paused).map_err(err)
}

#[tauri::command]
pub async fn media_producer_replace_source(
    media: State<'_, Media>,
    producer: u64,
    source: u64,
) -> Result<()> {
    let producer = self::producer(&media, producer)?;
    let source = get(&media.sources, source, "source")?;
    blocking(move || {
        producer
            .lock()
            .unwrap()
            .replace_source(&source)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn media_producer_parameters(media: State<'_, Media>, producer: u64) -> Result<Value> {
    let producer = self::producer(&media, producer)?;
    let producer = producer.lock().unwrap();
    producer.parameters().map_err(err)
}

#[tauri::command]
pub async fn media_producer_set_parameters(
    media: State<'_, Media>,
    producer: u64,
    parameters: Value,
) -> Result<()> {
    let producer = self::producer(&media, producer)?;
    blocking(move || {
        producer
            .lock()
            .unwrap()
            .set_parameters(&parameters)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn media_producer_stats(media: State<'_, Media>, producer: u64) -> Result<Value> {
    let producer = self::producer(&media, producer)?;
    blocking(move || producer.lock().unwrap().stats().map_err(err)).await
}

#[tauri::command]
pub async fn media_producer_close(media: State<'_, Media>, producer: u64) -> Result<()> {
    let removed = media.producers.lock().unwrap().remove(&producer);
    if let Some(producer) = removed {
        blocking(move || {
            drop(producer);
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// `params`: the server's consumer announcement as
/// `{"id","producerId","kind","rtpParameters","appData"?}`. Audio starts at
/// volume 0; `media_consumer_set_volume` makes it audible.
#[tauri::command]
pub async fn media_consume(
    media: State<'_, Media>,
    transport: u64,
    params: Value,
) -> Result<Value> {
    let transport = get(&media.transports, transport, "transport")?;
    let audio = params["kind"] == "audio";
    let consumer = blocking(move || {
        let consumer = transport.consume(&params).map_err(err)?;
        // Silent until the page attaches it to an output, like a browser
        // track without an audio element.
        if audio {
            consumer.set_volume(0.0).map_err(err)?;
        }
        Ok(consumer)
    })
    .await?;
    let id = consumer.id().to_owned();
    let handle = media.insert(&media.consumers, Arc::new(Mutex::new(consumer)));
    Ok(json!({ "consumer": handle, "id": id }))
}

fn consumer(media: &Media, handle: u64) -> Result<Shared<Consumer>> {
    get(&media.consumers, handle, "consumer")
}

#[tauri::command]
pub async fn media_consumer_pause(
    media: State<'_, Media>,
    consumer: u64,
    paused: bool,
) -> Result<()> {
    let consumer = self::consumer(&media, consumer)?;
    let consumer = consumer.lock().unwrap();
    consumer.set_paused(paused).map_err(err)
}

/// Audio playback volume, 0..2.
#[tauri::command]
pub async fn media_consumer_set_volume(
    media: State<'_, Media>,
    consumer: u64,
    volume: f64,
) -> Result<()> {
    let consumer = self::consumer(&media, consumer)?;
    let consumer = consumer.lock().unwrap();
    consumer.set_volume(volume).map_err(err)
}

#[tauri::command]
pub async fn media_consumer_stats(media: State<'_, Media>, consumer: u64) -> Result<Value> {
    let consumer = self::consumer(&media, consumer)?;
    blocking(move || consumer.lock().unwrap().stats().map_err(err)).await
}

/// The native end of a remote video's feed.
struct ConsumerTap(Shared<Consumer>);

impl Tap for ConsumerTap {
    fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
        self.0.lock().unwrap().set_video_sink(sink).map_err(err)
    }

    fn set_limits(&mut self, limits: VideoSinkLimits) -> Result<()> {
        let consumer = self.0.lock().unwrap();
        consumer.set_video_sink_limits(limits).map_err(err)
    }
}

/// The native end of a self view's feed (camera, screen).
struct SourceTap(Source);

impl Tap for SourceTap {
    fn set_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
        self.0.set_video_sink(sink).map_err(err)
    }

    fn set_limits(&mut self, limits: VideoSinkLimits) -> Result<()> {
        self.0.set_video_sink_limits(limits).map_err(err)
    }
}

/// Makes the native end of a feed when its first target arrives.
type MakeTap = Box<dyn FnOnce() -> Result<Box<dyn Tap>> + Send>;

fn consumer_tap(shared: Shared<Consumer>) -> MakeTap {
    Box::new(move || Ok(Box::new(ConsumerTap(shared))))
}

/// Ends a feed whose consumer or source was closed while a view or window
/// of it opened; true when it was.
async fn closed_meanwhile(media: &Media, origin: Origin) -> Result<bool> {
    if media.holds(origin) {
        return Ok(false);
    }
    let frames = media.frames.clone();
    blocking(move || {
        frames.close_origin(origin);
        Ok(true)
    })
    .await
}

/// Shows a video consumer in a native viewer window titled `title`, alone
/// or next to views of it in the page. `events` receives
/// `{"type":"height","height"}` (shown image height in physical pixels, for
/// layer choice) and `{"type":"closed"}` when the window goes away; the page
/// then calls `media_viewer_close`.
#[tauri::command]
pub async fn media_viewer_open(
    media: State<'_, Media>,
    consumer: u64,
    title: String,
    events: Channel<Value>,
) -> Result<()> {
    let shared = self::consumer(&media, consumer)?;
    let viewer = Viewer::get()?;
    let sink = viewer.open(
        consumer,
        title,
        Box::new(move |event| {
            let _ = events.send(match event {
                ViewerEvent::Height(height) => json!({"type": "height", "height": height}),
                ViewerEvent::Closed => json!({"type": "closed"}),
            });
        }),
    )?;
    let origin = Origin::Consumer(consumer);
    let frames = media.frames.clone();
    let mut installed =
        blocking(move || frames.set_window(origin, sink, consumer_tap(shared))).await;
    if installed.is_ok() && closed_meanwhile(&media, origin).await? {
        installed = Err(format!("unknown consumer {consumer}"));
    }
    if installed.is_err() {
        viewer.close(consumer);
    }
    installed
}

#[tauri::command]
pub async fn media_viewer_close(media: State<'_, Media>, consumer: u64) -> Result<()> {
    if let Some(viewer) = Viewer::running() {
        viewer.close(consumer);
    }
    let frames = media.frames.clone();
    blocking(move || {
        frames.clear_window(Origin::Consumer(consumer));
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn media_consumer_close(media: State<'_, Media>, consumer: u64) -> Result<()> {
    if let Some(viewer) = Viewer::running() {
        viewer.close(consumer);
    }
    // Out of the registry first, as in `media_source_close`.
    let removed = media.consumers.lock().unwrap().remove(&consumer);
    let frames = media.frames.clone();
    blocking(move || {
        frames.close_origin(Origin::Consumer(consumer));
        drop(removed);
        Ok(())
    })
    .await
}

/// A test pattern instead of a consumer or source, for smoke tests and
/// benches: `media_view_open` takes it only while this variable is set.
const TEST_PATTERN_ENV: &str = "GELABBER_VIDEO_TEST_PATTERN";

#[derive(serde::Deserialize)]
pub struct PatternOptions {
    width: u32,
    height: u32,
    fps: u32,
}

/// What a page may ask for: physical pixels and frames a second, rounded up
/// (a canvas is rarely a whole number of them) and kept within reason.
fn request(
    max_width: Option<f64>,
    max_height: Option<f64>,
    max_fps: Option<f64>,
) -> Result<VideoSinkLimits> {
    let whole = |value: f64, least: f64, most: f64| {
        if value.is_finite() && value >= 0.0 {
            Ok(value.ceil().clamp(least, most) as u32)
        } else {
            Err(format!("invalid view size or rate {value}"))
        }
    };
    let side = |value: Option<f64>, default: u32| match value {
        Some(value) => whole(value, 2.0, 16384.0),
        None => Ok(default),
    };
    Ok(VideoSinkLimits {
        max_width: side(max_width, DEFAULT_REQUEST.max_width)?,
        max_height: side(max_height, DEFAULT_REQUEST.max_height)?,
        max_fps: match max_fps {
            Some(value) => whole(value, 1.0, 240.0)?,
            None => DEFAULT_REQUEST.max_fps,
        },
    })
}

/// Shows video in the page: a remote video `consumer` or a local video
/// `source` (camera, screen), exactly one of them. Returns `{"view"}`; the
/// page pulls its frames with `media_view_frame` and draws them. Any number
/// of views may show the same consumer or source, next to a viewer window.
/// Frames are at most `maxWidth` x `maxHeight` physical pixels (1280x720
/// until the page says, here or with `media_view_configure`) and come at
/// most `maxFps` times a second.
#[tauri::command]
pub async fn media_view_open(
    media: State<'_, Media>,
    consumer: Option<u64>,
    source: Option<u64>,
    test_pattern: Option<PatternOptions>,
    max_width: Option<f64>,
    max_height: Option<f64>,
    max_fps: Option<f64>,
) -> Result<Value> {
    let request = request(max_width, max_height, max_fps)?;
    let test_pattern = test_pattern.filter(|_| std::env::var_os(TEST_PATTERN_ENV).is_some());
    let (origin, tap): (Origin, MakeTap) = match (consumer, source, test_pattern) {
        (Some(handle), None, None) => {
            let shared = self::consumer(&media, handle)?;
            (Origin::Consumer(handle), consumer_tap(shared))
        }
        (None, Some(handle), None) => {
            let source = get(&media.sources, handle, "source")?;
            let tap = move || Ok(Box::new(SourceTap(source)) as Box<dyn Tap>);
            (Origin::Source(handle), Box::new(tap))
        }
        (None, None, Some(PatternOptions { width, height, fps })) => {
            let tap = move || Ok(Box::new(TestPattern::new(width, height, fps)?) as Box<dyn Tap>);
            (Origin::Pattern(width, height, fps), Box::new(tap))
        }
        _ => return Err("a view shows either a consumer or a source".into()),
    };
    let frames = media.frames.clone();
    let view = blocking(move || frames.open(origin, request, tap)).await?;
    if closed_meanwhile(&media, origin).await? {
        return Err("the stream of the view was closed".into());
    }
    Ok(json!({ "view": view }))
}

/// The size the page draws the view at now, in physical pixels, and the rate
/// it wants at most (none without `maxFps`).
#[tauri::command]
pub async fn media_view_configure(
    media: State<'_, Media>,
    view: u64,
    max_width: f64,
    max_height: f64,
    max_fps: Option<f64>,
) -> Result<()> {
    let request = request(Some(max_width), Some(max_height), max_fps)?;
    let frames = media.frames.clone();
    blocking(move || frames.configure(view, request)).await
}

/// The next frame of `view` the page has not seen: `after` is the sequence
/// number of the frame it has (none at first). Resolves with the packet
/// (frames.rs) as an `ArrayBuffer` when a newer frame exists, which may take
/// as long as the stream stands still; fails when the view is closed. One
/// request per view at a time.
#[tauri::command]
pub async fn media_view_frame(
    media: State<'_, Media>,
    view: u64,
    after: Option<u32>,
) -> Result<Response> {
    let view = media.frames.view(view)?;
    let (tx, mut rx) = channel(1);
    view.pull(
        after,
        Box::new(move |answer| {
            let _ = tx.try_send(answer);
        }),
    );
    match rx.recv().await {
        Some(Ok(packet)) => Ok(Response::new(packet)),
        Some(Err(reason)) => Err(reason.into()),
        None => Err("view closed".into()),
    }
}

/// Closes a view; the last one of a stream stops its frames.
#[tauri::command]
pub async fn media_view_close(media: State<'_, Media>, view: u64) -> Result<()> {
    let frames = media.frames.clone();
    blocking(move || {
        frames.close(view);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    /// Every name the web client knows.
    const KNOWN_FEATURES: &[&str] = &[
        "screen",
        "camera",
        "app-audio",
        "app-audio-excludes-self",
        "video-frames",
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
        let today: &[&str] = if cfg!(target_os = "linux") {
            &[
                "screen",
                "camera",
                "app-audio",
                "app-audio-excludes-self",
                "video-frames",
            ]
        } else if cfg!(windows) {
            &["camera", "video-frames"]
        } else {
            &[]
        };
        assert_eq!(info["features"], serde_json::json!(today));
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
        let handle = media.insert(&media.sources, source);
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
        let in_flight = get(&media.sources, handle, "source").unwrap();

        media.reset();
        assert_eq!(held_at_the_end.try_recv(), Ok(false));
        assert!(media.frames.view(open).is_err());
        assert!(!media.holds(origin));

        // The command goes on after the load: its view gets frames, then
        // the check that follows every open ends it.
        let tap = move || Ok(Box::new(SourceTap(in_flight)) as Box<dyn Tap>);
        let late = media.frames.open(origin, DEFAULT_REQUEST, tap).unwrap();
        next_frame(&media.frames, late, None);
        assert_eq!(
            tauri::async_runtime::block_on(closed_meanwhile(&media, origin)),
            Ok(true)
        );
        assert!(media.frames.view(late).is_err());
    }

    /// The server origin's permission set grants exactly the media commands.
    #[test]
    fn media_permission_set_matches_the_commands() {
        let set = include_str!("../permissions/media.toml");
        let granted: Vec<&str> = set
            .lines()
            .filter_map(|line| line.trim().strip_prefix("\"allow-"))
            .filter_map(|rest| rest.strip_suffix("\","))
            .collect();
        let expected: Vec<String> = MEDIA_COMMANDS.iter().map(|c| c.replace('_', "-")).collect();
        assert_eq!(granted, expected);
        let main = include_str!("main.rs");
        for command in MEDIA_COMMANDS {
            assert!(
                main.contains(&format!("media::{command},")),
                "{command} missing from generate_handler!"
            );
        }
    }
}
