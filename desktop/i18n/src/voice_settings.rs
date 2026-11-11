//! Audio & video settings (web/src/i18n/catalogs/voiceSettings.ts): the
//! settings dialog, the quick call settings, push to talk and the
//! microphone test. The desktop app's own texts follow the web's entries.

use crate::{Locale, get};

crate::strings! {
    dialog_title = "Audio & Video"
        | "Audio & video";
    tabs_label = "Bereiche"
        | "Sections";
    tabs_audio = "Audio"
        | "Audio";
    tabs_video = "Video & Live"
        | "Video & Live";
    tabs_diagnostics = "Diagnose"
        | "Diagnostics";
    panel_load_failed = "Die Einstellungen konnten nicht geladen werden."
        | "The settings could not be loaded.";
    form_intro = "Passe Mikrofon, Lautsprecher und Kamera an. Die Übertragung hat standardmäßig keine Bitratengrenze durch Gelabber."
        | "Adjust your microphone, speakers and camera. By default, Gelabber puts no bitrate limit on what you send.";
    form_apply_note = "Änderungen gelten beim nächsten Start und werden, soweit unterstützt, auch auf laufende Streams angewendet."
        | "Changes apply from the next start and, where supported, to running streams too.";
    input_title = "Eingabe"
        | "Input";
    input_microphone = "Mikrofon"
        | "Microphone";
    input_test_microphone = "Mikrofon testen und vergleichen"
        | "Test and compare microphone";
    input_gain = "Mikrofonverstärkung"
        | "Microphone gain";
    input_gain_hint = "100 % fügt keine Verstärkung hinzu. Andere Werte laufen über Web Audio."
        | "100% adds no gain. Other values go through Web Audio.";
    input_filter = "Filter"
        | "Filter";
    output_title = "Ausgabe"
        | "Output";
    output_speaker = "Lautsprecher"
        | "Speaker";
    output_speaker_unsupported = "Dieser Browser kann den Lautsprecher nicht wählen. Der Ton folgt der Ausgabe deines Geräts."
        | "This browser can’t choose the speaker. Sound follows your device’s output.";
    output_calls = "Gespräche"
        | "Calls";
    output_calls_hint = "Über 100 % wird der Ton digital verstärkt. Nutze dafür Kopfhörer: Über Lautsprecher können die anderen sonst ihr eigenes Echo hören."
        | "Above 100%, sound is boosted digitally. Use headphones for that: through speakers, the others may hear their own echo.";
    output_live_sound = "Live-Ton"
        | "Live audio";
    output_live_sound_hint = "Der Ton von Lives hat eine eigene Lautstärke. Taub schaltet auch diesen Ton aus."
        | "Live audio has its own volume. Deafen turns it off too.";
    output_mute_live_sound = "Live-Ton stummschalten"
        | "Mute Live audio";
    output_call_sounds = "Signaltöne"
        | "Call sounds";
    output_call_sounds_hint = "Töne für Beitreten, Verlassen, Stumm, Taub, Kamera und Live."
        | "Sounds for join, leave, mute, deafen, camera and Live.";
    output_other_mute_sounds = "Töne, wenn andere sich stumm schalten"
        | "Sounds when others mute";
    output_call_sound_volume = "Lautstärke der Signaltöne"
        | "Call sound volume";
    output_call_sound_volume_speaker_hint = "Nutzt deinen gewählten Lautsprecher und die Wiedergabelautstärke."
        | "Uses your chosen speaker and the playback volume.";
    output_call_sound_volume_hint = "Nutzt die Wiedergabelautstärke."
        | "Uses the playback volume.";
    output_test_sound = "Testton abspielen"
        | "Play test sound";
    advanced_title = "Echo und Browserfilter"
        | "Echo and browser filters";
    advanced_subtitle = "Für Musik oder Lautsprecher"
        | "For music or speakers";
    advanced_echo = "Echo"
        | "Echo";
    advanced_echo_hint = "Echo-Unterdrückung (AEC). Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du sie ausschalten."
        | "Echo cancellation (AEC). Recommended with speakers. For music on headphones, you can turn it off.";
    advanced_noise = "Rauschfilter"
        | "Noise filter";
    advanced_noise_hint = "Rauschunterdrückung des Browsers."
        | "The browser’s noise suppression.";
    advanced_auto_gain = "Auto-Gain"
        | "Auto gain";
    advanced_auto_gain_hint = "Auto-Gain des Browsers. Gleicht leise und laute Mikrofone auf einen hörbaren Pegel an."
        | "The browser’s auto gain. Brings quiet and loud microphones to an audible level.";
    camera_title = "Kamera"
        | "Camera";
    camera_device = "Gerät"
        | "Device";
    devices_reveal_mic_failed = "Der Browser gibt die Gerätenamen ohne Mikrofonzugriff nicht heraus."
        | "Your browser won’t share device names without microphone access.";
    devices_reveal_mic_hint = "Die Gerätenamen zeigt der Browser erst, wenn du den Mikrofonzugriff erlaubst."
        | "Your browser shows device names once you allow microphone access.";
    devices_reveal_mic = "Gerätenamen anzeigen"
        | "Show device names";
    devices_reveal_camera_failed = "Der Browser gibt die Kameranamen ohne Kamerazugriff nicht heraus."
        | "Your browser won’t share camera names without camera access.";
    devices_reveal_camera_hint = "Die Kameranamen zeigt der Browser erst, wenn du den Kamerazugriff erlaubst."
        | "Your browser shows camera names once you allow camera access.";
    devices_reveal_camera = "Kameranamen anzeigen"
        | "Show camera names";
    devices_default = "Standard"
        | "Default";
    devices_chosen = "Gewähltes Gerät"
        | "Chosen device";
    quality_label = "Qualität"
        | "Quality";
    quality_resolution = "Auflösung"
        | "Resolution";
    quality_resolution_auto = "Auto (bis 1080p)"
        | "Auto (up to 1080p)";
    quality_frame_rate = "Bildrate"
        | "Frame rate";
    quality_frame_rate_auto = "Auto (bis 30 FPS)"
        | "Auto (up to 30 FPS)";
    quality_auto = "Automatisch"
        | "Automatic";
    quality_applied_live = "Auf den laufenden Stream angewendet."
        | "Applied to the running stream.";
    quality_applies_next = "Bitrate und FPS-Limit gelten sofort. Die Auflösung gilt ab dem nächsten Stream."
        | "Bitrate and FPS limit apply at once. Resolution applies from the next stream.";
    live_title = "Live"
        | "Live";
    live_quality_hint = "Höhere Werte brauchen mehr Rechenleistung und Upload. Die erreichbare Qualität hängt von Quelle, Gerät und Verbindung ab."
        | "Higher values need more processing power and upload. The quality you get depends on source, device and connection.";
    live_sound_label = "Ton im Live"
        | "Sound in a Live";
    live_share_sound = "Ton mitsenden"
        | "Share sound";
    live_share_sound_hint = "Gilt ab deinem nächsten Live; Ausschalten wirkt sofort. Wähle den Ton im Browserdialog aus; je nach Browser und Quelle ist nur Video verfügbar. Der Live-Ton wird als Stereo-Musik übertragen, ohne Mikrofonfilter."
        | "Applies from your next Live; turning it off works at once. Pick the sound in the browser dialog; depending on browser and source, only video may be available. Live audio is sent as stereo music, without microphone filters.";
    upload_title = "Upload"
        | "Upload";
    upload_no_limit_badge = "ohne Limit"
        | "no limit";
    upload_economy = "Sparmodus"
        | "Data saver";
    upload_video_limit = "Video-Limit"
        | "Video limit";
    upload_no_limit = "Ohne Limit"
        | "No limit";
    upload_custom_limit = "Eigenes Limit"
        | "Custom limit";
    upload_custom_limit_unit = "Mbit/s für alle Videos zusammen"
        | "Mbit/s for all videos combined";
    upload_custom_limit_label = "Eigenes gemeinsames Video-Limit (Mbit/s)"
        | "Custom combined video limit (Mbit/s)";
    upload_limit_hint = "Der Browser passt die tatsächliche Bitrate an die Verbindung an. Das Limit ist eine Obergrenze, keine feste Datenrate."
        | "The browser adapts the actual bitrate to the connection. The limit is a cap, not a fixed rate.";
    mode_enhanced = "Verbesserte Sprache"
        | "Enhanced voice";
    mode_enhanced_hint = "Lokale Rauschunterdrückung und Auto-Gain; Browserfilter als Ersatz"
        | "Local noise suppression and auto gain; browser filters as a fallback";
    mode_browser = "Browserfilter"
        | "Browser filters";
    mode_browser_hint = "Rauschunterdrückung und Auto-Gain deines Browsers"
        | "Your browser’s noise suppression and auto gain";
    mode_original = "Original / Musik"
        | "Original / music";
    mode_original_hint = "Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung"
        | "Stereo where supported, without noise filter, auto gain or silence detection";
    ptt_send = "Senden"
        | "Send";
    ptt_mode_label = "Sprachübertragung"
        | "Voice transmission";
    ptt_activity = "Sprachaktivierung"
        | "Voice activity";
    ptt_activity_hint = "Das Mikrofon sendet, sobald du sprichst."
        | "Your microphone sends as soon as you speak.";
    ptt_ptt = "Push-to-Talk"
        | "Push to talk";
    ptt_ptt_hint = "Das Mikrofon sendet nur, solange du die Taste hältst."
        | "Your microphone sends only while you hold the key.";
    ptt_key = "Taste"
        | "Key";
    ptt_recording = "Taste oder Maustaste drücken … (Esc bricht ab)"
        | "Press a key or mouse button… (Esc cancels)";
    ptt_set_key = "Taste festlegen"
        | "Set key";
    ptt_no_key_hint = "Ohne Taste bleibt dein Mikrofon im Gespräch stumm."
        | "Without a key, your microphone stays muted in calls.";
    ptt_key_hint = "Im Browser wirkt die Taste nur, solange Gelabber im Vordergrund ist. Die Desktop-App hört sie auch im Hintergrund."
        | "In the browser, the key only works while Gelabber is in the foreground. The desktop app hears it in the background too.";
    ptt_release_delay = "Nachlaufzeit"
        | "Release delay";
    ptt_release_delay_hint = "So lange sendet das Mikrofon nach dem Loslassen weiter."
        | "How long your microphone keeps sending after you let go.";
    mic_test_title = "Mikrofon testen"
        | "Test microphone";
    mic_test_intro = "Prüfe dein Mikrofon und vergleiche die Verarbeitung."
        | "Check your microphone and compare the processing.";
    mic_test_stopped = "Test beendet. Mikrofon und Aufnahmen wurden freigegeben."
        | "Test ended. Microphone and recordings have been released.";
    mic_test_local = "Der Test läuft lokal, ohne Übertragung. Aufnahmen dauern höchstens acht Sekunden und werden beim Schließen gelöscht."
        | "The test runs locally and sends nothing. Recordings last eight seconds at most and are deleted when you close it.";
    mic_test_input_level = "Eingangspegel"
        | "Input level";
    mic_test_processed = "Nach Verarbeitung"
        | "After processing";
    mic_test_processing = "Verarbeitung"
        | "Processing";
    mic_test_clipping = "Übersteuerung erkannt. Verringere die Mikrofonverstärkung oder den Gerätepegel."
        | "Clipping detected. Lower the microphone gain or the device level.";
    mic_test_stop = "Test beenden"
        | "Stop test";
    mic_test_start = "Test starten"
        | "Start test";
    mic_test_stop_recording = "Aufnahme beenden"
        | "Stop recording";
    mic_test_start_recording = "A/B-Aufnahme starten"
        | "Start A/B recording";
    mic_test_recording = "Lokale Aufnahme läuft (maximal acht Sekunden)."
        | "Recording locally (eight seconds at most).";
    mic_test_comparison = "Lokaler Audiovergleich"
        | "Local audio comparison";
    mic_test_input_clip = "Eingang (einschließlich Browser-Capture)"
        | "Input (including browser capture)";
    mic_test_delete_clips = "Aufnahmen löschen"
        | "Delete recordings";
    mic_test_switch_note = "Ein Wechsel der Verarbeitung startet den Test neu und löscht die Aufnahmen."
        | "Switching the processing restarts the test and deletes the recordings.";
    mic_test_error_not_allowed = "Der Mikrofonzugriff wurde nicht erlaubt. Du kannst die Berechtigung in den Browser-Einstellungen ändern und es erneut versuchen."
        | "Microphone access was not allowed. You can change the permission in your browser settings and try again.";
    mic_test_error_not_found = "Es wurde kein Mikrofon gefunden. Schließe ein Mikrofon an und versuche es erneut."
        | "No microphone was found. Connect a microphone and try again.";
    mic_test_error_not_readable = "Das Mikrofon ist gerade nicht verfügbar. Prüfe, ob eine andere Anwendung darauf zugreift."
        | "The microphone isn’t available right now. Check whether another app is using it.";
    mic_test_error_security = "Der Browser erlaubt hier keinen Mikrofonzugriff. Der Test benötigt eine sichere Verbindung oder localhost."
        | "Your browser doesn’t allow microphone access here. The test needs a secure connection or localhost.";
    mic_test_error_unsupported = "Dein Browser unterstützt hier keinen Mikrofonzugriff. Nutze einen aktuellen Browser mit HTTPS oder localhost."
        | "Your browser doesn’t support microphone access here. Use a current browser with HTTPS or localhost.";
    mic_test_error_ended = "Die Mikrofonverbindung wurde beendet. Du kannst den Test erneut starten."
        | "The microphone connection ended. You can start the test again.";
    mic_test_error_failed = "Der Mikrofontest konnte nicht gestartet werden. Prüfe dein Audiogerät und versuche es erneut."
        | "The microphone test could not start. Check your audio device and try again.";
    mic_test_allow_browser = "Bitte erlaube den lokalen Mikrofonzugriff im Browser."
        | "Please allow local microphone access in your browser.";
    mic_test_speak = "Sprich, um den Pegel zu prüfen."
        | "Speak to check the level.";
    mic_test_allow = "Bitte erlaube den lokalen Mikrofonzugriff."
        | "Please allow local microphone access.";
    mic_test_error_noise_filter = "Rauschfilter ausgefallen. Beende den Test und starte ihn erneut."
        | "Noise filter failed. Stop the test and start it again.";
    mic_test_error_connection_ended = "Mikrofonverbindung beendet. Starte den Test erneut."
        | "Microphone connection ended. Start the test again.";
    mic_test_error_denied = "Mikrofonzugriff nicht erlaubt. Prüfe die Browserberechtigung."
        | "Microphone access not allowed. Check the browser permission.";
    mic_test_error_start_failed = "Mikrofontest konnte nicht starten. Prüfe Gerät und Berechtigung."
        | "The microphone test couldn’t start. Check the device and permission.";
    mic_test_recording_unsupported = "Dieser Browser unterstützt keine lokale Aufnahme."
        | "This browser doesn’t support local recording.";
    mic_test_recording_failed = "Aufnahme fehlgeschlagen; Mikrofontest läuft weiter."
        | "Recording failed; the microphone test keeps running.";
    mic_test_recording_unsupported_here = "Lokale Aufnahme wird hier nicht unterstützt."
        | "Local recording isn’t supported here.";
    quick_call_settings = "Anrufeinstellungen"
        | "Call settings";
    quick_volume = "Lautstärke"
        | "Volume";
    quick_unmute_live_sound = "Live-Ton wieder einschalten"
        | "Unmute Live audio";
    quick_deafened = "Taubgeschaltet."
        | "Deafened.";
    quick_mic_denied = "Kein Zugriff aufs Mikrofon."
        | "No access to the microphone.";
    quick_names_after_permission = "Namen zeigt der Browser erst nach deiner Erlaubnis."
        | "Your browser shows names once you allow access.";
    quick_no_key = "Keine Taste festgelegt. Das Mikrofon bleibt stumm."
        | "No key set. Your microphone stays muted.";
    quick_all_settings = "Alle Einstellungen …"
        | "All settings…";
    quick_send_level = "Pegel, den die anderen hören"
        | "Level the others hear";

    // The desktop app's own.
    devices_system_default = "Systemstandard" | "System default";
    mic_test_local_only = "Der Test läuft lokal, ohne Aufnahme oder Übertragung."
        | "The test runs locally, without recording or sending.";
    mic_test_starting = "Mikrofon wird gestartet." | "Starting the microphone.";
}

