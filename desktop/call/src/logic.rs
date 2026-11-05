//! The call's rules that need no media core, ported from web/src/voice:
//! video codec choice (videoCodec.ts), codec options (session.ts), stream
//! profiles and upload limits (settings.ts, videoLimits.ts), priorities
//! (mediaPriority.ts), speaking (speaking.ts), the quality bars
//! (diagnostics.ts), volumes (userVolume.ts) and the seat retry
//! (mediaPeer.ts).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use uuid::Uuid;

pub const H264: &str = "video/h264";
pub const AV1: &str = "video/av1";
pub const VP8: &str = "video/vp8";
pub const VP9: &str = "video/vp9";
/// Three spatial and three temporal layers, inter-layer prediction on key
/// frames only.
pub const VP9_SVC_MODE: &str = "L3T3_KEY";
/// libwebrtc's start estimate in kbit/s for video.
pub const VIDEO_START_BITRATE_KBPS: u32 = 5000;

/// Which codecs the core encodes in hardware, and VP9 in layers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardwareVideo {
    pub av1: bool,
    pub h264: bool,
    pub vp9_svc: bool,
}

impl HardwareVideo {
    /// From `gelabber_media_core::video_codecs()`; the core always sends
    /// VP9 in layers (`vp9-svc` of the desktop app).
    pub fn from_codecs(codecs: &Value) -> Self {
        Self {
            av1: codecs["av1"]["hardwareEncoder"] == true,
            h264: codecs["h264"]["hardwareEncoder"] == true,
            vp9_svc: true,
        }
    }
}

fn mime(codec: &Value) -> String {
    codec["mimeType"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase()
}

/// VP9 profile 0, which every receiver decodes.
pub fn publishable_vp9(codec: &Value) -> bool {
    mime(codec) == VP9 && codec["parameters"]["profile-id"].as_f64().unwrap_or(0.0) == 0.0
}

/// H.264 packetization mode 1, Constrained Baseline.
pub fn publishable_h264(codec: &Value) -> bool {
    if mime(codec) != H264 {
        return false;
    }
    let parameters = &codec["parameters"];
    let profile = parameters["profile-level-id"]
        .as_str()
        .unwrap_or("42001f")
        .to_lowercase();
    let constraints = profile
        .get(2..4)
        .and_then(|c| u8::from_str_radix(c, 16).ok())
        .unwrap_or(0);
    let mode = parameters["packetization-mode"]
        .as_f64()
        .or_else(|| parameters["packetization-mode"].as_str()?.parse().ok());
    mode == Some(1.0) && profile.starts_with("42") && constraints & 0x40 != 0
}

/// The codec to send video in: the best of `codecs` every receiver decodes
/// (`chooseVideoCodec`).
pub fn choose_video_codec<'a>(
    codecs: &'a [Value],
    allowed: Option<&[String]>,
    hardware: HardwareVideo,
) -> Option<&'a Value> {
    let find = |m: &str, usable: &dyn Fn(&Value) -> bool| {
        codecs.iter().find(|c| mime(c) == m && usable(c))
    };
    if let Some(allowed) = allowed.filter(|a| !a.is_empty()) {
        let mut order = Vec::new();
        if hardware.av1 {
            order.push(AV1);
        }
        if hardware.h264 {
            order.push(H264);
        }
        order.extend([VP8, H264, VP9, AV1]);
        for m in order {
            if !allowed.iter().any(|a| a == m) {
                continue;
            }
            let usable =
                |c: &Value| (m != H264 || publishable_h264(c)) && (m != VP9 || publishable_vp9(c));
            if let Some(codec) = find(m, &usable) {
                return Some(codec);
            }
        }
    }
    [VP8, H264, VP9, AV1]
        .into_iter()
        .find_map(|m| find(m, &|_| true))
}

/// Receive capabilities without AV1 and its RTX unless AV1 decodes
/// smoothly here (`receivableCapabilities`).
pub fn receivable_capabilities(capabilities: &Value, av1_decoding: bool) -> Value {
    let Some(codecs) = capabilities["codecs"].as_array() else {
        return capabilities.clone();
    };
    if av1_decoding {
        return capabilities.clone();
    }
    let av1: Vec<i64> = codecs
        .iter()
        .filter(|c| mime(c) == AV1)
        .filter_map(|c| c["preferredPayloadType"].as_i64())
        .collect();
    let kept: Vec<Value> = codecs
        .iter()
        .filter(|c| {
            let m = mime(c);
            m != AV1
                && !(m == "video/rtx"
                    && c["parameters"]["apt"]
                        .as_i64()
                        .is_some_and(|apt| av1.contains(&apt)))
        })
        .cloned()
        .collect();
    let mut out = capabilities.clone();
    out["codecs"] = Value::Array(kept);
    out
}

