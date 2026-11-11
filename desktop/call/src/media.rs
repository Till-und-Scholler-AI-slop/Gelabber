//! The call on the native media core: web/src/voice/session.ts (seat,
//! sounds, recovery) and mediasoupConnection.ts (device, transports,
//! producers, consumers) over gelabber-media-core.
//!
//! Ordering rules from the Tauri app's media.rs: `produce` and `consume`
//! block until the transport's CONNECT/PRODUCE requests are answered, so
//! they run on blocking threads and the answers come from the runtime;
//! every request gets an answer (a refusal once the seat is gone), or the
//! transport's lock would stay held; a consumer gets its video sink before
//! it is resumed (`consumerReady`), so its first key frame is not lost.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use gelabber_client::{Client, GatewayEvent, ServerFrame, SigFrame, SigKind};
use gelabber_media_core as core;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::logic::{self, Kind};
use crate::socket::{MediaError, MediaSocket, SocketEvent};
use crate::{
    CallSettings, CallState, CallStatus, Device, Devices, Frame, Inner, LiveSound, LiveSource,
    MicStatus, MicTestPhase, Participant, Sound, VideoKey, VideoRequest,
};

/// What a sink gets until a tile says how large it draws.
const DEFAULT_REQUEST: VideoRequest = VideoRequest {
    width: 1280,
    height: 720,
    fps: 0,
};
/// What a released key's sink gets: next to nothing, until a tile asks.
pub(crate) const RELEASED: VideoRequest = VideoRequest {
    width: 2,
    height: 2,
    fps: 1,
};
/// How often a Live's capture is asked whether it still runs.
const SCREEN_POLL: Duration = Duration::from_millis(1000);
/// How often an open desktop picker is asked for an answer.
const PICKER_POLL: Duration = Duration::from_millis(250);
/// How often the quality bars are measured, on a speaking poll.
const QUALITY_PERIOD: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct Media {
    engine: OnceLock<Result<core::Engine, String>>,
    session: Mutex<Option<Arc<Session>>>,
    /// AV1 decodes smoothly here; asked once (it scans GStreamer).
    av1_decoding: OnceLock<bool>,
    hardware: OnceLock<logic::HardwareVideo>,
}

impl Media {
    /// The engine, made on first use: blocking, so off the UI thread.
    fn engine(&self) -> Result<core::Engine, String> {
        self.engine
            .get_or_init(|| {
                let audio = if crate::fake_media() {
                    core::Audio::Dummy
                } else {
                    core::Audio::Default
                };
                core::Engine::new(audio).map_err(|e| e.to_string())
            })
            .clone()
    }

    fn codecs(&self) -> (bool, logic::HardwareVideo) {
        let codecs = || core::video_codecs().unwrap_or(Value::Null);
        let mut probed = None;
        let av1 = *self.av1_decoding.get_or_init(|| {
            let c = probed.get_or_insert_with(codecs);
            c["av1"]["hardwareDecoder"] == true
        });
        let hardware = *self.hardware.get_or_init(|| {
            let c = probed.get_or_insert_with(codecs);
            logic::HardwareVideo::from_codecs(c)
        });
        (av1, hardware)
    }

    fn current(&self) -> Option<Arc<Session>> {
        self.session.lock().clone()
    }

    /// The app goes away in a call: leave like the web's page unload.
    pub(crate) fn shutdown(&self, client: &Client) {
        if let Some(session) = self.session.lock().take() {
            session.stop();
            client.voice_leave();
        }
    }
}

fn me(client: &Client) -> Option<Uuid> {
    client.with_state(|s| s.me().map(|u| u.id))
}

fn spawn(inner: &Inner, work: impl Future<Output = ()> + Send + 'static) {
    inner.runtime().spawn(work);
}

async fn blocking<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(job).await.ok()
}

// ------------------------------------------------------------- commands

pub(crate) fn join(inner: &Arc<Inner>, server: Uuid, channel: Uuid) {
    let client = inner.client().clone();
    let Some(me) = me(&client) else { return };
    let previous = inner.media.session.lock().take();
    let (was_joined, same) = inner.read(|s| {
        (
            s.channel.is_some(),
            s.server == Some(server) && s.channel == Some(channel),
        )
    });
    let (muted, deafened) = if was_joined {
        inner.read(|s| (s.muted || s.deafened, s.deafened))
    } else {
        (false, false)
    };
    let pre_deafen_muted = previous
        .as_ref()
        .is_some_and(|p| p.st.lock().pre_deafen_muted);
    if let Some(previous) = previous {
        previous.stop();
    }
    inner.drop_frames();
    inner.set_sounds_deafened(deafened);
    if !same {
        inner.diagnostics().clear();
    }
    inner.write(|s| {
        *s = CallState {
            status: CallStatus::Connecting,
            server: Some(server),
            channel: Some(channel),
            muted,
            deafened,
            devices: std::mem::take(&mut s.devices),
            push_to_talk_open: s.push_to_talk_open,
            ..CallState::default()
        }
    });
    // The gateway seat first, in socket order before any media work.
    client.voice_join_flags(server, channel, Some((muted, deafened)));
    let session = Arc::new(Session {
        call: Arc::downgrade(inner),
        server,
        channel,
        me,
        fake: crate::fake_media(),
        st: Mutex::new(St {
            awaiting_join: true,
            sound_on_join: !same,
            pre_deafen_muted,
            mic_epoch: Uuid::new_v4(),
            ..St::default()
        }),
        stopped: AtomicBool::new(false),
        gating: Mutex::new(()),
        seats: AtomicU64::new(0),
    });
    *inner.media.session.lock() = Some(session.clone());
    session.publish_participants();
    spawn(inner, async move { session.start().await });
}

pub(crate) fn leave(inner: &Arc<Inner>) {
    let Some(session) = inner.media.session.lock().take() else {
        return;
    };
    let had_joined = !session.st.lock().awaiting_join;
    let deafened = inner.read(|s| s.deafened);
    inner.stop_sounds();
    if had_joined && !deafened {
        inner.play(Sound::LeaveSelf, None);
    }
    inner.set_sounds_deafened(false);
    session.stop();
    inner.client().voice_leave();
    inner.drop_frames();
    inner.write(|s| {
        *s = CallState {
            devices: std::mem::take(&mut s.devices),
            push_to_talk_open: s.push_to_talk_open,
            ..CallState::default()
        }
    });
}

/// Kicked or banned: the client let the gateway seat go, so the call goes
/// too; the client's notice says why.
fn end_taken(inner: &Arc<Inner>, session: &Arc<Session>) {
    let current = inner
        .media
        .current()
        .is_some_and(|s| Arc::ptr_eq(&s, session));
    if current {
        leave(inner);
        sync_push_to_talk(inner);
    }
}

/// The push-to-talk gate as the call now stands (off outside a call), as
/// `Call::leave` sets it.
fn sync_push_to_talk(inner: &Arc<Inner>) {
    crate::Call {
        inner: inner.clone(),
    }
    .sync_push_to_talk();
}

pub(crate) fn set_muted(inner: &Arc<Inner>, muted: bool) {
    let Some(session) = inner.media.current() else {
        return;
    };
    let Some((muted, deafened)) = inner.read(|s| logic::mute_change(s.muted, s.deafened, muted))
    else {
        return;
    };
    if !deafened {
        session.st.lock().pre_deafen_muted = muted;
    }
    inner.write(|s| {
        s.muted = muted;
        s.deafened = deafened;
    });
    inner.set_sounds_deafened(deafened);
    inner.play(if muted { Sound::Mute } else { Sound::Unmute }, None);
    inner.client().set_voice_muted(muted);
    apply_local_audio(inner);
    session.publish_participants();
}

pub(crate) fn set_deafened(inner: &Arc<Inner>, deafened: bool) {
    let Some(session) = inner.media.current() else {
        return;
    };
    let (was_muted, was_deafened) = inner.read(|s| (s.muted, s.deafened));
    if was_deafened == deafened {
        return;
    }
    let muted = {
        let mut st = session.st.lock();
        if deafened {
            st.pre_deafen_muted = was_muted;
        }
        deafened || st.pre_deafen_muted
    };
    inner.write(|s| {
        s.muted = muted;
        s.deafened = deafened;
    });
    inner.set_sounds_deafened(deafened);
    inner.play(
        if deafened {
            Sound::Deafen
        } else {
            Sound::Undeafen
        },
        None,
    );
    inner.client().set_voice_deafened(deafened);
    apply_local_audio(inner);
    session.publish_participants();
}

/// Microphone on or off as mute, deafen and push-to-talk say
/// (`applyLocalAudio`), and every playback volume.
pub(crate) fn apply_local_audio(inner: &Arc<Inner>) {
    if let Some(session) = inner.media.current() {
        session.apply_local_audio();
        session.apply_volumes();
    }
}

pub(crate) fn apply_volumes(inner: &Arc<Inner>) {
    if let Some(session) = inner.media.current() {
        session.apply_volumes();
    }
}

pub(crate) fn set_camera(inner: &Arc<Inner>, on: bool) {
    let Some(session) = inner.media.current() else {
        return;
    };
    if on == inner.read(|s| s.camera) {
        return;
    }
    if on {
        inner.write(|s| s.camera = true);
        spawn(inner, async move { session.start_camera().await });
    } else {
        spawn(inner, async move { session.stop_video(Kind::V).await });
    }
}

pub(crate) fn start_live(inner: &Arc<Inner>, source: LiveSource) {
    let Some(session) = inner.media.current() else {
        return;
    };
    if inner.read(|s| s.live) {
        return;
    }
    inner.write(|s| s.live = true);
    let LiveSource::Screen { with_audio } = source;
    spawn(inner, async move { session.start_screen(with_audio).await });
}

pub(crate) fn stop_live(inner: &Arc<Inner>) {
    let Some(session) = inner.media.current() else {
        return;
    };
    spawn(inner, async move { session.stop_video(Kind::S).await });
}

pub(crate) fn watch(inner: &Arc<Inner>, user: Uuid, on: bool) {
    let Some(session) = inner.media.current() else {
        return;
    };
    if user == session.me {
        return;
    }
    {
        let mut st = session.st.lock();
        let changed = if on {
            st.watching.insert(user)
        } else {
            st.watching.remove(&user)
        };
        if !changed {
            return;
        }
        // Queued under the lock, so the requests keep the order of the
        // changes; without a seat the next one sends what is watched.
        if let Some(seat) = &st.seat {
            seat.queue_watch(user, on);
        }
    }
    if !on {
        // Off drops the stream and its sound at once.
        session.drop_received(user, &[Kind::S, Kind::Sa]);
        inner.drop_frame(VideoKey::live(user));
    }
    session.publish_participants();
}

pub(crate) fn refresh_devices(inner: &Arc<Inner>) {
    let call = Arc::downgrade(inner);
    spawn(inner, async move {
        let Some(inner) = call.upgrade() else { return };
        let listed = blocking({
            let call = call.clone();
            move || {
                let inner = call.upgrade()?;
                let engine = inner.media.engine().ok()?;
                Some(list_devices(&engine))
            }
        })
        .await
        .flatten();
        if let Some(mut devices) = listed {
            let settings = inner.settings();
            devices.input = Some(settings.input.clone());
            devices.output = Some(settings.output.clone());
            devices.camera = Some(settings.camera.clone());
            inner.write(|s| s.devices = devices);
        }
    });
}

