//! The call (web/src/i18n/catalogs/voice.ts): the voice room and its
//! stage, tiles, the call bar and the voice controls in the sidebar, Live,
//! and the call's notices and errors. The desktop app's own texts follow
//! the web's entries.

use crate::{Locale, get, plural};

crate::strings! {
    state_icons_deafened = "Taub"
        | "Deafened";
    state_icons_muted = "Stumm"
        | "Muted";
    state_icons_in_voice = "Im Sprachkanal"
        | "In a voice channel";
    user_volume_volume = "Lautstärke"
        | "Volume";
    user_volume_mute_locally = "Lokal stummschalten"
        | "Mute for me";
    user_volume_reset = "Zurücksetzen"
        | "Reset";
    user_volume_hint = "Nur für dich hörbar. Live-Ton bleibt unverändert."
        | "Only you hear this. Live audio stays as it is.";
    user_volume_locally_muted = "Lokal stummgeschaltet"
        | "Muted for me";
    controls_mute = "Stummschalten"
        | "Mute";
    controls_unmute = "Stummschaltung aufheben"
        | "Unmute";
    controls_deafen = "Taubschalten"
        | "Deafen";
    controls_undeafen = "Taubschaltung aufheben"
        | "Undeafen";
    controls_muted = "Stumm"
        | "Muted";
    controls_deafened = "Taub"
        | "Deafened";
    controls_ptt_badge = "PTT"
        | "PTT";
    controls_camera = "Kamera"
        | "Camera";
    controls_camera_on = "Kamera einschalten"
        | "Turn camera on";
    controls_camera_off = "Kamera ausschalten"
        | "Turn camera off";
    mic_none = "Kein Mikrofon"
        | "No microphone";
    mic_not_found = "Mikrofon nicht gefunden oder belegt."
        | "Microphone not found or in use.";
    mic_blocked = "Der Browser blockiert das Mikrofon. Erlaube den Zugriff in den Website-Einstellungen."
        | "Your browser is blocking the microphone. Allow access in the site settings.";
    mic_choose = "Mikrofon wählen …"
        | "Choose microphone…";
    live_badge = "Live"
        | "Live";
    live_go_live = "Live gehen"
        | "Go Live";
    live_end = "Live beenden"
        | "End Live";
    live_you_are_live = "Du bist live"
        | "You’re live";
    live_hint = "Sehen alle im Sprachkanal"
        | "Everyone in the voice channel sees it";
    live_not_allowed = "In diesem Server darfst du nicht live gehen."
        | "You can’t go live on this server.";
    live_share_sound = "Ton mitsenden"
        | "Share sound";
    live_sound_hint = "Wähle den Ton im Freigabedialog des Browsers aus."
        | "Pick the sound in your browser’s sharing dialog.";
    live_sound_hint_next_live = "Wähle den Ton im Freigabedialog des Browsers aus. Ab dem nächsten Live."
        | "Pick the sound in your browser’s sharing dialog. From your next Live.";
    hold_to_talk_hold = "Zum Sprechen gedrückt halten"
        | "Hold to talk";
    hold_to_talk_talking = "Du sprichst"
        | "You’re talking";
    push_to_talk_mouse4 = "Maustaste 4"
        | "Mouse button 4";
    push_to_talk_mouse5 = "Maustaste 5"
        | "Mouse button 5";
    push_to_talk_space = "Leertaste"
        | "Space";
    dock_label = "Aktive Medien"
        | "Active media";
    dock_dismiss_notice = "Hinweis schließen"
        | "Dismiss notice";
    dock_playback_blocked = "Der Browser blockiert den Ton."
        | "Your browser is blocking the sound.";
    dock_start_playback = "Ton starten"
        | "Start sound";
    call_voice_channel = "Sprachkanal"
        | "Voice channel";
    call_connected = "Verbunden"
        | "Connected";
    call_connecting = "Verbinde …"
        | "Connecting…";
    call_reconnecting = "Verbindung wird wiederhergestellt …"
        | "Reconnecting…";
    call_lost = "Verbindung verloren"
        | "Connection lost";
    call_leave = "Verlassen"
        | "Leave";
    quality_measuring = "Verbindung: wird gemessen …"
        | "Connection: measuring…";
    quality_popover_label = "Verbindung"
        | "Connection";
    quality_sending = "Du sendest"
        | "You send";
    quality_receiving = "Du empfängst"
        | "You receive";
    quality_settings = "Einstellungen"
        | "Settings";
    quality_all_details = "Alle Details"
        | "All details";
    people_title = "Im Anruf"
        | "In the call";
    people_speaking = "spricht"
        | "speaking";
    people_someone = "Jemand"
        | "Someone";
    notice_sound_unavailable = "Der Browser hat keinen Live-Ton freigegeben. Das Video läuft weiter."
        | "Your browser didn’t share any Live audio. The video keeps going.";
    notice_sound_unsupported = "Dieser Medienserver unterstützt Live-Ton noch nicht. Das Video läuft weiter."
        | "This media server doesn’t support Live audio yet. The video keeps going.";
    notice_sound_ended = "Der Live-Ton wurde beendet. Das Video läuft weiter."
        | "The Live audio has ended. The video keeps going.";
    notice_sound_next_live = "Ton ab dem nächsten Live."
        | "Sound from your next Live.";
    announce_lost = "Verbindung verloren"
        | "Connection lost";
    announce_disconnected = "Getrennt"
        | "Disconnected";
    announce_restored = "Verbindung wiederhergestellt"
        | "Reconnected";
    announce_connected_to_voice = "Mit dem Sprachkanal verbunden"
        | "Connected to the voice channel";
    announce_reconnecting = "Verbindung wird wiederhergestellt"
        | "Reconnecting";
    names_you = "Du"
        | "You";
    summary_only_you = "Nur du bist im Kanal."
        | "It’s just you in the channel.";
    summary_nobody = "Niemand im Kanal."
        | "Nobody in the channel.";
    watch_watch = "Zuschauen"
        | "Watch";
    watch_to_channel = "Zum Kanal"
        | "Go to channel";
    people_member = "Mitglied"
        | "Member";
    room_sound_not_shared = "Kein Live-Ton freigegeben · Video läuft weiter"
        | "No Live audio shared · Video keeps going";
    room_sound_ended = "Live-Ton beendet · Video läuft weiter"
        | "Live audio ended · Video keeps going";
    room_sound_sharing = "Ton wird mitgesendet"
        | "Sharing sound";
    room_camera_on = "Kamera an"
        | "Camera on";
    room_view_label = "Ansicht"
        | "View";
    room_view_grid = "Raster"
        | "Grid";
    room_view_focus = "Fokus"
        | "Focus";
    room_cinema = "Kino"
        | "Cinema";
    room_cinema_title = "Kino (K)"
        | "Cinema (K)";
    room_leave_cinema = "Kino verlassen"
        | "Leave Cinema";
    room_esc_key = "Esc"
        | "Esc";
    room_stage = "Bühne"
        | "Stage";
    room_streams = "Übertragungen"
        | "Streams";
    room_without_camera = "Ohne Kamera"
        | "Without camera";
    room_participants = "Teilnehmer im Sprachkanal"
        | "People in the voice channel";
    room_free_seat = "Platz frei"
        | "Free seat";
    room_switch_note_unnamed = "Du verlässt dabei deinen Sprachkanal."
        | "This leaves your voice channel.";
    room_join = "Beitreten"
        | "Join";
    room_switch_here = "Hierher wechseln"
        | "Switch here";
    room_no_permission = "Keine Berechtigung für Sprachkanäle."
        | "No permission for voice channels.";
    room_test_microphone = "Mikrofon testen"
        | "Test microphone";
    room_ask_company = "Fehlt noch jemand?"
        | "Anyone missing?";
    room_invite = "Einladen"
        | "Invite";
    participant_deafened = "Taub"
        | "Deafened";
    participant_muted = "Stumm"
        | "Muted";
    participant_speaking = "Spricht"
        | "Speaking";
    participant_present = "Im Raum"
        | "In the room";
    participant_self = "(du)"
        | "(you)";
    participant_view_profile = "Profil ansehen"
        | "View profile";
    participant_local_volume = "Lokale Lautstärke"
        | "Volume for you";
    tile_playback_blocked = "Die Videowiedergabe ist blockiert."
        | "Video playback is blocked.";
    tile_start_playback = "Wiedergabe starten"
        | "Start playback";
    tile_loading = "Video wird geladen …"
        | "Loading video…";
    tile_no_picture = "Kein Bild empfangen"
        | "No picture received";
    tile_reload = "Neu laden"
        | "Reload";
    tile_with_sound = "Mit Ton"
        | "With sound";
    tile_without_sound = "Ohne Ton"
        | "No sound";
    tile_stop_watching = "Nicht mehr zuschauen"
        | "Stop watching";
    tile_pin = "Anheften"
        | "Pin";
    tile_fullscreen = "Vollbild"
        | "Full screen";
    tile_fullscreen_title = "Vollbild (auch per Doppelklick)"
        | "Full screen (or double-click)";
    tile_exit_fullscreen = "Vollbild beenden"
        | "Exit full screen";
    tile_mute_live_sound = "Live-Ton stummschalten"
        | "Mute Live audio";
    tile_unmute_live_sound = "Live-Ton wieder einschalten"
        | "Unmute Live audio";
    tile_live_sound_volume = "Live-Ton-Lautstärke"
        | "Live audio volume";
    error_failed = "Die Sprachverbindung ist fehlgeschlagen."
        | "The voice connection failed.";
    error_slow = "Der Sprachserver antwortet nicht. Bitte noch einmal versuchen."
        | "The voice server isn’t responding. Please try again.";
    error_busy = "Die Sprachverbindung ist überlastet. Bitte noch einmal versuchen."
        | "The voice connection is overloaded. Please try again.";
    error_update = "Bitte Gelabber aktualisieren und den Sprachkanal erneut betreten."
        | "Please update Gelabber and join the voice channel again.";
    error_forbidden = "Dafür fehlt dir in diesem Sprachkanal die Berechtigung."
        | "You don’t have permission for that in this voice channel.";
    error_unsupported_codec = "Dein Browser kann kein Format senden, das dieser Sprachkanal empfängt."
        | "Your browser can’t send any format this voice channel receives.";
    error_unavailable = "Kein freier Sprachplatz."
        | "No free voice seat.";
    error_unauthorized = "Der Sprachserver hat die Verbindung nicht zugelassen."
        | "The voice server refused the connection.";
    error_gone = "Der Sprachserver ist gerade nicht erreichbar."
        | "The voice server can’t be reached right now.";
    error_connection_closed = "Die Verbindung zum Sprachserver ist abgebrochen."
        | "The connection to the voice server dropped.";
    error_sdk_load_failed = "Die Anruffunktion konnte nicht geladen werden. Bitte Gelabber neu laden und den Sprachkanal erneut betreten."
        | "Calls could not be loaded. Please reload Gelabber and join the voice channel again.";
    error_sdk_load_timeout = "Die Anruffunktion lädt gerade sehr langsam. Bitte gleich den Sprachkanal erneut betreten."
        | "Calls are loading very slowly right now. Please join the voice channel again in a moment.";
    error_reconnect_failed = "Die Sprachverbindung konnte nicht wiederhergestellt werden. Bitte erneut beitreten."
        | "The voice connection could not be restored. Please join again.";
    error_stream_failed = "Der Stream konnte nicht verbunden werden. Der Sprachkanal bleibt aktiv."
        | "The stream could not be connected. The voice channel stays active.";
    error_room_load_failed = "Der Sprachkanal konnte nicht geladen werden."
        | "The voice channel could not be loaded.";
    video_profile_fallback = "Dieses Streamprofil wird nicht unterstützt. Es läuft eine sicherere Auflösung."
        | "This stream profile isn’t supported. A safer resolution is running.";
    video_camera_profile_unsupported = "Die Kamera unterstützt das Streamprofil nicht."
        | "The camera doesn’t support the stream profile.";
    video_screen_profile_unsupported = "Die Bildschirmaufnahme unterstützt das Streamprofil nicht."
        | "Screen capture doesn’t support the stream profile.";
    video_camera_switch_failed = "Kamerawechsel fehlgeschlagen; bisherige Kamera bleibt aktiv"
        | "Couldn’t switch camera; the previous one stays on";
    video_camera_unavailable = "Die Kamera ist nicht verfügbar."
        | "The camera isn’t available.";
    video_screen_failed = "Die Bildschirmaufnahme konnte nicht gestartet werden."
        | "Screen capture could not be started.";
    video_rejoin_to_go_live = "Betritt den Sprachkanal neu, um live zu gehen."
        | "Rejoin the voice channel to go live.";
    mic_processor_fallback = "Audioprozessor ausgefallen; lokale Aufnahme beendet, Browser-Ersatz wird aktiviert"
        | "Audio processor failed; local capture ended, switching to the browser’s filters";
    mic_processor_failed_before_switch = "Audioprozessor vor Mikrofonwechsel ausgefallen; Browser-Ersatz wird aktiviert"
        | "Audio processor failed before the microphone switch; switching to the browser’s filters";
    mic_lost = "Mikrofon getrennt – Standardgerät wird verwendet"
        | "Microphone disconnected – using the default device";
    mic_switch_failed = "Mikrofonwechsel fehlgeschlagen; bisheriges Mikrofon bleibt aktiv"
        | "Couldn’t switch microphone; the previous one stays on";
    mic_gain_failed = "Mikrofonverstärkung konnte nicht aktiviert werden; bisheriges Mikrofon bleibt aktiv"
        | "Microphone gain could not be turned on; the previous microphone stays on";
    mic_gain_lost = "Mikrofonverstärkung ausgefallen; bisheriges Mikrofon bleibt aktiv"
        | "Microphone gain failed; the previous microphone stays on";
    mic_consequence_listen = "du hörst nur zu"
        | "you’re only listening";
    mic_consequence_switch = "der Wechsel hat nicht geklappt"
        | "the switch didn’t work";
}

