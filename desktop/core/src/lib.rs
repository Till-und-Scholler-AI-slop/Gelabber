//! Gelabber desktop media core.
//!
//! Safe Rust over the native libmediasoupclient/libwebrtc library
//! (desktop/native). Signaling stays with the caller: transport events ask it
//! to run `connect`/`produce` on the media server (media/src/protocol.rs) and
//! to answer with [`Transport::respond`].
//!
//! Ownership mirrors the native graph: a [`Producer`] keeps its [`Transport`]
//! alive, a transport its [`Device`], a device its [`Engine`]. Dropping the last
//! handle closes the native object.

mod ffi;

/// C ABI version of the native core this crate was written against.
pub const ABI_VERSION: u32 = ffi::GM_ABI_VERSION;

use serde_json::Value;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr::NonNull,
    sync::{Arc, Mutex, mpsc},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("native media core: {0}")]
    Native(String),
    #[error("invalid argument: {0}")]
    Invalid(&'static str),
    #[error("native media core ABI {found}, expected {expected}")]
    Abi { found: u32, expected: u32 },
}

pub type Result<T> = std::result::Result<T, Error>;

fn last_error() -> Error {
    // SAFETY: gm_last_error returns null or a thread-local NUL-terminated string
    // valid until the next gm_* call on this thread; it is copied immediately.
    let message = unsafe {
        let ptr = ffi::gm_last_error();
        if ptr.is_null() {
            "unknown error".to_owned()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    };
    Error::Native(message)
}

fn cstring(text: &str) -> Result<CString> {
    CString::new(text).map_err(|_| Error::Invalid("interior NUL byte"))
}

fn json_arg(value: &Value) -> Result<CString> {
    cstring(&value.to_string())
}

/// Takes ownership of a malloc'ed native string and parses it as JSON.
fn owned_json(ptr: *mut c_char) -> Result<Value> {
    if ptr.is_null() {
        return Err(last_error());
    }
    // SAFETY: non-null result of a gm_* string function; freed exactly once below.
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { ffi::gm_string_free(ptr) };
    serde_json::from_str(&text).map_err(|e| Error::Native(format!("invalid JSON from core: {e}")))
}

fn borrowed_str(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: points into a live native object owned by the caller's handle.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn check(code: i32) -> Result<()> {
    if code < 0 { Err(last_error()) } else { Ok(()) }
}

/// libwebrtc log verbosity on stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    None = 0,
    Error = 1,
    Warning = 2,
    Info = 3,
    Verbose = 4,
}

