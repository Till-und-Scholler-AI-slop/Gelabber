//! Calls for the native client: joining a voice channel, the media socket
//! (`/media/ws`, docs/native/protocol-gateway.md), mediasoup transports
//! through gelabber-media-core, and the state the interface draws. It is the
//! Rust counterpart of web/src/voice/session.ts and mediasoupConnection.ts.
//!
//! The interface reads [`Call::state`] each frame and calls the commands;
//! nothing here blocks the calling thread.
//!
//! `GELABBER_FAKE_MEDIA=1` runs without devices, like Chromium's fake
//! devices: no sound card (the core's dummy audio, a silent microphone) and
//! a test pattern for the camera and a Live.
#![cfg_attr(not(feature = "media"), allow(dead_code))]

pub mod logic;
#[cfg(feature = "media")]
mod media;
pub mod ptt;
mod socket;
mod sounds;
mod synth;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use gelabber_client::{Client, GatewayEvent};
use parking_lot::{Mutex, RwLock};
use uuid::Uuid;

pub use logic::{MAX_PLAYBACK_VOLUME, ProcessingMode};

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

/// One video in the call (a camera or a Live), for [`Call::frame`]. Stable
/// per person and kind, also across reconnects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoKey(pub u64);

impl VideoKey {
    pub fn camera(user: Uuid) -> Self {
        Self(logic::video_key(user, logic::Kind::V))
    }

    pub fn live(user: Uuid) -> Self {
        Self(logic::video_key(user, logic::Kind::S))
    }
}

/// A person in the call, the own seat included.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Participant {
    pub user: Uuid,
    pub me: bool,
    pub muted: bool,
    pub deafened: bool,
    /// Above the speaking threshold right now (the green ring); never while
    /// muted or deafened.
    pub speaking: bool,
    pub camera: Option<VideoKey>,
    pub live: Option<VideoKey>,
    /// The Live carries sound.
    pub live_audio: bool,
    /// This client watches the person's Live.
    pub watching: bool,
    /// Playback volume, 1.0 = 100 %, up to [`MAX_PLAYBACK_VOLUME`].
    pub volume: f32,
    /// Muted for this client only ("Lokal stummschalten").
    pub locally_muted: bool,
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
    /// The own Live sends application sound.
    pub live_audio: bool,
    pub participants: Vec<Participant>,
    /// 0..=4 bars, as the web's connection indicator; 0 = not measured.
    pub quality: u8,
    pub devices: Devices,
    /// Lives can be watched (the media connection is up).
    pub watch_supported: bool,
    /// Push-to-talk is on and its key (or the delay after it) lets the
    /// microphone send.
    pub push_to_talk_open: bool,
    /// The call whose connection was lost, to offer joining it again.
    pub lost_call: Option<(Uuid, Uuid)>,
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
    OtherUnmute,
    LiveStart,
    LiveEnd,
    CameraOn,
    CameraOff,
    Message,
    ViewerJoin,
    ViewerLeave,
}

/// The media settings a call follows (web voice/settings.ts), set by the
/// interface from its saved preferences with [`Call::set_settings`].
#[derive(Clone, Debug, PartialEq)]
pub struct CallSettings {
    pub processing_mode: ProcessingMode,
    pub echo_cancellation: bool,
    pub noise_suppression: bool,
    pub auto_gain_control: bool,
    /// Mic-Gain, 1.0 = 100 %, up to 2.0.
    pub input_gain: f64,
    /// Device ids; "" is the system default.
    pub input: String,
    pub output: String,
    pub camera: String,
    /// "Gespräche", up to 4.0.
    pub output_volume: f64,
    /// "Stream-Ton".
    pub stream_volume: f64,
    pub stream_muted: bool,
    /// `balanced`, `economy`, `detail` or `<height>p<fps>`.
    pub camera_profile: String,
    pub screen_profile: String,
    pub economy_mode: bool,
    /// All videos together, bit/s; 0 = no limit.
    pub video_upload_limit: f64,
    /// The application whose sound a Live sends; "" = every one but this.
    pub source_audio_app: String,
    pub call_sounds: bool,
    pub call_sound_volume: f64,
    pub other_mute_sounds: bool,
    pub message_sound: bool,
    pub push_to_talk: bool,
    pub ptt_release_delay_ms: u64,
}