/// "{percent} %".
pub fn percent(percent: &str) -> String {
    match get() {
        Locale::De => format!("{percent} %"),
        Locale::En => format!("{percent}%"),
    }
}

/// "Taub in {channel}".
pub fn state_icons_deafened_in(channel: &str) -> String {
    match get() {
        Locale::De => format!("Taub in {channel}"),
        Locale::En => format!("Deafened in {channel}"),
    }
}

/// "Stumm in {channel}".
pub fn state_icons_muted_in(channel: &str) -> String {
    match get() {
        Locale::De => format!("Stumm in {channel}"),
        Locale::En => format!("Muted in {channel}"),
    }
}

/// "In {channel}".
pub fn state_icons_in_channel(channel: &str) -> String {
    match get() {
        Locale::De => format!("In {channel}"),
        Locale::En => format!("In {channel}"),
    }
}

/// "Lautstärke von {name}".
pub fn user_volume_section_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Lautstärke von {name}"),
        Locale::En => format!("Volume of {name}"),
    }
}

/// "{shortcut} · Push-to-Talk: {key}".
pub fn controls_push_to_talk_detail(shortcut: &str, key: &str) -> String {
    match get() {
        Locale::De => format!("{shortcut} · Push-to-Talk: {key}"),
        Locale::En => format!("{shortcut} · Push to talk: {key}"),
    }
}