fn list_devices(engine: &core::Engine) -> Devices {
    let entries = |list: &Value| -> Vec<Device> {
        list.as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|d| Device {
                        id: d["id"].as_str().unwrap_or_default().to_owned(),
                        label: d["name"].as_str().unwrap_or_default().to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let audio = engine.audio_devices().unwrap_or(Value::Null);
    let mut cameras = entries(&engine.video_devices().unwrap_or(Value::Null));
    if crate::fake_media() && cameras.is_empty() {
        cameras.push(Device {
            id: "test-pattern".into(),
            label: "Testbild".into(),
        });
    }
    Devices {
        inputs: entries(&audio["inputs"]),
        outputs: entries(&audio["outputs"]),
        cameras,
        // Without PipeWire there are none to list, as in the web's catch.
        apps: entries(&engine.audio_apps().unwrap_or(Value::Null)),
        ..Devices::default()
    }
}

/// `selectInput`: the chosen microphone, else the system's when that one
/// does not open; returns the one in use ("" the system's).
fn select_input(engine: &core::Engine, id: &str) -> Result<String, String> {
    match engine.configure_audio(&json!({ "input": id })) {
        Ok(()) => Ok(id.to_owned()),
        Err(e) if id.is_empty() => Err(e.to_string()),
        Err(e) => {
            tracing::debug!("microphone {id}: {e}");
            engine
                .configure_audio(&json!({ "input": "" }))
                .map_err(|e| e.to_string())?;
            Ok(String::new())
        }
    }
}

pub(crate) fn retry_microphone(inner: &Arc<Inner>) {
    if let Some(session) = inner.media.current() {
        spawn(inner, async move { session.retry_microphone().await });
    }
}

/// How often the microphone test reads the meters (`POLL_MS`).
const MIC_TEST_POLL: Duration = Duration::from_millis(80);
/// A capture that delivers no block for this long has failed (`STALL_MS`).
const MIC_TEST_STALL: Duration = Duration::from_millis(2000);

/// `createNativeMicrophoneTest`: the core captures and processes without
/// a call; its meters every 80 ms until the turn moves on.
pub(crate) fn start_mic_test(inner: &Arc<Inner>, turn: u64) {
    let call = Arc::downgrade(inner);
    spawn(inner, async move {
        let Some(inner) = call.upgrade() else { return };
        let settings = inner.settings();
        let fail = |inner: &Inner, text: &str| {
            if inner.mic_test_turn() == turn {
                inner.set_mic_test(|t| t.phase = MicTestPhase::Error(text.into()));
            }
        };
        let options = json!({
            "processingMode": settings.processing_mode.as_str(),
            "inputGain": settings.input_gain.clamp(0.0, 2.0),
        });
        let call2 = call.clone();
        let chosen = settings.input.clone();
        let fake = crate::fake_media();
        let started = blocking(move || {
            let inner = call2.upgrade()?;
            let engine = inner.media.engine().ok()?;
            // The chosen microphone first, as a call takes it.
            let input = if fake {
                Ok(chosen)
            } else {
                select_input(&engine, &chosen)
            };
            Some(input.and_then(|input| {
                engine
                    .monitor_audio(Some(&options))
                    .map(|()| (engine, input))
                    .map_err(|e| e.to_string())
            }))
        })
        .await
        .flatten();
        let (engine, device) = match started {
            Some(Ok(started)) => started,
            _ => return fail(&inner, logic::mic_test_failed()),
        };
        if inner.mic_test_turn() != turn {
            let _ = blocking(move || engine.monitor_audio(None)).await;
            return;
        }
        inner.set_mic_test(|t| {
            t.phase = MicTestPhase::Active;
            t.device = Some(device);
        });
        drop(inner);
        let mut blocks = None;
        let mut moved = Instant::now();
        loop {
            tokio::time::sleep(MIC_TEST_POLL).await;
            let Some(inner) = call.upgrade() else { return };
            if inner.mic_test_turn() != turn {
                return;
            }
            let reader = engine.clone();
            let levels = blocking(move || reader.audio_levels()).await;
            let Some(Ok(levels)) = levels else {
                fail(&inner, logic::mic_test_no_levels());
                let engine = engine.clone();
                let _ = blocking(move || engine.monitor_audio(None)).await;
                return;
            };
            let now = Instant::now();
            let count = levels["blocks"].as_u64();
            if count != blocks {
                blocks = count;
                moved = now;
                inner.set_mic_test(|t| {
                    t.input = levels["input"].as_f64().unwrap_or(0.0);
                    t.processed = levels["processed"].as_f64().unwrap_or(0.0);
                    t.clipping = levels["clipping"].as_bool().unwrap_or(false);
                });
            } else if now.duration_since(moved) >= MIC_TEST_STALL {
                fail(&inner, logic::mic_test_stalled());
                let engine = engine.clone();
                let _ = blocking(move || engine.monitor_audio(None)).await;
                return;
            }
        }
    });
}

/// Ends the monitor; a call's own capture is not touched by it.
pub(crate) fn stop_mic_test(inner: &Arc<Inner>) {
    let call = Arc::downgrade(inner);
    spawn(inner, async move {
        let _ = blocking(move || {
            let inner = call.upgrade()?;
            let engine = inner.media.engine.get()?.as_ref().ok()?.clone();
            engine.monitor_audio(None).ok()
        })
        .await;
    });
}

/// False when no session runs (a state set by an interface test).
pub(crate) fn simulate_media_loss(inner: &Arc<Inner>) -> bool {
    let Some(session) = inner.media.current() else {
        return false;
    };
    if let Some(seat) = session.seat() {
        seat.sock.close();
    }
    true
}

pub(crate) fn apply_request(inner: &Arc<Inner>, key: VideoKey, request: VideoRequest) {
    if let Some(session) = inner.media.current() {
        session.apply_request(key, request);
    }
}

pub(crate) fn settings_changed(inner: &Arc<Inner>, previous: &CallSettings, next: &CallSettings) {
    let call = Arc::downgrade(inner);
    let (previous, next) = (previous.clone(), next.clone());
    spawn(inner, async move {
        let Some(inner) = call.upgrade() else { return };
        // Devices and gain are engine-wide, also outside a call.
        if previous.input != next.input
            || previous.output != next.output
            || previous.input_gain != next.input_gain
        {
            let mut options = json!({ "inputGain": next.input_gain.clamp(0.0, 2.0) });
            if previous.output != next.output {
                options["output"] = next.output.clone().into();
            }
            let engine = inner
                .media
                .engine
                .get()
                .and_then(|e| e.as_ref().ok())
                .cloned();
            if let Some(engine) = engine {
                let input = (previous.input != next.input && !crate::fake_media())
                    .then(|| next.input.clone());
                let result = blocking(move || {
                    let selected = input.map(|id| select_input(&engine, &id));
                    (selected, engine.configure_audio(&options))
                })
                .await;
                if let Some((selected, configured)) = result {
                    if let Err(e) = configured {
                        tracing::debug!("audio devices: {e}");
                    }
                    match selected {
                        // A running capture moves along with the engine.
                        Some(Ok(input)) => inner.write(|s| {
                            if s.mic_input.is_some() {
                                s.mic_input = Some(input);
                            }
                        }),
                        Some(Err(e)) => tracing::debug!("microphone: {e}"),
                        None => {}
                    }
                }
            }
            inner.write(|s| {
                s.devices.input = Some(next.input.clone());
                s.devices.output = Some(next.output.clone());
            });
        }
        inner.write(|s| s.devices.camera = Some(next.camera.clone()));
        let Some(session) = inner.media.current() else {
            return;
        };
        session.apply_volumes();
        let recapture = previous.processing_mode != next.processing_mode
            || previous.echo_cancellation != next.echo_cancellation
            || previous.noise_suppression != next.noise_suppression
            || previous.auto_gain_control != next.auto_gain_control;
        if recapture {
            session.restart_microphone().await;
        }
        let camera_changed =
            previous.camera != next.camera || previous.camera_profile != next.camera_profile;
        if camera_changed && inner.read(|s| s.camera) {
            session.start_camera().await;
        }
        let limits_changed = previous.video_upload_limit != next.video_upload_limit
            || previous.economy_mode != next.economy_mode
            || previous.camera_profile != next.camera_profile
            || previous.screen_profile != next.screen_profile;
        if limits_changed && let Some(seat) = session.seat() {
            seat.apply_parameters().await;
        }
        let app_changed = previous.source_audio_app != next.source_audio_app;
        if previous.share_source_audio != next.share_source_audio || app_changed {
            session
                .apply_source_audio_choice(next.share_source_audio, app_changed)
                .await;
        }
    });
}

pub(crate) fn on_gateway(inner: &Arc<Inner>, event: &GatewayEvent) {
    let Some(session) = inner.media.current() else {
        return;
    };
    if session.seat_taken() {
        return end_taken(inner, &session);
    }
    match event {
        GatewayEvent::Ready => session.on_ready(),
        GatewayEvent::Frame(ServerFrame::Sig(sig)) => session.on_sig(sig),
        GatewayEvent::Frame(ServerFrame::Err { e, s, c }) => session.on_gateway_error(e, *s, *c),
        _ => {}
    }
}

// --------------------------------------------------------------- session

#[derive(Clone, Copy)]
struct Peer {
    since: Instant,
    muted: bool,
}

/// A capture of this client: its source and the epoch it is sent under.
#[derive(Clone)]
struct Local {
    source: core::Source,
    epoch: Uuid,
    height: u32,
}

#[derive(Default)]
struct St {
    awaiting_join: bool,
    sound_on_join: bool,
    republish_on_join: bool,
    /// Awaiting the gateway's answer to `p s`.
    awaiting_screen: bool,
    pre_deafen_muted: bool,
    peers: HashMap<Uuid, Peer>,
    /// What each person publishes, from the gateway.
    pubs: HashMap<Uuid, Vec<Kind>>,
    mic: Option<Local>,
    /// Microphones a switch replaces, per switch: still sending until the
    /// producer took the new one, so gated with it.
    replaced_mics: Vec<(u64, Local)>,
    mic_switch: u64,
    mic_epoch: Uuid,
    camera: Option<Local>,
    screen: Option<Local>,
    screen_audio: Option<Local>,
    /// Bumped by every camera or Live start and stop, so a capture that
    /// answers late knows it is no longer wanted.
    camera_turn: u64,
    screen_turn: u64,
    /// Bumped by every change of the Live's sound while it runs.
    sound_turn: u64,
    seat: Option<Arc<Seat>>,
    /// The media connection under way until it holds its seat.
    connecting: Option<u64>,
    attempts: u32,
    rebuilding: bool,
    watching: HashSet<Uuid>,
    /// Kinds announced on the gateway by this connection.
    announced: HashSet<Kind>,
    /// Per direction, the recovery under way (its token).
    recovery: HashMap<&'static str, u64>,
    recovery_token: u64,
    speaking: logic::Speaking,
    speaking_now: HashSet<Uuid>,
    mic_blocks: Option<u64>,
    /// The fake microphone's first beep.
    beep_origin: Option<Instant>,
    played: HashMap<String, u64>,
    /// Bytes received per voice and when, for fake media's levels.
    received: HashMap<String, (u64, Instant)>,
    packets: (u64, u64),
    stream_error_shown: bool,
    /// A seat was refused (`unauthorized`) and the next one runs on a new
    /// ticket.
    renewed: bool,
    /// When a working seat was revoked unasked, for the bound on rebuilds.
    revocations: Vec<Instant>,
}

pub(crate) struct Session {
    call: Weak<Inner>,
    server: Uuid,
    channel: Uuid,
    me: Uuid,
    fake: bool,
    st: Mutex<St>,
    stopped: AtomicBool,
    /// Held while the microphone's track flag is decided and set.
    gating: Mutex<()>,
    seats: AtomicU64,
}

/// Marks [`St::connecting`] for one `run_seat` until it returns.
struct Connecting<'a>(&'a Session, u64);

impl<'a> Connecting<'a> {
    fn start(session: &'a Session, generation: u64) -> Self {
        session.st.lock().connecting = Some(generation);
        Self(session, generation)
    }
}

impl Drop for Connecting<'_> {
    fn drop(&mut self) {
        let mut st = self.0.st.lock();
        if st.connecting == Some(self.1) {
            st.connecting = None;
        }
    }
}

enum Leave {
    /// The server refused the seat: back to no call, with a message.
    Refused(String),
    /// The connection could not be rebuilt: offer to join again.
    Lost,
    /// Lost, with the media error's own message.
    LostWith(String),
}

impl Session {
    fn inner(&self) -> Option<Arc<Inner>> {
        self.call.upgrade()
    }

    fn live(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
    }

    fn seat(&self) -> Option<Arc<Seat>> {
        self.st.lock().seat.clone()
    }

    /// The client no longer holds this call's gateway seat, which it took
    /// before the own `j` came back (a kick or ban drops it).
    fn seat_taken(&self) -> bool {
        if self.st.lock().awaiting_join {
            return false;
        }
        let Some(inner) = self.inner() else {
            return false;
        };
        let held = inner
            .client()
            .with_state(|s| (s.voice_self.server, s.voice_self.channel));
        held != (Some(self.server), Some(self.channel))
    }

    fn engine(&self) -> Option<core::Engine> {
        self.inner()?
            .media
            .engine
            .get()
            .and_then(|e| e.as_ref().ok())
            .cloned()
    }

