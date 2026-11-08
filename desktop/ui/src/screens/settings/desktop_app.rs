//! "App" in the desktop app: the updates (the automatic check, a check now,
//! the install) and what closing the window does. The web's area offers
//! installing Gelabber as an app, which the desktop app is already.

use egui::{Rect, Sense, Ui, vec2};

use super::controls::{self, Scope};
use crate::platform::update::{self, Offer, SetupAsk, SetupView};
use crate::text;
use crate::theme::{Theme, corner};
use crate::widgets::Button;

#[derive(Default)]
pub struct State {
    /// desktop.json could not be written.
    tray_failed: bool,
}

/// Whether the area has anything to show: the updates (published by the
/// running app's Updater) or a tray to close into.
pub fn has_content(ctx: &egui::Context) -> bool {
    update::setup_view(ctx).is_some() || crate::platform::desktop_info(ctx).is_some_and(|d| d.tray)
}

pub fn area(ui: &mut Ui, s: &mut State) {
    let ctx = ui.ctx().clone();
    if let Some(view) = update::setup_view(&ctx) {
        updates(ui, &view);
        ui.add_space(32.0);
    }
    window(ui, s);
}

/// "Updates": the switch, the status line and what can be done now.
fn updates(ui: &mut Ui, view: &SetupView) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    controls::group_legend(ui, "Updates", Some(Scope::Device));
    let mut auto = view.auto_check;
    if controls::switch_row(
        ui,
        "auto-check",
        "Beim Start und alle 6 Stunden nach Updates suchen",
        None,
        &mut auto,
        view.checking,
    ) {
        update::ask(&ctx, SetupAsk::AutoCheck(auto));
    }
    ui.add_space(12.0);
    let width = ui.available_width();
    let line = view.status();
    let rect = controls::paragraph(ui, &line, controls::HINT, c.muted, width.min(529.9));
    let status = ui.interact(
        rect,
        egui::Id::unique("gelabber-app-update-status"),
        Sense::hover(),
    );
    crate::widgets::announce(&status, egui::Role::Status, &line);
    let notice = view.notice.as_ref();
    if let Some(done) = notice.and_then(update::Notice::progress) {
        ui.add_space(6.0);
        let (track, _) = ui.allocate_exact_size(vec2(width.min(320.0), 3.0), Sense::hover());
        ui.painter().rect_filled(track, corner(2.0), c.border);
        let bar = Rect::from_min_size(track.min, vec2(track.width() * done, 3.0));
        ui.painter().rect_filled(bar, corner(2.0), c.accent);
    }
    // What the update found offers here; pacman's says so in the line.
    let offer = notice.map(|n| n.offer);
    if offer == Some(Offer::PackageManager)
        && let Some(n) = notice
    {
        ui.add_space(6.0);
        controls::paragraph(ui, &n.text(), controls::HINT, c.muted, width.min(529.9));
    }
    ui.add_space(12.0);
    controls::row(ui, 8.0, 36.0, |ui| {
        if offer == Some(Offer::Restart) && (view.can_install() || view.installing()) {
            let install = Button::new("Jetzt installieren")
                .primary()
                .pending(view.installing().then_some("Wird installiert…"));
            if ui.add(install).clicked() {
                update::ask(&ctx, SetupAsk::Install);
            }
        }
        if offer == Some(Offer::Download)
            && let Some(n) = notice
            && ui.add(Button::new("Herunterladen").primary()).clicked()
        {
            ctx.open_url(egui::OpenUrl::new_tab(update::release_page(&n.version)));
        }
        let check = Button::new("Nach Updates suchen")
            .pending(view.checking.then_some("Suche läuft…"))
            .disabled(view.installing());
        if ui.add(check).clicked() {
            update::ask(&ctx, SetupAsk::CheckNow);
        }
    });
    if offer == Some(Offer::Restart) && view.can_install() {
        ui.add_space(6.0);
        controls::hint(ui, "Zum Installieren startet die App neu.");
    }
}

/// "Fenster": closing into the tray (CloseToTray.tsx), with a tray only.
fn window(ui: &mut Ui, s: &mut State) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let Some(info) = crate::platform::desktop_info(&ctx).filter(|d| d.tray) else {
        return;
    };
    controls::group_legend(ui, "Fenster", Some(Scope::Device));
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
            theme.c.danger,
            w,
        );
    }
}
