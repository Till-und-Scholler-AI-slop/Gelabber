//! Personal settings (web/src/i18n/catalogs/settings.ts): the settings
//! frame and its areas (Profil, Profilkarte, Sicherheit, Darstellung, Audio,
//! Video, App). The Audio and Video areas keep the web's audio and video
//! settings wording (voiceSettings), as the desktop app words them: its
//! native core filters, not a browser. The desktop app's own area ("App":
//! updates and the window) is written in the same tone. Themes and
//! notifications have namespaces of their own.

use crate::{Locale, get};

crate::strings! {
    title = "Einstellungen" | "Settings";

    group_konto = "Konto" | "Account";
    group_app = "App" | "App";
    area_profil_label = "Profil" | "Profile";
    area_profil_description = "Name und Profilbild." | "Name and profile picture.";
    area_profilkarte_label = "Profilkarte" | "Profile card";
    area_profilkarte_description = "So sehen dich andere." | "How others see you.";
    area_konto_label = "Sicherheit" | "Security";
    area_konto_description = "Passwort ändern und überall abmelden."
        | "Change your password and sign out everywhere.";
    area_darstellung_label = "Darstellung" | "Appearance";
    area_darstellung_description = "Sprache, Farbschema, Textgröße und Bewegung."
        | "Language, colour scheme, text size and motion.";
    area_themes_label = "Themes" | "Themes";
    area_themes_description = "Farbpaletten und eigene Themes." | "Colour palettes and custom themes.";
    area_audio_label = "Audio" | "Audio";
    area_audio_description = "Mikrofon, Lautsprecher und Lautstärke."
        | "Microphone, speakers and volume.";
    area_video_label = "Video" | "Video";
    area_video_description = "Kamera, Live gehen und Bildqualität."
        | "Camera, Go Live and picture quality.";
    area_benachrichtigungen_label = "Benachrichtigungen" | "Notifications";
    area_benachrichtigungen_description = "Hinweise auf neue Nachrichten." | "Alerts for new messages.";
    area_app_label = "App" | "App";
    /// The desktop app's "App" area: its updates and its window.
    area_app_description_desktop = "Updates und was beim Schließen des Fensters passiert."
        | "Updates and what closing the window does.";
    /// The desktop app's "App" area with a tray but no updates.
    area_app_description_window = "Was beim Schließen des Fensters passiert."
        | "What closing the window does.";
    /// The desktop app's "App" area with updates but no tray.
    area_app_description_updates = "Updates dieser App." | "Updates for this app.";

    appearance_color_scheme = "Farbschema" | "Colour scheme";
    appearance_mode_light = "Hell" | "Light";
    appearance_mode_dark = "Dunkel" | "Dark";
    appearance_mode_system = "System" | "System";
    appearance_mode_system_hint = "Folgt dem Gerät" | "Follows your device";
    appearance_retry_apply = "Erneut anwenden" | "Apply again";
    appearance_more_themes = "Weitere Themes" | "More themes";
    appearance_view = "Ansicht" | "View";
    appearance_text_size = "Textgröße" | "Text size";
    appearance_text_size_hint = "Gilt für die ganze Oberfläche." | "Applies to the whole interface.";
    appearance_scale_compact = "Kompakt" | "Compact";
    appearance_scale_standard = "Standard" | "Standard";
    appearance_scale_large = "Groß" | "Large";
    appearance_compact_rooms = "Kompakte Sprachkanäle" | "Compact voice channels";
    appearance_compact_rooms_hint = "Kleinere Avatare und weniger Abstand."
        | "Smaller avatars and less spacing.";
    appearance_reduced_motion = "Bewegung reduzieren" | "Reduce motion";
    appearance_reduced_motion_hint = "Berücksichtigt auch die Einstellung deines Geräts."
        | "Also follows your device’s setting.";

    profile_upload = "Bild hochladen" | "Upload picture";
    profile_uploading = "Lädt hoch…" | "Uploading…";
    profile_removing = "Entfernt…" | "Removing…";
    profile_pick_label = "Avatar-Bild auswählen" | "Choose avatar picture";
    profile_name = "Name" | "Name";
    profile_avatar_url = "Avatar-URL" | "Avatar URL";
    profile_avatar_url_uploaded = "Hochgeladenes Bild" | "Uploaded picture";
    profile_avatar_url_replaces_upload = "Eine URL hier ersetzt das hochgeladene Bild."
        | "A URL here replaces the uploaded picture.";
    profile_avatar_url_removed_on_save = "Wird beim Speichern entfernt." | "Removed when you save.";
    profile_avatar_url_hint = "Wird bei jedem Aufruf von der fremden Seite geladen."
        | "Loaded from the other site every time it is shown.";
    profile_saving = "Speichern…" | "Saving…";
    profile_unchanged = "Keine Änderungen." | "No changes.";
    /// The avatar URL field's placeholder.
    profile_avatar_url_placeholder = "https://…" | "https://…";
    /// A picked or dropped picture that is too large (web auth avatarImage).
    image_too_large = "Das Bild ist zu groß (höchstens 20 MB)." | "The image is too large (at most 20 MB).";
    /// A picture the desktop app cannot read (web auth avatarImage, with
    /// Gelabber reading it instead of a browser).
    image_unreadable = "Dieses Bild kann Gelabber nicht lesen. PNG, JPEG, WebP oder GIF gehen."
        | "Gelabber can’t read this image. PNG, JPEG, WebP or GIF work.";

    /// Split with [`crate::pieces`]; `{email}` is filled in afterwards.
    security_signed_in_as = "Angemeldet als <b>{email}</b>" | "Signed in as <b>{email}</b>";
    security_current_password = "Aktuelles Passwort" | "Current password";
    security_new_password = "Neues Passwort" | "New password";
    security_new_password_hint = "Andere Geräte werden dabei abgemeldet."
        | "This signs out your other devices.";
    security_change = "Passwort ändern" | "Change password";
    security_changing = "Ändern…" | "Changing…";
    security_changed = "Geändert. Andere Geräte sind abgemeldet."
        | "Changed. Your other devices are signed out.";
    security_everywhere_title = "Überall abmelden" | "Sign out everywhere";
    security_everywhere_hint = "Beendet jede Anmeldung, auch auf diesem Gerät."
        | "Ends every session, on this device too.";
    security_everywhere_button = "Auf allen Geräten abmelden" | "Sign out on all devices";
    security_everywhere_confirm_title = "Auf allen Geräten abmelden?" | "Sign out on all devices?";
    security_everywhere_confirm_message = "Auch dieses Gerät. Danach musst du dich überall neu anmelden."
        | "This device too. Afterwards you need to sign in again everywhere.";
    security_everywhere_confirm_label = "Überall abmelden" | "Sign out everywhere";

    card_discard_title = "Ungespeicherte Änderungen verwerfen?" | "Discard unsaved changes?";
    card_discard_message = "Deine Änderungen an der Profilkarte gehen verloren."
        | "Your changes to the profile card will be lost.";
    card_discard = "Verwerfen" | "Discard";
    card_banner = "Banner" | "Banner";
    card_banner_upload = "Bannerbild hochladen" | "Upload banner image";
    card_banner_remove = "Bannerbild entfernen" | "Remove banner image";
    card_banner_saved = "Banner gespeichert." | "Banner saved.";
    card_banner_pick_label = "Bannerbild auswählen" | "Choose banner image";
    card_banner_hint = "Wird auf 3 : 1 zugeschnitten und sofort gespeichert."
        | "Cropped to 3 : 1 and saved straight away.";
    card_banner_color = "Bannerfarbe" | "Banner colour";
    card_banner_color_reset_label = "Bannerfarbe zurücksetzen" | "Reset banner colour";
    card_accent_color = "Akzentfarbe" | "Accent colour";
    card_accent_color_reset_label = "Akzentfarbe zurücksetzen" | "Reset accent colour";
    card_color_unset = "Standard" | "Default";
    card_decoration = "Avatar-Rahmen" | "Avatar frame";
    card_decoration_none = "Keiner" | "None";
    /// The decorations' names (web shell decoration.*).
    card_decoration_ring = "Ring" | "Ring";
    card_decoration_glow = "Leuchten" | "Glow";
    card_decoration_rainbow = "Regenbogen" | "Rainbow";
    card_decoration_stars = "Sterne" | "Stars";
    card_pronouns = "Pronomen" | "Pronouns";
    card_pronouns_placeholder = "z. B. sie/ihr" | "e.g. she/her";
    card_status = "Status" | "Status";
    card_status_hint = "Steht in der Mitgliederliste unter deinem Namen."
        | "Shown under your name in the member list.";
    card_bio = "Über mich" | "About me";
    card_unsaved = "Ungespeicherte Änderungen" | "Unsaved changes";

    // Audio and video (web voiceSettings), as the desktop app words them.
    media_input_title = "Eingabe" | "Input";
    media_input_microphone = "Mikrofon" | "Microphone";
    media_input_test_microphone = "Mikrofon testen" | "Test microphone";
    media_input_gain = "Mikrofonverstärkung" | "Microphone gain";
    media_input_gain_hint = "100 % fügt keine Verstärkung hinzu." | "100% adds no gain.";
    media_input_filter = "Filter" | "Filter";
    media_output_title = "Ausgabe" | "Output";
    media_output_speaker = "Lautsprecher" | "Speaker";
    media_output_calls = "Gespräche" | "Calls";
    media_output_calls_hint = "Über 100 % wird der Ton digital verstärkt."
        | "Above 100%, sound is boosted digitally.";
    media_output_live_sound = "Live-Ton" | "Live audio";
    media_output_live_sound_hint = "Der Ton von Lives hat eine eigene Lautstärke. Taub schaltet auch diesen Ton aus."
        | "Live audio has its own volume. Deafen turns it off too.";
    media_output_mute_live_sound = "Live-Ton stummschalten" | "Mute Live audio";
    media_output_call_sounds = "Signaltöne" | "Call sounds";
    media_output_call_sounds_hint = "Töne für Beitreten, Verlassen, Stumm, Taub, Kamera und Live."
        | "Sounds for join, leave, mute, deafen, camera and Live.";
    media_output_other_mute_sounds = "Töne, wenn andere sich stumm schalten" | "Sounds when others mute";
    media_output_call_sound_volume = "Lautstärke der Signaltöne" | "Call sound volume";
    media_output_call_sound_volume_hint = "Nutzt die Wiedergabelautstärke. Signaltöne spielen auf dem Standard-Ausgabegerät des Systems."
        | "Uses the playback volume. Call sounds play on the system’s default output device.";
    media_output_test_sound = "Testton abspielen" | "Play test sound";
    media_advanced_title = "Echo und WebRTC-Filter" | "Echo and WebRTC filters";
    media_advanced_subtitle = "Für Musik oder Lautsprecher" | "For music or speakers";
    media_advanced_echo = "Echo" | "Echo";
    media_advanced_echo_hint = "Echo-Unterdrückung (AEC). Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du sie ausschalten."
        | "Echo cancellation (AEC). Recommended with speakers. For music on headphones, you can turn it off.";
    media_advanced_noise = "Rauschfilter" | "Noise filter";
    media_advanced_noise_hint = "Rauschunterdrückung von WebRTC." | "WebRTC’s noise suppression.";
    media_advanced_auto_gain = "Auto-Gain" | "Auto gain";
    media_advanced_auto_gain_hint = "Auto-Gain von WebRTC. Gleicht leise und laute Mikrofone auf einen hörbaren Pegel an."
        | "WebRTC’s auto gain. Brings quiet and loud microphones to an audible level.";
    media_camera_title = "Kamera" | "Camera";
    media_camera_device = "Gerät" | "Device";
    media_quality_label = "Qualität" | "Quality";
    media_quality_resolution = "Auflösung" | "Resolution";
    media_quality_resolution_auto = "Auto (bis 1080p)" | "Auto (up to 1080p)";
    media_quality_frame_rate = "Bildrate" | "Frame rate";
    media_quality_frame_rate_auto = "Auto (bis 30 FPS)" | "Auto (up to 30 FPS)";
    media_quality_auto = "Automatisch" | "Automatic";
    media_live_title = "Live" | "Live";
    media_live_quality_hint = "Höhere Werte brauchen mehr Rechenleistung und Upload. Die erreichbare Qualität hängt von Quelle, Gerät und Verbindung ab."
        | "Higher values need more processing power and upload. The quality you get depends on source, device and connection.";
    media_live_share_sound = "Ton mitsenden" | "Share sound";
    media_live_share_sound_hint = "Gilt auch während du live bist. Mitgesendet wird der Ton anderer Anwendungen, nie dein Mikrofon oder der Ton des Anrufs. Der Live-Ton wird als Stereo-Musik übertragen, ohne Mikrofonfilter."
        | "Applies while you are live too. It shares the sound of other applications, never your microphone or the call’s sound. Live audio is sent as stereo music, without microphone filters.";
    media_live_sound_from = "Ton von" | "Sound from";
    media_live_sound_all_apps = "Alle Anwendungen außer Gelabber" | "All applications except Gelabber";
    media_upload_title = "Upload" | "Upload";
    media_upload_no_limit_badge = "ohne Limit" | "no limit";
    media_upload_economy = "Sparmodus" | "Data saver";
    media_upload_video_limit = "Video-Limit" | "Video limit";
    media_upload_no_limit = "Ohne Limit" | "No limit";
    media_upload_custom_limit = "Eigenes Limit" | "Custom limit";
    media_upload_custom_limit_unit = "Mbit/s für alle Videos zusammen" | "Mbit/s for all videos combined";
    media_upload_limit_hint = "Die Desktop-App passt die tatsächliche Bitrate an die Verbindung an. Das Limit ist eine Obergrenze, keine feste Datenrate."
        | "The desktop app adapts the actual bitrate to the connection. The limit is a cap, not a fixed rate.";
    media_apply_note = "Änderungen gelten beim nächsten Start und werden, soweit unterstützt, auch auf laufende Streams angewendet."
        | "Changes apply from the next start and, where supported, to running streams too.";
    media_mode_enhanced = "Verbesserte Sprache" | "Enhanced voice";
    media_mode_enhanced_hint = "Lokale Rauschunterdrückung" | "Local noise suppression";
    media_mode_browser = "WebRTC-Filter" | "WebRTC filters";
    media_mode_browser_hint = "Rauschunterdrückung und Auto-Gain von WebRTC"
        | "WebRTC’s noise suppression and auto gain";
    media_mode_original = "Original / Musik" | "Original / music";
    media_mode_original_hint = "Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung"
        | "Stereo where supported, without noise filter, auto gain or silence detection";
    media_ptt_send = "Senden" | "Send";
    media_ptt_activity = "Sprachaktivierung" | "Voice activity";
    media_ptt_activity_hint = "Das Mikrofon sendet, sobald du sprichst."
        | "Your microphone sends as soon as you speak.";
    media_ptt_ptt = "Push-to-Talk" | "Push to talk";
    media_ptt_ptt_hint = "Das Mikrofon sendet nur, solange du die Taste hältst."
        | "Your microphone sends only while you hold the key.";
    media_ptt_key = "Taste" | "Key";
    media_ptt_recording = "Taste oder Maustaste drücken … (Esc bricht ab)"
        | "Press a key or mouse button… (Esc cancels)";
    media_ptt_set_key = "Taste festlegen" | "Set key";
    media_ptt_no_key_hint = "Ohne Taste bleibt dein Mikrofon im Gespräch stumm."
        | "Without a key, your microphone stays muted in calls.";
    media_ptt_release_delay = "Nachlaufzeit" | "Release delay";
    media_ptt_release_delay_hint = "So lange sendet das Mikrofon nach dem Loslassen weiter."
        | "How long your microphone keeps sending after you let go.";
    /// The key's place in the Hyprland bind line: `bind = , <Taste>, …`.
    media_ptt_hyprland_key = "Taste" | "key";
    media_ptt_global_in_call = "Im Gespräch hört die App die Taste auch, wenn ein anderes Fenster vorne ist."
        | "In a call, the app hears the key even when another window is in front.";
    media_ptt_global_setting_up = "Die globale Taste wird eingerichtet …" | "Setting up the global key…";
    media_ptt_global_active_windows = "Global aktiv, auch wenn ein anderes Fenster vorne ist."
        | "Active globally, even when another window is in front.";
    // The key names (web voice pushToTalk.*).
    media_key_mouse4 = "Maustaste 4" | "Mouse button 4";
    media_key_mouse5 = "Maustaste 5" | "Mouse button 5";
    media_key_space = "Leertaste" | "Space";

    // The desktop app's "App" area.
    app_updates_title = "Updates" | "Updates";
    app_auto_check = "Beim Start und alle 6 Stunden nach Updates suchen"
        | "Check for updates at start and every 6 hours";
    app_restart_note = "Zum Installieren startet die App neu." | "The app restarts to install.";
    app_install_now = "Jetzt installieren" | "Install now";
    app_installing = "Wird installiert…" | "Installing…";
    app_download = "Herunterladen" | "Download";
    app_check_updates = "Nach Updates suchen" | "Check for updates";
    app_checking_updates = "Suche läuft…" | "Checking…";
    app_window_title = "Fenster" | "Window";
    app_close_to_tray = "Fenster schließen minimiert in den Infobereich"
        | "Closing the window minimises it to the notification area";
    app_close_to_tray_hint = "Ein laufender Anruf geht weiter. Beenden über das Symbol im Infobereich."
        | "A running call goes on. Quit from the icon in the notification area.";
    app_setting_failed = "Die Einstellung konnte nicht gespeichert werden."
        | "The setting could not be saved.";
}