    /// Ends the session: closes the seat and every capture.
    fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let (seat, locals) = {
            let mut st = self.st.lock();
            let mut locals: Vec<Local> = [
                st.mic.take(),
                st.camera.take(),
                st.screen.take(),
                st.screen_audio.take(),
            ]
            .into_iter()
            .flatten()
            .collect();
            locals.extend(st.replaced_mics.drain(..).map(|(_, l)| l));
            (st.seat.take(), locals)
        };
        if let Some(seat) = seat {
            seat.close(true);
        }
        // Removing a sink waits for a frame in delivery: off this thread.
        std::thread::spawn(move || {
            for local in &locals {
                let _ = local.source.set_video_sink(None);
            }
            drop(locals);
        });
    }

    async fn start(self: Arc<Self>) {
        let Some(inner) = self.inner() else { return };
        let call = self.call.clone();
        let engine = blocking(move || call.upgrade().map(|i| i.media.engine()))
            .await
            .flatten();
        let engine = match engine {
            Some(Ok(engine)) => engine,
            Some(Err(e)) => {
                tracing::warn!("media engine: {e}");
                self.give_up(Leave::Refused(logic::engine_failed(&e)));
                return;
            }
            None => return,
        };
        if inner.read(|s| s.devices.inputs.is_empty() && s.devices.cameras.is_empty()) {
            refresh_devices(&inner);
        }
        // The output chosen in the settings; the microphone picks its input
        // as it starts.
        let settings = inner.settings();
        if !self.fake && !settings.output.is_empty() {
            let options = json!({
                "output": settings.output,
                "inputGain": settings.input_gain.clamp(0.0, 2.0),
            });
            let engine = engine.clone();
            let _ = blocking(move || engine.configure_audio(&options)).await;
        }
        drop(inner);
        self.run_seat().await;
    }

    /// One media connection: ticket, socket, device, receiving, then what
    /// this client sends. Ends with the seat in place or a rebuild planned.
    async fn run_seat(self: &Arc<Self>) {
        if !self.live() {
            return;
        }
        let Some(inner) = self.inner() else { return };
        let Some(engine) = self.engine() else { return };
        let generation = self.seats.fetch_add(1, Ordering::AcqRel) + 1;
        let _connecting = Connecting::start(self, generation);
        let client = inner.client().clone();
        let ticket = match client.http().media_ticket(self.channel).await {
            Ok(ticket) => ticket,
            Err(error) => {
                let code = error.code();
                let transient = matches!(code, "network" | "timeout" | "internal" | "rate_limited");
                if transient {
                    self.schedule_rebuild(generation);
                } else {
                    self.give_up(Leave::Refused(error.text().into()));
                }
                return;
            }
        };
        let (events_tx, events) = mpsc::unbounded_channel();
        let url = client.http().ws_url(&ticket.media_path);
        let sock = match MediaSocket::connect(&url, events_tx).await {
            Ok(sock) => sock,
            Err(e) => {
                tracing::debug!("media socket: {e}");
                self.schedule_rebuild(generation);
                return;
            }
        };
        let ice_servers = serde_json::to_value(&ticket.ice_servers).unwrap_or(json!([]));
        let joined = sock
            .request(
                "j",
                json!({ "tk": ticket.ticket, "v": logic::MEDIA_PROTOCOL_VERSION }),
            )
            .await;
        let joined = match joined {
            Ok(joined) => joined,
            Err(e) => {
                sock.close();
                self.on_media_error(generation, e.code(), logic::MediaStage::Join);
                return;
            }
        };
        let caps = joined["routerRtpCapabilities"].clone();
        let made = blocking(
            move || -> Result<(core::Device, Value, bool, bool), String> {
                let device = core::Device::new(&engine).map_err(|e| e.to_string())?;
                device.load(&caps).map_err(|e| e.to_string())?;
                let rtp = device.rtp_capabilities().map_err(|e| e.to_string())?;
                let audio = device.can_produce(core::MediaKind::Audio).unwrap_or(false);
                let video = device.can_produce(core::MediaKind::Video).unwrap_or(false);
                Ok((device, rtp, audio, video))
            },
        )
        .await;
        let Some(Ok((device, rtp, can_audio, can_video))) = made else {
            sock.close();
            self.give_up(Leave::Refused(
                logic::media_error_message("negotiation_failed").into(),
            ));
            return;
        };
        let (av1_decoding, hardware) = {
            let call = self.call.clone();
            blocking(move || call.upgrade().map(|i| i.media.codecs()))
                .await
                .flatten()
                .unwrap_or_default()
        };
        let seat = Arc::new(Seat {
            session: Arc::downgrade(self),
            generation,
            sock: sock.clone(),
            device,
            rtp: rtp.clone(),
            can_audio,
            can_video,
            hardware,
            ice_servers,
            send: tokio::sync::Mutex::new(None),
            send_tp: Mutex::new(None),
            recv: Mutex::new(None),
            pause_lock: tokio::sync::Mutex::new(()),
            watch_queue: Mutex::new(std::collections::VecDeque::new()),
            watch_lock: tokio::sync::Mutex::new(()),
            producers: Mutex::new(HashMap::new()),
            consumers: Mutex::new(HashMap::new()),
            removed: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashSet::new()),
            queued: Mutex::new(Vec::new()),
            consume_lock: tokio::sync::Mutex::new(()),
            codecs: Mutex::new(None),
            states: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            runtime: tokio::runtime::Handle::current(),
        });
        {
            let mut st = self.st.lock();
            if !self.live() || self.seats.load(Ordering::Acquire) != generation {
                drop(st);
                seat.close(true);
                return;
            }
            if let Some(old) = st.seat.replace(seat.clone()) {
                old.close(false);
            }
            st.connecting = None;
        }
        tokio::spawn(seat.clone().pump(events));
        let capabilities = logic::receivable_capabilities(&rtp, av1_decoding);
        match sock
            .request("capabilities", json!({ "rtp": capabilities }))
            .await
        {
            Ok(answer) => seat.note_codecs(&answer["codecs"]),
            Err(e) => {
                self.on_media_error(generation, e.code(), logic::MediaStage::Building);
                return;
            }
        }
        let recv = match seat.make_transport(core::Direction::Recv).await {
            Ok(recv) => recv,
            Err(e) => {
                self.on_media_error(generation, e.code(), logic::MediaStage::Building);
                return;
            }
        };
        // Under the queue's lock, so no event is queued after the drain.
        let queued = {
            let mut queued = seat.queued.lock();
            *seat.recv.lock() = Some(recv);
            std::mem::take(&mut *queued)
        };
        for event in queued {
            seat.clone().handle_event(event);
        }
        // The seat holds: a later refusal may renew its ticket once more.
        self.st.lock().renewed = false;
        inner.write(|s| {
            s.status = CallStatus::Connected;
            s.watch_supported = true;
        });
        let watching: Vec<Uuid> = self.st.lock().watching.iter().copied().collect();
        for user in watching {
            let _ = sock
                .request("w", json!({ "u": user, "k": "s", "on": true }))
                .await;
        }
        tokio::spawn(self.clone().ticker(generation));
        self.publish_all(&seat).await;
    }

    /// Sends every capture on a new seat: the microphone (captured on the
    /// first one), the camera, the Live and its sound.
    async fn publish_all(self: &Arc<Self>, seat: &Arc<Seat>) {
        let mic = self.st.lock().mic.clone();
        let mic = match mic {
            Some(mic) => Some(mic),
            None => self.capture_microphone().await,
        };
        if let Some(mic) = mic {
            let paused = self
                .inner()
                .is_some_and(|i| i.read(|s| s.muted || s.deafened));
            self.apply_local_audio();
            match seat.publish(Kind::A, &mic, None, paused).await {
                Ok(()) => self.announce(Kind::A, true),
                Err(e) => return self.mic_not_sent(seat, &e),
            }
        }
        let camera = self.st.lock().camera.clone();
        if let Some(camera) = camera {
            self.send_video(seat, Kind::V, &camera).await;
        }
        let screen = self.st.lock().screen.clone();
        if let Some(screen) = screen
            && self.send_video(seat, Kind::S, &screen).await
        {
            self.send_live_sound(seat, screen.epoch).await;
        }
    }

    /// Sends a camera or Live on `seat` and announces it unless it is
    /// already; whether it went out. A capture stopped meanwhile is taken
    /// back, one the server refuses stops with a word.
    async fn send_video(self: &Arc<Self>, seat: &Arc<Seat>, kind: Kind, local: &Local) -> bool {
        match seat.publish(kind, local, None, false).await {
            Ok(()) => {
                let kept = self.keep_published(seat, kind, local.epoch).await;
                self.announce_once(kept, kind);
                kept
            }
            Err(e) => {
                self.video_not_sent(seat, kind, local.epoch, &e).await;
                false
            }
        }
    }

    /// The Live's sound as it is now (it may have been switched on while
    /// the picture went out), once the picture of `epoch` is on the seat.
    async fn send_live_sound(self: &Arc<Self>, seat: &Arc<Seat>, epoch: Uuid) {
        let audio = self.st.lock().screen_audio.clone();
        let Some(audio) = audio.filter(|a| a.epoch == epoch) else {
            return;
        };
        match seat.publish(Kind::Sa, &audio, None, false).await {
            Ok(()) => {
                let kept = self.keep_published(seat, Kind::Sa, epoch).await;
                self.announce_once(kept, Kind::Sa);
            }
            Err(e) if e.code() == "connection_closed" || seat.closed.load(Ordering::Acquire) => {}
            Err(e) => {
                tracing::warn!("Live sound: {e}");
                let current = self
                    .st
                    .lock()
                    .screen_audio
                    .as_ref()
                    .is_some_and(|a| a.epoch == epoch);
                if current {
                    self.stop_live_sound(LiveSound::Unavailable(Some(e.code().to_owned())))
                        .await;
                }
            }
        }
    }

    /// `p` for a capture that went out and is not announced yet; decided on
    /// what was announced, so overlapping starts neither skip nor repeat it.
    fn announce_once(&self, kept: bool, kind: Kind) {
        if kept && !self.st.lock().announced.contains(&kind) {
            self.announce(kind, true);
        }
    }

    /// After a publication of `kind` under `epoch` came back: whether it is
    /// still the capture wanted. One stopped meanwhile is closed here (its
    /// stop found no producer yet); one replaced meanwhile stays for the
    /// newer capture.
    async fn keep_published(&self, seat: &Seat, kind: Kind, epoch: Uuid) -> bool {
        let (now, newer) = {
            let st = self.st.lock();
            let local = match kind {
                Kind::A => &st.mic,
                Kind::V => &st.camera,
                Kind::S => &st.screen,
                Kind::Sa => &st.screen_audio,
            };
            (
                local.as_ref().is_some_and(|l| l.epoch == epoch),
                local.is_some(),
            )
        };
        let ours = seat
            .producers
            .lock()
            .get(&kind)
            .is_some_and(|h| h.epoch == epoch);
        match logic::published_sound(now && self.live(), newer, ours) {
            logic::PublishedSound::Announce => true,
            logic::PublishedSound::Close => {
                seat.close_source(kind).await;
                false
            }
            logic::PublishedSound::Keep => false,
        }
    }

    /// The microphone could not be sent (`commitMicSend`): a word, then a
    /// new seat. A seat that closed meanwhile has its next one send it.
    fn mic_not_sent(self: &Arc<Self>, seat: &Seat, error: &MediaError) {
        if error.code() == "connection_closed" || seat.closed.load(Ordering::Acquire) {
            return;
        }
        tracing::warn!("microphone not sent: {error}");
        if let Some(inner) = self.inner() {
            let first = !std::mem::replace(&mut self.st.lock().stream_error_shown, true);
            if first {
                inner.notify(logic::media_error_message(error.code()));
            }
        }
        self.schedule_rebuild(seat.generation);
    }

    /// A camera or Live the seat would not take: its capture stops with a
    /// word, as the web's `publish` does.
    async fn video_not_sent(
        self: &Arc<Self>,
        seat: &Seat,
        kind: Kind,
        epoch: Uuid,
        error: &MediaError,
    ) {
        if error.code() == "connection_closed" || seat.closed.load(Ordering::Acquire) {
            return;
        }
        let current = {
            let st = self.st.lock();
            let local = if kind == Kind::S {
                &st.screen
            } else {
                &st.camera
            };
            local.as_ref().is_some_and(|l| l.epoch == epoch)
        };
        if !current || !self.live() {
            return;
        }
        tracing::warn!("{kind:?} not sent: {error}");
        let Some(inner) = self.inner() else { return };
        self.stop_video(kind).await;
        inner.notify(if kind == Kind::S && error.code() == "forbidden" {
            // The seat's ticket predates a go-live grant: only a new one has it.
            logic::live_needs_rejoin()
        } else {
            logic::media_error_message(error.code())
        });
    }

    fn microphone_options(&self) -> Value {
        let settings = self.inner().map(|i| i.settings()).unwrap_or_default();
        logic::microphone_options(
            settings.processing_mode,
            settings.echo_cancellation,
            settings.noise_suppression,
            settings.auto_gain_control,
            settings.input_gain,
        )
    }

    /// Captures the microphone; without one the call goes on listening
    /// (`reportJoinMicFailure`): the bar says so and offers to retry.
    async fn capture_microphone(self: &Arc<Self>) -> Option<Local> {
        let (source, input) = match self.open_microphone().await? {
            Ok(opened) => opened,
            Err(e) => {
                tracing::warn!("microphone: {e}");
                if self.live()
                    && let Some(inner) = self.inner()
                {
                    let denied = logic::mic_refused(&e);
                    inner.write(|s| {
                        s.mic = if denied {
                            MicStatus::Denied
                        } else {
                            MicStatus::Unavailable
                        };
                    });
                    inner.notify(if denied {
                        logic::mic_denied()
                    } else {
                        logic::mic_not_found()
                    });
                }
                return None;
            }
        };
        let mut st = self.st.lock();
        if !self.live() {
            return None;
        }
        let local = Local {
            source,
            epoch: st.mic_epoch,
            height: 0,
        };
        st.mic = Some(local.clone());
        drop(st);
        if let Some(inner) = self.inner() {
            inner.write(|s| {
                s.mic = MicStatus::Live;
                s.mic_input = Some(input);
            });
        }
        Some(local)
    }

    /// The chosen input (else the system's), then the capture on it; the
    /// source and the input it runs on.
    async fn open_microphone(&self) -> Option<Result<(core::Source, String), String>> {
        let engine = self.engine()?;
        let options = self.microphone_options();
        let chosen = self.inner().map(|i| i.settings().input).unwrap_or_default();
        let fake = self.fake;
        blocking(move || {
            // Fake media's dummy audio has no devices to pick.
            let input = if fake {
                chosen
            } else {
                select_input(&engine, &chosen)?
            };
            core::Source::microphone(&engine, &options)
                .map(|source| (source, input))
                .map_err(|e| e.to_string())
        })
        .await
    }

    /// `retryMicrophone`: capture again and send it on the seat there is.
    async fn retry_microphone(self: &Arc<Self>) {
        if self.st.lock().mic.is_some() {
            return;
        }
        let Some(mic) = self.capture_microphone().await else {
            return;
        };
        self.apply_local_audio();
        if let Some(seat) = self.seat() {
            let paused = self
                .inner()
                .is_some_and(|i| i.read(|s| s.muted || s.deafened));
            match seat.publish(Kind::A, &mic, None, paused).await {
                Ok(()) => self.announce(Kind::A, true),
                Err(e) => self.mic_not_sent(&seat, &e),
            }
        }
    }

    /// The microphone again with new processing; the producer keeps its id.
    async fn restart_microphone(self: &Arc<Self>) {
        let (source, input) = match self.open_microphone().await {
            Some(Ok(opened)) => opened,
            Some(Err(e)) => {
                tracing::warn!("microphone switch: {e}");
                // The old capture keeps sending; without one this is a retry.
                if let Some(inner) = self.inner()
                    && self.live()
                {
                    if self.st.lock().mic.is_some() {
                        inner.notify(logic::mic_switch_failed());
                    } else {
                        let denied = logic::mic_refused(&e);
                        inner.write(|s| {
                            s.mic = if denied {
                                MicStatus::Denied
                            } else {
                                MicStatus::Unavailable
                            };
                        });
                    }
                }
                return;
            }
            None => return,
        };
        let (local, switch) = {
            let mut st = self.st.lock();
            if !self.live() {
                drop(st);
                std::thread::spawn(move || drop(source));
                return;
            }
            let epoch = st.mic_epoch;
            let local = Local {
                source,
                epoch,
                height: 0,
            };
            st.mic_switch += 1;
            let switch = st.mic_switch;
            if let Some(previous) = st.mic.replace(local.clone()) {
                st.replaced_mics.push((switch, previous));
            }
            (local, switch)
        };
        if let Some(inner) = self.inner() {
            inner.write(|s| {
                s.mic = MicStatus::Live;
                s.mic_input = Some(input);
            });
        }
        self.apply_local_audio();
        if let Some(seat) = self.seat() {
            let paused = self
                .inner()
                .is_some_and(|i| i.read(|s| s.muted || s.deafened));
            if let Err(e) = seat.publish(Kind::A, &local, None, paused).await {
                self.mic_not_sent(&seat, &e);
            }
        }
        // The producer sends the new one now (or none): the old one goes.
        let replaced: Vec<Local> = {
            let mut st = self.st.lock();
            let (done, kept) = std::mem::take(&mut st.replaced_mics)
                .into_iter()
                .partition(|(s, _)| *s == switch);
            st.replaced_mics = kept;
            done.into_iter().map(|(_, l)| l).collect()
        };
        if !replaced.is_empty() {
            std::thread::spawn(move || drop(replaced));
        }
    }

    async fn start_camera(self: &Arc<Self>) {
        let Some(inner) = self.inner() else { return };
        let Some(engine) = self.engine() else { return };
        let settings = inner.settings();
        let size = logic::capture_size(false, &settings.camera_profile);
        let turn = {
            let mut st = self.st.lock();
            st.camera_turn += 1;
            st.camera_turn
        };
        let fake = self.fake || settings.camera == "test-pattern";
        let device = settings.camera.clone();
        let open = || {
            let (engine, device) = (engine.clone(), device.clone());
            blocking(move || {
                if fake {
                    return core::Source::test_pattern(&engine, size.width, size.height, size.fps);
                }
                let options = json!({
                    "width": size.width, "height": size.height, "fps": size.fps, "device": device,
                });
                core::Source::camera(&engine, &options).or_else(|e| {
                    if device.is_empty() {
                        return Err(e);
                    }
                    let fallback = json!({
                        "width": size.width, "height": size.height, "fps": size.fps, "device": "",
                    });
                    core::Source::camera(&engine, &fallback)
                })
            })
        };
        let mut captured = open().await;
        // A running camera may hold the device (V4L2 opens it once): it lets
        // go, and the new capture is tried once more.
        let mut released = None;
        if !matches!(captured, Some(Ok(_))) && self.st.lock().camera_turn == turn {
            if let Some(Err(e)) = &captured {
                tracing::debug!("camera beside the running one: {e}");
            }
            released = self.release_camera().await;
            if released.is_some() {
                if self.st.lock().camera_turn != turn {
                    return;
                }
                captured = open().await;
            }
        }
        let source = match captured {
            Some(Ok(source)) => source,
            other => {
                if let Some(Err(e)) = other {
                    tracing::warn!("camera: {e}");
                }
                if self.st.lock().camera_turn == turn {
                    // Nothing is captured any more: nothing may be sent.
                    inner.write(|s| s.camera = false);
                    if let Some(seat) = self.seat() {
                        seat.close_source(Kind::V).await;
                    }
                    // Whichever start let the running camera go, its `p v`
                    // is taken back here; a seat rebuild forgets what was
                    // announced, while the gateway keeps it.
                    if released.is_some() || self.st.lock().announced.contains(&Kind::V) {
                        self.announce(Kind::V, false);
                        inner.play(Sound::CameraOff, Some(false));
                    }
                    inner.drop_frame(VideoKey::camera(self.me));
                    self.publish_participants();
                    inner.notify(logic::camera_unavailable());
                }
                return;
            }
        };
        let replacing = {
            let mut st = self.st.lock();
            if st.camera_turn != turn || !self.live() {
                drop(st);
                std::thread::spawn(move || drop(source));
                return;
            }
            let epoch = st
                .camera
                .as_ref()
                .map(|c| c.epoch)
                .or(released)
                .unwrap_or_else(Uuid::new_v4);
            let local = Local {
                source: source.clone(),
                epoch,
                height: size.height,
            };
            st.camera.replace(local).is_some() || released.is_some()
        };
        self.preview(Kind::V, &source);
        if !replacing {
            inner.play(Sound::CameraOn, None);
        }
        self.publish_participants();
        let local = self.st.lock().camera.clone();
        if let (Some(seat), Some(local)) = (self.seat(), local) {
            self.send_video(&seat, Kind::V, &local).await;
        }
    }

    /// Lets the camera's capture go with its producer, its `p v` kept, so
    /// a new capture can open the device; the epoch it ran under.
    async fn release_camera(self: &Arc<Self>) -> Option<Uuid> {
        let local = self.st.lock().camera.take()?;
        if let Some(seat) = self.seat() {
            seat.close_source(Kind::V).await;
        }
        let epoch = local.epoch;
        let _ = blocking(move || {
            let _ = local.source.set_video_sink(None);
            drop(local);
        })
        .await;
        Some(epoch)
    }

    async fn start_screen(self: &Arc<Self>, with_audio: bool) {
        let Some(inner) = self.inner() else { return };
        let Some(engine) = self.engine() else { return };
        let settings = inner.settings();
        let size = logic::capture_size(true, &settings.screen_profile);
        let turn = {
            let mut st = self.st.lock();
            st.screen_turn += 1;
            st.screen_turn
        };
        let fake = self.fake;
        let fake_screen = fake && crate::fake_screen();
        let picked = {
            let engine = engine.clone();
            blocking(move || {
                if fake_screen {
                    return core::Source::test_pattern(&engine, size.width, size.height, size.fps);
                }
                let options = json!({
                    "type": "any", "fps": size.fps, "cursor": true, "contentHint": "detail",
                });
                core::Source::screen(&engine, &options)
            })
            .await
        };
        let fail = |reason: Option<String>| {
            if self.st.lock().screen_turn == turn {
                inner.write(|s| s.live = false);
                if let Some(reason) = reason {
                    inner.notify(logic::screen_failed(&reason));
                }
            }
        };
        let source = match picked {
            Some(Ok(source)) => source,
            Some(Err(e)) => return fail(Some(e.to_string())),
            None => return,
        };
        // The desktop picker stays open until the person chose.
        let mut state = source.state().unwrap_or(Value::Null);
        let mut pending = false;
        while state["state"] == "pending" {
            pending = true;
            tokio::time::sleep(PICKER_POLL).await;
            if self.st.lock().screen_turn != turn {
                std::thread::spawn(move || drop(source));
                return;
            }
            state = source.state().unwrap_or(Value::Null);
        }
        // `screenCancelled` of capture.ts: a failure after the picker was up
        // counts as a cancel; one before it gets the message without reason.
        match state["state"].as_str() {
            Some("live") => {}
            Some("cancelled") => return fail(None),
            Some("failed") if pending => return fail(None),
            _ => return fail(Some(String::new())),
        }
        let height = state["height"].as_u64().map_or(size.height, |h| h as u32);
        let mut sound = LiveSound::Off;
        let audio = if with_audio && !fake {
            let app = settings.source_audio_app.clone();
            let made =
                blocking(move || core::Source::app_audio(&engine, &json!({ "app": app }))).await;
            match made {
                Some(Ok(audio)) => {
                    sound = LiveSound::Sharing;
                    Some(audio)
                }
                Some(Err(e)) => {
                    tracing::warn!("Live sound: {e}");
                    sound = LiveSound::Unavailable(Some(e.to_string()).filter(|e| !e.is_empty()));
                    None
                }
                None => None,
            }
        } else {
            // The test pattern has no sound to give, like a browser picker
            // that withholds it.
            if with_audio {
                sound = LiveSound::Unavailable(None);
            }
            None
        };
        let (screen, screen_audio) = {
            let mut st = self.st.lock();
            if st.screen_turn != turn || !self.live() {
                drop(st);
                std::thread::spawn(move || drop((source, audio)));
                return;
            }
            let epoch = Uuid::new_v4();
            let screen = Local {
                source: source.clone(),
                epoch,
                height,
            };
            let screen_audio = audio.map(|source| Local {
                source,
                epoch,
                height: 0,
            });
            st.screen = Some(screen.clone());
            st.screen_audio = screen_audio.clone();
            st.awaiting_screen = true;
            (screen, screen_audio)
        };
        inner.write(|s| {
            s.live_audio = screen_audio.is_some();
            s.live_sound = sound;
        });
        inner.diagnostics().note("stream-start", "Live");
        inner.play(Sound::LiveStart, Some(false));
        self.preview(Kind::S, &source);
        self.publish_participants();
        tokio::spawn(self.clone().watch_screen(turn));
        let Some(seat) = self.seat() else { return };
        if self.send_video(&seat, Kind::S, &screen).await {
            self.send_live_sound(&seat, screen.epoch).await;
        }
    }

    /// `applySourceAudioChoice`: during a Live, "Ton mitsenden" and the
    /// chosen application take effect at once, the video untouched.
    async fn apply_source_audio_choice(self: &Arc<Self>, wanted: bool, app_changed: bool) {
        let Some(inner) = self.inner() else { return };
        let (live, running, turn) = {
            let mut st = self.st.lock();
            st.sound_turn += 1;
            (
                st.screen.is_some(),
                st.screen_audio.is_some(),
                st.sound_turn,
            )
        };
        if !live {
            return;
        }
        let (stop, start) = logic::source_audio_change(wanted, running, app_changed);
        if stop {
            self.stop_live_sound(LiveSound::Off).await;
        } else if !wanted {
            inner.write(|s| s.live_sound = LiveSound::Off);
        }
        if start {
            self.add_live_sound(turn).await;
        }
    }

    /// The Live's sound leaves, its video stays.
    async fn stop_live_sound(self: &Arc<Self>, status: LiveSound) {
        let Some(inner) = self.inner() else { return };
        let local = self.st.lock().screen_audio.take();
        inner.write(|s| {
            s.live_audio = false;
            s.live_sound = status;
        });
        if let Some(local) = local {
            self.announce(Kind::Sa, false);
            if let Some(seat) = self.seat() {
                seat.close_source(Kind::Sa).await;
            }
            let _ = blocking(move || drop(local)).await;
        }
        self.publish_participants();
    }

    /// `addLocalSourceAudio`: the chosen application's sound joins the
    /// running Live, under its epoch.
    async fn add_live_sound(self: &Arc<Self>, turn: u64) {
        let Some(inner) = self.inner() else { return };
        let Some(engine) = self.engine() else { return };
        let (screen_turn, epoch) = {
            let st = self.st.lock();
            let Some(screen) = &st.screen else { return };
            (st.screen_turn, screen.epoch)
        };
        let current =
            |st: &St| st.sound_turn == turn && st.screen_turn == screen_turn && st.screen.is_some();
        // The test pattern has no sound to give, as when a Live starts.
        let made = if self.fake {
            None
        } else {
            let app = inner.settings().source_audio_app;
            blocking(move || core::Source::app_audio(&engine, &json!({ "app": app }))).await
        };
        let source = match made {
            Some(Ok(source)) => source,
            other => {
                let reason = match other {
                    Some(Err(e)) => Some(e.to_string()).filter(|e| !e.is_empty()),
                    _ => None,
                };
                if let Some(reason) = &reason {
                    tracing::warn!("Live sound: {reason}");
                }
                if current(&self.st.lock()) {
                    inner.write(|s| s.live_sound = LiveSound::Unavailable(reason));
                }
                return;
            }
        };
        let audio = {
            let mut st = self.st.lock();
            if !current(&st) || !self.live() {
                drop(st);
                std::thread::spawn(move || drop(source));
                return;
            }
            let local = Local {
                source,
                epoch,
                height: 0,
            };
            st.screen_audio = Some(local.clone());
            local
        };
        inner.write(|s| {
            s.live_audio = true;
            s.live_sound = LiveSound::Sharing;
        });
        self.publish_participants();
        // Before the Live is on the wire, `publish_all` sends both.
        let Some(seat) = self.seat() else { return };
        if !seat.producers.lock().contains_key(&Kind::S) {
            return;
        }
        match seat.publish(Kind::Sa, &audio, None, false).await {
            Ok(()) => {
                // Switched off while it went up: the others never hear of it.
                let (now, newer) = {
                    let st = self.st.lock();
                    (current(&st), st.screen_audio.is_some())
                };
                let ours = seat
                    .producers
                    .lock()
                    .get(&Kind::Sa)
                    .is_some_and(|h| h.epoch == epoch);
                match logic::published_sound(now, newer, ours) {
                    logic::PublishedSound::Announce => self.announce(Kind::Sa, true),
                    logic::PublishedSound::Close => seat.close_source(Kind::Sa).await,
                    logic::PublishedSound::Keep => {}
                }
            }
            // The seat went: its next one publishes the sound with the video.
            Err(e) if e.code() == "connection_closed" => {}
            Err(e) => {
                tracing::warn!("Live sound: {e}");
                if current(&self.st.lock()) {
                    self.stop_live_sound(LiveSound::Unavailable(Some(e.code().to_owned())))
                        .await;
                }
            }
        }
    }

    /// Ends the Live when its capture stops (the person ended it in the
    /// desktop, the window closed).
    async fn watch_screen(self: Arc<Self>, turn: u64) {
        let mut app = String::new();
        let mut quiet = 0;
        loop {
            tokio::time::sleep(SCREEN_POLL).await;
            // The application may change during the Live.
            let now = self
                .inner()
                .map(|i| i.settings().source_audio_app)
                .unwrap_or_default();
            if now != app {
                app = now;
                quiet = 0;
            }
            let source = {
                let st = self.st.lock();
                if st.screen_turn != turn || !self.live() {
                    return;
                }
                st.screen.as_ref().map(|s| s.source.clone())
            };
            let Some(source) = source else { return };
            let state = source.state().unwrap_or(Value::Null);
            if state["state"] != "live" {
                self.stop_video(Kind::S).await;
                return;
            }
            // The sound can end on its own (the application quit); the
            // picture goes on.
            let audio = self
                .st
                .lock()
                .screen_audio
                .as_ref()
                .map(|a| a.source.clone());
            if let Some(audio) = audio
                && let Ok(state) = audio.state()
                && let Some(inner) = self.inner()
            {
                let ended = state["state"].as_str().is_some_and(|st| st != "live");
                // `noteSilentSource`: a chosen application that plays nothing.
                let silent = logic::silent_app(&app, state["streams"].as_u64(), &mut quiet);
                inner.write(|s| {
                    if !matches!(s.live_sound, LiveSound::Sharing | LiveSound::Silent(_)) {
                        return;
                    }
                    if ended {
                        s.live_sound = LiveSound::Ended;
                    } else if let Some(silent) = silent {
                        s.live_sound = silent.map_or(LiveSound::Sharing, LiveSound::Silent);
                    }
                });
            }
        }
    }

    /// Camera or Live off: `u` first, then the producers, then the capture.
    async fn stop_video(self: &Arc<Self>, kind: Kind) {
        let Some(inner) = self.inner() else { return };
        let (locals, had) = {
            let mut st = self.st.lock();
            if kind == Kind::S {
                st.screen_turn += 1;
                st.awaiting_screen = false;
                let had = st.screen.is_some();
                let locals: Vec<Local> = [st.screen.take(), st.screen_audio.take()]
                    .into_iter()
                    .flatten()
                    .collect();
                (locals, had)
            } else {
                st.camera_turn += 1;
                // A camera let go for its re-capture is still announced.
                let had = st.camera.is_some() || st.announced.contains(&Kind::V);
                (st.camera.take().into_iter().collect(), had)
            }
        };
        if kind == Kind::S {
            inner.write(|s| {
                s.live = false;
                s.live_audio = false;
                s.live_sound = LiveSound::Off;
            });
            if had {
                inner.diagnostics().note("stream-stop", "Live");
            }
        } else {
            inner.write(|s| s.camera = false);
        }
        if had {
            let kinds: &[Kind] = if kind == Kind::S {
                &[Kind::S, Kind::Sa]
            } else {
                &[Kind::V]
            };
            for k in kinds {
                self.announce(*k, false);
            }
            inner.play(
                if kind == Kind::S {
                    Sound::LiveEnd
                } else {
                    Sound::CameraOff
                },
                Some(false),
            );
        }
        inner.drop_frame(if kind == Kind::S {
            VideoKey::live(self.me)
        } else {
            VideoKey::camera(self.me)
        });
        self.publish_participants();
        if let Some(seat) = self.seat() {
            if kind == Kind::S {
                seat.close_source(Kind::Sa).await;
            }
            seat.close_source(kind).await;
        }
        let _ = blocking(move || {
            for local in &locals {
                let _ = local.source.set_video_sink(None);
            }
            drop(locals);
        })
        .await;
    }

    /// The own camera or Live as a tile, from its capture.
    fn preview(&self, kind: Kind, source: &core::Source) {
        let key = if kind == Kind::S {
            VideoKey::live(self.me)
        } else {
            VideoKey::camera(self.me)
        };
        let Some(inner) = self.inner() else { return };
        let request = sink_request(&inner, key);
        let _ = source.set_video_sink_limits(limits(request));
        let flag = inner.open_sink(key);
        let _ = source.set_video_sink(Some(frame_sink(&self.call, key, flag)));
    }

    fn apply_request(&self, key: VideoKey, request: VideoRequest) {
        let me = self.me;
        let local = {
            let st = self.st.lock();
            if key == VideoKey::camera(me) {
                st.camera.as_ref().map(|l| l.source.clone())
            } else if key == VideoKey::live(me) {
                st.screen.as_ref().map(|l| l.source.clone())
            } else {
                None
            }
        };
        if let Some(source) = local {
            let _ = source.set_video_sink_limits(limits(request));
            return;
        }
        if let Some(seat) = self.seat() {
            for received in seat.consumers.lock().values() {
                if received.key() == Some(key) {
                    let consumer = received.consumer.lock();
                    let _ = consumer.set_video_sink_limits(limits(request));
                }
            }
        }
    }

    /// `p`/`u` on the gateway for what this client sends.
    fn announce(&self, kind: Kind, on: bool) {
        let Some(inner) = self.inner() else { return };
        {
            let mut st = self.st.lock();
            if on {
                st.announced.insert(kind);
            } else {
                st.announced.remove(&kind);
            }
            let pubs = st.pubs.entry(self.me).or_default();
            pubs.retain(|k| *k != kind);
            if on {
                pubs.push(kind);
            }
        }
        inner.client().voice_publish(kind.track(), on);
        self.publish_participants();
    }

    fn apply_local_audio(&self) {
        let Some(inner) = self.inner() else { return };
        self.gate_mic(&inner);
        if let Some(seat) = self.seat() {
            inner.runtime().spawn(async move { seat.sync_mic().await });
        }
    }

    /// The microphone's track on or off as mute, deafen and push-to-talk say.
    fn gate_mic(&self, inner: &Inner) {
        // Read and set as one step: push-to-talk calls this on the UI
        // thread, a replaced microphone on the runtime.
        let _gating = self.gating.lock();
        let (muted, deafened) = inner.read(|s| (s.muted, s.deafened));
        let switches = logic::mic_switches(muted, deafened, inner.gate_open());
        // A microphone being replaced may still be the one sending.
        let mics: Vec<core::Source> = {
            let st = self.st.lock();
            st.mic
                .iter()
                .chain(st.replaced_mics.iter().map(|(_, l)| l))
                .map(|l| l.source.clone())
                .collect()
        };
        for mic in &mics {
            let _ = mic.set_enabled(switches.enabled);
        }
    }

    fn apply_volumes(&self) {
        let Some(inner) = self.inner() else { return };
        let Some(seat) = self.seat() else { return };
        let deafened = inner.read(|s| s.deafened);
        let settings = inner.settings();
        let consumers: Vec<(Arc<Mutex<core::Consumer>>, f64)> = seat
            .consumers
            .lock()
            .values()
            .filter(|r| r.kind == "audio")
            .map(|r| (r.consumer.clone(), r.volume(&inner, &settings, deafened)))
            .collect();
        for (consumer, volume) in consumers {
            let _ = consumer.lock().set_volume(volume);
        }
    }

    /// Drops what this client receives of `user` in `kinds`.
    fn drop_received(&self, user: Uuid, kinds: &[Kind]) {
        let Some(seat) = self.seat() else { return };
        let ids: Vec<String> = seat
            .consumers
            .lock()
            .iter()
            .filter(|(_, r)| r.owner == user && kinds.contains(&r.k))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            seat.remove_consumer(&id);
        }
    }

    // -------------------------------------------------------- gateway

    fn on_ready(self: &Arc<Self>) {
        let Some(inner) = self.inner() else { return };
        inner.stop_sounds();
        // A media connection still being made goes on: a rebuild would
        // only take a second seat after it.
        let connecting = self.st.lock().connecting.is_some();
        let keep_media = connecting || self.seat().is_some_and(|s| s.sock.is_open());
        {
            let mut st = self.st.lock();
            st.sound_on_join = false;
            st.peers.clear();
            let mine = st.pubs.remove(&self.me);
            st.pubs.clear();
            if let Some(mine) = mine {
                st.pubs.insert(self.me, mine);
            }
            st.awaiting_join = true;
            st.republish_on_join = keep_media;
        }
        self.publish_participants();
        if !keep_media {
            let generation = self.seats.load(Ordering::Acquire);
            self.schedule_rebuild(generation);
        }
    }

    fn on_sig(self: &Arc<Self>, sig: &SigFrame) {
        if sig.s != self.server || sig.c != Some(self.channel) {
            return;
        }
        let Some(user) = sig.u else { return };
        let Some(inner) = self.inner() else { return };
        let mine = user == self.me;
        match sig.t {
            SigKind::J => {
                let mut confirmed = (false, false);
                {
                    let mut st = self.st.lock();
                    if !mine {
                        let present = st.peers.contains_key(&user) || st.pubs.contains_key(&user);
                        if !present && !st.awaiting_join {
                            drop(st);
                            inner.play(Sound::JoinOther, None);
                            st = self.st.lock();
                        }
                        st.peers.insert(
                            user,
                            Peer {
                                since: Instant::now(),
                                muted: sig.m == Some(true),
                            },
                        );
                        st.pubs.entry(user).or_default();
                    } else if st.awaiting_join {
                        st.awaiting_join = false;
                        confirmed = (
                            std::mem::take(&mut st.sound_on_join),
                            std::mem::take(&mut st.republish_on_join),
                        );
                    }
                }
                if confirmed.0 {
                    inner.play(Sound::JoinSelf, None);
                }
                if confirmed.1 {
                    self.reannounce();
                }
            }
            SigKind::L => {
                if mine {
                    return;
                }
                let was = {
                    let mut st = self.st.lock();
                    st.peers.remove(&user);
                    st.pubs.remove(&user).is_some()
                };
                if was {
                    inner.play(Sound::LeaveOther, None);
                }
            }
            SigKind::M => {
                if mine {
                    return;
                }
                let muted = sig.on == Some(true);
                let sound = {
                    let mut st = self.st.lock();
                    let peer = st.peers.get(&user).copied();
                    let settled = peer.is_some_and(|p| p.since.elapsed() >= logic::PEER_SETTLE);
                    st.peers.insert(
                        user,
                        Peer {
                            since: peer.map_or_else(Instant::now, |p| p.since),
                            muted,
                        },
                    );
                    peer.is_some_and(|p| p.muted != muted) && settled
                };
                if sound {
                    inner.play(
                        if muted {
                            Sound::OtherMute
                        } else {
                            Sound::OtherUnmute
                        },
                        None,
                    );
                }
            }
            SigKind::P | SigKind::U => {
                let Some(kind) = sig.k.map(Kind::of_track) else {
                    return;
                };
                let on = sig.t == SigKind::P;
                let sound = {
                    let mut st = self.st.lock();
                    if on && kind == Kind::S && mine {
                        st.awaiting_screen = false;
                    }
                    let settled = st
                        .peers
                        .get(&user)
                        .is_some_and(|p| p.since.elapsed() >= logic::PEER_SETTLE);
                    let pubs = st.pubs.entry(user).or_default();
                    let had = pubs.contains(&kind);
                    pubs.retain(|k| *k != kind);
                    if on {
                        pubs.push(kind);
                    }
                    if kind == Kind::S && !mine {
                        if on && !had && settled {
                            Some(Sound::LiveStart)
                        } else if !on && had {
                            Some(Sound::LiveEnd)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(sound) = sound {
                    inner.play(sound, Some(true));
                }
                if !on && !mine {
                    match kind {
                        Kind::V => {
                            self.drop_received(user, &[Kind::V]);
                            inner.drop_frame(VideoKey::camera(user));
                        }
                        Kind::S => {
                            self.drop_received(user, &[Kind::S, Kind::Sa]);
                            inner.drop_frame(VideoKey::live(user));
                        }
                        Kind::Sa => self.drop_received(user, &[Kind::Sa]),
                        Kind::A => {}
                    }
                }
            }
            _ => return,
        }
        self.publish_participants();
    }

    fn on_gateway_error(self: &Arc<Self>, code: &str, s: Option<Uuid>, c: Option<Uuid>) {
        if s != Some(self.server) || c != Some(self.channel) {
            return;
        }
        let Some(inner) = self.inner() else { return };
        let (awaiting_join, awaiting_screen) = {
            let st = self.st.lock();
            (st.awaiting_join, st.awaiting_screen)
        };
        if awaiting_join && matches!(code, "forbidden" | "not_found" | "bad_request") {
            let current = inner.media.session.lock().take();
            if let Some(current) = current {
                current.stop();
            }
            inner.client().voice_leave();
            inner.drop_frames();
            inner.write(|st| {
                *st = CallState {
                    devices: std::mem::take(&mut st.devices),
                    push_to_talk_open: st.push_to_talk_open,
                    ..CallState::default()
                }
            });
            sync_push_to_talk(&inner);
            inner.notify(gelabber_client::error_message(code));
            return;
        }
        if awaiting_screen && !awaiting_join && code == "forbidden" {
            self.st.lock().awaiting_screen = false;
            let session = self.clone();
            inner
                .runtime()
                .spawn(async move { session.stop_video(Kind::S).await });
            inner.notify(logic::live_forbidden_message());
        }
    }

    /// The gateway dropped the pubs while away; the media still runs.
    fn reannounce(&self) {
        let (mic, camera, screen, audio) = {
            let st = self.st.lock();
            (
                st.mic.is_some() && st.announced.contains(&Kind::A),
                st.camera.is_some(),
                st.screen.is_some(),
                st.screen_audio.is_some(),
            )
        };
        if mic {
            self.announce(Kind::A, true);
        }
        if camera {
            self.announce(Kind::V, true);
        }
        if screen {
            self.announce(Kind::S, true);
            if audio {
                self.announce(Kind::Sa, true);
            }
        }
    }

    /// Who is in the room, from the gateway's roster and this session.
    fn publish_participants(&self) {
        let Some(inner) = self.inner() else { return };
        let roster: Vec<(Uuid, bool, bool)> = inner.client().with_state(|s| {
            s.voice
                .get(&self.server)
                .map(|users| {
                    users
                        .iter()
                        .filter(|(_, o)| o.channel_id == self.channel)
                        .map(|(u, o)| (*u, o.muted, o.deafened))
                        .collect()
                })
                .unwrap_or_default()
        });
        let (own_muted, own_deafened, own_camera, own_live, own_live_audio) =
            inner.read(|s| (s.muted, s.deafened, s.camera, s.live, s.live_audio));
        let st = self.st.lock();
        let mut people: Vec<Participant> = Vec::new();
        let mut others: Vec<(Uuid, bool, bool)> =
            roster.into_iter().filter(|(u, ..)| *u != self.me).collect();
        for user in st.pubs.keys() {
            if *user != self.me && !others.iter().any(|(u, ..)| u == user) {
                others.push((*user, false, false));
            }
        }
        others.sort_by_key(|(u, ..)| *u);
        let own_pubs = st.pubs.get(&self.me);
        let has_frame_source = st.camera.is_some();
        people.push(Participant {
            user: self.me,
            me: true,
            muted: own_muted || own_deafened,
            deafened: own_deafened,
            speaking: st.speaking_now.contains(&self.me) && !own_muted && !own_deafened,
            camera: (own_camera && has_frame_source).then(|| VideoKey::camera(self.me)),
            live: (own_live && st.screen.is_some()).then(|| VideoKey::live(self.me)),
            live_audio: own_live_audio
                && own_pubs.is_some_and(|p| p.contains(&Kind::Sa) || p.contains(&Kind::S)),
            watching: false,
            volume: 1.0,
            locally_muted: false,
        });
        for (user, muted, deafened) in others {
            let pubs = st.pubs.get(&user);
            let has = |k: Kind| pubs.is_some_and(|p| p.contains(&k));
            let volume = inner.volume_of(user);
            let muted = muted || deafened || st.peers.get(&user).is_some_and(|p| p.muted);
            people.push(Participant {
                user,
                me: false,
                muted,
                deafened,
                speaking: st.speaking_now.contains(&user) && !muted && !deafened,
                camera: has(Kind::V).then(|| VideoKey::camera(user)),
                live: has(Kind::S).then(|| VideoKey::live(user)),
                live_audio: has(Kind::Sa),
                watching: st.watching.contains(&user),
                volume: volume.volume as f32,
                locally_muted: volume.muted,
            });
        }
        drop(st);
        inner.write(|s| s.participants = people);
    }

    // -------------------------------------------------------- recovery

    fn on_transport_state(self: &Arc<Self>, generation: u64, direction: &'static str, state: &str) {
        let Some(inner) = self.inner() else { return };
        if self.seats.load(Ordering::Acquire) != generation || !self.live() {
            return;
        }
        if state == "connected" {
            let calm = {
                let mut st = self.st.lock();
                st.recovery.remove(direction);
                st.attempts = 0;
                st.recovery.is_empty()
            };
            if calm {
                inner.write(|s| s.status = CallStatus::Connected);
            }
            return;
        }
        if state != "failed" && state != "disconnected" {
            return;
        }
        inner.write(|s| s.status = CallStatus::Reconnecting);
        let token = {
            let mut st = self.st.lock();
            if st.recovery.contains_key(direction) {
                return;
            }
            st.recovery_token += 1;
            let token = st.recovery_token;
            st.recovery.insert(direction, token);
            token
        };
        let session = self.clone();
        let wait = if state == "failed" {
            Duration::ZERO
        } else {
            logic::ICE_DISCONNECTED_RESTART_DELAY
        };
        inner.runtime().spawn(async move {
            tokio::time::sleep(wait).await;
            if session.st.lock().recovery.get(direction) != Some(&token) {
                return;
            }
            let Some(seat) = session.seat().filter(|s| s.generation == generation) else {
                return;
            };
            if seat.state(direction) == "connected" {
                session.st.lock().recovery.remove(direction);
                return;
            }
            let deadline = {
                let session = session.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(logic::ICE_RECOVERY_DEADLINE).await;
                    if session.st.lock().recovery.get(direction) == Some(&token) {
                        session.schedule_rebuild(generation);
                    }
                })
            };
            if let Err(e) = seat.restart_ice(direction).await {
                tracing::debug!("restart ICE {direction}: {e}");
                deadline.abort();
                session.schedule_rebuild(generation);
            }
        });
    }

    /// A new seat after a pause, keeping what is captured (`scheduleSeatRebuild`).
    fn schedule_rebuild(self: &Arc<Self>, generation: u64) {
        let Some(inner) = self.inner() else { return };
        if !self.live() || self.seats.load(Ordering::Acquire) != generation {
            return;
        }
        let (attempt, seat) = {
            let mut st = self.st.lock();
            if st.rebuilding {
                return;
            }
            st.rebuilding = true;
            st.recovery.clear();
            st.announced.clear();
            let attempt = st.attempts;
            st.attempts += 1;
            (attempt, st.seat.take())
        };
        if let Some(seat) = seat {
            seat.close(true);
        }
        if attempt >= logic::SEAT_RETRIES {
            self.st.lock().rebuilding = false;
            self.give_up(Leave::Lost);
            return;
        }
        inner.write(|s| {
            s.status = CallStatus::Reconnecting;
            s.quality = 0;
        });
        let jitter = 0.8 + 0.4 * f64::from(rand_unit());
        let wait = logic::seat_backoff(attempt, jitter);
        let session = self.clone();
        inner.runtime().spawn(async move {
            tokio::time::sleep(wait).await;
            session.st.lock().rebuilding = false;
            if session.live() && session.seats.load(Ordering::Acquire) == generation {
                session.run_seat().await;
            }
        });
    }

    fn on_media_error(self: &Arc<Self>, generation: u64, code: &str, stage: logic::MediaStage) {
        let Some(inner) = self.inner() else { return };
        if self.seats.load(Ordering::Acquire) != generation {
            return;
        }
        tracing::debug!("media error {code}");
        let rebuilding = self.st.lock().attempts > 0;
        // Revoked again and again while the tickets keep coming: bounded
        // apart from `attempts`, which every connected transport resets.
        let revoked = stage == logic::MediaStage::Seat && matches!(code, "unauthorized" | "gone");
        if revoked && logic::revoked_too_often(&mut self.st.lock().revocations, Instant::now()) {
            self.give_up(Leave::LostWith(logic::media_error_message(code).into()));
            return;
        }
        match logic::media_error_action(code, stage, rebuilding) {
            logic::MediaErrorAction::Ignore => {}
            logic::MediaErrorAction::Rebuild => self.schedule_rebuild(generation),
            logic::MediaErrorAction::Renew => {
                // A seat with a renewed ticket that is refused again ends it.
                if std::mem::replace(&mut self.st.lock().renewed, true) {
                    self.give_up(Leave::Refused(logic::media_error_message(code).into()));
                } else {
                    self.schedule_rebuild(generation);
                }
            }
            logic::MediaErrorAction::Leave => {
                self.give_up(Leave::Refused(logic::media_error_message(code).into()));
            }
            logic::MediaErrorAction::Notify { once } => {
                let first = !std::mem::replace(&mut self.st.lock().stream_error_shown, true);
                if first || !once {
                    inner.notify(logic::media_error_message(code));
                }
            }
        }
    }

    /// Leaves after a failure (`rollbackSeat`): the gateway seat goes too.
    fn give_up(self: &Arc<Self>, why: Leave) {
        let Some(inner) = self.inner() else { return };
        {
            let mut current = inner.media.session.lock();
            if !current.as_ref().is_some_and(|s| Arc::ptr_eq(s, self)) {
                return;
            }
            *current = None;
        }
        self.stop();
        inner.client().voice_leave();
        inner.drop_frames();
        let (message, lost) = match why {
            Leave::Refused(message) => (message, None),
            Leave::Lost => (
                logic::lost_call_message().to_owned(),
                Some((self.server, self.channel)),
            ),
            Leave::LostWith(message) => (message, Some((self.server, self.channel))),
        };
        inner.write(|s| {
            *s = CallState {
                status: CallStatus::Failed(message.clone()),
                devices: std::mem::take(&mut s.devices),
                push_to_talk_open: s.push_to_talk_open,
                lost_call: lost,
                ..CallState::default()
            }
        });
        sync_push_to_talk(&inner);
        inner.notify(message);
    }

    // ------------------------------------------------- speaking, quality

    async fn ticker(self: Arc<Self>, generation: u64) {
        let poll = if self.fake {
            logic::BROWSER_SPEAKING_POLL
        } else {
            logic::SPEAKING_POLL
        };
        let quality_every = (QUALITY_PERIOD.as_millis() / poll.as_millis()).max(1) as u32;
        let mut tick = tokio::time::interval(poll);
        // A poll still running skips the tick, as the web's `polling` guard.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut count: u32 = 0;
        loop {
            tick.tick().await;
            if !self.live() || self.seats.load(Ordering::Acquire) != generation {
                return;
            }
            // A kick that came as an event, which the voice hook does not see.
            if self.seat_taken() {
                if let Some(inner) = self.inner() {
                    end_taken(&inner, &self);
                }
                return;
            }
            let Some(seat) = self.seat() else { return };
            let session = self.clone();
            let measure = count.is_multiple_of(quality_every);
            count = count.wrapping_add(1);
            let _ = blocking(move || session.measure(&seat, measure)).await;
        }
    }

    /// Speaking levels (own meter, each voice's playout) and every few
    /// rounds the quality bars. Blocking: stats wait for libwebrtc.
    fn measure(&self, seat: &Seat, quality: bool) {
        let Some(inner) = self.inner() else { return };
        let Some(engine) = self.engine() else { return };
        let now = Instant::now();
        let (muted, deafened) = inner.read(|s| (s.muted, s.deafened));
        let mut levels: Vec<(Uuid, f64)> = Vec::new();
        let sending = !muted && !deafened && inner.gate_open();
        if self.fake {
            // Dummy audio captures silence; the fake microphone beeps
            // instead, as Chromium's fake device does for the web.
            let mut st = self.st.lock();
            let origin = *st.beep_origin.get_or_insert(now);
            if sending && st.mic.is_some() {
                levels.push((self.me, logic::fake_beep_level(origin, now)));
            }
        } else if sending && let Ok(meters) = engine.audio_levels() {
            let blocks = meters["blocks"].as_u64().unwrap_or(0);
            let moved = self.st.lock().mic_blocks.replace(blocks) != Some(blocks);
            levels.push((
                self.me,
                logic::Speaking::level(meters["processed"].as_f64().unwrap_or(0.0), moved),
            ));
        }
        let voices: Vec<(String, Uuid, Arc<Mutex<core::Consumer>>)> = seat
            .consumers
            .lock()
            .iter()
            .filter(|(_, r)| r.k == Kind::A && !r.paused)
            .map(|(id, r)| (id.clone(), r.owner, r.consumer.clone()))
            .collect();
        for (id, owner, consumer) in voices {
            let stats = consumer.lock().stats();
            let Ok(stats) = stats else { continue };
            let samples = stats["samplesPlayed"].as_u64().unwrap_or(0);
            let moved = self.st.lock().played.insert(id.clone(), samples) != Some(samples);
            let mut level =
                logic::Speaking::level(stats["audioLevel"].as_f64().unwrap_or(0.0), moved);
            if self.fake && samples == 0 {
                // Dummy audio plays nothing out, so nothing is decoded:
                // the received Opus bitrate stands in for the level.
                let bytes: u64 = logic::stats_entries(&stats)
                    .iter()
                    .filter(|e| e["type"] == "inbound-rtp")
                    .filter_map(|e| e["bytesReceived"].as_u64())
                    .sum();
                let previous = self.st.lock().received.insert(id, (bytes, now));
                if let Some((before, at)) = previous {
                    let seconds = now.saturating_duration_since(at).as_secs_f64();
                    level = logic::bitrate_level(bytes.saturating_sub(before), seconds);
                }
            }
            levels.push((owner, level));
        }
        inner.set_levels(levels.iter().map(|(u, l)| (*u, *l as f32)).collect());
        let speaking: HashSet<Uuid> = {
            let mut st = self.st.lock();
            for (user, level) in levels {
                st.speaking.note(user, level, now);
            }
            if muted || deafened {
                st.speaking.forget(self.me);
            }
            st.speaking.speaking(now).into_iter().collect()
        };
        let changed = {
            let mut st = self.st.lock();
            let changed = st.speaking_now != speaking;
            st.speaking_now = speaking;
            changed
        };
        if changed {
            self.publish_participants();
        }
        if quality {
            // Both transports, as the web's stats: the receive one connects only
            // with a first consumer, so alone only the send one is measured.
            let send = seat.send_tp.lock().clone();
            let recv = seat.recv.lock().clone();
            let answers: Vec<Value> = [send, recv]
                .into_iter()
                .flatten()
                .filter_map(|tp| tp.native.stats().ok())
                .collect();
            if answers.is_empty() {
                return;
            }
            let stats = logic::merge_stats(&answers);
            let rtt = logic::transport_rtt_ms(&stats);
            let (lost, received) = logic::audio_packets(&stats);
            let loss = {
                let mut st = self.st.lock();
                let (lost0, received0) = std::mem::replace(&mut st.packets, (lost, received));
                let dl = lost.saturating_sub(lost0);
                let dr = received.saturating_sub(received0);
                (dl + dr > 0).then(|| dl as f64 / (dl + dr) as f64)
            };
            let mut connection = self.sample(&inner, seat, &stats);
            // The bars' own loss counts both directions since the last round.
            connection.loss = loss.or(connection.loss);
            connection.rtt_ms = rtt.or(connection.rtt_ms);
            // The bars and their tooltip read the same figures (`callQuality`).
            let level = logic::quality_level(connection.rtt_ms, connection.loss);
            inner.write(|s| {
                s.quality = level;
                s.connection = Some(connection);
            });
        }
    }

    /// One diagnostics sample (`sampleConnection`): the transports' pair,
    /// each producer's and consumer's flows.
    fn sample(&self, inner: &Inner, seat: &Seat, transport_stats: &Value) -> crate::Connection {
        use crate::diagnostics::{self, Sample};
        let at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let producers: Vec<(Kind, Arc<Mutex<core::Producer>>)> = seat
            .producers
            .lock()
            .iter()
            .filter(|(_, h)| !h.paused)
            .map(|(k, h)| (*k, h.producer.clone()))
            .collect();
        let consumers: Vec<(Kind, Arc<Mutex<core::Consumer>>)> = seat
            .consumers
            .lock()
            .values()
            .filter(|r| !r.paused)
            .map(|r| (r.k, r.consumer.clone()))
            .collect();
        let sent: Vec<(Kind, Value)> = producers
            .into_iter()
            .filter_map(|(k, p)| p.lock().stats().ok().map(|s| (k, s)))
            .collect();
        let received: Vec<(Kind, Value)> = consumers
            .into_iter()
            .filter_map(|(k, c)| c.lock().stats().ok().map(|s| (k, s)))
            .collect();
        let (live, watching) = {
            let st = self.st.lock();
            (st.screen.is_some(), !st.watching.is_empty())
        };
        let room_codecs = seat.codecs.lock().as_ref().map(|(list, _)| list.clone());
        let decodes_av1 = self
            .inner()
            .and_then(|i| i.media.av1_decoding.get().copied())
            .unwrap_or(false);
        let mut diag = inner.diagnostics();
        diag.video = Some(diagnostics::VideoCaps {
            room_codecs,
            encoders: seat.hardware,
            decodes_av1,
        });
        let mut flows = Vec::new();
        for (k, stats) in &sent {
            flows.extend(diag.flows(stats, true, *k, at_ms));
        }
        for (k, stats) in &received {
            flows.extend(diag.flows(stats, false, *k, at_ms));
        }
        let sample = Sample {
            at: diagnostics::iso(at_ms as i64),
            phase: if watching {
                "watch"
            } else if live {
                "stream-on"
            } else {
                "voice-only"
            },
            transport: diagnostics::transport(transport_stats),
            flows,
        };
        let connection = sample.connection();
        diag.push(sample);
        connection
    }
}