/// "{key} links".
pub fn push_to_talk_left(key: &str) -> String {
    match get() {
        Locale::De => format!("{key} links"),
        Locale::En => format!("Left {key}"),
    }
}

/// "{key} rechts".
pub fn push_to_talk_right(key: &str) -> String {
    match get() {
        Locale::De => format!("{key} rechts"),
        Locale::En => format!("Right {key}"),
    }
}

/// "{summary} – Details anzeigen".
pub fn quality_show_details(summary: &str) -> String {
    match get() {
        Locale::De => format!("{summary} – Details anzeigen"),
        Locale::En => format!("{summary} – show details"),
    }
}

/// "Im Anruf: {names}".
pub fn people_label(names: &str) -> String {
    match get() {
        Locale::De => format!("Im Anruf: {names}"),
        Locale::En => format!("In the call: {names}"),
    }
}

/// "{name} (du)".
pub fn people_you(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} (du)"),
        Locale::En => format!("{name} (you)"),
    }
}

/// "Mit {channel} verbunden".
pub fn announce_connected_to(channel: &str) -> String {
    match get() {
        Locale::De => format!("Mit {channel} verbunden"),
        Locale::En => format!("Connected to {channel}"),
    }
}

/// "{a} und {b}".
pub fn names_two(a: &str, b: &str) -> String {
    match get() {
        Locale::De => format!("{a} und {b}"),
        Locale::En => format!("{a} and {b}"),
    }
}