/// Source kinds of the media server (`k`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Microphone.
    A,
    /// Camera.
    V,
    /// Live picture.
    S,
    /// Live sound.
    Sa,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::V => "v",
            Self::S => "s",
            Self::Sa => "sa",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "a" => Self::A,
            "v" => Self::V,
            "s" => Self::S,
            "sa" => Self::Sa,
            _ => return None,
        })
    }

    pub fn video(self) -> bool {
        matches!(self, Self::V | Self::S)
    }

    /// The `kind` a consumer of this source must have.
    pub fn media(self) -> &'static str {
        if self.video() { "video" } else { "audio" }
    }

    pub fn track(self) -> gelabber_client::TrackKind {
        match self {
            Self::A => gelabber_client::TrackKind::A,
            Self::V => gelabber_client::TrackKind::V,
            Self::S => gelabber_client::TrackKind::S,
            Self::Sa => gelabber_client::TrackKind::Sa,
        }
    }

    pub fn of_track(kind: gelabber_client::TrackKind) -> Self {
        match kind {
            gelabber_client::TrackKind::A => Self::A,
            gelabber_client::TrackKind::V => Self::V,
            gelabber_client::TrackKind::S => Self::S,
            gelabber_client::TrackKind::Sa => Self::Sa,
        }
    }
}

/// Microphone processing (`AudioProcessingMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessingMode {
    /// RNNoise in the core.
    #[default]
    Enhanced,
    /// WebRTC's noise suppression and auto gain.
    Browser,
    /// Stereo where possible, no filters.
    Original,
}

impl ProcessingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enhanced => "enhanced",
            Self::Browser => "browser",
            Self::Original => "original",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "browser" => Self::Browser,
            "original" => Self::Original,
            _ => Self::Enhanced,
        }
    }
}

/// Opus options for a microphone or a Live's sound (`codecOptionsFor`);
/// video gets the start estimate only.
pub fn codec_options(kind: Kind, mode: ProcessingMode, economy: bool) -> Value {
    match kind {
        Kind::V | Kind::S => json!({ "videoGoogleStartBitrate": VIDEO_START_BITRATE_KBPS }),
        Kind::A | Kind::Sa => {
            let source = kind == Kind::Sa;
            let original = mode == ProcessingMode::Original;
            let bitrate = match (economy, source) {
                (true, true) => 128_000,
                (true, false) => 64_000,
                (false, _) if source || original => 510_000,
                (false, _) => 128_000,
            };
            json!({
                "opusFec": true,
                "opusDtx": !source && !original,
                "opusStereo": source || original,
                "opusMaxAverageBitrate": bitrate,
            })
        }
    }
}

/// Microphone options for the core (`Source::microphone`).
pub fn microphone_options(
    mode: ProcessingMode,
    echo_cancellation: bool,
    noise_suppression: bool,
    auto_gain_control: bool,
    input_gain: f64,
) -> Value {
    json!({
        "processingMode": mode.as_str(),
        "echoCancellation": echo_cancellation,
        "noiseSuppression": noise_suppression,
        "autoGainControl": auto_gain_control,
        "inputGain": input_gain.clamp(0.0, 2.0),
    })
}

/// Encodings for a video producer: three simulcast layers for VP8 and
/// H.264, three spatial layers inside VP9, one stream otherwise.
pub fn video_encodings(mime: &str, vp9_svc: bool) -> Option<Value> {
    match mime {
        VP8 | H264 => Some(json!([
            { "scaleResolutionDownBy": 4 },
            { "scaleResolutionDownBy": 2 },
            { "scaleResolutionDownBy": 1 },
        ])),
        VP9 if vp9_svc => Some(json!([{ "scalabilityMode": VP9_SVC_MODE }])),
        _ => None,
    }
}

/// A capture profile (`STREAM_PROFILES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamProfile {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

pub const VIDEO_MAX_FPS: u32 = 60;
/// The shared video pool in economy mode, bit/s.
pub const VIDEO_SEND_BUDGET: u64 = 2_500_000;

fn resolution_width(height: u32) -> Option<u32> {
    Some(match height {
        480 => 854,
        720 => 1280,
        1080 => 1920,
        1440 => 2560,
        2160 => 3840,
        _ => return None,
    })
}

