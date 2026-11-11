//! Voice diagnostics (web/src/i18n/catalogs/diagnostics.ts): the
//! connection's status and measurements, the Diagnose tab, the saved
//! report, and the audio processing's state. The desktop app's own texts
//! follow the web's entries.

use crate::{Locale, get, plural};

crate::strings! {
    processing_inactive = "Mikrofon inaktiv"
        | "Microphone inactive";
    processing_enhanced = "Lokale Rauschunterdrückung aktiv"
        | "Local noise suppression on";
    processing_browser = "Browserfilter aktiv"
        | "Browser filters on";
    processing_original = "Original / Musik aktiv"
        | "Original / music on";
    processing_browser_no_filters = "Browser-Modus aktiv; Browser meldet keine Rauschfilter"
        | "Browser mode on; the browser reports no noise filters";
    processing_resuming = "Audioverarbeitung pausiert; Wiederaufnahme läuft"
        | "Audio processing paused; resuming";
    processing_mic_lost = "Mikrofon getrennt; Standardgerät wird aktiviert"
        | "Microphone disconnected; switching to the default";
    processing_processor_fallback = "Audioprozessor ausgefallen; Ersatzmikrofon wird aktiviert"
        | "Audio processor failed; switching to a fallback microphone";
    processing_mic_failed = "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu"
        | "Microphone could not be turned on; you can still listen";
    processing_error_model_stalled = "Rauschfilter lädt nicht weiter"
        | "Noise filter stopped loading";
    processing_error_model_load = "Rauschfilter konnte nicht geladen werden"
        | "Noise filter could not be loaded";
    processing_error_model_checksum = "Rauschfilter-Prüfsumme stimmt nicht"
        | "Noise filter checksum doesn’t match";
    processing_error_ended = "Audioverarbeitung beendet"
        | "Audio processing ended";
    processing_error_resume = "Audioverarbeitung konnte nicht fortgesetzt werden"
        | "Audio processing could not resume";
    processing_error_paused = "Audioverarbeitung pausiert"
        | "Audio processing paused";
    processing_error_paused_at_start = "Audioverarbeitung pausiert beim Start"
        | "Audio processing paused at start";
    processing_error_noise_filter_unsupported = "Lokaler Rauschfilter wird nicht unterstützt"
        | "Local noise filter isn’t supported";
    processing_error_worklet_timeout = "Rauschfilter startet nicht"
        | "Noise filter doesn’t start";
    processing_error_worklet_failed = "Rauschfilter konnte nicht starten"
        | "Noise filter could not start";
    processing_error_cancelled = "Mikrofonanfrage abgebrochen"
        | "Microphone request cancelled";
    processing_error_cancelled_or_failed = "Mikrofonanfrage abgebrochen oder Audioprozessor ausgefallen"
        | "Microphone request cancelled or audio processor failed";
    value_unknown = "unbekannt"
        | "unknown";
    export_note = "Messwerte sind Anhaltspunkte und keine bewiesene Fehlerursache."
        | "Measurements are clues, not a proven cause of a fault.";
    phase_voice_only = "nur Sprache"
        | "voice only";
    phase_stream_on = "Stream an"
        | "stream on";
    phase_stream_off = "Stream aus"
        | "stream off";
    phase_watch = "Watch"
        | "Watch";
    source_voice = "Sprache"
        | "Voice";
    source_camera = "Kamera"
        | "Camera";
    source_screen = "Live"
        | "Live";
    source_screen_audio = "Live-Ton"
        | "Live audio";
    source_video = "Video"
        | "Video";
    source_watch = "Watch"
        | "Watch";
    limit_none = "keine"
        | "none";
    limit_cpu = "CPU"
        | "CPU";
    limit_bandwidth = "Bandbreite"
        | "bandwidth";
    limit_other = "sonstige"
        | "other";
    quality_good = "gut"
        | "good";
    quality_fair = "ordentlich"
        | "fair";
    quality_weak = "schwach"
        | "weak";
    quality_poor = "schlecht"
        | "poor";
    quality_direct = "Direkt verbunden"
        | "Connected directly";
    quality_relay = "Über einen Relay-Server verbunden"
        | "Connected through a relay server";
    quality_low_loss = "kaum Verlust"
        | "hardly any loss";
    status_label = "Verbindung"
        | "Connection";
    status_measuring = "Verbindung wird gemessen …"
        | "Measuring the connection…";
    status_not_measuring = "Keine laufende Messung"
        | "Not measuring";
    status_direct = "direkt verbunden"
        | "connected directly";
    status_relay = "über Relay"
        | "via relay";
    status_ping = "Ping"
        | "Ping";
    status_packet_loss = "Paketverlust"
        | "Packet loss";
    status_percent_unit = " %"
        | "%";
    status_connected = "Verbunden"
        | "Connected";
    status_path_direct = "Direkt"
        | "Direct";
    status_path_relay = "Relay"
        | "Relay";
    trace_now = "jetzt"
        | "now";
    ui_label = "Sprachdiagnose"
        | "Voice diagnostics";
    ui_download_title = "Speichert die Messwerte als JSON-Datei, etwa für einen Fehlerbericht."
        | "Saves the measurements as a JSON file, for example for a bug report.";
    ui_download = "Diagnose herunterladen"
        | "Download diagnostics";
    tip_text = "Viel Verlust beim Senden? Der Sparmodus begrenzt deinen Upload."
        | "Losing a lot when sending? Data saver limits your upload.";
    tip_action = "Zum Sparmodus"
        | "Go to data saver";
    flows_title = "Übertragung"
        | "Transmission";
    flows_last_measurement = "letzte Messung"
        | "last measurement";
    flows_waiting = "Noch nichts gemessen. Die ersten Werte kommen nach zwei Sekunden."
        | "Nothing measured yet. The first values arrive after two seconds.";
    flows_idle = "Keine laufende Messung. Werte gibt es, sobald du in einem Anruf bist."
        | "Not measuring. Values appear once you’re in a call.";
    flows_track = "Spur"
        | "Track";
    flows_codec = "Codec"
        | "Codec";
    flows_bitrate = "Bitrate"
        | "Bitrate";
    flows_loss = "Verlust"
        | "Loss";
    flows_jitter = "Jitter"
        | "Jitter";
    flows_no_limit = "Gelabber setzt keine Bitratengrenze."
        | "Gelabber sets no bitrate limit.";
    flows_sending = "Senden:"
        | "Sending:";
    flows_receiving = "Empfangen:"
        | "Receiving:";
    mic_title = "Mikrofon"
        | "Microphone";
    mic_processing = "Verarbeitung"
        | "Processing";
    mic_format = "Format"
        | "Format";
    mic_mono = "Mono"
        | "Mono";
    mic_stereo = "Stereo"
        | "Stereo";
    mic_echo = "Echo-Unterdrückung"
        | "Echo cancellation";
    mic_noise = "Rauschfilter (Browser)"
        | "Noise filter (browser)";
    mic_auto_gain = "Auto-Gain (Browser)"
        | "Auto gain (browser)";
    mic_gain = "Mikrofonverstärkung"
        | "Microphone gain";
    mic_buffer = "Filter-Puffer"
        | "Filter buffer";
    value_on = "an"
        | "on";
    value_off = "aus"
        | "off";
    value_yes = "ja"
        | "yes";
    value_no = "nein"
        | "no";
    debug_title = "Für die Fehlersuche"
        | "For troubleshooting";
    debug_subtitle = "Codecs, Prioritäten, Verlauf, Ereignisse"
        | "Codecs, priorities, history, events";
    debug_video = "Video auf diesem Gerät"
        | "Video on this device";
    debug_connections = "Verbindungen"
        | "Connections";
    debug_audio_graph = "Audio-Graph"
        | "Audio graph";
    debug_no_graph = "ohne Graph"
        | "no graph";
    debug_tracks = "Spuren einzeln"
        | "Individual tracks";
    debug_no_rtp = "Keine RTP-Statistik."
        | "No RTP statistics.";
    debug_hint = "Obergrenze meint die eingestellte Grenze, nicht die gemessene Bitrate. Prioritätswunsch: Audio hoch, Video normal. Browser und Netzwerk können diese Hinweise ignorieren; das garantiert keine Priorisierung auf dem SFU oder im Internet."
        | "Limit means the configured cap, not the measured bitrate. Requested priority: audio high, video normal. Browsers and networks may ignore these hints; nothing guarantees priority on the SFU or the internet.";
    debug_history = "Verlauf"
        | "History";
    debug_no_history = "noch kein Verlauf"
        | "no history yet";
    debug_time = "Zeit"
        | "Time";
    debug_phase = "Phase"
        | "Phase";
    debug_voice_sent = "Sprache senden"
        | "Voice sent";
    debug_voice_received = "Sprache empfangen"
        | "Voice received";
    debug_own_video = "Eigenes Video"
        | "Own video";
    debug_events = "Ereignisse"
        | "Events";
    debug_no_events = "noch keine Ereignisse"
        | "no events yet";
    debug_room_codecs = "Raum-Codecs"
        | "Room codecs";
    debug_av1_hardware = "AV1 in Hardware"
        | "AV1 in hardware";
    debug_h264_hardware = "H.264 in Hardware"
        | "H.264 in hardware";
    debug_vp9_layers = "VP9 in Schichten"
        | "VP9 in layers";
    debug_av1_receive = "AV1 empfangen"
        | "Receives AV1";
    transport_direct = "direkt"
        | "direct";
    line_send = "Senden"
        | "Send";
    line_receive = "Empfangen"
        | "Receive";
    line_no_cap = "keine Gelabber-Bitratengrenze"
        | "no Gelabber bitrate limit";
    line_not_reported = "nicht gemeldet"
        | "not reported";
    line_resolution_unknown = "Auflösung unbekannt"
        | "Resolution unknown";
    line_fps_unknown = "FPS unbekannt"
        | "FPS unknown";
    event_stream_start = "Streamstart"
        | "Stream start";
    event_stream_stop = "Streamstopp"
        | "Stream stop";
    event_device_change = "Gerätewechsel"
        | "Device change";
    event_sdp_error = "SDP-Fehler"
        | "SDP error";
    event_ice_error = "ICE-Fehler"
        | "ICE error";
    event_recovery = "Recovery"
        | "Recovery";
}

