//! Settings kept on this device only (the web's localStorage stores
//! `gelabber.interface` and the media settings of voice/settings.ts), in
//! `<config>/gelabber/settings.json`. Other parts of the app read them
//! through [`get`].

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Context, Id};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};

/// `UI_SCALES` of interface/preferences.ts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiScale {
    #[default]
    Compact,
    Standard,
    Large,
}

impl UiScale {
    pub const ALL: [UiScale; 3] = [Self::Compact, Self::Standard, Self::Large];

    pub fn factor(self) -> f32 {
        match self {
            Self::Compact => 1.0,
            Self::Standard => 1.125,
            Self::Large => 1.25,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Kompakt",
            Self::Standard => "Standard",
            Self::Large => "Groß",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Standard => "standard",
            Self::Large => "large",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "standard" => Self::Standard,
            "large" => Self::Large,
            _ => Self::Compact,
        }
    }
}

/// Every device-local setting, with the web's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub ui_scale: UiScale,
    pub compact_rooms: bool,
    pub reduced_motion: bool,
    pub audio_input_id: String,
    pub audio_output_id: String,
    pub video_input_id: String,
    pub echo_cancellation: bool,
    pub noise_suppression: bool,
    pub auto_gain_control: bool,
    pub output_volume: f64,
    pub input_gain: f64,
    /// `auto`, `on` or `off`.
    pub source_audio_share: String,
    pub source_audio_app: String,
    pub source_audio_volume: f64,
    pub source_audio_muted: bool,
    /// `enhanced`, `browser` or `original`.
    pub processing_mode: String,
    pub economy_mode: bool,
    /// `balanced` or `<height>p<fps>`.
    pub camera_profile: String,
    pub screen_profile: String,
    /// Bits per second; 0 = no limit.
    pub video_upload_limit: f64,
    pub call_sounds: bool,
    pub call_sound_volume: f64,
    pub other_mute_sounds: bool,
    pub message_toasts: bool,
    pub desktop_notify: bool,
    pub message_sound: bool,
    /// `activity` or `ptt`.
    pub voice_input: String,
    pub ptt_key: String,
    pub ptt_key_label: String,
    pub ptt_release_delay: f64,
    /// The account's last theme, painted at start before /me/themes answers.
    pub theme: Option<gelabber_client::Theme>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            ui_scale: UiScale::Compact,
            compact_rooms: false,
            reduced_motion: false,
            audio_input_id: String::new(),
            audio_output_id: String::new(),
            video_input_id: String::new(),
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            output_volume: 1.0,
            input_gain: 1.0,
            source_audio_share: "auto".into(),
            source_audio_app: String::new(),
            source_audio_volume: 1.0,
            source_audio_muted: false,
            processing_mode: "enhanced".into(),
            economy_mode: false,
            camera_profile: "balanced".into(),
            screen_profile: "balanced".into(),
            video_upload_limit: 0.0,
            call_sounds: true,
            call_sound_volume: 0.35,
            other_mute_sounds: true,
            message_toasts: true,
            desktop_notify: false,
            message_sound: true,
            voice_input: "activity".into(),
            ptt_key: String::new(),
            ptt_key_label: String::new(),
            ptt_release_delay: 200.0,
            theme: None,
        }
    }
}

