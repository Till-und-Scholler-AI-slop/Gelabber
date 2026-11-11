//! The measurements behind the diagnostics tab and its JSON export (web
//! voice/diagnostics.ts, connectionSummary.ts): one sample every two
//! seconds while a call runs, its flows from libwebrtc's stats, the flows
//! grouped as people think of them, and the export's shape.

use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use gelabber_i18n::{diagnostics as texts, intl};
use serde_json::{Value, json};

use crate::logic::{self, Kind};
use crate::{CallSettings, Connection};

/// `DIAGNOSTIC_LIMITS`.
pub const INTERVAL_MS: u64 = 2_000;
pub const SAMPLES: usize = 48;
pub const EVENTS: usize = 64;
pub const PHASES: usize = 16;

/// One sent or received track (`FlowStats`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub send: bool,
    pub audio: bool,
    /// `voice`, `camera`, `screen`, `screen-audio` or `video`.
    pub source: &'static str,
    pub codec: Option<String>,
    pub bitrate_bps: Option<f64>,
    pub loss: Option<f64>,
    pub packets_lost: Option<u64>,
    pub jitter_ms: Option<f64>,
    pub rtt_ms: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// `cpu`, `bandwidth` or `other`.
    pub limited_by: Option<String>,
}

/// The transport of a sample (`TransportSnapshot`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transport {
    pub relay: Option<bool>,
    pub rtt_ms: Option<f64>,
    pub available_out_bps: Option<f64>,
    pub available_in_bps: Option<f64>,
}

/// `DiagnosticSample` of the call's own connection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    /// ISO 8601, UTC.
    pub at: String,
    /// `voice-only`, `stream-on` …
    pub phase: &'static str,
    pub transport: Transport,
    pub flows: Vec<Flow>,
}

impl Sample {
    /// `callQuality` of the sample, as the call bar and popover show it.
    pub fn connection(&self) -> Connection {
        let audio: Vec<&Flow> = self.flows.iter().filter(|f| f.audio).collect();
        let rtt = self
            .transport
            .rtt_ms
            .or_else(|| worst(audio.iter().map(|f| f.rtt_ms)));
        let total = |send: bool| {
            let rates: Vec<f64> = self
                .flows
                .iter()
                .filter(|f| f.send == send)
                .filter_map(|f| f.bitrate_bps)
                .collect();
            (!rates.is_empty()).then(|| rates.iter().sum())
        };
        Connection {
            rtt_ms: rtt,
            loss: worst(audio.iter().map(|f| f.loss)),
            relay: self.transport.relay,
            sent_bps: total(true),
            received_bps: total(false),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "at": self.at,
            "phase": self.phase,
            "voice": {
                "role": "voice",
                "transport": {
                    "path": self.transport.relay.map(|r| if r { "turn" } else { "direct" }),
                    "rttMs": self.transport.rtt_ms,
                    "availableOutgoingBps": self.transport.available_out_bps,
                    "availableIncomingBps": self.transport.available_in_bps,
                },
                "flows": self.flows.iter().map(Flow::to_json).collect::<Vec<_>>(),
            },
            "watch": null,
        })
    }
}

impl Flow {
    fn to_json(&self) -> Value {
        json!({
            "direction": if self.send { "send" } else { "recv" },
            "kind": if self.audio { "audio" } else { "video" },
            "source": self.source,
            "codec": self.codec,
            "measuredBitrateBps": self.bitrate_bps,
            "packetLoss": self.loss,
            "packetsLost": self.packets_lost,
            "jitterMs": self.jitter_ms,
            "rttMs": self.rtt_ms,
            "width": self.width,
            "height": self.height,
            "fps": self.fps,
            "qualityLimitationReason": self.limited_by,
        })
    }
}

fn worst(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values
        .flatten()
        .filter(|v| v.is_finite())
        .fold(None, |a: Option<f64>, v| Some(a.map_or(v, |a| a.max(v))))
}

/// A noted event (`DiagnosticEvent`): stream start and stop, recovery …
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub at: String,
    pub kind: &'static str,
    pub detail: String,
}