/// A profile other than `balanced`: `economy`, `detail` or
/// `<height>p<fps>`.
fn explicit_profile(id: &str) -> Option<StreamProfile> {
    match id {
        "economy" => Some(StreamProfile {
            width: 854,
            height: 480,
            fps: 15,
        }),
        "detail" => Some(StreamProfile {
            width: 1920,
            height: 1080,
            fps: 30,
        }),
        other => {
            let (h, f) = other.split_once('p')?;
            let height: u32 = h.parse().ok()?;
            let fps: u32 = f.parse().ok()?;
            matches!(fps, 15 | 24 | 30 | 45 | 60).then_some(())?;
            Some(StreamProfile {
                width: resolution_width(height)?,
                height,
                fps,
            })
        }
    }
}

/// The profile of a setting; anything unknown is `balanced` (720p30).
pub fn stream_profile(id: &str) -> StreamProfile {
    explicit_profile(id).unwrap_or(StreamProfile {
        width: 1280,
        height: 720,
        fps: 30,
    })
}

/// What a camera or screen is captured at: the profile, or for `balanced`
/// the web's safe defaults (camera 720p30, screen up to 1080p30).
pub fn capture_size(screen: bool, id: &str) -> StreamProfile {
    match explicit_profile(id) {
        Some(p) => StreamProfile {
            fps: p.fps.min(VIDEO_MAX_FPS),
            ..p
        },
        None if screen => StreamProfile {
            width: 1920,
            height: 1080,
            fps: 30,
        },
        None => stream_profile("balanced"),
    }
}

/// Per video sender, its share of an explicit upload limit
/// (`allocateVideoBitrates`); `None` without a limit.
pub fn allocate_video_bitrates(
    profiles: &[StreamProfile],
    upload_limit: f64,
    economy: bool,
) -> Vec<Option<u64>> {
    let limit = if upload_limit.is_finite() && upload_limit > 0.0 {
        upload_limit.clamp(1.0, f64::from(u32::MAX)).round() as u64
    } else {
        0
    };
    let pool = if limit > 0 {
        limit
    } else if economy {
        VIDEO_SEND_BUDGET
    } else {
        0
    };
    if pool == 0 {
        return vec![None; profiles.len()];
    }
    let weights: Vec<u64> = profiles
        .iter()
        .map(|p| u64::from(p.width) * u64::from(p.height) * u64::from(p.fps))
        .collect();
    let sum: u64 = weights.iter().sum::<u64>().max(1);
    weights
        .iter()
        .map(|w| Some(((pool as u128 * *w as u128) / sum as u128).max(1) as u64))
        .collect()
}

/// 0.1 bit per pixel (`videoBitrateCeiling`).
pub fn video_bitrate_ceiling(width: f64, height: f64, fps: f64) -> u64 {
    (width * height * fps * 0.1).round() as u64
}

/// Per encoding, its ceiling, split by `share` when one is set.
pub fn encoding_bitrates(
    scales: &[f64],
    width: f64,
    height: f64,
    fps: f64,
    share: Option<u64>,
) -> Vec<u64> {
    let ceilings: Vec<u64> = scales
        .iter()
        .map(|s| video_bitrate_ceiling(width / s, height / s, fps))
        .collect();
    let Some(share) = share else {
        return ceilings;
    };
    let total: u64 = ceilings.iter().sum::<u64>().max(1);
    ceilings
        .iter()
        .map(|c| (*c).min((share as u128 * *c as u128 / total as u128) as u64))
        .collect()
}

/// The sender parameters for a video producer: bitrate and rate per
/// layer, low priority (audio goes first, `prioritizeSender`).
pub fn video_parameters(
    current: &Value,
    profile: StreamProfile,
    width: u32,
    height: u32,
    share: Option<u64>,
) -> Value {
    let encodings = current["encodings"].as_array().cloned().unwrap_or_default();
    let scales: Vec<f64> = encodings
        .iter()
        .map(|e| e["scaleResolutionDownBy"].as_f64().unwrap_or(1.0))
        .collect();
    let fps = profile.fps.min(VIDEO_MAX_FPS);
    let (w, h) = if width > 0 && height > 0 {
        (width, height)
    } else {
        (profile.width, profile.height)
    };
    let bitrates = encoding_bitrates(&scales, f64::from(w), f64::from(h), f64::from(fps), share);
    let encodings: Vec<Value> = encodings
        .iter()
        .zip(bitrates)
        .map(|(e, bitrate)| {
            let mut e = e.clone();
            e["maxBitrate"] = bitrate.into();
            e["maxFramerate"] = fps.into();
            e["priority"] = "low".into();
            if e.get("networkPriority").is_some() {
                e["networkPriority"] = "low".into();
            }
            e
        })
        .collect();
    json!({ "encodings": encodings })
}