impl Prefs {
    fn from_json(v: &Value) -> Self {
        let d = Self::default();
        let b = |k: &str, def: bool| v.get(k).and_then(Value::as_bool).unwrap_or(def);
        let n = |k: &str, def: f64| {
            v.get(k)
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite())
                .unwrap_or(def)
        };
        let s = |k: &str, def: &str| v.get(k).and_then(Value::as_str).unwrap_or(def).to_owned();
        Self {
            ui_scale: UiScale::parse(&s("uiScale", "compact")),
            compact_rooms: b("compactRooms", d.compact_rooms),
            reduced_motion: b("reducedMotion", d.reduced_motion),
            audio_input_id: s("audioInputId", ""),
            audio_output_id: s("audioOutputId", ""),
            video_input_id: s("videoInputId", ""),
            echo_cancellation: b("echoCancellation", d.echo_cancellation),
            noise_suppression: b("noiseSuppression", d.noise_suppression),
            auto_gain_control: b("autoGainControl", d.auto_gain_control),
            output_volume: n("outputVolume", d.output_volume).clamp(0.0, 4.0),
            input_gain: n("inputGain", d.input_gain).clamp(0.0, 2.0),
            source_audio_share: match s("sourceAudioShare", "auto").as_str() {
                "on" => "on".into(),
                "off" => "off".into(),
                _ => "auto".into(),
            },
            source_audio_app: s("sourceAudioApp", ""),
            source_audio_volume: n("sourceAudioVolume", d.source_audio_volume).clamp(0.0, 4.0),
            source_audio_muted: b("sourceAudioMuted", d.source_audio_muted),
            processing_mode: match s("processingMode", "enhanced").as_str() {
                "browser" => "browser".into(),
                "original" => "original".into(),
                _ => "enhanced".into(),
            },
            economy_mode: b("economyMode", d.economy_mode),
            camera_profile: s("cameraProfile", "balanced"),
            screen_profile: s("screenProfile", "balanced"),
            video_upload_limit: n("videoUploadLimit", 0.0).max(0.0),
            call_sounds: b("callSounds", d.call_sounds),
            call_sound_volume: n("callSoundVolume", d.call_sound_volume).clamp(0.0, 1.0),
            other_mute_sounds: b("otherMuteSounds", d.other_mute_sounds),
            message_toasts: b("messageToasts", d.message_toasts),
            desktop_notify: b("desktopNotify", d.desktop_notify),
            message_sound: b("messageSound", d.message_sound),
            voice_input: if s("voiceInput", "activity") == "ptt" {
                "ptt".into()
            } else {
                "activity".into()
            },
            ptt_key: s("pttKey", ""),
            ptt_key_label: s("pttKeyLabel", ""),
            ptt_release_delay: n("pttReleaseDelay", d.ptt_release_delay).clamp(0.0, 1000.0),
            theme: v
                .get("theme")
                .and_then(|t| serde_json::from_value(t.clone()).ok()),
        }
    }

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_owned(), v);
        };
        put("uiScale", json!(self.ui_scale.key()));
        put("compactRooms", json!(self.compact_rooms));
        put("reducedMotion", json!(self.reduced_motion));
        put("audioInputId", json!(self.audio_input_id));
        put("audioOutputId", json!(self.audio_output_id));
        put("videoInputId", json!(self.video_input_id));
        put("echoCancellation", json!(self.echo_cancellation));
        put("noiseSuppression", json!(self.noise_suppression));
        put("autoGainControl", json!(self.auto_gain_control));
        put("outputVolume", json!(self.output_volume));
        put("inputGain", json!(self.input_gain));
        put("sourceAudioShare", json!(self.source_audio_share));
        put("sourceAudioApp", json!(self.source_audio_app));
        put("sourceAudioVolume", json!(self.source_audio_volume));
        put("sourceAudioMuted", json!(self.source_audio_muted));
        put("processingMode", json!(self.processing_mode));
        put("economyMode", json!(self.economy_mode));
        put("cameraProfile", json!(self.camera_profile));
        put("screenProfile", json!(self.screen_profile));
        put("videoUploadLimit", json!(self.video_upload_limit));
        put("callSounds", json!(self.call_sounds));
        put("callSoundVolume", json!(self.call_sound_volume));
        put("otherMuteSounds", json!(self.other_mute_sounds));
        put("messageToasts", json!(self.message_toasts));
        put("desktopNotify", json!(self.desktop_notify));
        put("messageSound", json!(self.message_sound));
        put("voiceInput", json!(self.voice_input));
        put("pttKey", json!(self.ptt_key));
        put("pttKeyLabel", json!(self.ptt_key_label));
        put("pttReleaseDelay", json!(self.ptt_release_delay));
        if let Some(theme) = &self.theme {
            put("theme", serde_json::to_value(theme).unwrap_or(Value::Null));
        }
        Value::Object(m)
    }
}

/// `<config>/gelabber/settings.json`, next to the remembered server.
fn file() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GELABBER_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("gelabber").join("settings.json"))
}

fn load() -> Prefs {
    file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| Prefs::from_json(&v))
        .unwrap_or_default()
}

fn save(prefs: &Prefs) {
    let Some(path) = file() else { return };
    let text = serde_json::to_string_pretty(&prefs.to_json()).unwrap_or_default();
    // Off the UI thread; a failed write only loses the setting on restart.
    std::thread::spawn(move || {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    });
}

type Shared = Arc<Mutex<Prefs>>;

fn shared(ctx: &Context) -> Shared {
    let id = Id::unique("gelabber-device-prefs");
    if let Some(s) = ctx.data(|d| d.get_temp::<Shared>(id)) {
        return s;
    }
    let s: Shared = Arc::new(Mutex::new(load()));
    ctx.data_mut(|d| d.insert_temp(id, s.clone()));
    s
}

/// The current device settings.
pub fn get(ctx: &Context) -> Prefs {
    shared(ctx).lock().clone()
}

/// Changes and stores them (`patch`), and applies what affects the whole
/// window (text size, motion).
pub fn update(ctx: &Context, change: impl FnOnce(&mut Prefs)) {
    let s = shared(ctx);
    let next = {
        let mut p = s.lock();
        let before = p.clone();
        change(&mut p);
        (*p != before).then(|| p.clone())
    };
    if let Some(p) = next {
        save(&p);
        apply(ctx, &p);
    }
}

/// Text size as egui zoom (the web's `zoom: var(--lr-ui-scale)`) and
/// reduced motion as instant animations.
pub fn apply(ctx: &Context, prefs: &Prefs) {
    let factor = prefs.ui_scale.factor();
    if (ctx.zoom_factor() - factor).abs() > 0.001 {
        ctx.set_zoom_factor(factor);
    }
    let time = if prefs.reduced_motion {
        0.0
    } else {
        crate::theme::motion::STANDARD
    };
    if ctx.global_style().animation_time != time {
        ctx.all_styles_mut(|s| s.animation_time = time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_values_and_fills_defaults() {
        let p = Prefs {
            ui_scale: UiScale::Large,
            input_gain: 1.5,
            voice_input: "ptt".into(),
            ..Prefs::default()
        };
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        let partial = Prefs::from_json(&json!({"inputGain": 9, "voiceInput": "x"}));
        assert_eq!(partial.input_gain, 2.0);
        assert_eq!(partial.voice_input, "activity");
        assert!(partial.message_toasts);
    }
}
