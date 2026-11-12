//! Themes (web/src/i18n/catalogs/themes.ts): the gallery and editor
//! (Einstellungen → Themes), the theme sync's messages and the small
//! preview of the app. The presets' names ("Gelabber Dunkel") are names
//! and stay as they are, in both languages.

use crate::{Locale, get};

crate::strings! {
    color_background = "Hintergrund" | "Background";
    color_panel = "Seitenleiste" | "Sidebar";
    color_surface = "Flächen" | "Surfaces";
    color_rail = "Serverleiste" | "Server bar";
    color_text = "Text" | "Text";
    color_muted = "Sekundärtext" | "Secondary text";
    color_accent = "Akzent" | "Accent";
    color_border = "Rahmen" | "Borders";
    style_clear = "Klar" | "Clear";
    style_soft = "Weich" | "Soft";
    style_terminal = "Terminal" | "Terminal";
    mode_light = "Hell" | "Light";
    mode_dark = "Dunkel" | "Dark";
    filter_label = "Themes filtern" | "Filter themes";
    filter_all = "Alle" | "All";
    filter_light = "Hell" | "Light";
    filter_dark = "Dunkel" | "Dark";
    filter_own = "Eigene" | "Custom";

    gallery_label = "Themes" | "Themes";
    gallery_empty = "Keine eigenen Themes." | "No custom themes.";
    gallery_active = "Aktiv" | "Active";
    system_name = "System" | "System";
    system_hint = "Folgt dem Gerät" | "Follows your device";
    system_title = "Wie dein Gerät" | "Same as your device";

    action_create = "+ Eigenes Theme" | "+ Custom theme";
    action_import = "Importieren" | "Import";
    action_import_label = "Theme-Datei importieren" | "Import theme file";
    /// The desktop app's save dialog for an exported theme.
    action_export_title = "Theme exportieren" | "Export theme";
    action_template = "Als Vorlage verwenden" | "Use as template";
    action_edit = "Bearbeiten" | "Edit";
    action_export = "Exportieren" | "Export";
    action_retry_apply = "Erneut anwenden" | "Apply again";
    action_load_saved = "Gespeicherten Stand laden" | "Load saved version";
    action_keep = "Behalten" | "Keep";

    notice_saved = "Theme gespeichert und angewendet." | "Theme saved and applied.";
    notice_deleted = "Theme gelöscht." | "Theme deleted.";
    notice_import_failed = "Import fehlgeschlagen." | "Import failed.";
    sync_saving = "Wird mit deinem Konto synchronisiert …" | "Syncing with your account…";
    discard_pending_title = "Lokale Änderung verwerfen?" | "Discard local change?";
    discard_pending_message = "Gelabber lädt dann wieder den Theme-Stand deines Kontos."
        | "Gelabber then loads your account’s themes again.";

    workbench_draft = "Dein Entwurf" | "Your draft";
    workbench_preview = "Vorschau" | "Preview";
    workbench_new_theme = "Neues Theme" | "New theme";
    /// The preview bar; split with [`crate::pieces`], then `{name}` is
    /// filled in, so a name cannot add tags.
    preview_bar_showing = "<b>„{name}“</b> wird angezeigt" | "Showing <b>“{name}”</b>";
    contrast_title = "Kontrast prüfen" | "Check contrast";
    contrast_fix = "Kontraste verbessern" | "Improve contrast";
    editor_name = "Name" | "Name";
    editor_mode = "Modus" | "Mode";
    editor_style = "Stil" | "Style";
    editor_colors = "Farben" | "Colours";
    editor_save = "Speichern und anwenden" | "Save and apply";
    draft_discard_title = "Ungespeicherten Theme-Entwurf verwerfen?" | "Discard unsaved theme draft?";
    draft_discard_message = "Deine Änderungen an diesem Theme gehen verloren."
        | "Your changes to this theme will be lost.";
    draft_discard = "Verwerfen" | "Discard";

    footnote_synced = "Mit deinem Konto synchronisiert." | "Synced with your account.";
    footnote_account = "Deine Themes gehören zu deinem Konto." | "Your themes belong to your account.";
    footnote_credit = "Paletten nach <link>Omarchy</link>, für Gelabber angepasst."
        | "Palettes based on <link>Omarchy</link>, adapted for Gelabber.";

    issue_text = "Text ist auf mindestens einer Fläche schwer lesbar."
        | "Text is hard to read on at least one surface.";
    issue_muted = "Sekundärtext braucht mehr Kontrast." | "Secondary text needs more contrast.";
    issue_accent_ink = "Schrift auf der Akzentfarbe ist schwer lesbar."
        | "Text on the accent colour is hard to read.";
    issue_accent_text = "Links in der Akzentfarbe heben sich zu wenig ab."
        | "Links in the accent colour don’t stand out enough.";
    issue_border = "Rahmen von Eingabefeldern heben sich zu wenig ab."
        | "Input field borders don’t stand out enough.";

    error_file_too_large = "Die Datei darf höchstens 64 KiB groß sein." | "The file can be 64 KiB at most.";
    error_invalid_json = "Die Datei enthält kein gültiges JSON." | "The file does not contain valid JSON.";
    error_invalid_format = "Das Theme-Format ist ungültig oder wird noch nicht unterstützt."
        | "The theme format is invalid or not supported yet.";
    error_not_synced = "Noch nicht synchronisiert." | "Not synced yet.";
    error_too_many = "Du kannst höchstens 50 eigene Themes speichern." | "You can save up to 50 custom themes.";
    error_unavailable = "Das ausgewählte Theme ist nicht mehr verfügbar."
        | "The selected theme is no longer available.";
    error_server_format = "Unbekanntes Theme-Format vom Server." | "Unknown theme format from the server.";
    error_unknown_format = "Unbekanntes Theme-Format." | "Unknown theme format.";
    error_load_failed = "Themes konnten nicht geladen werden. Bitte erneut versuchen."
        | "Themes could not be loaded. Please try again.";
    error_save_failed = "Theme konnte nicht gespeichert werden." | "Theme could not be saved.";
    error_conflict = "Auf einem anderen Gerät geändert. Deine Änderung bleibt erhalten. Erneut anwenden übernimmt sie auf den aktuellen Stand."
        | "Changed on another device. Your change is kept. “Apply again” puts it on top of the current version.";
    error_offline = "Noch nicht synchronisiert. Deine Änderung ist hier gespeichert."
        | "Not synced yet. Your change is saved here.";

    // The small picture of the app on every theme card.
    preview_channels = "KANÄLE" | "CHANNELS";
    preview_overview = "Übersicht" | "Overview";
    preview_general = "allgemein" | "general";
    preview_living_room = "Wohnzimmer" | "Living room";
    preview_topic = "Allgemeiner Chat" | "General chat";
    preview_question = "Wer ist heute Abend dabei?" | "Who’s in tonight?";
    preview_you = "Du" | "You";
    preview_you_initials = "DU" | "YO";
    preview_answer = "Bin im Wohnzimmer." | "I’m in the living room.";
    preview_composer = "Nachricht an #allgemein" | "Message #general";
    preview_send = "Senden" | "Send";
    preview_connected = "Verbunden · Wohnzimmer" | "Connected · Living room";
    preview_mic_on = "Mikrofon an" | "Mic on";
}

/// A colour's picker: "{name} wählen".
pub fn color_pick_label(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} wählen"),
        Locale::En => format!("Pick {name}"),
    }
}

/// A colour's hex field: "{name} Hexwert".
pub fn color_hex_label(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} Hexwert"),
        Locale::En => format!("{name} hex value"),
    }
}

/// A copied preset's name: "{name} – Eigene".
pub fn copy_name(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} – Eigene"),
        Locale::En => format!("{name} – Custom"),
    }
}

/// "Zurück zu {name}".
pub fn action_revert(name: &str) -> String {
    match get() {
        Locale::De => format!("Zurück zu {name}"),
        Locale::En => format!("Back to {name}"),
    }
}

/// "„{name}“ angewendet."
pub fn notice_applied(name: &str) -> String {
    match get() {
        Locale::De => format!("„{name}“ angewendet."),
        Locale::En => format!("“{name}” applied."),
    }
}

/// "Theme „{name}“ löschen?"
pub fn delete_title(name: &str) -> String {
    match get() {
        Locale::De => format!("Theme „{name}“ löschen?"),
        Locale::En => format!("Delete theme “{name}”?"),
    }
}