/// 0..1 from the clock, enough for a retry's jitter.
fn rand_unit() -> f32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    (nanos % 1000) as f32 / 1000.0
}

/// What a new sink of `key` gets: the tile's size, else a default.
fn sink_request(inner: &Inner, key: VideoKey) -> VideoRequest {
    match inner.request_of(key) {
        Some(request) => request,
        None if inner.released(key) => RELEASED,
        None => DEFAULT_REQUEST,
    }
}

fn limits(request: VideoRequest) -> core::VideoSinkLimits {
    core::VideoSinkLimits {
        max_width: request.width,
        max_height: request.height,
        max_fps: request.fps,
    }
}

/// Copies each picture into the call's newest frame of `key`, while
/// `flag` holds.
fn frame_sink(call: &Weak<Inner>, key: VideoKey, flag: Arc<AtomicBool>) -> core::VideoSink {
    let call = call.clone();
    let mut serial = 0u64;
    Box::new(move |frame: &core::VideoFrame<'_>| {
        if !flag.load(Ordering::Acquire) {
            return;
        }
        let Some(inner) = call.upgrade() else { return };
        // Not copied while no tile shows it.
        if inner.released(key) {
            crate::release_elsewhere(inner);
            return;
        }
        serial += 1;
        inner.put_frame(
            key,
            &flag,
            Frame {
                width: frame.width,
                height: frame.height,
                y: frame.y.to_vec(),
                u: frame.u.to_vec(),
                v: frame.v.to_vec(),
                stride_y: frame.stride_y as u32,
                stride_u: frame.stride_u as u32,
                stride_v: frame.stride_v as u32,
                rotation: frame.rotation,
                serial,
            },
        );
        // The app may have let go of the call meanwhile: its teardown frees
        // the engine, which waits for this delivery to return.
        crate::release_elsewhere(inner);
    })
}