/// "Audioprozessor ausgefallen; {state} (Mikrofonverstärkung 100 %)".
pub fn processing_processor_failed(state: &str) -> String {
    match get() {
        Locale::De => format!("Audioprozessor ausgefallen; {state} (Mikrofonverstärkung 100 %)"),
        Locale::En => format!("Audio processor failed; {state} (microphone gain 100%)"),
    }
}

/// "Lokaler Rauschfilter nicht unterstützt; {state}".
pub fn processing_noise_filter_unsupported(state: &str) -> String {
    match get() {
        Locale::De => format!("Lokaler Rauschfilter nicht unterstützt; {state}"),
        Locale::En => format!("Local noise filter not supported; {state}"),
    }
}

/// "Lokaler Rauschfilter nicht verfügbar; {state} (Mikrofonverstärkung 100 %)".
pub fn processing_noise_filter_unavailable(state: &str) -> String {
    match get() {
        Locale::De => {
            format!("Lokaler Rauschfilter nicht verfügbar; {state} (Mikrofonverstärkung 100 %)")
        }
        Locale::En => format!("Local noise filter unavailable; {state} (microphone gain 100%)"),
    }
}

/// "Audioverarbeitung nicht verfügbar; {state} (Mikrofonverstärkung 100 %)".
pub fn processing_unavailable(state: &str) -> String {
    match get() {
        Locale::De => {
            format!("Audioverarbeitung nicht verfügbar; {state} (Mikrofonverstärkung 100 %)")
        }
        Locale::En => format!("Audio processing unavailable; {state} (microphone gain 100%)"),
    }
}

