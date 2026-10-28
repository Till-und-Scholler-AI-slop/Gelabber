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
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
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
    /// Page loads so far. A command that makes something for the page notes
    /// the count when it starts ([`Media::page`]); what it made is the
    /// page's only while the count stands ([`Media::insert`]). Shared with
    /// the blocking pool, where the microphone test is switched.
    page: Arc<AtomicU64>,
    /// Held while the engine's microphone test is switched on or off
    /// ([`switch_monitor`]).
    monitor: Arc<Mutex<()>>,
    devices: Mutex<HashMap<u64, Device>>,
    transports: Mutex<HashMap<u64, Arc<PageTransport>>>,
    sources: Mutex<HashMap<u64, Source>>,
    producers: Mutex<HashMap<u64, Shared<Producer>>>,
    consumers: Mutex<HashMap<u64, Shared<Consumer>>>,
    /// Video shown in the page and in viewer windows (frames.rs).
    frames: Frames,
    /// When the page last closed a transport or a source, or went away
    /// with some ([`Media::busy`]).
    last_busy: Mutex<Option<Instant>>,
}

/// How long the page counts as busy after its last transport or source
/// closed: a call that reconnects, rejoins or comes back after a reload has
/// none for a moment.
const BUSY_AFTER: Duration = Duration::from_secs(60);

type Result<T> = std::result::Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// How a command ends whose page was replaced by a page load before the
/// command was through, and why a transport refuses the requests of such a
/// page: nobody is left to answer them.
const PAGE_GONE: &str = "the page went away";
/// Or the page closed the transport, and with it its way to answer.
const TRANSPORT_CLOSED: &str = "transport closed";

/// A transport and what the page still owes it. The core waits for the
/// page's answer to a `connect` or `produce` request without a limit and
/// holds the transport's lock meanwhile, which closing a producer or consumer
/// of the transport needs too. So every request gets an answer: the page's,
/// or a refusal from here once the page cannot give one any more.
struct PageTransport {
    native: Transport,
    requests: Mutex<Requests>,
}

#[derive(Default)]
struct Requests {
    /// Sent to the page and not answered yet.
    open: HashSet<u64>,
    /// Why nobody answers any more, once that is so.
    refused: Option<&'static str>,
}

impl PageTransport {
    /// Waits in the core for the page's answers, so it runs on the blocking
    /// pool. The call holds `self` until it ends, which is how the events
    /// thread reaches the transport to refuse a request of the call.
    fn produce(&self, source: &Source, options: &Value) -> Result<Producer> {
        self.native.produce(source, options).map_err(err)
    }

    /// Like [`PageTransport::produce`], for the first consumer.
    fn consume(&self, params: &Value) -> Result<Consumer> {
        self.native.consume(params).map_err(err)
    }

    /// Notes a request on its way to the page. False when nobody answers
    /// any more: the request is refused then.
    fn ask(&self, request: u64) -> bool {
        let refused = {
            let mut requests = self.requests.lock().unwrap();
            if requests.refused.is_none() {
                requests.open.insert(request);
            }
            requests.refused
        };
        if let Some(reason) = refused {
            self.refuse(request, reason);
        }
        refused.is_none()
    }

    /// The page's answer to a request.
    fn answer(&self, request: u64, answer: std::result::Result<Value, String>) -> Result<()> {
        self.native.respond(request, answer).map_err(err)?;
        self.requests.lock().unwrap().open.remove(&request);
        Ok(())
    }

    /// Nobody answers the transport's requests from here on: the open ones
    /// are refused, and so is every later one (`ask`). A call that waited for
    /// the page ends with an error and lets go of the transport's lock.
    fn orphan(&self, reason: &'static str) {
        let open = {
            let mut requests = self.requests.lock().unwrap();
            requests.refused.get_or_insert(reason);
            std::mem::take(&mut requests.open)
        };
        for request in open {
            self.refuse(request, reason);
        }
    }

    fn refuse(&self, request: u64, reason: &str) {
        // Unknown by now when the page answered in the same moment.
        let _ = self.native.respond(request, Err(reason.into()));
    }
}