/// `VideoChoice`: what this device and the room can do with video.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoCaps {
    /// `video/vp8` …, as the server lists them for the room.
    pub room_codecs: Option<Vec<String>>,
    pub encoders: logic::HardwareVideo,
    pub decodes_av1: bool,
}

impl VideoCaps {
    fn to_json(&self, app_version: &str) -> Value {
        json!({
            "roomCodecs": self.room_codecs,
            "encoders": {
                "av1": self.encoders.av1,
                "h264": self.encoders.h264,
                "vp9Svc": self.encoders.vp9_svc,
            },
            "decodesAv1": self.decodes_av1,
            "desktopApp": app_version,
        })
    }
}

/// What the tab and the export read.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub video: Option<VideoCaps>,
    pub samples: VecDeque<Sample>,
    pub events: VecDeque<Event>,
    /// `(at, phase)` whenever the phase changed.
    pub phases: VecDeque<(String, &'static str)>,
    /// Bytes and the time they were counted, per stats entry, for rates.
    counters: HashMap<String, (u64, f64)>,
}

impl Diagnostics {
    pub fn latest(&self) -> Option<&Sample> {
        self.samples.back()
    }

    pub fn push(&mut self, sample: Sample) {
        if self.phases.back().is_none_or(|(_, p)| *p != sample.phase) {
            self.phases.push_back((sample.at.clone(), sample.phase));
            while self.phases.len() > PHASES {
                self.phases.pop_front();
            }
        }
        self.samples.push_back(sample);
        while self.samples.len() > SAMPLES {
            self.samples.pop_front();
        }
    }

    pub fn note(&mut self, kind: &'static str, detail: impl Into<String>) {
        self.events.push_back(Event {
            at: now_iso(),
            kind,
            detail: detail.into(),
        });
        while self.events.len() > EVENTS {
            self.events.pop_front();
        }
    }

    /// A new call starts with nothing measured.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The ping of the last `count` samples, oldest first, padded with
    /// `None` at the start (`pingHistory`).
    pub fn pings(&self, count: usize) -> Vec<Option<f64>> {
        let recent: Vec<Option<f64>> = self
            .samples
            .iter()
            .rev()
            .take(count)
            .map(|s| s.connection().rtt_ms)
            .collect();
        let mut out = vec![None; count - recent.len()];
        out.extend(recent.into_iter().rev());
        out
    }