/// Audio senders go first on a full uplink.
pub fn audio_parameters(current: &Value, bitrate: Option<u64>) -> Value {
    let encodings: Vec<Value> = current["encodings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut e| {
            e["priority"] = "high".into();
            if e.get("networkPriority").is_some() {
                e["networkPriority"] = "high".into();
            }
            if let Some(bitrate) = bitrate {
                e["maxBitrate"] = bitrate.into();
            }
            e
        })
        .collect();
    json!({ "encodings": encodings })
}

// ------------------------------------------------------------- speaking

/// `SPEAKING_LEVEL` on the 0..1 RMS scale.
pub const SPEAKING_LEVEL: f64 = 0.02;
pub const SPEAKING_HOLD: Duration = Duration::from_millis(400);
/// The core's meters are RMS x 350, capped at 100.
pub const NATIVE_METER_SCALE: f64 = 350.0;
pub const SPEAKING_POLL: Duration = Duration::from_millis(250);

/// Who spoke lately (`watchSpeaking`): loud once, speaking for the hold.
#[derive(Default)]
pub struct Speaking {
    last_loud: HashMap<Uuid, Instant>,
}

impl Speaking {
    /// A meter value of the core, counted only when its counter moved.
    pub fn level(meter: f64, moved: bool) -> f64 {
        if moved {
            meter / NATIVE_METER_SCALE
        } else {
            0.0
        }
    }

    pub fn note(&mut self, user: Uuid, level: f64, now: Instant) {
        if level >= SPEAKING_LEVEL {
            self.last_loud.insert(user, now);
        }
    }

    pub fn forget(&mut self, user: Uuid) {
        self.last_loud.remove(&user);
    }

    pub fn clear(&mut self) {
        self.last_loud.clear();
    }

    /// Everyone loud within the hold; older entries go.
    pub fn speaking(&mut self, now: Instant) -> Vec<Uuid> {
        self.last_loud
            .retain(|_, at| now.saturating_duration_since(*at) < SPEAKING_HOLD);
        self.last_loud.keys().copied().collect()
    }
}

/// The fake microphone as the web's parity shots meet it in Chromium: its
/// fake capture device beeps every 500 ms (sine peak about 0.65 after
/// processing), and the page reads it as libwebrtc's send level.
pub const FAKE_BEEP_PERIOD: Duration = Duration::from_millis(500);
const FAKE_BEEP_PEAK: f64 = 0.65;
/// libwebrtc's `AudioLevel` publishes the peak every 11 frames of 10 ms
/// and then quarters it, so a beep stays visible for a few windows.
const SEND_LEVEL_WINDOW: Duration = Duration::from_millis(110);
/// `PEAK_PER_RMS` of speaking.ts.
const PEAK_PER_RMS: f64 = 3.0;
/// The browser's poll (`POLL_MS`), which fake calls use: the desktop app's
/// slower one misses the gaps between beeps the browser bridges.
pub const BROWSER_SPEAKING_POLL: Duration = Duration::from_millis(120);

/// The fake microphone's level on the RMS scale at `now`, for beeps at
/// `origin` + k·period: the beep's peak, quartered each meter window since.
pub fn fake_beep_level(origin: Instant, now: Instant) -> f64 {
    let since = now.saturating_duration_since(origin).as_nanos() % FAKE_BEEP_PERIOD.as_nanos();
    let windows = (since / SEND_LEVEL_WINDOW.as_nanos()) as i32;
    FAKE_BEEP_PEAK / 4f64.powi(windows) / PEAK_PER_RMS
}

/// A level from a voice's received bitrate, for calls without playout
/// (dummy audio): Opus in DTX silence sends a few hundred bit/s, speech or a
/// tone tens of kbit/s.
pub fn bitrate_level(bytes: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }
    let kbps = bytes as f64 * 8.0 / seconds / 1000.0;
    ((kbps - 12.0) / 1000.0).max(0.0)
}

// --------------------------------------------------------------- quality

/// Round trip and audio loss limits of the bars (`CALL_QUALITY_LIMITS`).
const RTT_LIMITS: [f64; 3] = [150.0, 300.0, 500.0];
const LOSS_LIMITS: [f64; 3] = [0.02, 0.05, 0.1];

fn steps(value: Option<f64>, limits: &[f64; 3]) -> u8 {
    match value {
        None => 0,
        Some(v) => limits.iter().position(|l| v < *l).unwrap_or(3) as u8,
    }
}

