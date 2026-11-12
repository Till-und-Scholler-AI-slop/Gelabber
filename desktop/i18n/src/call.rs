//! The call engine's own messages (desktop/call): why a call failed or
//! ended, and the notices it raises while running. The wording follows
//! the web's voice catalog (error.*, video.*, mic.*) where the texts are
//! the same; the rest is the desktop app's own.

use crate::{Locale, get};

crate::strings! {
    // A media error code of the voice server (voice error.*).
    error_update = "Bitte Gelabber aktualisieren und den Sprachkanal erneut betreten."
        | "Please update Gelabber and join the voice channel again.";
    error_unauthorized = "Der Sprachserver hat die Verbindung nicht zugelassen."
        | "The voice server refused the connection.";
    error_gone = "Der Sprachserver ist gerade nicht erreichbar."
        | "The voice server can’t be reached right now.";
    error_unavailable = "Kein freier Sprachplatz." | "No free voice seat.";
    error_forbidden = "Dafür fehlt dir die Berechtigung."
        | "You don’t have permission for that.";
    error_stream_failed = "Der Stream konnte nicht verbunden werden. Der Sprachkanal bleibt aktiv."
        | "The stream could not be connected. The voice channel stays active.";
    error_reconnect_failed = "Die Sprachverbindung konnte nicht wiederhergestellt werden. Bitte erneut beitreten."
        | "The voice connection could not be restored. Please join again.";

    // Live and the camera.
    live_not_allowed = "In diesem Server darfst du nicht live gehen."
        | "You can’t go live on this server.";
    camera_unavailable = "Die Kamera ist nicht verfügbar." | "The camera isn’t available.";
    rejoin_to_go_live = "Betritt den Sprachkanal neu, um live zu gehen."
        | "Rejoin the voice channel to go live.";
    screen_failed = "Die Bildschirmaufnahme konnte nicht gestartet werden."
        | "Screen capture could not be started.";

    // The microphone.
    mic_not_found = "Kein Mikrofon gefunden, du hörst nur zu."
        | "No microphone found, you’re only listening.";
    mic_denied = "Mikrofonzugriff nicht erlaubt, du hörst nur zu."
        | "Microphone access not allowed, you’re only listening.";
    mic_listen_only = "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu"
        | "Microphone could not be turned on; you can still listen";
    mic_switch_failed = "Mikrofonwechsel fehlgeschlagen; bisheriges Mikrofon bleibt aktiv"
        | "Couldn’t switch microphone; the previous one stays on";
    mic_test_failed = "Mikrofontest konnte nicht starten. Prüfe das Eingabegerät in den Systemeinstellungen."
        | "The microphone test couldn’t start. Check the input device in your system settings.";
    mic_test_stalled = "Das Mikrofon liefert keine Daten. Prüfe das Eingabegerät und starte den Test erneut."
        | "The microphone is sending no data. Check the input device and start the test again.";
    mic_test_no_levels = "Mikrofonpegel nicht verfügbar." | "Microphone level unavailable.";

    // The media core.
    no_media_core = "Anrufe brauchen den Medienkern der Desktop-App, der in diesem Build fehlt."
        | "Calls need the desktop app’s media core, which this build doesn’t have.";
}

/// "Der Medienkern konnte nicht starten ({reason})."
pub fn engine_failed(reason: &str) -> String {
    match get() {
        Locale::De => format!("Der Medienkern konnte nicht starten ({reason})."),
        Locale::En => format!("The media core couldn’t start ({reason})."),
    }
}

/// "Die Bildschirmaufnahme konnte nicht gestartet werden ({reason})."
pub fn screen_failed_because(reason: &str) -> String {
    match get() {
        Locale::De => {
            format!("Die Bildschirmaufnahme konnte nicht gestartet werden ({reason}).")
        }
        Locale::En => format!("Screen capture could not be started ({reason})."),
    }
}