/// "{a}, {b} und {c}".
pub fn names_three(a: &str, b: &str, c: &str) -> String {
    match get() {
        Locale::De => format!("{a}, {b} und {c}"),
        Locale::En => format!("{a}, {b} and {c}"),
    }
}

/// "{a}, {b} und {more} weitere".
pub fn names_more(a: &str, b: &str, more: &str) -> String {
    match get() {
        Locale::De => format!("{a}, {b} und {more} weitere"),
        Locale::En => format!("{a}, {b} and {more} more"),
    }
}

/// "{names} ist live".
pub fn is_live_one(names: &str) -> String {
    match get() {
        Locale::De => format!("{names} ist live"),
        Locale::En => format!("{names} is live"),
    }
}

/// "{names} sind live".
pub fn is_live_many(names: &str) -> String {
    match get() {
        Locale::De => format!("{names} sind live"),
        Locale::En => format!("{names} are live"),
    }
}

/// "{names} bist live".
pub fn is_live_self(names: &str) -> String {
    match get() {
        Locale::De => format!("{names} bist live"),
        Locale::En => format!("{names} are live"),
    }
}

/// "{names} seid live".
pub fn is_live_self_and_others(names: &str) -> String {
    match get() {
        Locale::De => format!("{names} seid live"),
        Locale::En => format!("{names} are live"),
    }
}