/// 4 is good, 1 bad, 0 nothing measured (`callQuality`).
pub fn quality_level(rtt_ms: Option<f64>, loss: Option<f64>) -> u8 {
    if rtt_ms.is_none() && loss.is_none() {
        return 0;
    }
    4 - steps(rtt_ms, &RTT_LIMITS).max(steps(loss, &LOSS_LIMITS))
}

/// Visits every stats entry of a core stats answer (an array, or an
/// object of entries).
pub fn stats_entries(stats: &Value) -> Vec<&Value> {
    match stats {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => {
            if let Some(rtc) = map.get("rtc") {
                return stats_entries(rtc);
            }
            if map.contains_key("type") {
                vec![stats]
            } else {
                map.values().collect()
            }
        }
        _ => Vec::new(),
    }
}

/// The round trip of the selected candidate pair, in ms.
pub fn transport_rtt_ms(stats: &Value) -> Option<f64> {
    let entries = stats_entries(stats);
    let pair = entries.iter().find(|e| {
        e["type"] == "candidate-pair"
            && (e["nominated"] == true || e["selected"] == true)
            && e["state"] == "succeeded"
    })?;
    pair["currentRoundTripTime"].as_f64().map(|s| s * 1000.0)
}

/// Packets lost and received of the audio streams in `stats`, for a loss
/// ratio between two samples.
pub fn audio_packets(stats: &Value) -> (u64, u64) {
    let mut lost = 0;
    let mut received = 0;
    for e in stats_entries(stats) {
        let audio = e["kind"] == "audio" || e["mediaType"] == "audio";
        if e["type"] == "inbound-rtp" && audio {
            lost += e["packetsLost"].as_i64().unwrap_or(0).max(0) as u64;
            received += e["packetsReceived"].as_u64().unwrap_or(0);
        }
        if e["type"] == "remote-inbound-rtp" && audio {
            lost += e["packetsLost"].as_i64().unwrap_or(0).max(0) as u64;
        }
    }
    (lost, received)
}

// --------------------------------------------------------------- volumes

/// `MAX_PLAYBACK_VOLUME`: 400 % through the core's playout limiter.
pub const MAX_PLAYBACK_VOLUME: f64 = 4.0;

pub fn clamp_playback_volume(volume: f64) -> f64 {
    if volume.is_finite() {
        volume.clamp(0.0, MAX_PLAYBACK_VOLUME)
    } else {
        1.0
    }
}

/// A participant's voice: output volume times their own, silent while
/// deafened or muted here (`effectiveVoiceVolume`).
pub fn voice_volume(output: f64, user: f64, locally_muted: bool, deafened: bool) -> f64 {
    if deafened || locally_muted {
        return 0.0;
    }
    (clamp_playback_volume(output) * clamp_playback_volume(user)).min(MAX_PLAYBACK_VOLUME)
}

/// A Live's sound: its own volume, silent when muted or deafened.
pub fn stream_volume(volume: f64, muted: bool, deafened: bool) -> f64 {
    if deafened || muted {
        0.0
    } else {
        clamp_playback_volume(volume)
    }
}

// ------------------------------------------------------------ recovery

/// Seat rebuild attempts before giving up (`MediaRetry`).
pub const SEAT_RETRIES: u32 = 7;

/// Wait before rebuild `attempt` (0-based): 250 ms doubling up to 4 s,
/// times `jitter` in 0.8..1.2.
pub fn seat_backoff(attempt: u32, jitter: f64) -> Duration {
    let base = (250u64 << attempt.min(5)).min(4000) as f64;
    Duration::from_millis((base * jitter.clamp(0.8, 1.2)).round() as u64)
}

pub const ICE_DISCONNECTED_RESTART_DELAY: Duration = Duration::from_millis(2000);
pub const ICE_RECOVERY_DEADLINE: Duration = Duration::from_millis(10_000);
pub const MEDIA_REQUEST_DEADLINE: Duration = Duration::from_millis(10_000);
pub const MEDIA_PROTOCOL_VERSION: u8 = 4;
/// A peer's mute change sounds once they were in the room this long.
pub const PEER_SETTLE: Duration = Duration::from_millis(2000);

/// German text for a media error code of the server (session.ts).
pub fn media_error_message(code: &str) -> &'static str {
    match code {
        "update_required" => "Bitte Gelabber aktualisieren und den Sprachkanal erneut betreten.",
        "unauthorized" => "Bitte melde dich erneut an.",
        "unavailable" => "Kein freier Sprachplatz.",
        "forbidden" => "Dafür fehlt dir die Berechtigung.",
        _ => "Der Stream konnte nicht verbunden werden. Der Sprachkanal bleibt aktiv.",
    }
}