// ------------------------------------------------------------------ seat

/// A transport and the requests it still waits on (the Tauri app's
/// `PageTransport`): each gets an answer, the server's or a refusal.
struct Tp {
    native: core::Transport,
    direction: &'static str,
    open: Mutex<HashSet<u64>>,
    refused: AtomicBool,
    /// The code of the last request answered with a refusal, so a failed
    /// produce can tell the server's no from the SDK's.
    refusal: Mutex<Option<String>>,
}

impl Tp {
    fn ask(&self, request: u64) -> bool {
        let refused = {
            let mut open = self.open.lock();
            let refused = self.refused.load(Ordering::Acquire);
            if !refused {
                open.insert(request);
            }
            refused
        };
        if refused {
            self.refuse(request, "connection_closed");
        }
        !refused
    }

    /// Refuses `request`, noting why before the waiting call returns.
    fn refuse(&self, request: u64, code: &str) {
        *self.refusal.lock() = Some(code.to_owned());
        let _ = self.native.respond(request, Err(code.to_owned()));
    }

    fn answer(&self, request: u64, answer: Result<Value, String>) {
        if let Err(code) = &answer {
            *self.refusal.lock() = Some(code.clone());
        }
        let _ = self.native.respond(request, answer);
        self.open.lock().remove(&request);
    }