    /// Flows of one producer's or consumer's stats answer.
    pub fn flows(&mut self, stats: &Value, send: bool, kind: Kind, at_ms: f64) -> Vec<Flow> {
        let entries = logic::stats_entries(stats);
        let codec_of = |id: &Value| -> Option<String> {
            let id = id.as_str()?;
            entries
                .iter()
                .find(|e| e["type"] == "codec" && e["id"] == id)
                .and_then(|e| e["mimeType"].as_str())
                .map(|m| m.split('/').nth(1).unwrap_or(m).to_owned())
        };
        let source = match (send, kind) {
            (_, Kind::A) => "voice",
            (_, Kind::Sa) => "screen-audio",
            (true, Kind::V) => "camera",
            (true, Kind::S) => "screen",
            (false, Kind::V) => "video",
            (false, Kind::S) => "screen",
        };
        let wanted = if send { "outbound-rtp" } else { "inbound-rtp" };
        let mut out = Vec::new();
        for e in entries.iter().filter(|e| e["type"] == wanted) {
            let id = e["id"].as_str().unwrap_or_default().to_owned();
            let bytes_key = if send { "bytesSent" } else { "bytesReceived" };
            let bytes = e[bytes_key].as_u64().unwrap_or(0);
            // libwebrtc stamps stats in microseconds.
            let at = e["timestamp"].as_f64().unwrap_or(at_ms * 1000.0);
            let bitrate = self
                .counters
                .insert(id.clone(), (bytes, at))
                .and_then(|(b0, t0)| {
                    let seconds = (at - t0) / 1_000_000.0;
                    (seconds > 0.0).then(|| bytes.saturating_sub(b0) as f64 * 8.0 / seconds)
                });
            // Loss of a sent stream is what the far end reports back.
            let remote = e["remoteId"].as_str().and_then(|rid| {
                entries
                    .iter()
                    .find(|r| r["id"] == rid && r["type"] == "remote-inbound-rtp")
            });
            let (loss, lost, jitter, rtt) = if send {
                match remote {
                    Some(r) => (
                        r["fractionLost"].as_f64(),
                        r["packetsLost"].as_i64().map(|n| n.max(0) as u64),
                        r["jitter"].as_f64().map(|s| s * 1000.0),
                        r["roundTripTime"].as_f64().map(|s| s * 1000.0),
                    ),
                    None => (None, None, None, None),
                }
            } else {
                let lost = e["packetsLost"].as_i64().map(|n| n.max(0) as u64);
                let received = e["packetsReceived"].as_u64();
                let loss = match (lost, received) {
                    (Some(l), Some(r)) if l + r > 0 => Some(l as f64 / (l + r) as f64),
                    _ => None,
                };
                (loss, lost, e["jitter"].as_f64().map(|s| s * 1000.0), None)
            };
            let audio = matches!(kind, Kind::A | Kind::Sa);
            out.push(Flow {
                send,
                audio,
                source,
                codec: codec_of(&e["codecId"]),
                bitrate_bps: bitrate,
                loss,
                packets_lost: lost,
                jitter_ms: jitter,
                rtt_ms: rtt,
                width: e["frameWidth"].as_u64().map(|v| v as u32),
                height: e["frameHeight"].as_u64().map(|v| v as u32),
                fps: e["framesPerSecond"].as_f64(),
                limited_by: e["qualityLimitationReason"]
                    .as_str()
                    .filter(|r| *r != "none")
                    .map(str::to_owned),
            });
        }
        out
    }
}

/// Path, round trip and available bitrates of a transport's stats.
pub fn transport(stats: &Value) -> Transport {
    let entries = logic::stats_entries(stats);
    let pair = entries.iter().find(|e| {
        e["type"] == "candidate-pair"
            && (e["nominated"] == true || e["selected"] == true)
            && e["state"] == "succeeded"
    });
    let Some(pair) = pair else {
        return Transport::default();
    };
    let relay = pair["localCandidateId"].as_str().and_then(|id| {
        entries
            .iter()
            .find(|e| e["id"] == id)
            .and_then(|c| c["candidateType"].as_str())
            .map(|t| t == "relay")
    });
    Transport {
        relay,
        rtt_ms: logic::transport_rtt_ms(stats),
        available_out_bps: pair["availableOutgoingBitrate"].as_f64(),
        available_in_bps: pair["availableIncomingBitrate"].as_f64(),
    }
}

/// `flowGroups`: one row per direction, kind and name.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowGroup {
    pub send: bool,
    pub audio: bool,
    pub name: &'static str,
    pub count: usize,
    pub codec: Option<String>,
    pub bitrate_bps: Option<f64>,
    pub loss: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub limited_by: Option<String>,
}

/// `SOURCE_LABEL`, with received video as the others' cameras.
fn flow_name(flow: &Flow) -> &'static str {
    match flow.source {
        "voice" => texts::source_voice(),
        "camera" => texts::source_camera(),
        "screen" => texts::source_screen(),
        "screen-audio" => texts::source_screen_audio(),
        _ => texts::source_video(),
    }
}

