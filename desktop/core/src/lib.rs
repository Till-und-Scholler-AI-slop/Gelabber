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

use serde_json::Value;
use std::{
    ffi::{CStr, CString, c_char, c_void},
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
    let text = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    unsafe { ffi::gm_string_free(ptr) };
    serde_json::from_str(&text).map_err(|e| Error::Native(format!("invalid JSON from core: {e}")))
}

fn borrowed_str(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: points into a live native object owned by the caller's handle.
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
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
    /// System audio devices (PulseAudio API, served by pipewire-pulse on Omarchy).
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
    Connect { request: u64, dtls_parameters: Value },
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
            unsafe { ffi::gm_transport_produce(self.0.ptr.as_ptr(), source.raw(), options.as_ptr()) }
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
        })
    }
}

struct SourceInner {
    ptr: NonNull<ffi::gm_source>,
    _engine: Engine,
}
// SAFETY: sources are libwebrtc tracks, internally synchronized.
unsafe impl Send for SourceInner {}
unsafe impl Sync for SourceInner {}
impl Drop for SourceInner {
    fn drop(&mut self) {
        // SAFETY: last reference; producers hold an Arc to their source.
        unsafe { ffi::gm_source_free(self.ptr.as_ptr()) }
    }
}

/// Local capture track.
#[derive(Clone)]
pub struct Source(Arc<SourceInner>);

impl Source {
    pub fn microphone(engine: &Engine) -> Result<Self> {
        // SAFETY: live engine.
        Self::wrap(engine, unsafe { ffi::gm_source_new_microphone(engine.raw()) })
    }

    pub fn test_pattern(engine: &Engine, width: u32, height: u32, fps: u32) -> Result<Self> {
        let arg = |v: u32| i32::try_from(v).map_err(|_| Error::Invalid("test pattern size"));
        // SAFETY: live engine; bounds are checked natively.
        let ptr = unsafe {
            ffi::gm_source_new_test_pattern(engine.raw(), arg(width)?, arg(height)?, arg(fps)?)
        };
        Self::wrap(engine, ptr)
    }

    fn wrap(engine: &Engine, ptr: *mut ffi::gm_source) -> Result<Self> {
        NonNull::new(ptr)
            .map(|ptr| {
                Self(Arc::new(SourceInner {
                    ptr,
                    _engine: engine.clone(),
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

    pub fn stats(&self) -> Result<Value> {
        // SAFETY: live producer.
        owned_json(unsafe { ffi::gm_producer_stats(self.ptr.as_ptr()) })
    }
}

pub struct Consumer {
    ptr: NonNull<ffi::gm_consumer>,
    id: String,
    transport: Transport,
}
// SAFETY: see Producer.
unsafe impl Send for Consumer {}
impl Drop for Consumer {
    fn drop(&mut self) {
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

    /// `{"framesReceived","width","height"}` for video plus libwebrtc stats in `rtc`.
    pub fn stats(&self) -> Result<Value> {
        // SAFETY: live consumer.
        owned_json(unsafe { ffi::gm_consumer_stats(self.ptr.as_ptr()) })
    }
}