    fn orphan(&self) {
        let open = {
            let mut open = self.open.lock();
            self.refused.store(true, Ordering::Release);
            std::mem::take(&mut *open)
        };
        for request in open {
            self.refuse(request, "connection_closed");
        }
    }
}

struct Held {
    producer: Arc<Mutex<core::Producer>>,
    id: String,
    epoch: Uuid,
    mime: String,
    paused: bool,
}

struct Received {
    consumer: Arc<Mutex<core::Consumer>>,
    owner: Uuid,
    k: Kind,
    generation: String,
    kind: &'static str,
    paused: bool,
    /// The frame sink's flag of a video.
    sink: Option<Arc<AtomicBool>>,
}

fn received_key(k: Kind, owner: Uuid) -> Option<VideoKey> {
    match k {
        Kind::V => Some(VideoKey::camera(owner)),
        Kind::S => Some(VideoKey::live(owner)),
        _ => None,
    }
}

/// A voice at its person's volume, a Live's sound at the stream volume.
fn received_volume(
    k: Kind,
    owner: Uuid,
    inner: &Inner,
    settings: &CallSettings,
    deafened: bool,
) -> f64 {
    match k {
        Kind::Sa => logic::stream_volume(settings.stream_volume, settings.stream_muted, deafened),
        _ => {
            let user = inner.volume_of(owner);
            logic::voice_volume(settings.output_volume, user.volume, user.muted, deafened)
        }
    }
}

impl Received {
    fn key(&self) -> Option<VideoKey> {
        received_key(self.k, self.owner)
    }

    /// Its picture goes at once; the sink itself with the consumer.
    fn close_sink(&self, inner: &Inner) {
        if let (Some(key), Some(flag)) = (self.key(), &self.sink) {
            inner.close_sink(key, flag);
        }
    }

    fn volume(&self, inner: &Inner, settings: &CallSettings, deafened: bool) -> f64 {
        received_volume(self.k, self.owner, inner, settings, deafened)
    }
}

pub(crate) struct Seat {
    session: Weak<Session>,
    generation: u64,
    sock: Arc<MediaSocket>,
    device: core::Device,
    rtp: Value,
    can_audio: bool,
    can_video: bool,
    hardware: logic::HardwareVideo,
    ice_servers: Value,
    /// The send transport, made with the first publication; the lock also
    /// keeps publications one at a time.
    send: tokio::sync::Mutex<Option<Arc<Tp>>>,
    /// The send transport too, for `close` to refuse its requests while a
    /// publication holds `send`.
    send_tp: Mutex<Option<Arc<Tp>>>,
    recv: Mutex<Option<Arc<Tp>>>,
    /// Keeps the microphone's pause in order: each turn sends the state
    /// wanted when it runs, so the last one wins.
    pause_lock: tokio::sync::Mutex<()>,
    /// `w` requests in the order they were given; one turn of
    /// `watch_lock` sends the oldest.
    watch_queue: Mutex<std::collections::VecDeque<(Uuid, bool)>>,
    watch_lock: tokio::sync::Mutex<()>,
    producers: Mutex<HashMap<Kind, Held>>,
    consumers: Mutex<HashMap<String, Received>>,
    /// `consumerId:generation` of consumers that closed.
    removed: Mutex<HashSet<String>>,
    pending: Mutex<HashSet<String>>,
    /// Events before the receive transport exists.
    queued: Mutex<Vec<Value>>,
    consume_lock: tokio::sync::Mutex<()>,
    /// What every receiver decodes: codecs and revision.
    codecs: Mutex<Option<(Vec<String>, u64)>>,
    states: Mutex<HashMap<&'static str, String>>,
    closed: AtomicBool,
    runtime: tokio::runtime::Handle,
}

fn consumer_key(id: &str, generation: &str) -> String {
    format!("{id}:{generation}")
}

impl Seat {
    fn state(&self, direction: &str) -> String {
        self.states
            .lock()
            .get(direction)
            .cloned()
            .unwrap_or_else(|| "new".into())
    }