impl Default for CallSettings {
    fn default() -> Self {
        Self {
            processing_mode: ProcessingMode::Enhanced,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            input_gain: 1.0,
            input: String::new(),
            output: String::new(),
            camera: String::new(),
            output_volume: 1.0,
            stream_volume: 1.0,
            stream_muted: false,
            camera_profile: "balanced".into(),
            screen_profile: "balanced".into(),
            economy_mode: false,
            video_upload_limit: 0.0,
            source_audio_app: String::new(),
            call_sounds: true,
            call_sound_volume: 0.5,
            other_mute_sounds: false,
            message_sound: true,
            push_to_talk: false,
            ptt_release_delay_ms: ptt::RELEASE_DELAY_DEFAULT_MS,
        }
    }
}

impl CallSettings {
    fn sounds(&self) -> sounds::SoundSettings {
        sounds::SoundSettings {
            enabled: self.call_sounds,
            volume: self.call_sound_volume,
            output_volume: self.output_volume,
            other_mute: self.other_mute_sounds,
        }
    }
}

/// One person's playback level here (web voice/userVolume.ts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UserVolume {
    pub volume: f64,
    pub muted: bool,
}

impl Default for UserVolume {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
        }
    }
}

/// What a tile asked for a video ([`Call::request_video`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct VideoRequest {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

pub(crate) struct Inner {
    client: Client,
    state: RwLock<CallState>,
    settings: RwLock<CallSettings>,
    volumes: Mutex<HashMap<Uuid, UserVolume>>,
    frames: Mutex<HashMap<VideoKey, Arc<Frame>>>,
    /// Per key, whether its current sink may still put frames.
    sinks: Mutex<HashMap<VideoKey, Arc<AtomicBool>>>,
    /// The last measured level per person, 0..1 RMS.
    levels: Mutex<HashMap<Uuid, f32>>,
    requests: Mutex<HashMap<VideoKey, VideoRequest>>,
    notices: Mutex<Vec<String>>,
    player: sounds::Player,
    gate: Mutex<ptt::Gate>,
    runtime: OnceLock<tokio::runtime::Runtime>,
    #[cfg(feature = "media")]
    media: media::Media,
}

impl Drop for Inner {
    fn drop(&mut self) {
        #[cfg(feature = "media")]
        self.media.shutdown(&self.client);
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl Inner {
    pub(crate) fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("gelabber-call")
                .enable_all()
                .build()
                .expect("call runtime")
        })
    }

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    pub(crate) fn settings(&self) -> CallSettings {
        self.settings.read().clone()
    }

    pub(crate) fn write(&self, change: impl FnOnce(&mut CallState)) {
        let changed = {
            let mut state = self.state.write();
            let before = state.clone();
            change(&mut state);
            *state != before
        };
        if changed {
            self.client.repaint();
        }
    }

    pub(crate) fn read<R>(&self, read: impl FnOnce(&CallState) -> R) -> R {
        read(&self.state.read())
    }

    pub(crate) fn notify(&self, text: impl Into<String>) {
        self.notices.lock().push(text.into());
        self.client.repaint();
    }

    pub(crate) fn play(&self, sound: Sound, remote: Option<bool>) {
        let settings = self.settings.read().sounds();
        self.player.play(synth::Cue::of(sound), remote, settings);
    }

    pub(crate) fn set_sounds_deafened(&self, deafened: bool) {
        self.player.set_deafened(deafened);
    }

    pub(crate) fn stop_sounds(&self) {
        self.player.stop();
    }

    pub(crate) fn volume_of(&self, user: Uuid) -> UserVolume {
        self.volumes.lock().get(&user).copied().unwrap_or_default()
    }

    pub(crate) fn request_of(&self, key: VideoKey) -> Option<VideoRequest> {
        self.requests.lock().get(&key).copied()
    }

    /// A new sink for `key`: its frames count while the flag holds; an
    /// older sink of the key is cut off.
    pub(crate) fn open_sink(&self, key: VideoKey) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(true));
        let _frames = self.frames.lock();
        if let Some(old) = self.sinks.lock().insert(key, flag.clone()) {
            old.store(false, Ordering::Release);
        }
        flag
    }

    /// Takes a frame from a sink that is still `flag`'s; checked under the
    /// frames' lock, so a closed sink never puts a picture back.
    pub(crate) fn put_frame(&self, key: VideoKey, flag: &AtomicBool, frame: Frame) {
        {
            let mut frames = self.frames.lock();
            if !flag.load(Ordering::Acquire) {
                return;
            }
            frames.insert(key, Arc::new(frame));
        }
        self.client.repaint();
    }

    /// Ends the sink `flag` and, while it is the key's, the key's picture.
    pub(crate) fn close_sink(&self, key: VideoKey, flag: &Arc<AtomicBool>) {
        let mut frames = self.frames.lock();
        flag.store(false, Ordering::Release);
        let mut sinks = self.sinks.lock();
        if sinks.get(&key).is_some_and(|f| Arc::ptr_eq(f, flag)) {
            sinks.remove(&key);
            frames.remove(&key);
        }
    }

    /// No more pictures of `key`, from any sink.
    pub(crate) fn drop_frame(&self, key: VideoKey) {
        let removed = {
            let mut frames = self.frames.lock();
            if let Some(flag) = self.sinks.lock().remove(&key) {
                flag.store(false, Ordering::Release);
            }
            frames.remove(&key).is_some()
        };
        if removed {
            self.client.repaint();
        }
    }

    pub(crate) fn drop_frames(&self) {
        let mut frames = self.frames.lock();
        for (_, flag) in self.sinks.lock().drain() {
            flag.store(false, Ordering::Release);
        }
        frames.clear();
        self.levels.lock().clear();
    }

    pub(crate) fn set_levels(&self, levels: HashMap<Uuid, f32>) {
        *self.levels.lock() = levels;
    }

    pub(crate) fn gate_open(&self) -> bool {
        self.gate.lock().is_open()
    }
}

