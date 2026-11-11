//! "Benachrichtigungen" (components/MessageAlertSettings.tsx,
//! NotificationPrivacy.tsx, NotificationPermission.tsx).

use egui::Ui;
use gelabber_i18n::notifications as texts;

use super::controls::{self, Scope};
use super::{op_pending, prefs, settled};
use crate::app::AppCtx;
use crate::text;
use crate::theme::Theme;

#[derive(Default)]
pub struct State {
    failed: bool,
    saving: bool,
}

pub fn area(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    controls::group_legend(ui, texts::alerts_title(), Some(Scope::Device));
    controls::switch_row(
        ui,
        "message-toasts",
        texts::alerts_toasts(),
        None,
        &mut p.message_toasts,
        false,
    );
    // The desktop app's notifications come from its window, not a tab.
    // The permission note under its switch (NotificationPermission.tsx).
    // The system shows the desktop app's notifications: there is no
    // browser permission to ask for.
    controls::switch_row_noted(
        ui,
        "desktop-notify",
        texts::alerts_desktop_app(),
        &mut p.desktop_notify,
        false,
        texts::alerts_desktop_app_note(),
    );
    let off = !p.desktop_notify;
    controls::switch_row(
        ui,
        "message-sound",
        texts::alerts_sound(),
        Some(texts::alerts_sound_hint()),
        &mut p.message_sound,
        off,
    );
    if p != before {
        prefs::update(&ctx, |x| *x = p);
    }
    // Closing into the tray (CloseToTray.tsx) is in "App".

    ui.add_space(32.0);
    controls::group_legend(ui, texts::privacy_title(), Some(Scope::Account));
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
        texts::privacy_hide_content(),
        Some(texts::privacy_hide_content_hint()),
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
        controls::paragraph(ui, texts::privacy_failed(), text::META, c.danger, w);
    }
}
