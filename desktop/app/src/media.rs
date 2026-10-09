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
}

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
/// (`media_info.features`); the web client hides what is missing. The names
/// are a contract with it (`KNOWN_FEATURES` in the tests):
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
#[tauri::command]
pub async fn media_source_screen(media: State<'_, Media>, options: Value) -> Result<u64> {
    let page = media.page();
    let source = Source::screen(media.engine()?, &options).map_err(err)?;
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
    let page = media.page();
    let request = request(max_width, max_height, max_fps)?;
    let patterns = std::env::var_os(TEST_PATTERN_ENV).is_some();
    let (origin, tap) = media.view_target(consumer, source, test_pattern, patterns)?;
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
mod tests {
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