/// Whether `GELABBER_FAKE_MEDIA=1` asks for a call without devices.
pub fn fake_media() -> bool {
    std::env::var("GELABBER_FAKE_MEDIA").is_ok_and(|v| v == "1" || v == "true")
}

/// Cheap to clone; one per app.
#[derive(Clone)]
pub struct Call {
    inner: Arc<Inner>,
}

impl Call {
    pub fn new(client: Client) -> Self {
        let call = Self {
            inner: Arc::new(Inner {
                client: client.clone(),
                state: RwLock::new(CallState {
                    push_to_talk_open: true,
                    ..CallState::default()
                }),
                settings: RwLock::new(CallSettings::default()),
                volumes: Mutex::new(HashMap::new()),
                frames: Mutex::new(HashMap::new()),
                sinks: Mutex::new(HashMap::new()),
                levels: Mutex::new(HashMap::new()),
                requests: Mutex::new(HashMap::new()),
                notices: Mutex::new(Vec::new()),
                player: sounds::Player::default(),
                gate: Mutex::new(ptt::Gate::default()),
                runtime: OnceLock::new(),
                #[cfg(feature = "media")]
                media: media::Media::default(),
            }),
        };
        let weak: Weak<Inner> = Arc::downgrade(&call.inner);
        client.set_voice_hook(Some(Arc::new(move |event: &GatewayEvent| {
            if let Some(inner) = weak.upgrade() {
                Self { inner }.on_gateway(event);
            }
        })));
        call
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

    /// Replaces the whole state, as a running call would (interface tests).
    #[cfg(feature = "testing")]
    pub fn set_state(&self, state: CallState) {
        *self.inner.state.write() = state;
    }

    /// True while joining or a device change is in flight.
    pub fn busy(&self) -> bool {
        matches!(self.inner.state.read().status, CallStatus::Connecting)
    }

    /// German messages for toasts since the last call (refused camera,
    /// lost connection, …).
    pub fn take_notices(&self) -> Vec<String> {
        std::mem::take(&mut *self.inner.notices.lock())
    }

    pub fn settings(&self) -> CallSettings {
        self.inner.settings()
    }

    /// New media settings; a running call follows them (volumes, devices,
    /// microphone processing, push-to-talk, video limits).
    pub fn set_settings(&self, settings: CallSettings) {
        let previous = std::mem::replace(&mut *self.inner.settings.write(), settings.clone());
        if previous == settings {
            return;
        }
        let sounds_changed = previous.call_sounds != settings.call_sounds
            || previous.call_sound_volume != settings.call_sound_volume
            || previous.output_volume != settings.output_volume;
        if sounds_changed {
            self.inner.stop_sounds();
        }
        self.sync_push_to_talk();
        #[cfg(feature = "media")]
        media::settings_changed(&self.inner, &previous, &settings);
    }

    /// Joins (or moves to) a voice channel: the gateway seat at once, the
    /// media connection in the background.
    pub fn join(&self, server: Uuid, channel: Uuid) {
        #[cfg(feature = "media")]
        {
            media::join(&self.inner, server, channel);
            self.sync_push_to_talk();
        }
        #[cfg(not(feature = "media"))]
        {
            let _ = (server, channel);
            self.inner
                .write(|s| s.status = CallStatus::Failed(logic::NO_MEDIA_CORE.into()));
            self.inner.notify(logic::NO_MEDIA_CORE);
        }
    }

    /// Tests: drops the media socket as a network loss would, so the call
    /// rebuilds its seat.
    #[doc(hidden)]
    pub fn simulate_media_loss(&self) {
        #[cfg(feature = "media")]
        media::simulate_media_loss(&self.inner);
    }

    /// Puts away the note of a call whose connection was lost or failed.
    pub fn dismiss_lost_call(&self) {
        self.inner.write(|s| {
            if s.channel.is_none() {
                s.status = CallStatus::Idle;
                s.lost_call = None;
            }
        });
    }

    pub fn leave(&self) {
        #[cfg(feature = "media")]
        media::leave(&self.inner);
        self.sync_push_to_talk();
    }

    /// Mute on or off; unmuting while deafened also undeafens.
    pub fn set_muted(&self, muted: bool) {
        #[cfg(feature = "media")]
        media::set_muted(&self.inner, muted);
        #[cfg(not(feature = "media"))]
        let _ = muted;
    }

    /// Deafen also mutes; undeafening restores the mute from before.
    pub fn set_deafened(&self, deafened: bool) {
        #[cfg(feature = "media")]
        media::set_deafened(&self.inner, deafened);
        #[cfg(not(feature = "media"))]
        let _ = deafened;
    }

    pub fn toggle_mute(&self) {
        let (muted, deafened) = self.with_state(|s| (s.muted, s.deafened));
        self.set_muted(!(muted || deafened));
    }

    pub fn toggle_deafen(&self) {
        let deafened = self.with_state(|s| s.deafened);
        self.set_deafened(!deafened);
    }

    pub fn set_camera(&self, on: bool) {
        #[cfg(feature = "media")]
        media::set_camera(&self.inner, on);
        #[cfg(not(feature = "media"))]
        let _ = on;
    }

    pub fn start_live(&self, source: LiveSource) {
        #[cfg(feature = "media")]
        media::start_live(&self.inner, source);
        #[cfg(not(feature = "media"))]
        let _ = source;
    }

    pub fn stop_live(&self) {
        #[cfg(feature = "media")]
        media::stop_live(&self.inner);
    }

    /// Watches someone's Live in this call (`w`), or stops.
    pub fn watch(&self, user: Uuid, on: bool) {
        #[cfg(feature = "media")]
        media::watch(&self.inner, user, on);
        #[cfg(not(feature = "media"))]
        let _ = (user, on);
    }

    /// A person's playback volume here, 1.0 = 100 %, up to 4.0.
    pub fn set_volume(&self, user: Uuid, volume: f32) {
        {
            let mut volumes = self.inner.volumes.lock();
            let entry = volumes.entry(user).or_default();
            entry.volume = logic::clamp_playback_volume(f64::from(volume));
        }
        self.volumes_changed();
    }

    /// Mutes a person for this client only.
    pub fn set_user_muted(&self, user: Uuid, muted: bool) {
        self.inner.volumes.lock().entry(user).or_default().muted = muted;
        self.volumes_changed();
    }

    /// Every person's level, to keep across starts.
    pub fn volumes(&self) -> HashMap<Uuid, UserVolume> {
        self.inner.volumes.lock().clone()
    }

    /// Levels kept from before (no effect on people not in the call yet).
    pub fn set_volumes(&self, volumes: HashMap<Uuid, UserVolume>) {
        *self.inner.volumes.lock() = volumes;
        self.volumes_changed();
    }

    fn volumes_changed(&self) {
        #[cfg(feature = "media")]
        media::apply_volumes(&self.inner);
        let volumes = self.inner.volumes.lock().clone();
        self.inner.write(|s| {
            for p in &mut s.participants {
                let v = volumes.get(&p.user).copied().unwrap_or_default();
                p.volume = v.volume as f32;
                p.locally_muted = v.muted;
            }
        });
    }

    pub fn set_input(&self, device: Option<String>) {
        let mut settings = self.settings();
        settings.input = device.unwrap_or_default();
        self.set_settings(settings);
    }

    pub fn set_output(&self, device: Option<String>) {
        let mut settings = self.settings();
        settings.output = device.unwrap_or_default();
        self.set_settings(settings);
    }

    pub fn set_camera_device(&self, device: Option<String>) {
        let mut settings = self.settings();
        settings.camera = device.unwrap_or_default();
        self.set_settings(settings);
    }

    /// Lists microphones, speakers and cameras again into
    /// [`CallState::devices`].
    pub fn refresh_devices(&self) {
        #[cfg(feature = "media")]
        media::refresh_devices(&self.inner);
    }

    /// Push-to-talk key down or up (window keys or a global hook).
    pub fn push_to_talk(&self, pressed: bool) {
        let now = Instant::now();
        let (changed, pending) = {
            let mut gate = self.inner.gate.lock();
            (gate.set_held(pressed, now), gate.pending())
        };
        if changed {
            self.gate_changed();
        }
        if let Some(at) = pending {
            let call = Arc::downgrade(&self.inner);
            self.inner.runtime().spawn(async move {
                tokio::time::sleep_until(at.into()).await;
                if let Some(inner) = call.upgrade() {
                    let closed = inner.gate.lock().tick(Instant::now());
                    if closed {
                        Self { inner }.gate_changed();
                    }
                }
            });
        }
    }

    /// Whether the push-to-talk key is held down right now.
    pub fn push_to_talk_held(&self) -> bool {
        self.inner.gate.lock().held()
    }

    fn sync_push_to_talk(&self) {
        let settings = self.settings();
        let joined = self.with_state(|s| s.channel.is_some());
        let delay = (joined && settings.push_to_talk).then(|| {
            Duration::from_millis(settings.ptt_release_delay_ms.min(ptt::RELEASE_DELAY_MAX_MS))
        });
        let changed = self.inner.gate.lock().configure(delay);
        if changed {
            self.gate_changed();
        }
    }

    fn gate_changed(&self) {
        let open = self.inner.gate_open();
        self.inner.write(|s| s.push_to_talk_open = open);
        #[cfg(feature = "media")]
        media::apply_local_audio(&self.inner);
    }

    pub fn play_sound(&self, sound: Sound) {
        if sound == Sound::Message {
            let settings = self.settings();
            let message = sounds::SoundSettings {
                enabled: settings.message_sound,
                ..settings.sounds()
            };
            self.inner
                .player
                .play(synth::Cue::Message, Some(false), message);
            return;
        }
        // The test tone ("Testton abspielen") is not an event of a call.
        self.inner.play(sound, Some(false));
    }

    /// The last measured sound level per person in the call, 0..1 RMS
    /// (own microphone after processing, others as they play out); for
    /// meters and diagnostics. `speaking` in the state is what tiles show.
    pub fn levels(&self) -> HashMap<Uuid, f32> {
        self.inner.levels.lock().clone()
    }

    /// The newest picture of `key`, if any arrived.
    pub fn frame(&self, key: VideoKey) -> Option<Arc<Frame>> {
        self.inner.frames.lock().get(&key).cloned()
    }

    /// The size and rate a tile shows `key` at, so the core scales and
    /// drops frames before they reach Rust. Physical pixels; 0 = no limit.
    pub fn request_video(&self, key: VideoKey, width: u32, height: u32, fps: u32) {
        let request = VideoRequest { width, height, fps };
        let previous = self.inner.requests.lock().insert(key, request);
        #[cfg(feature = "media")]
        if previous != Some(request) {
            media::apply_request(&self.inner, key, request);
        }
        #[cfg(not(feature = "media"))]
        let _ = previous;
    }

    fn on_gateway(&self, event: &GatewayEvent) {
        #[cfg(feature = "media")]
        media::on_gateway(&self.inner, event);
        #[cfg(not(feature = "media"))]
        let _ = event;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_keys_differ_per_kind() {
        let user = Uuid::new_v4();
        assert_ne!(VideoKey::camera(user), VideoKey::live(user));
    }

    #[cfg(not(feature = "media"))]
    #[test]
    fn without_the_core_a_call_says_why() {
        let call = Call::new(Client::new("http://127.0.0.1:9").unwrap());
        call.join(Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(
            call.state().status,
            CallStatus::Failed(logic::NO_MEDIA_CORE.into())
        );
        assert_eq!(call.take_notices(), vec![logic::NO_MEDIA_CORE.to_owned()]);
    }

    #[test]
    fn push_to_talk_gates_only_in_a_call() {
        let call = Call::new(Client::new("http://127.0.0.1:9").unwrap());
        call.set_settings(CallSettings {
            push_to_talk: true,
            ptt_release_delay_ms: 0,
            ..CallSettings::default()
        });
        assert!(call.state().push_to_talk_open, "no call, no gate");
    }
}