/// "{percent} %".
pub fn percent(percent: &str) -> String {
    match get() {
        Locale::De => format!("{percent} %"),
        Locale::En => format!("{percent}%"),
    }
}

/// "Standard – {name}".
pub fn devices_default_named(name: &str) -> String {
    match get() {
        Locale::De => format!("Standard – {name}"),
        Locale::En => format!("Default – {name}"),
    }
}

/// "Ausdrückliche Obergrenzen: Sprache {voice} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}.".
pub fn upload_economy_on(voice: &str, video: &str) -> String {
    match get() {
        Locale::De => format!(
            "Ausdrückliche Obergrenzen: Sprache {voice} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}."
        ),
        Locale::En => format!(
            "Explicit limits: voice {voice} kbit/s, Live audio 128 kbit/s, video {video} combined."
        ),
    }
}

/// "Für wenig Upload: Sprache {voice} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}.".
pub fn upload_economy_off(voice: &str, video: &str) -> String {
    match get() {
        Locale::De => format!(
            "Für wenig Upload: Sprache {voice} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}."
        ),
        Locale::En => format!(
            "For low upload: voice {voice} kbit/s, Live audio 128 kbit/s, video {video} combined."
        ),
    }
}

/// "Sparmodus ({limit})".
pub fn upload_economy_limit(limit: &str) -> String {
    match get() {
        Locale::De => format!("Sparmodus ({limit})"),
        Locale::En => format!("Data saver ({limit})"),
    }
}
