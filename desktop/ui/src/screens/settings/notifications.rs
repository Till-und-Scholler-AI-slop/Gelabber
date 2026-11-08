//! "Benachrichtigungen" (components/MessageAlertSettings.tsx,
//! NotificationPrivacy.tsx, NotificationPermission.tsx).

use egui::Ui;

use super::controls::{self, Scope};
use super::{op_pending, prefs, settled};
use crate::app::AppCtx;
use crate::text;
use crate::theme::Theme;

#[derive(Default)]
pub struct State {
    failed: bool,
    saving: bool,
    /// desktop.json could not be written.
    tray_failed: bool,
}

pub fn area(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    controls::group_legend(ui, "Nachrichten", Some(Scope::Device));
    controls::switch_row(
        ui,
        "message-toasts",
        "Popup bei neuen Nachrichten in anderen Chats",
        None,
        &mut p.message_toasts,
        false,
    );
    // The desktop app's notifications come from its window, not a tab.
    let desktop = crate::platform::desktop_info(&ctx);
    // The permission note under its switch (NotificationPermission.tsx).
    // The system shows the desktop app's notifications: there is no
    // browser permission to ask for.
    controls::switch_row_noted(
        ui,
        "desktop-notify",
        "Benachrichtigung, wenn das Gelabber-Fenster nicht im Vordergrund ist",
        &mut p.desktop_notify,
        false,
        "Benachrichtigungen laufen über dein System. Sperrbildschirm und „Nicht stören“ stellst du dort ein.",
    );
    let off = !p.desktop_notify;
    controls::switch_row(
        ui,
        "message-sound",
        "Ton bei Benachrichtigungen am Computer",
        Some(
            "Ein Ton für mehrere Nachrichten kurz hintereinander, keiner, während du taubgeschaltet bist. Auf dem Handy gilt die Geräteeinstellung.",
        ),
        &mut p.message_sound,
        off,
    );
    if p != before {
        prefs::update(&ctx, |x| *x = p);
    }
    // CloseToTray.tsx: with a tray only.
    if let Some(info) = desktop.filter(|d| d.tray) {
        let mut on = info.close_to_tray;
        if controls::switch_row(
            ui,
            "close-to-tray",
            "Fenster schließen minimiert in den Infobereich",
            Some("Ein laufender Anruf geht weiter. Beenden über das Symbol im Infobereich."),
            &mut on,
            false,
        ) {
            s.tray_failed = crate::platform::set_close_to_tray(&ctx, on).is_err();
        }
        if s.tray_failed {
            ui.add_space(6.0);
            let w = ui.available_width();
            controls::paragraph(
                ui,
                "Die Einstellung konnte nicht gespeichert werden.",
                text::META,
                c.danger,
                w,
            );
        }
    }

    ui.add_space(32.0);
    controls::group_legend(ui, "Sperrbildschirm", Some(Scope::Account));
    if s.saving
        && let Some(result) = settled(app, "privacy")
    {
        s.saving = false;
        s.failed = result.is_err();
    }
    let known = app.client.with_state(|st| st.notification_privacy);
    let mut hide = known.is_some_and(|p| p.hide_content);
    let busy = known.is_none() && !app.client.with_state(|st| st.bootstrapped)
        || s.saving
        || op_pending(app, "privacy");
    if controls::switch_row(
        ui,
        "hide-content",
        "Inhalt in Benachrichtigungen verbergen",
        Some(
            "Benachrichtigungen zeigen nur „Neue Nachricht von …“, ohne Text und ohne Chat. Gilt für alle Geräte deines Kontos.",
        ),
        &mut hide,
        busy,
    ) {
        s.saving = true;
        s.failed = false;
        app.client.set_notification_privacy(hide);
    }
    if s.failed {
        ui.add_space(6.0);
        let w = ui.available_width();
        controls::paragraph(
            ui,
            "Die Einstellung konnte nicht gespeichert werden. Versuche es erneut.",
            text::META,
            c.danger,
            w,
        );
    }
}