/// "{percent} %".
pub fn percent(percent: &str) -> String {
    match get() {
        Locale::De => format!("{percent} %"),
        Locale::En => format!("{percent}%"),
    }
}

/// "Verbindung: {label}".
pub fn quality_summary(label: &str) -> String {
    match get() {
        Locale::De => format!("Verbindung: {label}"),
        Locale::En => format!("Connection: {label}"),
    }
}

/// "{percent} Verlust".
pub fn quality_loss_summary(percent: &str) -> String {
    match get() {
        Locale::De => format!("{percent} Verlust"),
        Locale::En => format!("{percent} loss"),
    }
}

/// "Verbindung {label}.".
pub fn quality_verdict(label: &str) -> String {
    match get() {
        Locale::De => format!("Verbindung {label}."),
        Locale::En => format!("Connection {label}."),
    }
}

/// "etwas Verlust ({percent})".
pub fn quality_some_loss(percent: &str) -> String {
    match get() {
        Locale::De => format!("etwas Verlust ({percent})"),
        Locale::En => format!("some loss ({percent})"),
    }
}

/// "hoher Verlust ({percent})".
pub fn quality_high_loss(percent: &str) -> String {
    match get() {
        Locale::De => format!("hoher Verlust ({percent})"),
        Locale::En => format!("high loss ({percent})"),
    }
}