    /// Ends the seat: every request refused, `l` to the server, and the
    /// media objects freed off the runtime (closing may wait for the core).
    fn close(&self, leave: bool) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let recv = self.recv.lock().take();
        let send = self.send.try_lock().ok().and_then(|mut s| s.take());
        // A publication in flight holds `send`; its transport is refused
        // here all the same and freed when `publish` returns.
        let in_flight = self.send_tp.lock().take();
        for tp in recv.iter().chain(send.iter()).chain(in_flight.iter()) {
            tp.orphan();
        }
        if leave && self.sock.is_open() {
            self.sock.notify("l", json!({}));
        }
        self.sock.close();
        let consumers: Vec<Received> = self.consumers.lock().drain().map(|(_, r)| r).collect();
        let producers: Vec<Held> = self.producers.lock().drain().map(|(_, h)| h).collect();
        if let Some(inner) = self.session.upgrade().and_then(|s| s.inner()) {
            for received in &consumers {
                received.close_sink(&inner);
            }
        }
        let device = self.device.clone();
        std::thread::spawn(move || {
            drop(consumers);
            drop(producers);
            drop(recv);
            drop(send);
            drop(device);
        });
    }

    async fn pump(self: Arc<Self>, mut events: mpsc::UnboundedReceiver<SocketEvent>) {
        while let Some(event) = events.recv().await {
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            match event {
                SocketEvent::Event(frame) => self.clone().handle_event(frame),
                SocketEvent::Error(code) => {
                    if let Some(session) = self.session.upgrade() {
                        session.on_media_error(self.generation, &code, logic::MediaStage::Seat);
                    }
                }
                SocketEvent::Closed => {
                    if let Some(session) = self.session.upgrade() {
                        session.schedule_rebuild(self.generation);
                    }
                    return;
                }
            }
        }
    }

    fn note_codecs(&self, codecs: &Value) {
        let Some(video) = codecs["video"].as_array() else {
            return;
        };
        let revision = codecs["revision"].as_u64().unwrap_or(0);
        let list: Vec<String> = video
            .iter()
            .filter_map(|c| c.as_str().map(str::to_lowercase))
            .collect();
        let changed = {
            let mut current = self.codecs.lock();
            if current.as_ref().is_some_and(|(_, r)| revision <= *r) {
                return;
            }
            *current = Some((list, revision));
            !self.producers.lock().is_empty()
        };
        if changed && let Some(session) = self.session.upgrade() {
            let seat = session.seat();
            if let Some(seat) = seat.filter(|s| s.generation == self.generation) {
                self.runtime
                    .spawn(async move { seat.recode(&session).await });
            }
        }
    }

    /// Sends video again where the best codec changed, its sound after it.
    async fn recode(self: &Arc<Self>, session: &Arc<Session>) {
        let Some(best) = self.choose_codec().map(|(m, _)| m) else {
            return;
        };
        for kind in [Kind::V, Kind::S] {
            let current = self.producers.lock().get(&kind).map(|h| h.mime.clone());
            if current.is_none() || current.as_deref() == Some(best.as_str()) {
                continue;
            }
            let local = {
                let st = session.st.lock();
                if kind == Kind::V {
                    st.camera.clone()
                } else {
                    st.screen.clone()
                }
            };
            let Some(local) = local else { continue };
            if self.publish(kind, &local, None, false).await.is_err() || kind == Kind::V {
                continue;
            }
            // The server ended the Live's sound with its parent.
            let held = self.producers.lock().remove(&Kind::Sa);
            if let Some(held) = held {
                std::thread::spawn(move || drop(held));
            }
            let audio = session.st.lock().screen_audio.clone();
            if let Some(audio) = audio {
                let _ = self.publish(Kind::Sa, &audio, None, false).await;
            }
        }
    }

    /// The codec to send video in and whether VP9 goes in layers.
    fn choose_codec(&self) -> Option<(String, bool)> {
        let allowed = self.codecs.lock().as_ref().map(|(v, _)| v.clone());
        let codecs = self.rtp["codecs"].as_array().cloned().unwrap_or_default();
        let chosen = logic::choose_video_codec(&codecs, allowed.as_deref(), self.hardware)?;
        Some((
            chosen["mimeType"].as_str()?.to_lowercase(),
            self.hardware.vp9_svc,
        ))
    }

    async fn make_transport(
        self: &Arc<Self>,
        direction: core::Direction,
    ) -> Result<Arc<Tp>, MediaError> {
        let name = match direction {
            core::Direction::Send => "send",
            core::Direction::Recv => "recv",
        };
        let mut params = self
            .sock
            .request("transport", json!({ "direction": name }))
            .await?;
        if params["id"].is_null()
            || params["iceParameters"].is_null()
            || !params["iceCandidates"].is_array()
            || params["dtlsParameters"].is_null()
        {
            return Err(MediaError::new("invalid_transport"));
        }
        params["iceServers"] = self.ice_servers.clone();
        let device = self.device.clone();
        let (native, events) = blocking(move || core::Transport::new(&device, direction, &params))
            .await
            .ok_or_else(|| MediaError::new("connection_closed"))?
            .map_err(|e| {
                tracing::warn!("transport: {e}");
                MediaError::new("negotiation_failed")
            })?;
        let tp = Arc::new(Tp {
            native,
            direction: name,
            open: Mutex::new(HashSet::new()),
            refused: AtomicBool::new(self.closed.load(Ordering::Acquire)),
            refusal: Mutex::new(None),
        });
        let weak = Arc::downgrade(&tp);
        let seat = Arc::downgrade(self);
        let runtime = self.runtime.clone();
        std::thread::Builder::new()
            .name("gelabber-call-transport".into())
            .spawn(move || forward(weak, seat, runtime, events))
            .map_err(|_| MediaError::new("connection_closed"))?;
        Ok(tp)
    }

    async fn restart_ice(&self, direction: &str) -> Result<(), MediaError> {
        let tp = if direction == "recv" {
            self.recv.lock().clone()
        } else {
            self.send.lock().await.clone()
        };
        let tp = tp.ok_or_else(|| MediaError::new("transport_missing"))?;
        let answer = self
            .sock
            .request("restartIce", json!({ "transportId": tp.native.id() }))
            .await?;
        let ice = answer["iceParameters"].clone();
        blocking(move || tp.native.restart_ice(&ice))
            .await
            .ok_or_else(|| MediaError::new("connection_closed"))?
            .map_err(|_| MediaError::new("sdk_failed"))
    }

    /// Sends `local` as `kind`: a new producer, or the same one with a new
    /// source where the codec and epoch stay (mediasoup `replaceTrack`).
    async fn publish(
        self: &Arc<Self>,
        kind: Kind,
        local: &Local,
        parent: Option<String>,
        paused: bool,
    ) -> Result<(), MediaError> {
        let result = self.publish_now(kind, local, parent, paused).await;
        // `close` cannot take the send transport while a publication holds
        // it: the last one out frees it, off the runtime.
        if self.closed.load(Ordering::Acquire)
            && let Ok(mut send) = self.send.try_lock()
            && let Some(tp) = send.take()
        {
            std::thread::spawn(move || drop(tp));
        }
        result
    }

    async fn publish_now(
        self: &Arc<Self>,
        kind: Kind,
        local: &Local,
        parent: Option<String>,
        paused: bool,
    ) -> Result<(), MediaError> {
        let mut send = self.send.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(MediaError::new("connection_closed"));
        }
        let can = if kind.video() {
            self.can_video
        } else {
            self.can_audio
        };
        if !can {
            return Err(MediaError::new("unsupported_codec"));
        }
        let (mime, svc) = if kind.video() {
            self.choose_codec()
                .ok_or_else(|| MediaError::new("unsupported_codec"))?
        } else {
            ("audio/opus".to_owned(), false)
        };
        let settings = self
            .session
            .upgrade()
            .and_then(|s| s.inner())
            .map(|i| i.settings())
            .unwrap_or_default();
        let replace = self.producers.lock().get(&kind).and_then(|h| {
            (h.mime == mime && h.epoch == local.epoch).then(|| (h.producer.clone(), h.id.clone()))
        });
        if let Some((producer, _)) = replace {
            let source = local.source.clone();
            let swapped = producer.clone();
            let replaced = blocking(move || swapped.lock().replace_source(&source)).await;
            return match replaced {
                Some(Ok(())) => {
                    drop(send);
                    // The new capture's size and profile set the limits.
                    self.apply_parameters_of(kind, &producer, local, &settings);
                    if kind == Kind::A {
                        // The new track took the old one's state: the gate's again.
                        if let Some(session) = self.session.upgrade()
                            && let Some(inner) = session.inner()
                        {
                            session.gate_mic(&inner);
                        }
                        self.sync_mic().await;
                    } else {
                        self.set_paused(kind, paused).await;
                    }
                    Ok(())
                }
                _ => Err(MediaError::new("sdk_failed")),
            };
        }
        let parent = match kind {
            Kind::Sa => {
                let producers = self.producers.lock();
                let screen = producers
                    .get(&Kind::S)
                    .filter(|h| h.epoch == local.epoch)
                    .ok_or_else(|| MediaError::new("missing_parent"))?;
                Some(screen.id.clone())
            }
            _ => parent,
        };
        if send.is_none() {
            let tp = self.make_transport(core::Direction::Send).await?;
            *self.send_tp.lock() = Some(tp.clone());
            *send = Some(tp);
        }
        let tp = send.clone().expect("send transport");
        let old = self.producers.lock().get(&kind).map(|h| h.id.clone());
        let mut app = json!({
            "k": kind.as_str(),
            "epoch": local.epoch,
            "paused": paused,
        });
        if let Some(parent) = &parent {
            app["parent"] = parent.clone().into();
        }
        if let Some(old) = &old {
            app["expectedOldProducerId"] = old.clone().into();
        }
        if kind.video() {
            app["height"] = local.height.min(65535).into();
        }
        let mut options = json!({
            "codec": mime,
            "codecOptions": logic::codec_options(kind, settings.processing_mode, settings.economy_mode),
            "appData": app,
        });
        if kind.video()
            && let Some(encodings) = logic::video_encodings(&mime, svc)
        {
            options["encodings"] = encodings;
        }
        let source = local.source.clone();
        tp.refusal.lock().take();
        let produced = {
            let tp = tp.clone();
            let options = options.clone();
            blocking(move || tp.native.produce(&source, &options)).await
        };
        let producer = match produced {
            Some(Ok(producer)) => producer,
            // The SDK, not the server, refused the layers: one stream instead.
            Some(Err(e))
                if kind.video()
                    && options.get("encodings").is_some()
                    && !self.closed.load(Ordering::Acquire)
                    && tp.refusal.lock().is_none() =>
            {
                tracing::debug!("produce {kind:?} in layers: {e}");
                options.as_object_mut().map(|o| o.remove("encodings"));
                let source = local.source.clone();
                let retry = tp.clone();
                match blocking(move || retry.native.produce(&source, &options)).await {
                    Some(Ok(producer)) => producer,
                    Some(Err(e)) => return Err(self.produce_error(&tp, &e)),
                    None => return Err(MediaError::new("connection_closed")),
                }
            }
            Some(Err(e)) => return Err(self.produce_error(&tp, &e)),
            None => return Err(MediaError::new("connection_closed")),
        };
        let id = producer.id().to_owned();
        if self.closed.load(Ordering::Acquire) {
            std::thread::spawn(move || drop(producer));
            return Err(MediaError::new("connection_closed"));
        }
        let producer = Arc::new(Mutex::new(producer));
        let previous = self.producers.lock().insert(
            kind,
            Held {
                producer: producer.clone(),
                id,
                epoch: local.epoch,
                mime,
                paused,
            },
        );
        // The server already replaced the old producer.
        if let Some(previous) = previous {
            std::thread::spawn(move || drop(previous));
        }
        drop(send);
        if paused && kind != Kind::A {
            let _ = producer.lock().set_paused(true);
        }
        self.apply_parameters_of(kind, &producer, local, &settings);
        if kind == Kind::A {
            // Mute may have changed while the producer was made.
            self.sync_mic().await;
        }
        Ok(())
    }

    /// Why a produce failed: a closed seat, else the server's answer, else
    /// the SDK.
    fn produce_error(&self, tp: &Tp, error: &core::Error) -> MediaError {
        if self.closed.load(Ordering::Acquire) {
            return MediaError::new("connection_closed");
        }
        match tp.refusal.lock().take() {
            Some(code) => MediaError(code),
            None => {
                tracing::debug!("produce: {error}");
                MediaError::new("sdk_failed")
            }
        }
    }

    /// Bitrates, rates and priorities of every sender (`VideoLimits`).
    async fn apply_parameters(self: &Arc<Self>) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let Some(inner) = session.inner() else { return };
        let settings = inner.settings();
        let held: Vec<(Kind, Arc<Mutex<core::Producer>>)> = self
            .producers
            .lock()
            .iter()
            .map(|(k, h)| (*k, h.producer.clone()))
            .collect();
        for (kind, producer) in held {
            let local = {
                let st = session.st.lock();
                match kind {
                    Kind::A => st.mic.clone(),
                    Kind::V => st.camera.clone(),
                    Kind::S => st.screen.clone(),
                    Kind::Sa => st.screen_audio.clone(),
                }
            };
            if let Some(local) = local {
                self.apply_parameters_of(kind, &producer, &local, &settings);
            }
        }
    }

    fn apply_parameters_of(
        &self,
        kind: Kind,
        producer: &Arc<Mutex<core::Producer>>,
        local: &Local,
        settings: &CallSettings,
    ) {
        let producer = producer.lock();
        let Ok(current) = producer.parameters() else {
            return;
        };
        let parameters = if kind.video() {
            let profile_id = if kind == Kind::V {
                &settings.camera_profile
            } else {
                &settings.screen_profile
            };
            let profile = logic::capture_size(kind == Kind::S, profile_id);
            let profiles = self.video_profiles(settings);
            let shares = logic::allocate_video_bitrates(
                &profiles.iter().map(|(_, p)| *p).collect::<Vec<_>>(),
                settings.video_upload_limit,
                settings.economy_mode,
            );
            let share = profiles
                .iter()
                .position(|(k, _)| *k == kind)
                .and_then(|i| shares.get(i).copied().flatten());
            let state = local.source.state().unwrap_or(Value::Null);
            let width = state["width"].as_u64().unwrap_or(0) as u32;
            let height = state["height"].as_u64().unwrap_or(0) as u32;
            logic::video_parameters(&current, profile, width, height, share)
        } else {
            let bitrate = match (settings.economy_mode, kind) {
                (true, Kind::Sa) => Some(128_000),
                (true, _) => Some(64_000),
                _ => None,
            };
            logic::audio_parameters(&current, bitrate)
        };
        if let Err(e) = producer.set_parameters(&parameters) {
            tracing::debug!("sender parameters {kind:?}: {e}");
        }
    }

    fn video_profiles(&self, settings: &CallSettings) -> Vec<(Kind, logic::StreamProfile)> {
        let producers = self.producers.lock();
        [Kind::V, Kind::S]
            .into_iter()
            .filter(|k| producers.contains_key(k))
            .map(|k| {
                let id = if k == Kind::V {
                    &settings.camera_profile
                } else {
                    &settings.screen_profile
                };
                (k, logic::stream_profile(id))
            })
            .collect()
    }

    /// Pauses or resumes the microphone's producer as mute and deafen are
    /// now, in order with the other calls of this.
    async fn sync_mic(&self) {
        let _turn = self.pause_lock.lock().await;
        let Some(inner) = self.session.upgrade().and_then(|s| s.inner()) else {
            return;
        };
        let (muted, deafened) = inner.read(|s| (s.muted, s.deafened));
        let paused = logic::mic_switches(muted, deafened, true).paused;
        self.set_paused(Kind::A, paused).await;
    }

    /// Queues a `w` behind the ones given before it. Each step counts: an
    /// off already dropped the stream here, so the server must see it too.
    fn queue_watch(self: &Arc<Self>, user: Uuid, on: bool) {
        self.watch_queue.lock().push_back((user, on));
        let seat = self.clone();
        self.runtime.spawn(async move {
            let _turn = seat.watch_lock.lock().await;
            let Some((user, on)) = seat.watch_queue.lock().pop_front() else {
                return;
            };
            let data = json!({ "u": user, "k": "s", "on": on });
            if let Err(e) = seat.sock.request("w", data).await {
                tracing::debug!("watch {user}: {e}");
            }
        });
    }

    /// Pauses or resumes a producer here (not the microphone's) and on the
    /// server.
    async fn set_paused(&self, kind: Kind, paused: bool) {
        let (producer, id, changed) = {
            let mut producers = self.producers.lock();
            let Some(held) = producers.get_mut(&kind) else {
                return;
            };
            let changed = held.paused != paused;
            held.paused = paused;
            (held.producer.clone(), held.id.clone(), changed)
        };
        // The microphone's track belongs to its gate: libmediasoupclient's
        // resume would enable it with push-to-talk closed.
        if kind != Kind::A {
            let _ = producer.lock().set_paused(paused);
        }
        if !changed {
            return;
        }
        let op = if paused {
            "pauseProducer"
        } else {
            "resumeProducer"
        };
        if let Err(e) = self.sock.request(op, json!({ "producerId": id })).await {
            tracing::debug!("{op}: {e}");
        }
    }

    async fn close_source(&self, kind: Kind) {
        let held = self.producers.lock().remove(&kind);
        let Some(held) = held else { return };
        let id = held.id.clone();
        let _ = blocking(move || drop(held)).await;
        let _ = self
            .sock
            .request("closeProducer", json!({ "producerId": id }))
            .await;
    }

    fn handle_event(self: Arc<Self>, frame: Value) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let op = frame["op"].as_str().unwrap_or_default();
        match op {
            "producerClosed" => {
                let id = frame["producerId"].as_str().unwrap_or_default();
                let epoch = frame["epoch"].as_str().and_then(|e| e.parse::<Uuid>().ok());
                let mut producers = self.producers.lock();
                let kinds: Vec<Kind> = producers
                    .iter()
                    .filter(|(_, h)| h.id == id && Some(h.epoch) == epoch)
                    .map(|(k, _)| *k)
                    .collect();
                for kind in kinds {
                    if let Some(held) = producers.remove(&kind) {
                        std::thread::spawn(move || drop(held));
                    }
                }
                return;
            }
            "codecs" => {
                self.note_codecs(&frame);
                return;
            }
            "consumerClosed" => {
                let id = frame["consumerId"].as_str().unwrap_or_default().to_owned();
                let generation = frame["generation"].as_str().unwrap_or_default();
                let key = consumer_key(&id, generation);
                self.remember_closed(key.clone());
                self.queued.lock().retain(|q| {
                    consumer_key(
                        q["consumerId"].as_str().unwrap_or_default(),
                        q["generation"].as_str().unwrap_or_default(),
                    ) != key
                });
                let matches = self
                    .consumers
                    .lock()
                    .get(&id)
                    .is_some_and(|r| r.generation == generation);
                if matches {
                    self.remove_consumer(&id);
                }
                return;
            }
            _ => {}
        }
        let Some(id) = frame["consumerId"].as_str().map(str::to_owned) else {
            return;
        };
        let generation = frame["generation"].as_str().unwrap_or_default().to_owned();
        let key = consumer_key(&id, &generation);
        if self.removed.lock().contains(&key) {
            return;
        }
        {
            // Decided under the queue's lock, which `run_seat` drains it under.
            let mut queued = self.queued.lock();
            if self.recv.lock().is_none() {
                if let Some(slot) = queued.iter_mut().find(|q| {
                    q["op"] == op
                        && q["consumerId"] == id.as_str()
                        && q["generation"] == generation.as_str()
                }) {
                    *slot = frame;
                } else if queued.len() < 3 * 1024 {
                    queued.push(frame);
                }
                return;
            }
        }
        if op == "consumer" {
            {
                let mut pending = self.pending.lock();
                if pending.contains(&key) || self.consumers.lock().contains_key(&id) {
                    return;
                }
                pending.insert(key.clone());
            }
            let runtime = self.runtime.clone();
            runtime.spawn(async move {
                let _turn = self.consume_lock.lock().await;
                self.consume(&frame).await;
                self.pending.lock().remove(&key);
            });
            return;
        }
        let paused = frame["paused"].as_bool();
        let mut consumers = self.consumers.lock();
        let Some(received) = consumers.get_mut(&id) else {
            drop(consumers);
            // An event for a consumer still being made: after it.
            self.queued.lock().push(frame);
            return;
        };
        if received.generation != generation {
            return;
        }
        if op == "consumerState"
            && let Some(paused) = paused
        {
            received.paused = paused;
            let _ = received.consumer.lock().set_paused(paused);
        }
    }

    fn remember_closed(&self, key: String) {
        let mut removed = self.removed.lock();
        removed.insert(key);
        if removed.len() > 256 {
            let pending = self.pending.lock();
            if let Some(old) = removed.iter().find(|k| !pending.contains(*k)).cloned() {
                removed.remove(&old);
            }
        }
    }

    async fn consume(self: &Arc<Self>, event: &Value) {
        let id = event["consumerId"].as_str().unwrap_or_default().to_owned();
        let generation = event["generation"].as_str().unwrap_or_default().to_owned();
        let key = consumer_key(&id, &generation);
        if self.closed.load(Ordering::Acquire)
            || self.removed.lock().contains(&key)
            || self.consumers.lock().contains_key(&id)
        {
            return;
        }
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let Some(inner) = session.inner() else { return };
        let k = event["k"].as_str().and_then(Kind::parse);
        let owner = event["owner"].as_str().and_then(|o| o.parse::<Uuid>().ok());
        let media = event["kind"].as_str().unwrap_or_default();
        let (Some(k), Some(owner)) = (k, owner) else {
            return self.consumer_failed(&id, &generation).await;
        };
        if media != k.media() || (k == Kind::Sa && event["parent"].as_str().is_none()) {
            return self.consumer_failed(&id, &generation).await;
        }
        // A Live no longer watched: the server closes it after the `w`.
        if !self.wanted(k, owner) {
            return;
        }
        let Some(recv) = self.recv.lock().clone() else {
            return;
        };
        let params = json!({
            "id": id,
            "producerId": event["producerId"],
            "kind": media,
            "rtpParameters": event["rtpParameters"],
        });
        let consumed = blocking(move || recv.native.consume(&params)).await;
        let mut consumer = match consumed {
            Some(Ok(consumer)) => consumer,
            other => {
                if let Some(Err(e)) = other {
                    tracing::debug!("consume {id}: {e}");
                }
                if logic::reports_consumer_failure(self.wanted(k, owner), None) {
                    self.consumer_failed(&id, &generation).await;
                }
                return;
            }
        };
        let settings = inner.settings();
        let deafened = inner.read(|s| s.deafened);
        let kind = if media == "video" { "video" } else { "audio" };
        let paused = event["paused"].as_bool().unwrap_or(false);
        let mut sink = None;
        if kind == "audio" {
            let _ = consumer.set_volume(received_volume(k, owner, &inner, &settings, deafened));
        } else if let Some(video) = received_key(k, owner) {
            // The sink before `consumerReady` resumes it: the first key
            // frame reaches it.
            let request = sink_request(&inner, video);
            let _ = consumer.set_video_sink_limits(limits(request));
            let flag = inner.open_sink(video);
            sink = Some(flag.clone());
            let _ = consumer.set_video_sink(Some(frame_sink(&session.call, video, flag)));
        }
        if paused {
            let _ = consumer.set_paused(true);
        }
        let received = Received {
            consumer: Arc::new(Mutex::new(consumer)),
            owner,
            k,
            generation: generation.clone(),
            kind,
            paused,
            sink,
        };
        let gone = !self.wanted(k, owner)
            || self.closed.load(Ordering::Acquire)
            || self.removed.lock().contains(&key);
        if gone {
            std::thread::spawn(move || drop(received));
            return;
        }
        self.consumers.lock().insert(id.clone(), received);
        let ready = self
            .sock
            .request(
                "consumerReady",
                json!({ "consumerId": id, "generation": generation }),
            )
            .await;
        if let Err(e) = ready {
            tracing::debug!("consumerReady {id}: {e}");
            // Dropped here meanwhile (a watch switched off) or closed by the
            // server: no failure of this client's to count.
            let wanted = self.wanted(k, owner)
                && self.consumers.lock().contains_key(&id)
                && !self.removed.lock().contains(&key);
            self.remove_consumer(&id);
            if !self.closed.load(Ordering::Acquire)
                && logic::reports_consumer_failure(wanted, Some(e.code()))
            {
                self.consumer_failed(&id, &generation).await;
            }
            return;
        }
        // Events that came while it was made.
        let later: Vec<Value> = {
            let mut queued = self.queued.lock();
            let (mine, rest): (Vec<Value>, Vec<Value>) = queued.drain(..).partition(|q| {
                q["consumerId"] == id.as_str() && q["generation"] == generation.as_str()
            });
            *queued = rest;
            mine
        };
        for event in later {
            self.clone().handle_event(event);
        }
    }

    /// Whether this client still wants `owner`'s `k`: a Live and its sound
    /// only while watched.
    fn wanted(&self, k: Kind, owner: Uuid) -> bool {
        if !matches!(k, Kind::S | Kind::Sa) {
            return true;
        }
        self.session
            .upgrade()
            .is_some_and(|s| s.st.lock().watching.contains(&owner))
    }

    async fn consumer_failed(&self, id: &str, generation: &str) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let _ = self
            .sock
            .request(
                "consumerFailed",
                json!({ "consumerId": id, "generation": generation }),
            )
            .await;
    }

    fn remove_consumer(&self, id: &str) {
        let Some(received) = self.consumers.lock().remove(id) else {
            return;
        };
        if let Some(inner) = self.session.upgrade().and_then(|s| s.inner()) {
            received.close_sink(&inner);
        }
        // Removing the sink waits for a frame in delivery.
        std::thread::spawn(move || drop(received));
    }
}