pub fn set_log_level(level: LogLevel) {
    // SAFETY: plain value argument.
    unsafe { ffi::gm_set_log_level(level as i32) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Audio {
    /// System audio devices (Linux: PulseAudio API, served by pipewire-pulse
    /// on Omarchy; Windows: Core Audio).
    #[default]
    Default,
    /// No audio devices; for tests and headless runs.
    Dummy,
}

struct EngineInner(NonNull<ffi::gm_engine>);
// SAFETY: the engine owns libwebrtc threads and a thread-safe factory; all
// entry points marshal onto those threads.
unsafe impl Send for EngineInner {}
unsafe impl Sync for EngineInner {}
impl Drop for EngineInner {
    fn drop(&mut self) {
        // SAFETY: last reference; every child holds an Arc to this engine.
        unsafe { ffi::gm_engine_free(self.0.as_ptr()) }
    }
}

/// libwebrtc threads, PeerConnectionFactory, audio device module and codecs.
#[derive(Clone)]
pub struct Engine(Arc<EngineInner>);

impl Engine {
    pub fn new(audio: Audio) -> Result<Self> {
        // SAFETY: plain value call.
        let found = unsafe { ffi::gm_abi_version() };
        if found != ffi::GM_ABI_VERSION {
            return Err(Error::Abi {
                found,
                expected: ffi::GM_ABI_VERSION,
            });
        }
        let options = json_arg(&serde_json::json!({
            "audio": match audio { Audio::Default => "default", Audio::Dummy => "dummy" }
        }))?;
        // SAFETY: valid NUL-terminated JSON; null result means failure.
        let ptr = unsafe { ffi::gm_engine_new(options.as_ptr()) };
        NonNull::new(ptr)
            .map(|p| Self(Arc::new(EngineInner(p))))
            .ok_or_else(last_error)
    }

    /// `{"inputs":[{"id","name"}],"outputs":[...],"input","output"}`; id `""`
    /// is the system default (on Windows the default device, not the default
    /// communications device).
    pub fn audio_devices(&self) -> Result<Value> {
        // SAFETY: live engine.
        owned_json(unsafe { ffi::gm_audio_devices(self.raw()) })
    }

    /// Live audio settings, each optional:
    /// `{"input"?: id, "output"?: id, "inputGain"?: 0..2}`.
    pub fn configure_audio(&self, options: &Value) -> Result<()> {
        let options = json_arg(options)?;
        // SAFETY: live engine, NUL-terminated JSON.
        check(unsafe { ffi::gm_audio_configure(self.raw(), options.as_ptr()) })
    }

    /// Applications playing sound, without this process:
    /// `[{"id","name","streams"}]`.
    pub fn audio_apps(&self) -> Result<Value> {
        // SAFETY: live engine.
        owned_json(unsafe { ffi::gm_audio_apps(self.raw()) })
    }

    /// Cameras: `[{"id","name"}]`.
    pub fn video_devices(&self) -> Result<Value> {
        // SAFETY: live engine.
        owned_json(unsafe { ffi::gm_video_devices(self.raw()) })
    }

    /// Microphone test: `Some(options)` (`{"processingMode","inputGain"}`)
    /// keeps capture running so the meters move without a call, `None` ends
    /// it. Mode and gain are engine-wide.
    pub fn monitor_audio(&self, options: Option<&Value>) -> Result<()> {
        let options = options.map(json_arg).transpose()?;
        let ptr = options.as_ref().map_or(std::ptr::null(), |o| o.as_ptr());
        // SAFETY: live engine; JSON is NUL-terminated or NULL.
        check(unsafe { ffi::gm_audio_monitor(self.raw(), ptr) })
    }

    /// Microphone meters while capture runs, 0..100:
    /// `{"input","processed","clipping","denoised","blocks","channels"}`.
    pub fn audio_levels(&self) -> Result<Value> {
        // SAFETY: live engine.
        owned_json(unsafe { ffi::gm_audio_levels(self.raw()) })
    }

    fn raw(&self) -> *mut ffi::gm_engine {
        self.0.0.as_ptr()
    }
}

struct DeviceInner {
    ptr: NonNull<ffi::gm_device>,
    _engine: Engine,
    lock: Mutex<()>,
}
// SAFETY: access to the native device is serialized by `lock`.
unsafe impl Send for DeviceInner {}
unsafe impl Sync for DeviceInner {}
impl Drop for DeviceInner {
    fn drop(&mut self) {
        // SAFETY: last reference; transports hold an Arc to this device.
        unsafe { ffi::gm_device_free(self.ptr.as_ptr()) }
    }
}

/// mediasoup Device: router capabilities and local codec support.
#[derive(Clone)]
pub struct Device(Arc<DeviceInner>);

impl Device {
    pub fn new(engine: &Engine) -> Result<Self> {
        // SAFETY: engine pointer is live while `engine` is.
        let ptr = unsafe { ffi::gm_device_new(engine.raw()) };
        NonNull::new(ptr)
            .map(|ptr| {
                Self(Arc::new(DeviceInner {
                    ptr,
                    _engine: engine.clone(),
                    lock: Mutex::new(()),
                }))
            })
            .ok_or_else(last_error)
    }

    /// Loads `routerRtpCapabilities` from the server `capabilities` event.
    pub fn load(&self, router_rtp_capabilities: &Value) -> Result<()> {
        let caps = json_arg(router_rtp_capabilities)?;
        let _guard = self.0.lock.lock().unwrap();
        // SAFETY: live device, valid C string.
        check(unsafe { ffi::gm_device_load(self.0.ptr.as_ptr(), caps.as_ptr()) })
    }

    /// Receive capabilities for the client `capabilities` request.
    pub fn rtp_capabilities(&self) -> Result<Value> {
        let _guard = self.0.lock.lock().unwrap();
        // SAFETY: live device.
        owned_json(unsafe { ffi::gm_device_rtp_capabilities(self.0.ptr.as_ptr()) })
    }

    pub fn can_produce(&self, kind: MediaKind) -> Result<bool> {
        let kind = cstring(kind.as_str())?;
        let _guard = self.0.lock.lock().unwrap();
        // SAFETY: live device, valid C string.
        match unsafe { ffi::gm_device_can_produce(self.0.ptr.as_ptr(), kind.as_ptr()) } {
            n if n < 0 => Err(last_error()),
            n => Ok(n == 1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Send,
    Recv,
}

/// Requests from a transport that the caller answers via the media server.
#[derive(Debug)]
pub enum TransportEvent {
    /// Run `connect` with these DtlsParameters, then [`Transport::respond`].
    Connect {
        request: u64,
        dtls_parameters: Value,
    },
    /// Run `produce`, then respond with `{"id": producerId}`.
    Produce {
        request: u64,
        kind: String,
        rtp_parameters: Value,
        app_data: Value,
    },
    ConnectionState(String),
}

struct EventSink(Mutex<mpsc::Sender<TransportEvent>>);

unsafe extern "C" fn on_event(
    user: *mut c_void,
    kind: ffi::gm_event_kind,
    request: u64,
    json: *const c_char,
) {
    // SAFETY: `user` is the EventSink owned by TransportInner, freed only after
    // gm_transport_free returned; `json` is a valid C string for this call.
    let sink = unsafe { &*(user as *const EventSink) };
    let text = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let payload: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let event = match kind {
        ffi::GM_EVENT_CONNECT => TransportEvent::Connect {
            request,
            dtls_parameters: payload,
        },
        ffi::GM_EVENT_PRODUCE => TransportEvent::Produce {
            request,
            kind: payload["kind"].as_str().unwrap_or_default().to_owned(),
            rtp_parameters: payload["rtpParameters"].clone(),
            app_data: payload["appData"].clone(),
        },
        ffi::GM_EVENT_CONNECTION_STATE => TransportEvent::ConnectionState(
            payload["state"].as_str().unwrap_or_default().to_owned(),
        ),
        _ => return,
    };
    // A dropped receiver only means nobody listens any more.
    let _ = sink.0.lock().map(|tx| tx.send(event));
}

struct TransportInner {
    ptr: NonNull<ffi::gm_transport>,
    sink: *mut EventSink,
    device: Device,
    id: String,
    direction: Direction,
    /// libmediasoupclient transports are not thread-safe; responses do not
    /// take this lock so a blocked produce/consume can be answered.
    lock: Mutex<()>,
}
// SAFETY: native calls are serialized by `lock` except gm_transport_respond,
// which the core guards itself.
unsafe impl Send for TransportInner {}
unsafe impl Sync for TransportInner {}
impl Drop for TransportInner {
    fn drop(&mut self) {
        // SAFETY: last reference (producers/consumers hold Arcs); the sink is
        // freed after the native transport can no longer call back.
        unsafe {
            ffi::gm_transport_free(self.ptr.as_ptr());
            drop(Box::from_raw(self.sink));
        }
    }
}

/// mediasoup send or receive transport.
#[derive(Clone)]
pub struct Transport(Arc<TransportInner>);

impl Transport {
    /// `params` is the server `transport` result
    /// (`id`, `iceParameters`, `iceCandidates`, `dtlsParameters`) plus optional
    /// `iceServers` and `iceTransportPolicy`.
    pub fn new(
        device: &Device,
        direction: Direction,
        params: &Value,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let (tx, rx) = mpsc::channel();
        let sink = Box::into_raw(Box::new(EventSink(Mutex::new(tx))));
        let text = match json_arg(params) {
            Ok(text) => text,
            Err(error) => {
                // SAFETY: just allocated above, never shared.
                drop(unsafe { Box::from_raw(sink) });
                return Err(error);
            }
        };
        let raw_direction = match direction {
            Direction::Send => ffi::GM_SEND,
            Direction::Recv => ffi::GM_RECV,
        };
        let ptr = {
            let _guard = device.0.lock.lock().unwrap();
            // SAFETY: live device; sink outlives the native transport.
            unsafe {
                ffi::gm_device_create_transport(
                    device.0.ptr.as_ptr(),
                    raw_direction,
                    text.as_ptr(),
                    on_event,
                    sink as *mut c_void,
                )
            }
        };
        let Some(ptr) = NonNull::new(ptr) else {
            let error = last_error();
            // SAFETY: creation failed, the core holds no reference to the sink.
            drop(unsafe { Box::from_raw(sink) });
            return Err(error);
        };
        // SAFETY: live transport.
        let id = borrowed_str(unsafe { ffi::gm_transport_id(ptr.as_ptr()) });
        Ok((
            Self(Arc::new(TransportInner {
                ptr,
                sink,
                device: device.clone(),
                id,
                direction,
                lock: Mutex::new(()),
            })),
            rx,
        ))
    }

    pub fn id(&self) -> &str {
        &self.0.id
    }

    pub fn direction(&self) -> Direction {
        self.0.direction
    }

    pub fn device(&self) -> &Device {
        &self.0.device
    }

    /// Answers a [`TransportEvent::Connect`] or [`TransportEvent::Produce`].
    pub fn respond(&self, request: u64, result: std::result::Result<Value, String>) -> Result<()> {
        let (ok, err) = match result {
            Ok(value) => (Some(json_arg(&value)?), None),
            Err(message) => (None, Some(cstring(&message)?)),
        };
        // SAFETY: live transport; exactly one non-null argument.
        check(unsafe {
            ffi::gm_transport_respond(
                self.0.ptr.as_ptr(),
                request,
                ok.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                err.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            )
        })
    }

    pub fn restart_ice(&self, ice_parameters: &Value) -> Result<()> {
        let ice = json_arg(ice_parameters)?;
        let _guard = self.0.lock.lock().unwrap();
        // SAFETY: live transport, valid C string.
        check(unsafe { ffi::gm_transport_restart_ice(self.0.ptr.as_ptr(), ice.as_ptr()) })
    }

    pub fn stats(&self) -> Result<Value> {
        let _guard = self.0.lock.lock().unwrap();
        // SAFETY: live transport.
        owned_json(unsafe { ffi::gm_transport_stats(self.0.ptr.as_ptr()) })
    }

    /// Blocks until the server answered the resulting Connect/Produce events,
    /// so call it off the thread that handles them.
    pub fn produce(&self, source: &Source, options: &Value) -> Result<Producer> {
        let options = json_arg(options)?;
        let ptr = {
            let _guard = self.0.lock.lock().unwrap();
            // SAFETY: live transport and source.
            unsafe {
                ffi::gm_transport_produce(self.0.ptr.as_ptr(), source.raw(), options.as_ptr())
            }
        };
        let ptr = NonNull::new(ptr).ok_or_else(last_error)?;
        // SAFETY: live producer.
        let id = borrowed_str(unsafe { ffi::gm_producer_id(ptr.as_ptr()) });
        Ok(Producer {
            ptr,
            id,
            transport: self.clone(),
            _source: source.clone(),
        })
    }

    /// `params`: server `consumer` announcement as
    /// `{"id","producerId","kind","rtpParameters","appData"?}`. Blocks like
    /// [`Transport::produce`] on the first consumer (Connect).
    pub fn consume(&self, params: &Value) -> Result<Consumer> {
        let params = json_arg(params)?;
        let ptr = {
            let _guard = self.0.lock.lock().unwrap();
            // SAFETY: live transport, valid C string.
            unsafe { ffi::gm_transport_consume(self.0.ptr.as_ptr(), params.as_ptr()) }
        };
        let ptr = NonNull::new(ptr).ok_or_else(last_error)?;
        // SAFETY: live consumer.
        let id = borrowed_str(unsafe { ffi::gm_consumer_id(ptr.as_ptr()) });
        Ok(Consumer {
            ptr,
            id,
            transport: self.clone(),
            sink: None,
        })
    }
}

struct SourceInner {
    ptr: NonNull<ffi::gm_source>,
    _engine: Engine,
    /// The video sink the native side calls (double box: it holds a thin
    /// pointer to the inner box). Also keeps the calls that change the sink
    /// or its limits apart.
    sink: Mutex<Option<Box<VideoSink>>>,
}
// SAFETY: sources are libwebrtc tracks, internally synchronized; the sink is
// only touched under its mutex.
unsafe impl Send for SourceInner {}
unsafe impl Sync for SourceInner {}
impl Drop for SourceInner {
    fn drop(&mut self) {
        // SAFETY: last reference; producers hold an Arc to their source.
        // gm_source_free removes a sink that is still set and returns once
        // it no longer runs; `sink` drops after this body.
        unsafe { ffi::gm_source_free(self.ptr.as_ptr()) }
    }
}

/// Local capture track.
#[derive(Clone)]
pub struct Source(Arc<SourceInner>);

impl Source {
    /// Microphone with the web client's processing modes. `options`, all
    /// optional: `{"processingMode": "enhanced|browser|original",
    /// "echoCancellation", "noiseSuppression", "autoGainControl": bool,
    /// "inputGain": 0..2}`. Processing is per engine; the newest microphone
    /// sets it.
    pub fn microphone(engine: &Engine, options: &Value) -> Result<Self> {
        let options = json_arg(options)?;
        // SAFETY: live engine, NUL-terminated JSON.
        Self::wrap(engine, unsafe {
            ffi::gm_source_new_microphone(engine.raw(), options.as_ptr())
        })
    }

    pub fn test_pattern(engine: &Engine, width: u32, height: u32, fps: u32) -> Result<Self> {
        let arg = |v: u32| i32::try_from(v).map_err(|_| Error::Invalid("test pattern size"));
        // SAFETY: live engine; bounds are checked natively.
        let ptr = unsafe {
            ffi::gm_source_new_test_pattern(engine.raw(), arg(width)?, arg(height)?, arg(fps)?)
        };
        Self::wrap(engine, ptr)
    }

    /// Screen or window chosen in the desktop's own picker (Linux:
    /// xdg-desktop-portal + PipeWire). Returns while the picker is still open;
    /// poll [`Source::state`] until it leaves `pending`. Not available on
    /// Windows yet: an error there.
    ///
    /// `options`: `{"type"?: "any|screen|window", "fps"?: 30, "cursor"?: true,
    /// "contentHint"?: "detail|text|motion"}`.
    pub fn screen(engine: &Engine, options: &Value) -> Result<Self> {
        let options = json_arg(options)?;
        // SAFETY: live engine, NUL-terminated JSON.
        Self::wrap(engine, unsafe {
            ffi::gm_source_new_screen(engine.raw(), options.as_ptr())
        })
    }

    /// Sound of other applications (source audio), separate from the
    /// microphone: `{"app"?: id from [`Engine::audio_apps`]}`; without an
    /// id, every application but this one. Not available on Windows yet: an
    /// error there, and [`Engine::audio_apps`] lists nothing.
    pub fn app_audio(engine: &Engine, options: &Value) -> Result<Self> {
        let options = json_arg(options)?;
        // SAFETY: live engine, NUL-terminated JSON.
        Self::wrap(engine, unsafe {
            ffi::gm_source_new_app_audio(engine.raw(), options.as_ptr())
        })
    }

    /// Camera at the closest format it supports: `{"device"?: id, "width"?,
    /// "height"?, "fps"?}` (default: first camera, 1280x720 at 30).
    pub fn camera(engine: &Engine, options: &Value) -> Result<Self> {
        let options = json_arg(options)?;
        // SAFETY: live engine, NUL-terminated JSON.
        Self::wrap(engine, unsafe {
            ffi::gm_source_new_camera(engine.raw(), options.as_ptr())
        })
    }

    /// `{"state": "pending|live|cancelled|ended|failed", ...}`; microphone and
    /// test pattern are always `live`.
    pub fn state(&self) -> Result<Value> {
        // SAFETY: live source.
        owned_json(unsafe { ffi::gm_source_state(self.raw()) })
    }

    /// Like `MediaStreamTrack.enabled`: a disabled source keeps its
    /// producers and sends silence or black frames.
    pub fn set_enabled(&self, enabled: bool) -> Result<()> {
        // SAFETY: live source.
        check(unsafe { ffi::gm_source_set_enabled(self.raw(), enabled as i32) })
    }

    /// Hands each frame of a local video source (camera, screen, test
    /// pattern) to `sink` as it goes to the encoders: I420, black while the
    /// source is disabled, and at the size and rate an encoder has the
    /// source step down to (a weak uplink), not the capture's. `sink` runs
    /// on the capture thread and holds it up, so it copies what it needs and
    /// returns; it must not call back into this source. `None` removes it;
    /// the previous sink is not called again once this returns. A sink keeps
    /// the source capturing without a producer. Fails for audio sources.
    pub fn set_video_sink(&self, sink: Option<VideoSink>) -> Result<()> {
        let mut sink = sink.map(Box::new);
        let (callback, user) = sink_arguments(&mut sink);
        let mut held = self.0.sink.lock().unwrap();
        // SAFETY: live source; `user` stays alive in `held` until replaced
        // through this call, which waits for a running call; the mutex keeps
        // calls for one source from overlapping.
        check(unsafe { ffi::gm_source_set_video_sink(self.raw(), callback, user) })?;
        *held = sink;
        Ok(())
    }

    /// What the source's sink gets at most; kept across sinks.
    pub fn set_video_sink_limits(&self, limits: VideoSinkLimits) -> Result<()> {
        let limits = limits.raw()?;
        let _held = self.0.sink.lock().unwrap();
        // SAFETY: live source, valid limits; no other sink call overlaps.
        check(unsafe { ffi::gm_source_set_video_sink_limits(self.raw(), &limits) })
    }

    fn wrap(engine: &Engine, ptr: *mut ffi::gm_source) -> Result<Self> {
        NonNull::new(ptr)
            .map(|ptr| {
                Self(Arc::new(SourceInner {
                    ptr,
                    _engine: engine.clone(),
                    sink: Mutex::new(None),
                }))
            })
            .ok_or_else(last_error)
    }

    fn raw(&self) -> *mut ffi::gm_source {
        self.0.ptr.as_ptr()
    }
}

pub struct Producer {
    ptr: NonNull<ffi::gm_producer>,
    id: String,
    transport: Transport,
    _source: Source,
}
// SAFETY: producer calls go through libmediasoupclient, which marshals onto
// libwebrtc threads; the handle is not shared (no Sync).
unsafe impl Send for Producer {}
impl Drop for Producer {
    fn drop(&mut self) {
        let _guard = self.transport.0.lock.lock().unwrap();
        // SAFETY: owned producer, freed before its transport.
        unsafe { ffi::gm_producer_free(self.ptr.as_ptr()) }
    }
}

impl Producer {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn rtp_parameters(&self) -> Result<Value> {
        // SAFETY: live producer.
        owned_json(unsafe { ffi::gm_producer_rtp_parameters(self.ptr.as_ptr()) })
    }

    pub fn set_paused(&self, paused: bool) -> Result<()> {
        // SAFETY: live producer.
        check(unsafe { ffi::gm_producer_pause(self.ptr.as_ptr(), paused as i32) })
    }

    /// Swaps the source, keeping the producer id (mediasoup `replaceTrack`).
    pub fn replace_source(&mut self, source: &Source) -> Result<()> {
        {
            let _guard = self.transport.0.lock.lock().unwrap();
            // SAFETY: live producer and source; the old source stays alive in
            // `self._source` until the call returned.
            check(unsafe { ffi::gm_producer_replace_source(self.ptr.as_ptr(), source.raw()) })?;
        }
        self._source = source.clone();
        Ok(())
    }

    /// Sender encodings like `RTCRtpSender.getParameters()`:
    /// `{"encodings":[{"active","maxBitrate"?,"maxFramerate"?,
    /// "scaleResolutionDownBy"?,"priority"?,"networkPriority"?}]}`.
    pub fn parameters(&self) -> Result<Value> {
        // SAFETY: live producer.
        owned_json(unsafe { ffi::gm_producer_get_parameters(self.ptr.as_ptr()) })
    }

    /// Updates encodings by index; `null` clears `maxBitrate`/`maxFramerate`.
    pub fn set_parameters(&self, parameters: &Value) -> Result<()> {
        let parameters = json_arg(parameters)?;
        // SAFETY: live producer, NUL-terminated JSON.
        check(unsafe { ffi::gm_producer_set_parameters(self.ptr.as_ptr(), parameters.as_ptr()) })
    }

    pub fn stats(&self) -> Result<Value> {
        // SAFETY: live producer.
        owned_json(unsafe { ffi::gm_producer_stats(self.ptr.as_ptr()) })
    }
}

/// A video frame in I420, borrowed for the duration of the sink call.
pub struct VideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub stride_y: usize,
    pub stride_u: usize,
    pub stride_v: usize,
    /// Clockwise degrees to rotate for display: 0, 90, 180 or 270.
    pub rotation: u32,
    pub timestamp_us: i64,
    /// Size of the picture before the sink's limits scaled it down; `width`
    /// and `height` when they did not.
    pub source_width: u32,
    pub source_height: u32,
}

/// Receives a video consumer's decoded frames on a decoder thread, or a
/// local video source's frames on its capture thread.
pub type VideoSink = Box<dyn FnMut(&VideoFrame<'_>) + Send>;

/// What a video sink gets at most; `0` leaves a value unlimited. A picture
/// larger than `max_width` x `max_height` (as displayed, after rotation) is
/// scaled down to fit, aspect kept, to even dimensions; it is never scaled
/// up. Frames that arrive faster than `max_fps` are dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoSinkLimits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
}

impl VideoSinkLimits {
    fn raw(self) -> Result<ffi::gm_video_sink_limits> {
        let arg = |v: u32| c_int::try_from(v).map_err(|_| Error::Invalid("video sink limit"));
        Ok(ffi::gm_video_sink_limits {
            max_width: arg(self.max_width)?,
            max_height: arg(self.max_height)?,
            max_fps: arg(self.max_fps)?,
        })
    }
}

/// The native arguments for a boxed sink: the callback and its user data.
fn sink_arguments(sink: &mut Option<Box<VideoSink>>) -> (ffi::gm_video_frame_fn, *mut c_void) {
    match sink.as_mut() {
        Some(sink) => (
            Some(video_sink_trampoline),
            &mut **sink as *mut VideoSink as *mut c_void,
        ),
        None => (None, std::ptr::null_mut()),
    }
}

unsafe extern "C" fn video_sink_trampoline(user: *mut c_void, frame: *const ffi::gm_video_frame) {
    // SAFETY: `user` is the boxed sink the consumer or source keeps alive
    // until it is removed; the native side serializes calls and `frame` is
    // valid for this call.
    let (sink, frame) = unsafe { (&mut *(user as *mut VideoSink), &*frame) };
    let (Ok(width), Ok(height)) = (u32::try_from(frame.width), u32::try_from(frame.height)) else {
        return;
    };
    let rows = height as usize;
    let chroma_rows = rows.div_ceil(2);
    let stride = |s: c_int| usize::try_from(s).unwrap_or(0);
    let (stride_y, stride_u, stride_v) = (
        stride(frame.stride_y),
        stride(frame.stride_u),
        stride(frame.stride_v),
    );
    if frame.y.is_null() || frame.u.is_null() || frame.v.is_null() {
        return;
    }
    // SAFETY: I420 planes of the given strides and heights.
    let frame = unsafe {
        VideoFrame {
            width,
            height,
            y: std::slice::from_raw_parts(frame.y, stride_y * rows),
            u: std::slice::from_raw_parts(frame.u, stride_u * chroma_rows),
            v: std::slice::from_raw_parts(frame.v, stride_v * chroma_rows),
            stride_y,
            stride_u,
            stride_v,
            rotation: u32::try_from(frame.rotation).unwrap_or(0),
            timestamp_us: frame.timestamp_us,
            source_width: u32::try_from(frame.source_width).unwrap_or(width),
            source_height: u32::try_from(frame.source_height).unwrap_or(height),
        }
    };
    // A panicking sink must not unwind into C++.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(&frame)));
}

pub struct Consumer {
    ptr: NonNull<ffi::gm_consumer>,
    id: String,
    transport: Transport,
    // Double box: the native side holds a thin pointer to the inner box.
    sink: Option<Box<VideoSink>>,
}
// SAFETY: see Producer.
unsafe impl Send for Consumer {}
impl Drop for Consumer {
    fn drop(&mut self) {
        if self.sink.is_some() {
            let _ = self.set_video_sink(None);
        }
        let _guard = self.transport.0.lock.lock().unwrap();
        // SAFETY: owned consumer, freed before its transport.
        unsafe { ffi::gm_consumer_free(self.ptr.as_ptr()) }
    }
}

impl Consumer {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn set_paused(&self, paused: bool) -> Result<()> {
        // SAFETY: live consumer.
        check(unsafe { ffi::gm_consumer_pause(self.ptr.as_ptr(), paused as i32) })
    }

    /// Playback volume of an audio consumer, 0..2.
    pub fn set_volume(&self, volume: f64) -> Result<()> {
        // SAFETY: live consumer.
        check(unsafe { ffi::gm_consumer_set_volume(self.ptr.as_ptr(), volume) })
    }

    /// Hands each decoded frame of a video consumer to `sink` (on a decoder
    /// thread); `None` removes it. The previous sink is not called again
    /// once this returns.
    pub fn set_video_sink(&mut self, sink: Option<VideoSink>) -> Result<()> {
        let mut sink = sink.map(Box::new);
        let (callback, user) = sink_arguments(&mut sink);
        // SAFETY: live consumer; `user` stays alive in `self.sink` until
        // replaced through this call, which waits for running calls.
        check(unsafe { ffi::gm_consumer_set_video_sink(self.ptr.as_ptr(), callback, user) })?;
        self.sink = sink;
        Ok(())
    }

    /// What the consumer's sink gets at most; kept across sinks.
    pub fn set_video_sink_limits(&self, limits: VideoSinkLimits) -> Result<()> {
        let limits = limits.raw()?;
        // SAFETY: live consumer, valid limits.
        check(unsafe { ffi::gm_consumer_set_video_sink_limits(self.ptr.as_ptr(), &limits) })
    }

    /// `{"framesReceived","width","height"}` for video, `{"audioLevel",
    /// "samplesPlayed"}` for audio, plus libwebrtc stats in `rtc`.
    pub fn stats(&self) -> Result<Value> {
        // SAFETY: live consumer.
        owned_json(unsafe { ffi::gm_consumer_stats(self.ptr.as_ptr()) })
    }
}