pub fn flow_groups(flows: &[Flow]) -> Vec<FlowGroup> {
    let mut order: Vec<(bool, bool, &'static str)> = Vec::new();
    let mut groups: HashMap<(bool, bool, &'static str), Vec<&Flow>> = HashMap::new();
    for flow in flows {
        let key = (flow.send, flow.audio, flow_name(flow));
        if !groups.contains_key(&key) {
            order.push(key);
        }
        groups.entry(key).or_default().push(flow);
    }
    let rank = |g: &FlowGroup| (if g.send { 0 } else { 2 }) + usize::from(!g.audio);
    let mut out: Vec<FlowGroup> = order
        .into_iter()
        .map(|key| {
            let list = &groups[&key];
            let first = list[0];
            let same_codec = list.iter().all(|f| f.codec == first.codec);
            let rates: Vec<f64> = list.iter().filter_map(|f| f.bitrate_bps).collect();
            let largest = list
                .iter()
                .max_by_key(|f| f.width.unwrap_or(0) as u64 * f.height.unwrap_or(0) as u64)
                .copied()
                .unwrap_or(first);
            FlowGroup {
                send: key.0,
                audio: key.1,
                name: key.2,
                count: list.len(),
                codec: if same_codec {
                    first.codec.clone()
                } else {
                    None
                },
                bitrate_bps: (!rates.is_empty()).then(|| rates.iter().sum()),
                loss: worst(list.iter().map(|f| f.loss)),
                jitter_ms: worst(list.iter().map(|f| f.jitter_ms)),
                width: largest.width,
                height: largest.height,
                fps: largest.fps,
                limited_by: list.iter().find_map(|f| f.limited_by.clone()),
            }
        })
        .collect();
    out.sort_by_key(rank);
    out
}

/// `formatBps`: "38 kbit/s", "1,2 Mbit/s"; "unbekannt" without a value.
pub fn format_bps(bps: Option<f64>) -> String {
    let Some(bps) = bps.filter(|b| b.is_finite()) else {
        return texts::value_unknown().into();
    };
    let abs = bps.abs();
    let (scaled, unit) = if abs >= 1_000_000.0 {
        (abs / 1_000_000.0, "Mbit/s")
    } else {
        (abs / 1_000.0, "kbit/s")
    };
    let digits = if scaled.fract() == 0.0 || scaled >= 100.0 {
        0
    } else {
        1
    };
    format!("{} {unit}", number(scaled, digits, false))
}

/// `formatRatio`: "0,0 %".
pub fn format_ratio(ratio: Option<f64>) -> String {
    match ratio.filter(|r| r.is_finite()) {
        Some(r) => texts::percent(&number(r * 100.0, 1, true)),
        None => texts::value_unknown().into(),
    }
}

/// `formatMs`: up to one decimal.
pub fn format_ms(ms: Option<f64>) -> String {
    match ms.filter(|m| m.is_finite()) {
        Some(m) => format!("{} ms", number(m, 1, false)),
        None => texts::value_unknown().into(),
    }
}

/// `formatNumber` in the current language with up to (or, `fixed`,
/// exactly) `digits` decimals: "1.234,5" / "1,234.5".
pub fn number(value: f64, digits: usize, fixed: bool) -> String {
    intl::format_number(value, if fixed { digits } else { 0 }, digits)
}

/// Now as ISO 8601 in UTC with milliseconds, like `Date.toISOString`.
pub fn now_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    iso(now.as_millis() as i64)
}

/// `ms` since the epoch as `2026-10-28T20:00:00.000Z`.
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// `relevantSettings` of the media settings.
fn relevant_settings(settings: &CallSettings, processing: &Value) -> Value {
    let fps = logic::stream_profile(&settings.camera_profile)
        .fps
        .max(logic::stream_profile(&settings.screen_profile).fps);
    json!({
        "audioQuality": "standard",
        "economyMode": settings.economy_mode,
        "processingMode": settings.processing_mode.as_str(),
        "effectiveProcessing": processing,
        "audioMaxBitrate": null,
        "cameraProfile": settings.camera_profile,
        "screenProfile": settings.screen_profile,
        "echoCancellation": settings.echo_cancellation,
        "noiseSuppression": settings.noise_suppression,
        "autoGainControl": settings.auto_gain_control,
        "outputVolume": settings.output_volume,
        "inputGain": settings.input_gain,
        "shareSourceAudio": settings.share_source_audio,
        "sourceAudioVolume": settings.stream_volume,
        "sourceAudioMuted": settings.stream_muted,
        "sourceAudioMaxBitrate": null,
        "videoSendBudget": (settings.video_upload_limit > 0.0).then_some(settings.video_upload_limit),
        "videoMaxFps": fps,
        "customAudioInput": !settings.input.is_empty(),
        "customAudioOutput": !settings.output.is_empty(),
        "customVideoInput": !settings.camera.is_empty(),
    })
}

/// `buildDiagnosticExport`: what "Diagnose herunterladen" saves.
pub fn export(
    diagnostics: &Diagnostics,
    settings: &CallSettings,
    processing: &Value,
    app_version: &str,
    exported_at: &str,
) -> Value {
    let video = diagnostics
        .video
        .as_ref()
        .map_or(Value::Null, |v| v.to_json(app_version));
    json!({
        "exportedAt": exported_at,
        "appVersion": app_version,
        "browser": {
            "userAgent": format!("Gelabber Desktop {app_version}"), // i18n-ok: a user agent
            "platform": std::env::consts::OS,
            "language": gelabber_i18n::get().intl_tag(),
        },
        "settings": relevant_settings(settings, processing),
        "video": video,
        "note": texts::export_note(),
        "phases": diagnostics.phases.iter().map(|(at, phase)| json!({ "at": at, "phase": phase })).collect::<Vec<_>>(),
        "events": diagnostics.events.iter().map(|e| json!({
            "at": e.at, "kind": e.kind, "connection": "voice", "detail": e.detail,
        })).collect::<Vec<_>>(),
        "samples": diagnostics.samples.iter().map(Sample::to_json).collect::<Vec<_>>(),
    })
}

/// The export's file name (`gelabber-diagnose-<time>.json`, `:` and `.`
/// as dashes).
pub fn export_name(exported_at: &str) -> String {
    format!(
        "gelabber-diagnose-{}.json",
        exported_at.replace([':', '.'], "-")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_793_217_600_123), "2026-10-28T20:00:00.123Z");
        assert_eq!(
            export_name("2026-10-28T20:00:00.123Z"),
            "gelabber-diagnose-2026-10-28T20-00-00-123Z.json"
        );
    }

    #[test]
    fn german_numbers() {
        assert_eq!(format_bps(Some(38_000.0)), "38 kbit/s");
        assert_eq!(format_bps(Some(1_250_000.0)), "1,3 Mbit/s");
        assert_eq!(format_bps(Some(352_000.0)), "352 kbit/s");
        assert_eq!(format_bps(None), "unbekannt");
        assert_eq!(format_ratio(Some(0.06)), "6,0 %");
        assert_eq!(format_ms(Some(14.0)), "14 ms");
        assert_eq!(number(1234.5, 1, false), "1.234,5");
    }

    fn flow(send: bool, loss: f64) -> Flow {
        Flow {
            send,
            audio: true,
            source: "voice",
            codec: Some("opus".into()),
            bitrate_bps: Some(40_000.0),
            loss: Some(loss),
            ..Flow::default()
        }
    }

    #[test]
    fn everyones_voice_is_one_row() {
        let groups = flow_groups(&[flow(false, 0.0), flow(true, 0.0), flow(false, 0.06)]);
        assert_eq!(groups.len(), 2);
        assert!(groups[0].send);
        assert_eq!(groups[1].name, "Sprache");
        assert_eq!(groups[1].count, 2);
        assert_eq!(groups[1].loss, Some(0.06));
        assert_eq!(groups[1].bitrate_bps, Some(80_000.0));
    }

    #[test]
    fn the_export_has_the_web_keys() {
        let mut d = Diagnostics::default();
        d.push(Sample {
            at: iso(1_793_217_600_000),
            phase: "voice-only",
            transport: Transport {
                relay: Some(false),
                rtt_ms: Some(14.0),
                ..Transport::default()
            },
            flows: vec![flow(true, 0.0)],
        });
        d.note("recovery", "Verbindung wiederhergestellt");
        let out = export(
            &d,
            &CallSettings::default(),
            &Value::Null,
            "0.9.0",
            "2026-10-28T20:00:00.000Z",
        );
        let keys = |v: &Value| -> Vec<String> {
            let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
            k.sort();
            k
        };
        let mut top = vec![
            "exportedAt",
            "appVersion",
            "browser",
            "settings",
            "video",
            "note",
            "phases",
            "events",
            "samples",
        ];
        top.sort();
        assert_eq!(keys(&out), top);
        let mut settings = vec![
            "audioQuality",
            "economyMode",
            "processingMode",
            "effectiveProcessing",
            "audioMaxBitrate",
            "cameraProfile",
            "screenProfile",
            "echoCancellation",
            "noiseSuppression",
            "autoGainControl",
            "outputVolume",
            "inputGain",
            "shareSourceAudio",
            "sourceAudioVolume",
            "sourceAudioMuted",
            "sourceAudioMaxBitrate",
            "videoSendBudget",
            "videoMaxFps",
            "customAudioInput",
            "customAudioOutput",
            "customVideoInput",
        ];
        settings.sort();
        assert_eq!(keys(&out["settings"]), settings);
        assert_eq!(keys(&out["browser"]), ["language", "platform", "userAgent"]);
        let sample = &out["samples"][0];
        assert_eq!(keys(sample), ["at", "phase", "voice", "watch"]);
        assert_eq!(sample["voice"]["transport"]["path"], "direct");
        assert_eq!(sample["voice"]["flows"][0]["direction"], "send");
        assert_eq!(out["phases"][0]["phase"], "voice-only");
        assert_eq!(out["events"][0]["kind"], "recovery");
        assert_eq!(
            out["note"],
            "Messwerte sind Anhaltspunkte und keine bewiesene Fehlerursache."
        );
    }

    #[test]
    fn rates_come_from_two_samples() {
        let mut d = Diagnostics::default();
        let stats = |bytes: u64, at: f64| {
            json!([
                {"type": "codec", "id": "c1", "mimeType": "audio/opus"},
                {"type": "inbound-rtp", "id": "i1", "kind": "audio", "codecId": "c1",
                 "bytesReceived": bytes, "timestamp": at, "packetsLost": 1, "packetsReceived": 99,
                 "jitter": 0.002}
            ])
        };
        let first = d.flows(&stats(0, 0.0), false, Kind::A, 0.0);
        assert_eq!(first[0].bitrate_bps, None);
        assert_eq!(first[0].codec.as_deref(), Some("opus"));
        let second = d.flows(&stats(10_000, 2_000_000.0), false, Kind::A, 2000.0);
        assert_eq!(second[0].bitrate_bps, Some(40_000.0));
        assert_eq!(second[0].loss, Some(0.01));
        assert_eq!(second[0].jitter_ms, Some(2.0));
    }

    /// libwebrtc stamps each stats object in microseconds
    /// (`RTCStats::ToJson`: `timestamp_.us()`); the core passes them on.
    #[test]
    fn rates_read_microsecond_timestamps() {
        let mut d = Diagnostics::default();
        let sample = |bytes: u64, at_us: u64| {
            json!([
                {"type": "codec", "id": "COT01_111", "timestamp": at_us,
                 "mimeType": "audio/opus", "payloadType": 111, "clockRate": 48000, "channels": 2},
                {"type": "outbound-rtp", "id": "OT01A1234", "timestamp": at_us, "ssrc": 1234,
                 "kind": "audio", "mediaType": "audio", "transportId": "T01",
                 "codecId": "COT01_111", "packetsSent": 100, "bytesSent": bytes,
                 "headerBytesSent": 1200, "mid": "0", "mediaSourceId": "SA1", "active": true}
            ])
        };
        let at_ms = 1_793_217_600_000.0;
        let first = d.flows(&sample(10_000, 1_793_217_600_000_000), true, Kind::A, at_ms);
        assert_eq!(first[0].bitrate_bps, None);
        // 10 kB more two seconds later: 40 kbit/s.
        let second = d.flows(
            &sample(20_000, 1_793_217_602_000_000),
            true,
            Kind::A,
            at_ms + 2000.0,
        );
        assert_eq!(second[0].bitrate_bps, Some(40_000.0));
    }
}
