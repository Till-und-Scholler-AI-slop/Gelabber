//! Calls for the native client: joining a voice channel, the media socket
//! (`/media/ws`, docs/native/protocol-gateway.md), mediasoup transports
//! through gelabber-media-core, and the state the interface draws. It is the
//! Rust counterpart of web/src/voice/session.ts and mediasoupConnection.ts.
//!
//! The interface reads [`Call::state`] each frame and calls the commands;
//! nothing here blocks the calling thread.

use std::sync::Arc;

use gelabber_client::Client;
use parking_lot::RwLock;
use uuid::Uuid;

/// Where a call stands, as the call bar shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum CallStatus {
    #[default]
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    /// German text for the person, e.g. a refused microphone.
    Failed(String),
}

/// One video in the call (a camera or a Live), for [`Call::frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoKey(pub u64);

/// A person in the call, the own seat included.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Participant {
    pub user: Uuid,
    pub me: bool,
    pub muted: bool,
    pub deafened: bool,
    /// Above the speaking threshold right now (the green ring).
    pub speaking: bool,
    pub camera: Option<VideoKey>,
    pub live: Option<VideoKey>,
    /// The Live carries sound.
    pub live_audio: bool,
    /// This client watches the person's Live.
    pub watching: bool,
    /// Playback volume, 1.0 = 100 %, up to 2.0.
    pub volume: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Devices {
    pub inputs: Vec<Device>,
    pub outputs: Vec<Device>,
    pub cameras: Vec<Device>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub camera: Option<String>,
}

/// What to share as a Live.
#[derive(Clone, Debug, PartialEq)]
pub enum LiveSource {
    /// The system picker (ScreenCast portal on Linux, WGC on Windows).
    Screen { with_audio: bool },
}

/// Everything the interface draws about the call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallState {
    pub status: CallStatus,
    pub server: Option<Uuid>,
    pub channel: Option<Uuid>,
    pub muted: bool,
    pub deafened: bool,
    pub camera: bool,
    pub live: bool,
    pub participants: Vec<Participant>,
    /// 0..=4 bars, as the web's connection indicator.
    pub quality: u8,
    pub devices: Devices,
}

/// The newest decoded picture of a video, I420.
#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub stride_y: u32,
    pub stride_u: u32,
    pub stride_v: u32,
    /// Clockwise degrees: 0, 90, 180 or 270.
    pub rotation: u32,
    /// Counts up with every new picture.
    pub serial: u64,
}

/// Sounds the call plays (web voice/callSoundSynth.ts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    JoinSelf,
    LeaveSelf,
    JoinOther,
    LeaveOther,
    Mute,
    Unmute,
    Deafen,
    Undeafen,
    OtherMute,
    LiveStart,
    LiveEnd,
    CameraOn,
    CameraOff,
    Message,
}

struct Inner {
    client: Client,
    state: RwLock<CallState>,
}

/// Cheap to clone; one per app.
#[derive(Clone)]
pub struct Call {
    inner: Arc<Inner>,
}

impl Call {
    pub fn new(client: Client) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                state: RwLock::new(CallState::default()),
            }),
        }
    }

    pub fn client(&self) -> &Client {
        &self.inner.client
    }

    pub fn state(&self) -> CallState {
        self.inner.state.read().clone()
    }

    pub fn with_state<R>(&self, read: impl FnOnce(&CallState) -> R) -> R {
        read(&self.inner.state.read())
    }

    /// True while joining or a device change is in flight.
    pub fn busy(&self) -> bool {
        matches!(self.inner.state.read().status, CallStatus::Connecting)
    }

    pub fn join(&self, _server: Uuid, _channel: Uuid) {}
    pub fn leave(&self) {}
    pub fn set_muted(&self, _muted: bool) {}
    pub fn set_deafened(&self, _deafened: bool) {}
    pub fn set_camera(&self, _on: bool) {}
    pub fn start_live(&self, _source: LiveSource) {}
    pub fn stop_live(&self) {}
    pub fn watch(&self, _user: Uuid, _on: bool) {}
    pub fn set_volume(&self, _user: Uuid, _volume: f32) {}
    pub fn set_input(&self, _device: Option<String>) {}
    pub fn set_output(&self, _device: Option<String>) {}
    pub fn set_camera_device(&self, _device: Option<String>) {}
    /// Push-to-talk key down or up.
    pub fn push_to_talk(&self, _pressed: bool) {}
    pub fn play_sound(&self, _sound: Sound) {}

    /// The newest picture of `key`, if any arrived.
    pub fn frame(&self, _key: VideoKey) -> Option<Arc<Frame>> {
        None
    }

    /// The size and rate a tile shows `key` at, so the core scales and
    /// drops frames before they reach Rust and the SFU sends a fitting layer.
    pub fn request_video(&self, _key: VideoKey, _width: u32, _height: u32, _fps: u32) {}
}