pub const LOST_CALL_MESSAGE: &str =
    "Die Sprachverbindung konnte nicht wiederhergestellt werden. Bitte erneut beitreten.";
pub const LIVE_FORBIDDEN_MESSAGE: &str = "In diesem Server darfst du nicht live gehen.";
pub const CAMERA_UNAVAILABLE: &str = "Die Kamera ist nicht verfügbar.";
/// `microphoneFailure` for a join that goes on listening.
pub const MIC_NOT_FOUND: &str = "Kein Mikrofon gefunden, du hörst nur zu.";
pub const MIC_DENIED: &str = "Mikrofonzugriff nicht erlaubt, du hörst nur zu.";
/// The processing note of a call without a microphone.
pub const MIC_LISTEN_ONLY: &str = "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu";
/// A device or processing change that failed while the old one still runs.
pub const MIC_SWITCH_FAILED: &str =
    "Mikrofonwechsel fehlgeschlagen; bisheriges Mikrofon bleibt aktiv";
/// The microphone test (voice/native/microphoneTest.ts).
pub const MIC_TEST_FAILED: &str =
    "Mikrofontest konnte nicht starten. Prüfe das Eingabegerät in den Systemeinstellungen.";
pub const MIC_TEST_STALLED: &str =
    "Das Mikrofon liefert keine Daten. Prüfe das Eingabegerät und starte den Test erneut.";
pub const MIC_TEST_NO_LEVELS: &str = "Mikrofonpegel nicht verfügbar.";

/// Whether the core's microphone error is a refused permission rather
/// than a missing or busy device (`micRefused`).
pub fn mic_refused(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    [
        "permission",
        "denied",
        "not allowed",
        "notallowed",
        "access",
    ]
    .iter()
    .any(|w| e.contains(w))
}
pub const NO_MEDIA_CORE: &str =
    "Anrufe brauchen den Medienkern der Desktop-App, der in diesem Build fehlt.";

pub fn engine_failed(reason: &str) -> String {
    format!("Der Medienkern konnte nicht starten ({reason}).")
}

pub fn screen_failed(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "Die Bildschirmaufnahme konnte nicht gestartet werden.".into()
    } else {
        format!("Die Bildschirmaufnahme konnte nicht gestartet werden ({reason}).")
    }
}