/// "{percent} %" / "{percent}%".
pub fn percent(percent: i64) -> String {
    match get() {
        Locale::De => format!("{percent} %"),
        Locale::En => format!("{percent}%"),
    }
}

/// "Gerade aktiv: {name}."
pub fn appearance_active_theme(name: &str) -> String {
    match get() {
        Locale::De => format!("Gerade aktiv: {name}."),
        Locale::En => format!("Active now: {name}."),
    }
}

/// "{length} / {max} Zeichen".
pub fn card_bio_count(length: usize, max: usize) -> String {
    match get() {
        Locale::De => format!("{length} / {max} Zeichen"),
        Locale::En => format!("{length} / {max} characters"),
    }
}

/// A key on the left of the keyboard: "{key} links".
pub fn media_key_left(key: &str) -> String {
    match get() {
        Locale::De => format!("{key} links"),
        Locale::En => format!("Left {key}"),
    }
}

/// A key on the right of the keyboard: "{key} rechts".
pub fn media_key_right(key: &str) -> String {
    match get() {
        Locale::De => format!("{key} rechts"),
        Locale::En => format!("Right {key}"),
    }
}

/// Where Linux desktops take the key from, with the Hyprland line.
pub fn media_ptt_global_linux(bind: &str) -> String {
    match get() {
        Locale::De => format!(
            "Unter Linux bestimmt der Desktop die Taste: KDE und GNOME fragen beim ersten Mal nach, unter Hyprland gehört „{bind}“ in die hyprland.conf."
        ),
        Locale::En => format!(
            "On Linux, the desktop decides the key: KDE and GNOME ask the first time; on Hyprland, “{bind}” goes into hyprland.conf."
        ),
    }
}