/// "hohe Verzögerung ({ms} ms)".
pub fn quality_high_delay(ms: &str) -> String {
    match get() {
        Locale::De => format!("hohe Verzögerung ({ms} ms)"),
        Locale::En => format!("high delay ({ms} ms)"),
    }
}

/// "Verbindung {label}".
pub fn status_verdict(label: &str) -> String {
    match get() {
        Locale::De => format!("Verbindung {label}"),
        Locale::En => format!("Connection {label}"),
    }
}

/// "misst alle {seconds} s".
pub fn status_interval(seconds: &str) -> String {
    match get() {
        Locale::De => format!("misst alle {seconds} s"),
        Locale::En => format!("measures every {seconds} s"),
    }
}

/// "Ping der letzten {seconds} s".
pub fn trace_caption(seconds: &str) -> String {
    match get() {
        Locale::De => format!("Ping der letzten {seconds} s"),
        Locale::En => format!("Ping over the last {seconds} s"),
    }
}

/// "Ping der letzten {seconds} Sekunden: {low} bis {high} Millisekunden".
pub fn trace_label(seconds: &str, low: &str, high: &str) -> String {
    match get() {
        Locale::De => {
            format!("Ping der letzten {seconds} Sekunden: {low} bis {high} Millisekunden")
        }
        Locale::En => format!("Ping over the last {seconds} seconds: {low} to {high} milliseconds"),
    }
}

/// "Ping der letzten {seconds} Sekunden: noch nichts gemessen".
pub fn trace_label_empty(seconds: &str) -> String {
    match get() {
        Locale::De => format!("Ping der letzten {seconds} Sekunden: noch nichts gemessen"),
        Locale::En => format!("Ping over the last {seconds} seconds: nothing measured yet"),
    }
}

/// "vor {seconds} s".
pub fn trace_ago(seconds: &str) -> String {
    match get() {
        Locale::De => format!("vor {seconds} s"),
        Locale::En => format!("{seconds} s ago"),
    }
}

/// "Obergrenzen: {limits}.".
pub fn flows_limits(count: u64, limits: &str) -> String {
    match get() {
        Locale::De => plural(
            count,
            format!("Obergrenze: {limits}."),
            format!("Obergrenzen: {limits}."),
        ),
        Locale::En => plural(
            count,
            format!("Limit: {limits}."),
            format!("Limits: {limits}."),
        ),
    }
}

/// "Sprache {bitrate}".
pub fn flows_limit_voice(bitrate: &str) -> String {
    match get() {
        Locale::De => format!("Sprache {bitrate}"),
        Locale::En => format!("Voice {bitrate}"),
    }
}

/// "Video {bitrate}".
pub fn flows_limit_video(bitrate: &str) -> String {
    match get() {
        Locale::De => format!("Video {bitrate}"),
        Locale::En => format!("Video {bitrate}"),
    }
}

/// "Video höchstens {fps} FPS.".
pub fn flows_max_fps(fps: &str) -> String {
    match get() {
        Locale::De => format!("Video höchstens {fps} FPS."),
        Locale::En => format!("Video at most {fps} FPS."),
    }
}

/// "{count} Schichten".
pub fn flows_layers(count: u64) -> String {
    match get() {
        Locale::De => plural(
            count,
            format!("{count} Schicht"),
            format!("{count} Schichten"),
        ),
        Locale::En => plural(count, format!("{count} layer"), format!("{count} layers")),
    }
}

/// "gebremst durch {reason}".
pub fn flows_limited_by(reason: &str) -> String {
    match get() {
        Locale::De => format!("gebremst durch {reason}"),
        Locale::En => format!("held back by {reason}"),
    }
}