/// Hands a transport's requests to the media server and its connection
/// state to the session, until the transport is freed.
fn forward(
    tp: Weak<Tp>,
    seat: Weak<Seat>,
    runtime: tokio::runtime::Handle,
    events: std::sync::mpsc::Receiver<core::TransportEvent>,
) {
    for event in events {
        let Some(transport) = tp.upgrade() else {
            continue;
        };
        let Some(owner) = seat.upgrade() else {
            if let core::TransportEvent::Connect { request, .. }
            | core::TransportEvent::Produce { request, .. } = event
            {
                transport.refuse(request, "connection_closed");
            }
            continue;
        };
        match event {
            core::TransportEvent::Connect {
                request,
                dtls_parameters,
            } => {
                if !transport.ask(request) {
                    continue;
                }
                runtime.spawn(async move {
                    let answer = owner
                        .sock
                        .request(
                            "connect",
                            json!({ "transportId": transport.native.id(), "dtls": dtls_parameters }),
                        )
                        .await;
                    transport.answer(request, answer.map(|_| json!({})).map_err(|e| e.0));
                });
            }
            core::TransportEvent::Produce {
                request,
                rtp_parameters,
                app_data,
                ..
            } => {
                if !transport.ask(request) {
                    continue;
                }
                runtime.spawn(async move {
                    let mut data = json!({
                        "k": app_data["k"],
                        "rtp": rtp_parameters,
                        "epoch": app_data["epoch"],
                        "paused": app_data["paused"].as_bool().unwrap_or(false),
                    });
                    for field in ["parent", "expectedOldProducerId", "height"] {
                        if !app_data[field].is_null() {
                            data[field] = app_data[field].clone();
                        }
                    }
                    let answer = owner.sock.request("produce", data).await;
                    let answer = match answer {
                        Ok(result) => match result["producerId"].as_str() {
                            Some(id) => Ok(json!({ "id": id })),
                            None => Err("invalid_producer".to_owned()),
                        },
                        Err(e) => Err(e.0),
                    };
                    transport.answer(request, answer);
                });
            }
            core::TransportEvent::ConnectionState(state) => {
                owner
                    .states
                    .lock()
                    .insert(transport.direction, state.clone());
                let direction = transport.direction;
                let generation = owner.generation;
                if let Some(session) = owner.session.upgrade() {
                    runtime.spawn(async move {
                        session.on_transport_state(generation, direction, &state);
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Call;
    use crate::synth::Cue;

    fn offline() -> Call {
        Call::new(Client::new("http://127.0.0.1:9").expect("client"))
    }

    /// A session as `join` leaves it, without its media connection.
    fn session(call: &Call) -> Arc<Session> {
        let session = Arc::new(Session {
            call: Arc::downgrade(&call.inner),
            server: Uuid::new_v4(),
            channel: Uuid::new_v4(),
            me: Uuid::new_v4(),
            fake: true,
            st: Mutex::new(St::default()),
            stopped: AtomicBool::new(false),
            gating: Mutex::new(()),
            seats: AtomicU64::new(0),
        });
        *call.inner.media.session.lock() = Some(session.clone());
        session
    }

    #[test]
    fn muting_what_is_muted_plays_no_cue() {
        let call = offline();
        let session = session(&call);
        call.set_muted(true);
        call.set_muted(true);
        call.set_muted(false);
        call.set_muted(false);
        assert_eq!(
            call.inner.player.played(),
            vec![Cue::MuteSelf, Cue::UnmuteSelf]
        );
        session.stop();
    }

    #[test]
    fn a_refused_request_of_a_half_built_seat_builds_a_new_one() {
        let call = offline();
        let session = session(&call);
        session.seats.store(1, Ordering::Release);
        session.on_media_error(1, "unavailable", logic::MediaStage::Building);
        let st = session.st.lock();
        assert!(st.rebuilding, "a rebuild is planned");
        assert_eq!(st.attempts, 1);
        drop(st);
        session.stop();
    }

    #[test]
    fn a_seat_revoked_again_and_again_ends_with_a_retry_offer() {
        let call = offline();
        let session = session(&call);
        session.seats.store(1, Ordering::Release);
        call.inner.write(|s| s.channel = Some(session.channel));
        let mut settings = call.settings();
        settings.push_to_talk = true;
        call.set_settings(settings);
        assert!(!call.inner.read(|s| s.push_to_talk_open));
        // Each new seat would connect and reset `attempts`; the bound holds.
        for _ in 0..3 {
            session.on_media_error(1, "gone", logic::MediaStage::Seat);
            session.st.lock().attempts = 0;
            assert!(call.inner.media.current().is_some());
        }
        session.on_media_error(1, "unauthorized", logic::MediaStage::Seat);
        assert!(call.inner.media.current().is_none());
        let (status, lost) = call.inner.read(|s| (s.status.clone(), s.lost_call));
        assert_eq!(
            status,
            CallStatus::Failed("Der Sprachserver hat die Verbindung nicht zugelassen.".into())
        );
        assert_eq!(lost, Some((session.server, session.channel)));
        assert!(
            call.inner.read(|s| s.push_to_talk_open),
            "no gate outside a call"
        );
    }

    #[test]
    fn an_unauthorized_seat_gets_one_new_ticket() {
        let call = offline();
        let session = session(&call);
        session.seats.store(1, Ordering::Release);
        session.on_media_error(1, "unauthorized", logic::MediaStage::Seat);
        assert!(session.st.lock().rebuilding, "a new seat, not the end");
        assert!(call.inner.media.current().is_some());
        // The seat on the new ticket is refused as well: the call ends.
        session.on_media_error(1, "unauthorized", logic::MediaStage::Join);
        assert!(call.inner.media.current().is_none());
        // The media server's no, not a sign-in error.
        assert_eq!(
            call.inner.read(|s| s.status.clone()),
            CallStatus::Failed("Der Sprachserver hat die Verbindung nicht zugelassen.".into())
        );
    }

    #[test]
    fn a_kick_ends_the_call_without_a_sign_in_error() {
        let call = offline();
        let session = session(&call);
        let client = call.client().clone();
        client.voice_join(session.server, session.channel);
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.with_state(|s| s.voice_self.channel) != Some(session.channel) {
            assert!(Instant::now() < deadline, "the client took the seat");
            std::thread::sleep(Duration::from_millis(10));
        }
        call.inner.write(|s| {
            s.status = CallStatus::Connected;
            s.channel = Some(session.channel);
        });
        let mut settings = call.settings();
        settings.push_to_talk = true;
        call.set_settings(settings);
        assert!(!call.inner.read(|s| s.push_to_talk_open), "the key is up");
        // Another server's frame: the call stays.
        let other = GatewayEvent::Frame(ServerFrame::Err {
            e: "kicked".into(),
            s: Some(Uuid::new_v4()),
            c: None,
        });
        call.on_gateway(&other);
        assert!(call.inner.media.current().is_some());
        // The client dropped the seat (`removed_from`): the call leaves.
        client.voice_leave();
        while client.with_state(|s| s.voice_self.channel).is_some() {
            assert!(Instant::now() < deadline, "the client left the seat");
            std::thread::sleep(Duration::from_millis(10));
        }
        let kicked = GatewayEvent::Frame(ServerFrame::Err {
            e: "kicked".into(),
            s: Some(session.server),
            c: None,
        });
        call.on_gateway(&kicked);
        assert!(call.inner.media.current().is_none());
        assert_eq!(call.inner.read(|s| s.status.clone()), CallStatus::default());
        assert!(session.stopped.load(Ordering::Acquire));
        // Out of the call, push-to-talk no longer holds the microphone shut.
        assert!(call.inner.read(|s| s.push_to_talk_open));
    }

    #[test]
    fn a_released_video_is_neither_copied_nor_repainted() {
        let call = offline();
        let repaints = Arc::new(AtomicU64::new(0));
        let counted = repaints.clone();
        call.client().set_repaint(Arc::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
        }));
        let key = VideoKey::camera(Uuid::new_v4());
        let flag = call.inner.open_sink(key);
        let mut sink = frame_sink(&Arc::downgrade(&call.inner), key, flag);
        let (y, uv) = ([16u8; 4], [128u8; 1]);
        let picture = core::VideoFrame {
            width: 2,
            height: 2,
            y: &y,
            u: &uv,
            v: &uv,
            stride_y: 2,
            stride_u: 1,
            stride_v: 1,
            rotation: 0,
            timestamp_us: 0,
            source_width: 2,
            source_height: 2,
        };
        let serial = || call.frame(key).map(|f| f.serial);
        let repainted = || repaints.load(Ordering::Relaxed);
        // The first picture repaints, so a view can ask for the key.
        sink(&picture);
        assert_eq!((serial(), repainted()), (Some(1), 1));
        // Nobody asked: the picture is kept current without a repaint.
        sink(&picture);
        assert_eq!((serial(), repainted()), (Some(2), 1));
        call.request_video(key, 640, 360, 30);
        sink(&picture);
        assert_eq!((serial(), repainted()), (Some(3), 2));
        // Released: the last picture stays, no new one is copied.
        call.release_video(key);
        sink(&picture);
        assert_eq!((serial(), repainted()), (Some(3), 2));
        // Asked again: frames flow.
        call.request_video(key, 640, 360, 30);
        sink(&picture);
        assert_eq!((serial(), repainted()), (Some(4), 3));
    }

    #[test]
    fn a_gateway_reconnect_during_the_first_connect_takes_no_second_seat() {
        let call = offline();
        let session = session(&call);
        session.seats.store(1, Ordering::Release);
        let connecting = Connecting::start(&session, 1);
        session.on_ready();
        {
            let st = session.st.lock();
            assert!(!st.rebuilding, "no rebuild next to the connect");
            assert_eq!(st.attempts, 0);
            // The connect's announcements follow the gateway's new seat.
            assert!(st.republish_on_join);
        }
        drop(connecting);
        assert_eq!(session.st.lock().connecting, None);
        // Without a connect under way, a lost media socket is rebuilt.
        session.on_ready();
        assert_eq!(session.st.lock().attempts, 1);
        session.stop();
    }
}