/// "Global aktiv: {trigger}."
pub fn media_ptt_global_active_trigger(trigger: &str) -> String {
    match get() {
        Locale::De => format!("Global aktiv: {trigger}."),
        Locale::En => format!("Active globally: {trigger}."),
    }
}

/// "Global aktiv." followed by the Linux note.
pub fn media_ptt_global_active(linux: &str) -> String {
    match get() {
        Locale::De => format!("Global aktiv. {linux}"),
        Locale::En => format!("Active globally. {linux}"),
    }
}

/// The key only works in the foreground: why.
pub fn media_ptt_global_failed(reason: &str) -> String {
    match get() {
        Locale::De => {
            format!("Die Taste wirkt nur, solange Gelabber im Vordergrund ist: {reason}")
        }
        Locale::En => format!("The key only works while Gelabber is in the foreground: {reason}"),
    }
}

/// A saved application that is quiet now: "{name} (gerade still)".
pub fn media_live_sound_quiet(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} (gerade still)"),
        Locale::En => format!("{name} (quiet right now)"),
    }
}

/// The data saver's explicit limits, with the video limit.
pub fn media_upload_economy_on(video: &str) -> String {
    match get() {
        Locale::De => format!(
            "Ausdrückliche Obergrenzen: Sprache 64 kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}."
        ),
        Locale::En => format!(
            "Explicit limits: voice 64 kbit/s, Live audio 128 kbit/s, video {video} combined."
        ),
    }
}

/// What the data saver would do while it is off.
pub fn media_upload_economy_off(video: &str) -> String {
    match get() {
        Locale::De => format!(
            "Für wenig Upload: Sprache 64 kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {video}."
        ),
        Locale::En => {
            format!(
                "For low upload: voice 64 kbit/s, Live audio 128 kbit/s, video {video} combined."
            )
        }
    }
}

/// "Sparmodus ({limit})".
pub fn media_upload_economy_limit(limit: &str) -> String {
    match get() {
        Locale::De => format!("Sparmodus ({limit})"),
        Locale::En => format!("Data saver ({limit})"),
    }
}

/// A bitrate in Mbit/s, with the language's decimal comma or point.
pub fn mbit(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{} Mbit/s", rounded as i64)
    } else {
        let number = format!("{rounded:.1}");
        match get() {
            Locale::De => format!("{} Mbit/s", number.replace('.', ",")),
            Locale::En => format!("{number} Mbit/s"),
        }
    }
}