/// "Du bist hier mit {names}.".
pub fn summary_you_with(names: &str) -> String {
    match get() {
        Locale::De => format!("Du bist hier mit {names}."),
        Locale::En => format!("You’re here with {names}."),
    }
}

/// "{names} ist da.".
pub fn summary_there_one(names: &str) -> String {
    match get() {
        Locale::De => format!("{names} ist da."),
        Locale::En => format!("{names} is here."),
    }
}

/// "{names} sind da.".
pub fn summary_there_many(names: &str) -> String {
    match get() {
        Locale::De => format!("{names} sind da."),
        Locale::En => format!("{names} are here."),
    }
}

/// "{there} {live}.".
pub fn summary_there_and_live(there: &str, live: &str) -> String {
    match get() {
        Locale::De => format!("{there} {live}."),
        Locale::En => format!("{there} {live}."),
    }
}

/// "{count} im Kanal".
pub fn facts_people(count: &str) -> String {
    match get() {
        Locale::De => format!("{count} im Kanal"),
        Locale::En => format!("{count} in the channel"),
    }
}

/// "{count} Kameras".
pub fn facts_cameras(count: u64) -> String {
    match get() {
        Locale::De => plural(count, format!("{count} Kamera"), format!("{count} Kameras")),
        Locale::En => plural(count, format!("{count} camera"), format!("{count} cameras")),
    }
}

/// "{sentence} in {channel}.".
pub fn live_in_channel(sentence: &str, channel: &str) -> String {
    match get() {
        Locale::De => format!("{sentence} in {channel}."),
        Locale::En => format!("{sentence} in {channel}."),
    }
}

/// "{what} in {channel}".
pub fn hint_in(what: &str, channel: &str) -> String {
    match get() {
        Locale::De => format!("{what} in {channel}"),
        Locale::En => format!("{what} in {channel}"),
    }
}

/// "{sentence}.".
pub fn hint_sentence(sentence: &str) -> String {
    match get() {
        Locale::De => format!("{sentence}."),
        Locale::En => format!("{sentence}."),
    }
}

/// "{sentence} in <link>{channel}</link>.".
pub fn hint_sentence_in(sentence: &str, channel: &str) -> String {
    match get() {
        Locale::De => format!("{sentence} in <link>{channel}</link>."),
        Locale::En => format!("{sentence} in <link>{channel}</link>."),
    }
}

/// "{name} — Live".
pub fn room_live_tile(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} — Live"),
        Locale::En => format!("{name} — Live"),
    }
}

/// "Du verlässt dabei {channel}.".
pub fn room_switch_note(channel: &str) -> String {
    match get() {
        Locale::De => format!("Du verlässt dabei {channel}."),
        Locale::En => format!("This leaves {channel}."),
    }
}

/// "Profil von {name}".
pub fn participant_profile_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Profil von {name}"),
        Locale::En => format!("{name}’s profile"),
    }
}

/// "Optionen für {name}".
pub fn participant_options_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Optionen für {name}"),
        Locale::En => format!("Options for {name}"),
    }
}

/// "Vollbild: {name}".
pub fn tile_fullscreen_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Vollbild: {name}"),
        Locale::En => format!("Full screen: {name}"),
    }
}

/// "Steuerung: {name}".
pub fn tile_controls_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Steuerung: {name}"),
        Locale::En => format!("Controls: {name}"),
    }
}

/// "Kein Mikrofon gefunden, {consequence}.".
pub fn mic_failure_not_found(consequence: &str) -> String {
    match get() {
        Locale::De => format!("Kein Mikrofon gefunden, {consequence}."),
        Locale::En => format!("No microphone found, {consequence}."),
    }
}

/// "Mikrofonzugriff nicht erlaubt, {consequence}.".
pub fn mic_failure_not_allowed(consequence: &str) -> String {
    match get() {
        Locale::De => format!("Mikrofonzugriff nicht erlaubt, {consequence}."),
        Locale::En => format!("Microphone access not allowed, {consequence}."),
    }
}

/// "Das Mikrofon ist gerade nicht verfügbar, {consequence}.".
pub fn mic_failure_unavailable(consequence: &str) -> String {
    match get() {
        Locale::De => format!("Das Mikrofon ist gerade nicht verfügbar, {consequence}."),
        Locale::En => format!("The microphone isn’t available right now, {consequence}."),
    }
}