/// Hands a transport's events to the page until the transport is freed,
/// which ends `events`. `send` is false when the page cannot be reached.
fn forward_events(
    transport: Weak<PageTransport>,
    events: mpsc::Receiver<TransportEvent>,
    send: impl Fn(Value) -> bool,
) {
    for event in events {
        let (request, message) = match event {
            TransportEvent::Connect {
                request,
                dtls_parameters,
            } => (
                Some(request),
                json!({
                    "type": "connect",
                    "request": request,
                    "dtlsParameters": dtls_parameters,
                }),
            ),
            TransportEvent::Produce {
                request,
                kind,
                rtp_parameters,
                app_data,
            } => (
                Some(request),
                json!({
                    "type": "produce",
                    "request": request,
                    "kind": kind,
                    "rtpParameters": rtp_parameters,
                    "appData": app_data,
                }),
            ),
            TransportEvent::ConnectionState(state) => (
                None,
                json!({
                    "type": "connectionstatechange",
                    "state": state,
                }),
            ),
        };
        let Some(request) = request else {
            send(message);
            continue;
        };
        // A request comes from a produce or consume in progress, which holds
        // the transport until it has the answer; without one nothing waits.
        let Some(transport) = transport.upgrade() else {
            continue;
        };
        if transport.ask(request) && !send(message) {
            transport.orphan(PAGE_GONE);
        }
    }
}

/// What a page left behind.
struct Leftovers {
    consumers: HashMap<u64, Shared<Consumer>>,
    producers: HashMap<u64, Shared<Producer>>,
    sources: HashMap<u64, Source>,
    transports: HashMap<u64, Arc<PageTransport>>,
    devices: HashMap<u64, Device>,
}

impl Leftovers {
    /// Producers and consumers close on their transports, a transport on its
    /// device. Each may wait for the core.
    fn free(self) {
        drop(self.consumers);
        drop(self.producers);
        drop(self.sources);
        drop(self.transports);
        drop(self.devices);
    }
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

    /// The count of page loads, for a command to note before it makes
    /// anything or looks anything up. Tauri starts a command on the async
    /// runtime, a moment after the UI thread took it from the page: one the
    /// page sent in the last instant before a page load may note the count
    /// after it, and then counts for the page that follows.
    fn page(&self) -> u64 {
        self.page.load(Ordering::SeqCst)
    }

