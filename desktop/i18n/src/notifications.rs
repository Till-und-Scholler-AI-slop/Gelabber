//! Notifications (web/src/i18n/catalogs/notifications.ts): the settings
//! area (Nachrichten, Sperrbildschirm) and the per-server, channel or DM
//! choices with their mute and level names. The desktop app's own lines,
//! where the system and not a browser shows the notifications, are written
//! in the same tone.

use crate::{Locale, get};

crate::strings! {
    alerts_title = "Nachrichten" | "Messages";
    alerts_toasts = "Popup bei neuen Nachrichten in anderen Chats" | "Pop-up for new messages in other chats";
    alerts_desktop = "Browser-Benachrichtigung, wenn der Tab nicht im Vordergrund ist"
        | "Browser notification when the tab is in the background";
    /// The desktop app's switch: its window, not a browser tab.
    alerts_desktop_app = "Benachrichtigung, wenn das Gelabber-Fenster nicht im Vordergrund ist"
        | "Notification when the Gelabber window is not in the foreground";
    /// The desktop app's note under that switch: there is no browser
    /// permission to ask for.
    alerts_desktop_app_note = "Benachrichtigungen laufen über dein System. Sperrbildschirm und „Nicht stören“ stellst du dort ein."
        | "Notifications go through your system. You set the lock screen and “Do not disturb” there.";
    alerts_sound = "Ton bei Benachrichtigungen am Computer" | "Sound for notifications on a computer";
    alerts_sound_hint = "Ein Ton für mehrere Nachrichten kurz hintereinander, keiner, während du taubgeschaltet bist. Auf dem Handy gilt die Geräteeinstellung."
        | "One sound for several messages in a row, none while you are deafened. On a phone, the device setting applies.";

    permission_push_on = "Direktnachrichten und @Erwähnungen kommen auch an, wenn Gelabber geschlossen ist."
        | "Direct messages and @mentions arrive even when Gelabber is closed.";
    permission_push_after_allow = "Danach kommen Direktnachrichten und @Erwähnungen auch bei geschlossener App an, sofern der Server das unterstützt."
        | "Then direct messages and @mentions arrive even when the app is closed, if the server supports it.";
    permission_while_running = "Auf diesem Gerät kommen Benachrichtigungen nur an, solange Gelabber im Hintergrund läuft."
        | "On this device, notifications only arrive while Gelabber runs in the background.";
    permission_allow = "Benachrichtigungen erlauben" | "Allow notifications";
    permission_failed = "Die Berechtigung konnte nicht abgefragt werden. Versuche es erneut."
        | "Could not ask for permission. Try again.";

    privacy_title = "Sperrbildschirm" | "Lock screen";
    privacy_hide_content = "Inhalt in Benachrichtigungen verbergen" | "Hide content in notifications";
    privacy_hide_content_hint = "Benachrichtigungen zeigen nur „Neue Nachricht von …“, ohne Text und ohne Chat. Gilt für alle Geräte deines Kontos."
        | "Notifications only show “New message from …”, without text or chat. Applies to all devices on your account.";
    privacy_failed = "Die Einstellung konnte nicht gespeichert werden. Versuche es erneut."
        | "The setting could not be saved. Try again.";

    dialog_title = "Benachrichtigungen" | "Notifications";
    dialog_gone = "Nicht mehr verfügbar." | "No longer available.";
    dialog_not_saved = "Nicht gespeichert. Versuch es noch einmal." | "Not saved. Try again.";
    dialog_mute = "Stummschalten" | "Mute";
    dialog_unmute = "Aufheben" | "Unmute";
    dialog_mute_hint_dm = "Stumm: kein Zähler, keine Benachrichtigung und kein Ton."
        | "Muted: no count, no notification and no sound.";
    dialog_mute_hint_channel = "Stumm: kein Punkt, keine Benachrichtigung und kein Ton. @Erwähnungen zählen weiter."
        | "Muted: no dot, no notification and no sound. @mentions still count.";
    dialog_server_muted = "Der ganze Server ist gerade stumm." | "The whole server is muted right now.";
    dialog_level_legend = "Benachrichtigen bei" | "Notify me about";
    dialog_mark_read = "Als gelesen markieren" | "Mark as read";

    bell_title = "Benachrichtigungen" | "Notifications";
    bell_title_muted = "Stummgeschaltet" | "Muted";

    mute_quarter_hour = "15 Minuten" | "15 minutes";
    mute_quarter_hour_short = "15 Min." | "15 min";
    mute_hour = "1 Stunde" | "1 hour";
    mute_hour_short = "1 Std." | "1 hr";
    mute_eight_hours = "8 Stunden" | "8 hours";
    mute_eight_hours_short = "8 Std." | "8 hrs";
    mute_day = "24 Stunden" | "24 hours";
    mute_day_short = "24 Std." | "24 hrs";
    mute_forever = "Bis ich es wieder einschalte" | "Until I turn it back on";
    mute_forever_short = "Unbegrenzt" | "Forever";

    level_all = "Alle Nachrichten" | "All messages";
    level_all_hint = "Jede neue Nachricht" | "Every new message";
    level_mentions = "Nur @Erwähnungen" | "Only @mentions";
    level_mentions_hint = "Wenn dich jemand erwähnt oder @everyone/@here schreibt"
        | "When someone mentions you or writes @everyone/@here";
    level_none = "Nichts" | "Nothing";
    level_none_hint = "Keine Benachrichtigungen, kein Zähler" | "No notifications, no count";

    muted_until_unmuted = "Stumm, bis du es aufhebst" | "Muted until you unmute";
}

/// "Benachrichtigungen: {name}".
pub fn dialog_title_for(name: &str) -> String {
    match get() {
        Locale::De => format!("Benachrichtigungen: {name}"),
        Locale::En => format!("Notifications: {name}"),
    }
}

/// The bell's accessible name.
pub fn bell_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Benachrichtigungen für {name}"),
        Locale::En => format!("Notifications for {name}"),
    }
}

/// The bell's accessible name while muted.
pub fn bell_label_muted(name: &str) -> String {
    match get() {
        Locale::De => format!("Benachrichtigungen für {name} (stumm)"),
        Locale::En => format!("Notifications for {name} (muted)"),
    }
}

/// A channel's level that follows the server's: "Wie der Server ({level})".
pub fn level_inherit(level: &str) -> String {
    match get() {
        Locale::De => format!("Wie der Server ({level})"),
        Locale::En => format!("Same as the server ({level})"),
    }
}

/// "Stumm bis {until}".
pub fn muted_until(until: &str) -> String {
    match get() {
        Locale::De => format!("Stumm bis {until}"),
        Locale::En => format!("Muted until {until}"),
    }
}

/// A mute's end today: "{time} Uhr" / "{time}".
pub fn until_today(time: &str) -> String {
    match get() {
        Locale::De => format!("{time} Uhr"),
        Locale::En => time.to_owned(),
    }
}

/// A mute's end on a later day: "{date}, {time} Uhr" / "{date}, {time}".
pub fn until_later(date: &str, time: &str) -> String {
    match get() {
        Locale::De => format!("{date}, {time} Uhr"),
        Locale::En => format!("{date}, {time}"),
    }
}