/// "{rate} kHz".
pub fn mic_kilo_hertz(rate: &str) -> String {
    match get() {
        Locale::De => format!("{rate} kHz"),
        Locale::En => format!("{rate} kHz"),
    }
}

/// "{count} Kanäle".
pub fn mic_channels(count: &str) -> String {
    match get() {
        Locale::De => format!("{count} Kanäle"),
        Locale::En => format!("{count} channels"),
    }
}

/// "{phase} · Senden {send} · Empfang {recv} · Video {video}".
pub fn debug_phase_line(phase: &str, send: &str, recv: &str, video: &str) -> String {
    match get() {
        Locale::De => format!("{phase} · Senden {send} · Empfang {recv} · Video {video}"),
        Locale::En => format!("{phase} · Send {send} · Receive {recv} · Video {video}"),
    }
}

/// "{path} · RTT {rtt} · Bandbreite senden {up}, empfangen {down}".
pub fn transport_line(path: &str, rtt: &str, up: &str, down: &str) -> String {
    match get() {
        Locale::De => format!("{path} · RTT {rtt} · Bandbreite senden {up}, empfangen {down}"),
        Locale::En => format!("{path} · RTT {rtt} · bandwidth send {up}, receive {down}"),
    }
}

/// "{name} (Video)".
pub fn line_video(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} (Video)"),
        Locale::En => format!("{name} (video)"),
    }
}

/// "Codec: {codec}".
pub fn line_codec(codec: &str) -> String {
    match get() {
        Locale::De => format!("Codec: {codec}"),
        Locale::En => format!("Codec: {codec}"),
    }
}

/// "Bitrate: {bitrate}".
pub fn line_bitrate(bitrate: &str) -> String {
    match get() {
        Locale::De => format!("Bitrate: {bitrate}"),
        Locale::En => format!("Bitrate: {bitrate}"),
    }
}

/// "Obergrenze: {cap}".
pub fn line_cap(cap: &str) -> String {
    match get() {
        Locale::De => format!("Obergrenze: {cap}"),
        Locale::En => format!("Limit: {cap}"),
    }
}

/// "Paketverlust: {loss}".
pub fn line_loss(loss: &str) -> String {
    match get() {
        Locale::De => format!("Paketverlust: {loss}"),
        Locale::En => format!("Packet loss: {loss}"),
    }
}

/// "Paketverlust: {loss} ({packets} Pakete)".
pub fn line_loss_packets(loss: &str, packets: &str) -> String {
    match get() {
        Locale::De => format!("Paketverlust: {loss} ({packets} Pakete)"),
        Locale::En => format!("Packet loss: {loss} ({packets} packets)"),
    }
}

/// "Jitter: {jitter}".
pub fn line_jitter(jitter: &str) -> String {
    match get() {
        Locale::De => format!("Jitter: {jitter}"),
        Locale::En => format!("Jitter: {jitter}"),
    }
}

/// "RTT: {rtt}".
pub fn line_rtt(rtt: &str) -> String {
    match get() {
        Locale::De => format!("RTT: {rtt}"),
        Locale::En => format!("RTT: {rtt}"),
    }
}

/// "Senderpriorität: {priority}".
pub fn line_sender_priority(priority: &str) -> String {
    match get() {
        Locale::De => format!("Senderpriorität: {priority}"),
        Locale::En => format!("Sender priority: {priority}"),
    }
}

/// "Netzpriorität: {priority}".
pub fn line_network_priority(priority: &str) -> String {
    match get() {
        Locale::De => format!("Netzpriorität: {priority}"),
        Locale::En => format!("Network priority: {priority}"),
    }
}

/// "Ersetzte Samples: {value}".
pub fn line_concealed(value: &str) -> String {
    match get() {
        Locale::De => format!("Ersetzte Samples: {value}"),
        Locale::En => format!("Concealed samples: {value}"),
    }
}

/// "FPS-Obergrenze: {fps}".
pub fn line_fps_cap(fps: &str) -> String {
    match get() {
        Locale::De => format!("FPS-Obergrenze: {fps}"),
        Locale::En => format!("FPS limit: {fps}"),
    }
}

/// "Qualitätslimit: {limit}".
pub fn line_quality_limit(limit: &str) -> String {
    match get() {
        Locale::De => format!("Qualitätslimit: {limit}"),
        Locale::En => format!("Quality limit: {limit}"),
    }
}