    /// Registers what a command made for the page and returns its handle;
    /// `page` is the count the command noted when it started. After a page
    /// load the handle would reach nobody and the object would stay, a
    /// camera capturing with nothing left to close it. It is freed then, on
    /// the blocking pool because that may wait for the core.
    fn insert<T: Send + 'static>(
        &self,
        page: u64,
        map: &Mutex<HashMap<u64, T>>,
        value: T,
    ) -> Result<u64> {
        let mut registry = map.lock().unwrap();
        // Under the registry's lock: a page load counts first and empties
        // the registries after, so it finds the object here or the object
        // finds the new count.
        if self.page() != page {
            drop(registry);
            spawn_blocking(move || drop(value));
            return Err(PAGE_GONE.into());
        }
        let handle = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        registry.insert(handle, value);
        Ok(handle)
    }

    /// Whether the page still holds the consumer or source a feed shows.
    fn holds(&self, origin: Origin) -> bool {
        match origin {
            Origin::Consumer(handle) => self.consumers.lock().unwrap().contains_key(&handle),
            Origin::Source(handle) => self.sources.lock().unwrap().contains_key(&handle),
            Origin::Pattern(..) => true,
        }
    }

    /// Creates a transport of `device` for the page, for a command that
    /// started at the page count `page`. `send` hands the page its events
    /// and is false when the page cannot be reached. Returns the transport's
    /// handle and its id on the server.
    fn open_transport(
        &self,
        page: u64,
        device: u64,
        direction: &str,
        options: &Value,
        send: impl Fn(Value) -> bool + Send + 'static,
    ) -> Result<(u64, String)> {
        let device = get(&self.devices, device, "device")?;
        let direction = match direction {
            "send" => Direction::Send,
            "recv" => Direction::Recv,
            other => return Err(format!("unknown direction {other}")),
        };
        let (native, events) = Transport::new(&device, direction, options).map_err(err)?;
        let id = native.id().to_owned();
        let transport = Arc::new(PageTransport {
            native,
            requests: Mutex::default(),
        });
        // Not the transport itself: the thread ends when the transport is
        // freed, which it would keep from happening.
        let weak = Arc::downgrade(&transport);
        std::thread::Builder::new()
            .name("gelabber-transport-events".into())
            .spawn(move || forward_events(weak, events, send))
            .map_err(err)?;
        Ok((self.insert(page, &self.transports, transport)?, id))
    }

    /// The page closes a transport. Producers and consumers keep it alive
    /// until they close; a request the page has not answered is refused, for
    /// `media_transport_respond` no longer finds the transport.
    fn close_transport(&self, transport: u64) {
        let removed = self.transports.lock().unwrap().remove(&transport);
        if let Some(transport) = removed {
            self.note_busy();
            transport.orphan(TRANSPORT_CLOSED);
        }
    }

    /// The page closes a source: it is out of the registry at once. What is
    /// returned ends the page's views of it and frees it, unless a producer
    /// keeps it running until that closes too. That waits for a frame in
    /// delivery, so it is for the blocking pool.
    fn close_source(&self, source: u64) -> impl FnOnce() + Send + use<> {
        // Out of the registry first: a view opening right now then finds the
        // source gone, or is closed with the others.
        let removed = self.sources.lock().unwrap().remove(&source);
        if removed.is_some() {
            self.note_busy();
        }
        let frames = self.frames.clone();
        move || {
            frames.close_origin(Origin::Source(source));
            drop(removed);
        }
    }

    /// The page closes a consumer: its viewer window and its views in the
    /// page go with it. Returns what is left to do on the blocking pool, as
    /// [`Media::close_source`] does.
    fn close_consumer(&self, consumer: u64) -> impl FnOnce() + Send + use<> {
        if let Some(viewer) = Viewer::running() {
            viewer.close(consumer);
        }
        // Out of the registry first, as in `close_source`.
        let removed = self.consumers.lock().unwrap().remove(&consumer);
        let frames = self.frames.clone();
        move || {
            frames.close_origin(Origin::Consumer(consumer));
            drop(removed);
        }
    }

    /// The page closes a consumer's viewer window, or the person did. The
    /// consumer's views in the page stay and get the stream at their own
    /// size again. Returns what is left to do on the blocking pool.
    fn close_viewer(&self, consumer: u64) -> impl FnOnce() + Send + use<> {
        if let Some(viewer) = Viewer::running() {
            viewer.close(consumer);
        }
        let frames = self.frames.clone();
        move || frames.clear_window(Origin::Consumer(consumer))
    }

    /// What a view shows and how to make the native end of its feed: a
    /// video consumer or a video source of the page, exactly one of them.
    /// A test pattern instead only when `patterns` is set
    /// ([`TEST_PATTERN_ENV`]).
    fn view_target(
        &self,
        consumer: Option<u64>,
        source: Option<u64>,
        pattern: Option<PatternOptions>,
        patterns: bool,
    ) -> Result<(Origin, MakeTap)> {
        match (consumer, source, pattern.filter(|_| patterns)) {
            (Some(handle), None, None) => {
                let shared = self::consumer(self, handle)?;
                Ok((Origin::Consumer(handle), consumer_tap(shared)))
            }
            (None, Some(handle), None) => {
                let source = get(&self.sources, handle, "source")?;
                let tap = move || Ok(Box::new(SourceTap(source)) as Box<dyn Tap>);
                Ok((Origin::Source(handle), Box::new(tap)))
            }
            (None, None, Some(PatternOptions { width, height, fps })) => {
                let tap =
                    move || Ok(Box::new(TestPattern::new(width, height, fps)?) as Box<dyn Tap>);
                Ok((Origin::Pattern(width, height, fps), Box::new(tap)))
            }
            _ => Err("a view shows either a consumer or a source".into()),
        }
    }

    /// Whether the page is in a call or captures something: it has a
    /// transport (voice, watching a stream) or a source (microphone, camera,
    /// screen), or had one less than [`BUSY_AFTER`] ago. Restarting the app
    /// for an update would end that (update.rs).
    pub fn busy(&self) -> bool {
        !self.transports.lock().unwrap().is_empty()
            || !self.sources.lock().unwrap().is_empty()
            || self
                .last_busy
                .lock()
                .unwrap()
                .is_some_and(|last| last.elapsed() < BUSY_AFTER)
    }

    fn note_busy(&self) {
        *self.last_busy.lock().unwrap() = Some(Instant::now());
    }

    /// Drops every object of a page that went away. A page load calls this
    /// on the UI thread, which must not wait for the core: closing a producer
    /// or consumer takes its transport's lock and may take the core a while,
    /// so what is left is freed on the blocking pool.
    pub fn reset(&self) {
        let leftovers = self.forget();
        spawn_blocking(move || leftovers.free());
    }

    /// The part of [`Media::reset`] the next page must find done; returns
    /// what is left to free. A page load is reported when the navigation
    /// commits: the old page is gone by then, but commands it sent may still
    /// be at work. So the page count moves first, and what such a command
    /// makes from here on is not registered ([`Media::insert`]). Then the
    /// registries are emptied, before anything is freed: a view such a
    /// command opens has to find its consumer or source gone
    /// (`closed_meanwhile`), or a camera would go on capturing with no handle
    /// left to close it. Then the views end, which hold the sinks of
    /// consumers and sources.
    fn forget(&self) -> Leftovers {
        fn take<T>(map: &Mutex<HashMap<u64, T>>) -> HashMap<u64, T> {
            std::mem::take(&mut *map.lock().unwrap())
        }
        self.page.fetch_add(1, Ordering::SeqCst);
        let leftovers = Leftovers {
            consumers: take(&self.consumers),
            producers: take(&self.producers),
            sources: take(&self.sources),
            transports: take(&self.transports),
            devices: take(&self.devices),
        };
        if !leftovers.transports.is_empty() || !leftovers.sources.is_empty() {
            self.note_busy();
        }
        // A produce or consume of the old page may still wait for its answer,
        // with the lock of its transport that freeing the leftovers needs.
        for transport in leftovers.transports.values() {
            transport.orphan(PAGE_GONE);
        }
        self.frames.reset();
        if let Some(viewer) = Viewer::running() {
            viewer.close_all();
        }
        if let Some(engine) = self.engine.get() {
            // A microphone test the page left open. After a switch that is
            // under way, so that this is the last word (`switch_monitor`).
            let _switching = self.monitor.lock().unwrap();
            let _ = engine.monitor_audio(None);
        }
        leftovers
    }
}