/// A stable picture key per person and kind, so a tile keeps its key
/// across reconnects.
pub fn video_key(user: Uuid, kind: Kind) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in user.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash & !3) | if kind == Kind::S { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec(mime: &str, parameters: Value) -> Value {
        json!({ "mimeType": mime, "parameters": parameters, "kind": "video" })
    }

    fn device_codecs() -> Vec<Value> {
        vec![
            codec("video/VP8", json!({})),
            codec("video/VP9", json!({ "profile-id": 2 })),
            codec("video/VP9", json!({ "profile-id": 0 })),
            codec(
                "video/H264",
                json!({ "packetization-mode": 0, "profile-level-id": "42e01f" }),
            ),
            codec(
                "video/H264",
                json!({ "packetization-mode": 1, "profile-level-id": "42e01f" }),
            ),
            codec("video/AV1", json!({})),
        ]
    }

    #[test]
    fn codec_follows_the_room_and_the_hardware() {
        let codecs = device_codecs();
        let soft = HardwareVideo::default();
        let pick = |allowed: Option<&[String]>, hardware| {
            choose_video_codec(&codecs, allowed, hardware).cloned()
        };
        let all: Vec<String> = [VP8, H264, VP9, AV1].map(String::from).to_vec();
        assert_eq!(pick(Some(&all), soft).unwrap()["mimeType"], "video/VP8");
        let hw = HardwareVideo { h264: true, ..soft };
        let h264 = pick(Some(&all), hw).unwrap();
        assert_eq!(h264["parameters"]["packetization-mode"], 1, "mode 1 only");
        let av1 = HardwareVideo { av1: true, ..hw };
        assert_eq!(pick(Some(&all), av1).unwrap()["mimeType"], "video/AV1");
        let vp9: Vec<String> = vec![VP9.into()];
        assert_eq!(
            pick(Some(&vp9), soft).unwrap()["parameters"]["profile-id"],
            0,
            "profile 0 only"
        );
        assert_eq!(pick(None, av1).unwrap()["mimeType"], "video/VP8");
    }

    #[test]
    fn av1_is_left_out_of_receiving_unless_it_decodes() {
        let caps = json!({ "codecs": [
            { "mimeType": "video/AV1", "preferredPayloadType": 45 },
            { "mimeType": "video/rtx", "preferredPayloadType": 46, "parameters": { "apt": 45 } },
            { "mimeType": "video/VP8", "preferredPayloadType": 96 },
            { "mimeType": "video/rtx", "preferredPayloadType": 97, "parameters": { "apt": 96 } },
        ], "headerExtensions": [] });
        let out = receivable_capabilities(&caps, false);
        let types: Vec<i64> = out["codecs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["preferredPayloadType"].as_i64().unwrap())
            .collect();
        assert_eq!(types, vec![96, 97]);
        assert_eq!(receivable_capabilities(&caps, true), caps);
    }

    #[test]
    fn opus_options_follow_the_mode() {
        let mic = codec_options(Kind::A, ProcessingMode::Enhanced, false);
        assert_eq!(mic["opusDtx"], true);
        assert_eq!(mic["opusStereo"], false);
        assert_eq!(mic["opusMaxAverageBitrate"], 128_000);
        let music = codec_options(Kind::A, ProcessingMode::Original, false);
        assert_eq!(music["opusDtx"], false);
        assert_eq!(music["opusStereo"], true);
        assert_eq!(music["opusMaxAverageBitrate"], 510_000);
        let live = codec_options(Kind::Sa, ProcessingMode::Enhanced, true);
        assert_eq!(live["opusMaxAverageBitrate"], 128_000);
        assert_eq!(live["opusStereo"], true);
        assert_eq!(
            codec_options(Kind::A, ProcessingMode::Browser, true)["opusMaxAverageBitrate"],
            64_000
        );
        assert_eq!(
            codec_options(Kind::V, ProcessingMode::Enhanced, false)["videoGoogleStartBitrate"],
            5000
        );
    }

    #[test]
    fn layers_by_codec() {
        assert_eq!(
            video_encodings(VP8, true)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            video_encodings(VP9, true).unwrap()[0]["scalabilityMode"],
            VP9_SVC_MODE
        );
        assert!(video_encodings(VP9, false).is_none());
        assert!(video_encodings(AV1, true).is_none());
    }

    #[test]
    fn profiles_parse_like_the_web() {
        assert_eq!(
            stream_profile("1440p60"),
            StreamProfile {
                width: 2560,
                height: 1440,
                fps: 60
            }
        );
        assert_eq!(stream_profile("2160p24").width, 3840);
        assert_eq!(stream_profile("nonsense"), stream_profile("balanced"));
        assert_eq!(stream_profile("720p31"), stream_profile("balanced"));
        assert_eq!(capture_size(true, "balanced").height, 1080);
        assert_eq!(capture_size(false, "balanced").height, 720);
        assert_eq!(capture_size(false, "480p15").fps, 15);
    }

    #[test]
    fn upload_limits_split_by_pixels() {
        let profiles = [stream_profile("balanced"), stream_profile("detail")];
        assert_eq!(
            allocate_video_bitrates(&profiles, 0.0, false),
            vec![None, None]
        );
        let shares = allocate_video_bitrates(&profiles, 0.0, true);
        let total: u64 = shares.iter().map(|s| s.unwrap()).sum();
        assert!(total <= VIDEO_SEND_BUDGET);
        assert!(shares[1] > shares[0]);
        let ceilings = encoding_bitrates(&[4.0, 2.0, 1.0], 1280.0, 720.0, 30.0, None);
        assert_eq!(ceilings, vec![172_800, 691_200, 2_764_800]);
        let split = encoding_bitrates(&[4.0, 2.0, 1.0], 1280.0, 720.0, 30.0, Some(1_000_000));
        assert!(split.iter().sum::<u64>() <= 1_000_000);
    }

    #[test]
    fn video_parameters_set_every_layer() {
        let current = json!({ "encodings": [
            { "active": true, "scaleResolutionDownBy": 4 },
            { "active": true, "scaleResolutionDownBy": 2 },
            { "active": true, "scaleResolutionDownBy": 1, "networkPriority": "medium" },
        ]});
        let out = video_parameters(&current, stream_profile("balanced"), 1280, 720, None);
        assert_eq!(out["encodings"][2]["maxBitrate"], 2_764_800);
        assert_eq!(out["encodings"][0]["maxFramerate"], 30);
        assert_eq!(out["encodings"][2]["networkPriority"], "low");
        assert!(out["encodings"][0].get("networkPriority").is_none());
        let audio = audio_parameters(&json!({"encodings": [{}]}), Some(64_000));
        assert_eq!(audio["encodings"][0]["priority"], "high");
        assert_eq!(audio["encodings"][0]["maxBitrate"], 64_000);
    }

    #[test]
    fn the_fake_microphone_keeps_speaking_through_its_beeps() {
        let origin = Instant::now();
        let at = |ms: u64| origin + Duration::from_millis(ms);
        assert!(fake_beep_level(origin, at(0)) >= SPEAKING_LEVEL);
        assert!(
            fake_beep_level(origin, at(150)) >= SPEAKING_LEVEL,
            "the next window"
        );
        assert!(
            fake_beep_level(origin, at(250)) < SPEAKING_LEVEL,
            "quartered twice"
        );
        assert!(
            fake_beep_level(origin, at(510)) >= SPEAKING_LEVEL,
            "the next beep"
        );
        // Polled as fake calls do, with up to 40 ms of lateness, the ring
        // never drops between beeps.
        let user = Uuid::from_u128(1);
        let mut speaking = Speaking::default();
        for poll in 0..80u64 {
            let late = (poll * 37) % 41;
            let now = at(poll * BROWSER_SPEAKING_POLL.as_millis() as u64 + late);
            speaking.note(user, fake_beep_level(origin, now), now);
            assert_eq!(speaking.speaking(now), vec![user], "poll {poll}");
        }
    }

    #[test]
    fn speaking_holds_for_a_moment() {
        let start = Instant::now();
        let user = Uuid::new_v4();
        let mut speaking = Speaking::default();
        speaking.note(user, Speaking::level(10.0, true), start);
        assert_eq!(speaking.speaking(start), vec![user]);
        assert_eq!(
            speaking.speaking(start + Duration::from_millis(399)),
            vec![user]
        );
        assert!(speaking.speaking(start + SPEAKING_HOLD).is_empty());
        speaking.note(user, Speaking::level(10.0, false), start);
        assert!(
            speaking.speaking(start).is_empty(),
            "a still meter is silence"
        );
        speaking.note(user, Speaking::level(5.0, true), start);
        assert!(speaking.speaking(start).is_empty(), "below the threshold");
    }

    #[test]
    fn bitrate_stands_in_for_the_level() {
        assert_eq!(bitrate_level(19, 0.25), 0.0, "DTX silence");
        assert!(bitrate_level(2742, 0.25) >= SPEAKING_LEVEL, "a tone");
        assert_eq!(bitrate_level(100, 0.0), 0.0);
    }

    #[test]
    fn quality_bars() {
        assert_eq!(quality_level(None, None), 0);
        assert_eq!(quality_level(Some(20.0), Some(0.0)), 4);
        assert_eq!(quality_level(Some(200.0), None), 3);
        assert_eq!(quality_level(Some(20.0), Some(0.06)), 2);
        assert_eq!(quality_level(Some(600.0), Some(0.0)), 1);
        let stats = json!([
            { "type": "candidate-pair", "state": "succeeded", "nominated": true, "currentRoundTripTime": 0.042 },
            { "type": "inbound-rtp", "kind": "audio", "packetsLost": 2, "packetsReceived": 98 },
        ]);
        assert_eq!(transport_rtt_ms(&stats), Some(42.0));
        assert_eq!(audio_packets(&stats), (2, 98));
    }

    #[test]
    fn volumes() {
        assert_eq!(voice_volume(1.0, 2.0, false, false), 2.0);
        assert_eq!(voice_volume(3.0, 3.0, false, false), 4.0);
        assert_eq!(voice_volume(1.0, 1.0, true, false), 0.0);
        assert_eq!(voice_volume(1.0, 1.0, false, true), 0.0);
        assert_eq!(stream_volume(0.5, false, false), 0.5);
        assert_eq!(stream_volume(0.5, true, false), 0.0);
    }

    #[test]
    fn backoff_doubles_to_four_seconds() {
        let waits: Vec<u64> = (0..7)
            .map(|n| seat_backoff(n, 1.0).as_millis() as u64)
            .collect();
        assert_eq!(waits, vec![250, 500, 1000, 2000, 4000, 4000, 4000]);
        assert_eq!(seat_backoff(0, 0.5).as_millis(), 200);
    }

    #[test]
    fn video_keys_are_stable_and_distinct() {
        let user = Uuid::new_v4();
        assert_eq!(video_key(user, Kind::V), video_key(user, Kind::V));
        assert_ne!(video_key(user, Kind::V), video_key(user, Kind::S));
    }
}