/// Runs a native call that may block (produce/consume wait for the server
/// round trips the page answers) off the async runtime.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    spawn_blocking(job).await.map_err(err)?
}

/// [`blocking`] for work that cannot fail.
async fn off_runtime(job: impl FnOnce() + Send + 'static) -> Result<()> {
    spawn_blocking(job).await.map_err(err)
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
/// - "h264-hardware": the core encodes H264 on the GPU or a media engine;
///   the web client then prefers H264 where the room can decode it. Added by
///   `media_info` when the system has such an encoder.
/// - "av1-hardware": the same for AV1, which the web client prefers even to
///   H264 (issue #318).
/// - "av1-hardware-decode": the core decodes AV1 on the GPU. Without it the
///   web client does not offer to receive AV1, which comes as one stream, so
///   that the room stays on H264 or VP8 with layers.
/// - "vp9-svc": the core sends VP9 in three spatial layers
///   (`scalabilityMode` "L3T3_KEY", one encoding). VP9 is libvpx in
///   software inside libwebrtc, so Linux and Windows both report it. The web
///   client sends VP8 in layers first, because libwebrtc caps each VP9
///   spatial layer at about 1.6 x pixels + 50 kbit/s, and uses VP9 in layers
///   where the room has no VP8 or H264.
///
/// The only place that decides the list. An app before v0.6 reports none.
const FEATURES: &[&str] = if cfg!(any(target_os = "linux", windows)) {
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

#[tauri::command]
pub async fn media_info() -> Value {
    // The first look at GStreamer scans its plugins: off the async runtime.
    let codecs = spawn_blocking(gelabber_media_core::video_codecs)
        .await
        .ok()
        .and_then(|codecs| codecs.ok())
        .unwrap_or(Value::Null);
    let mut features = FEATURES.to_vec();
    if codecs["h264"]["hardwareEncoder"] == true {
        features.push("h264-hardware");
    }
    if codecs["av1"]["hardwareEncoder"] == true {
        features.push("av1-hardware");
    }
    if codecs["av1"]["hardwareDecoder"] == true {
        features.push("av1-hardware-decode");
    }
    json!({
        "abi": gelabber_media_core::ABI_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "features": features,
        "videoCodecs": codecs,
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
    let page = media.page();
    let engine = media.engine()?.clone();
    let (loads, switching) = (media.page.clone(), media.monitor.clone());
    blocking(move || {
        let switch = || engine.monitor_audio(options.as_ref()).map_err(err);
        switch_monitor(&loads, &switching, page, switch)
    })
    .await
}

/// Switches the engine's microphone test for a command that started at the
/// page count `page`. The test has no handle a page load could take away:
/// the load ends it, under the lock held here, and nothing may switch it on
/// for the page that left, or it would capture with nobody to end it.
fn switch_monitor(
    loads: &AtomicU64,
    switching: &Mutex<()>,
    page: u64,
    switch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let _switching = switching.lock().unwrap();
    if loads.load(Ordering::SeqCst) != page {
        return Err(PAGE_GONE.into());
    }
    switch()
}

#[tauri::command]
pub async fn media_device_load(media: State<'_, Media>, capabilities: Value) -> Result<Value> {
    let page = media.page();
    let device = Device::new(media.engine()?).map_err(err)?;
    device.load(&capabilities).map_err(err)?;
    let mut result = json!({
        "rtpCapabilities": device.rtp_capabilities().map_err(err)?,
        "canProduce": {
            "audio": device.can_produce(MediaKind::Audio).map_err(err)?,
            "video": device.can_produce(MediaKind::Video).map_err(err)?,
        },
    });
    result["device"] = media.insert(page, &media.devices, device)?.into();
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
    let page = media.page();
    let send = move |message| events.send(message).is_ok();
    let (handle, id) = media.open_transport(page, device, &direction, &options, send)?;
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
    transport.answer(request, answer)
}

#[tauri::command]
pub async fn media_transport_restart_ice(
    media: State<'_, Media>,
    transport: u64,
    ice_parameters: Value,
) -> Result<()> {
    let transport = get(&media.transports, transport, "transport")?;
    blocking(move || transport.native.restart_ice(&ice_parameters).map_err(err)).await
}

#[tauri::command]
pub async fn media_transport_stats(media: State<'_, Media>, transport: u64) -> Result<Value> {
    let transport = get(&media.transports, transport, "transport")?;
    blocking(move || transport.native.stats().map_err(err)).await
}

#[tauri::command]
pub async fn media_transport_close(media: State<'_, Media>, transport: u64) -> Result<()> {
    media.close_transport(transport);
    Ok(())
}

#[tauri::command]
pub async fn media_source_microphone(media: State<'_, Media>, options: Value) -> Result<u64> {
    let page = media.page();
    let source = Source::microphone(media.engine()?, &options).map_err(err)?;
    media.insert(page, &media.sources, source)
}

/// Opens the desktop's own screen/window picker; poll `media_source_state`.
/// Off the runtime: on Windows the core waits for its capture thread to
/// find the window the picker opens over.
#[tauri::command]
pub async fn media_source_screen(media: State<'_, Media>, options: Value) -> Result<u64> {
    let page = media.page();
    let engine = media.engine()?.clone();
    let source = blocking(move || Source::screen(&engine, &options).map_err(err)).await?;
    media.insert(page, &media.sources, source)
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
    let page = media.page();
    let engine = media.engine()?.clone();
    let source = blocking(move || Source::app_audio(&engine, &options).map_err(err)).await?;
    media.insert(page, &media.sources, source)
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
    let page = media.page();
    let engine = media.engine()?.clone();
    let source = blocking(move || Source::camera(&engine, &options).map_err(err)).await?;
    media.insert(page, &media.sources, source)
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
    off_runtime(media.close_source(source)).await
}

#[tauri::command]
pub async fn media_produce(
    media: State<'_, Media>,
    transport: u64,
    source: u64,
    options: Value,
) -> Result<Value> {
    let page = media.page();
    let transport = get(&media.transports, transport, "transport")?;
    let source = get(&media.sources, source, "source")?;
    let producer = blocking(move || transport.produce(&source, &options)).await?;
    let mut result = json!({
        "id": producer.id(),
        "rtpParameters": producer.rtp_parameters().map_err(err)?,
    });
    result["producer"] = media
        .insert(page, &media.producers, Arc::new(Mutex::new(producer)))?
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
    let page = media.page();
    let transport = get(&media.transports, transport, "transport")?;
    let audio = params["kind"] == "audio";
    let consumer = blocking(move || {
        let consumer = transport.consume(&params)?;
        // Silent until the page attaches it to an output, like a browser
        // track without an audio element.
        if audio {
            consumer.set_volume(0.0).map_err(err)?;
        }
        Ok(consumer)
    })
    .await?;
    let id = consumer.id().to_owned();
    let handle = media.insert(page, &media.consumers, Arc::new(Mutex::new(consumer)))?;
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

/// Opens a view of `origin` in the page, for a command that started at the
/// page count `page`, and returns its handle. The page may close the
/// consumer or source in the same moment, or be replaced: the feed made for
/// the view would then keep a camera capturing with nothing left to end it,
/// so the view is checked once it stands.
async fn open_view(
    media: &Media,
    page: u64,
    origin: Origin,
    request: VideoSinkLimits,
    tap: MakeTap,
) -> Result<u64> {
    let frames = media.frames.clone();
    let view = blocking(move || frames.open(origin, request, tap)).await?;
    if closed_meanwhile(media, origin).await? {
        return Err("the stream of the view was closed".into());
    }
    // A test pattern is nobody's to close, so no registry says that the
    // page left: the page count does.
    if media.page() != page {
        let frames = media.frames.clone();
        off_runtime(move || frames.close(view)).await?;
        return Err(PAGE_GONE.into());
    }
    Ok(view)
}

/// Feeds a consumer's viewer window through `sink`, with the same check as
/// [`open_view`].
async fn open_window(media: &Media, consumer: u64, sink: VideoSink, tap: MakeTap) -> Result<()> {
    let origin = Origin::Consumer(consumer);
    let frames = media.frames.clone();
    blocking(move || frames.set_window(origin, sink, tap)).await?;
    if closed_meanwhile(media, origin).await? {
        return Err(format!("unknown consumer {consumer}"));
    }
    Ok(())
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
    let installed = open_window(&media, consumer, sink, consumer_tap(shared)).await;
    if installed.is_err() {
        viewer.close(consumer);
    }
    installed
}

#[tauri::command]
pub async fn media_viewer_close(media: State<'_, Media>, consumer: u64) -> Result<()> {
    off_runtime(media.close_viewer(consumer)).await
}

#[tauri::command]
pub async fn media_consumer_close(media: State<'_, Media>, consumer: u64) -> Result<()> {
    off_runtime(media.close_consumer(consumer)).await
}

/// A test pattern instead of a consumer or source, for smoke tests and
/// benches: `media_view_open` takes it only while this variable is set.
const TEST_PATTERN_ENV: &str = "GELABBER_VIDEO_TEST_PATTERN";

/// Whether this app was started for tests ([`TEST_PATTERN_ENV`]): only then
/// may a page open a view of a test pattern.
fn test_patterns_allowed() -> bool {
    std::env::var_os(TEST_PATTERN_ENV).is_some()
}

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
/// most `maxFps` times a second. Next to a viewer window of the consumer
/// only `maxFps` holds: its views get the window's frames, unscaled.
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
    let page = media.page();
    let request = request(max_width, max_height, max_fps)?;
    let (origin, tap) =
        media.view_target(consumer, source, test_pattern, test_patterns_allowed())?;
    let view = open_view(&media, page, origin, request, tap).await?;
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
mod tests;
