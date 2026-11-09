//! "App" in the desktop app: the updates (the automatic check, a check now,
//! the install) and what closing the window does. The web's area offers
//! installing Gelabber as an app, which the desktop app is already.

use egui::{Color32, Id, Rect, Role, Sense, Ui, vec2};

use super::controls::{self, Scope};
use crate::platform::update::{self, Offer, SetupAsk, SetupView};
use crate::text::{self, TextStyle};
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
    update::setup_view(ctx).is_some() || tray(ctx)
}

fn tray(ctx: &egui::Context) -> bool {
    crate::platform::desktop_info(ctx).is_some_and(|d| d.tray)
}

/// The page's subtitle: what it holds here ("Fenster" with a tray only).
pub fn description(ctx: &egui::Context) -> &'static str {
    match (update::setup_view(ctx).is_some(), tray(ctx)) {
        (true, true) => "Updates und was beim Schließen des Fensters passiert.",
        (false, true) => "Was beim Schließen des Fensters passiert.",
        _ => "Updates dieser App.",
    }
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
    // An update there (or at work) is the news: body text, not a hint.
    let news = view.available.is_some() && !view.checking;
    let (style, color) = if news {
        (text::BODY, c.text)
    } else {
        (controls::HINT, c.muted)
    };
    let width_text = width.min(529.9);
    say(
        ui,
        "status",
        &line,
        (style, color),
        width_text,
        Role::Status,
    );
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
        let hint = (controls::HINT, c.muted);
        say(ui, "pacman", &n.text(), hint, width_text, Role::Label);
    }
    if offer == Some(Offer::Restart) && view.can_install() {
        // What the install does, with the status it explains.
        ui.add_space(2.0);
        let hint = (controls::HINT, c.muted);
        let restart = "Zum Installieren startet die App neu.";
        say(ui, "restart", restart, hint, width_text, Role::Label);
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
}

/// A line of text that is also in the accessibility tree, as `role`.
fn say(ui: &mut Ui, id: &str, line: &str, look: (TextStyle, Color32), width: f32, role: Role) {
    let rect = controls::paragraph(ui, line, look.0, look.1, width);
    let response = ui.interact(rect, Id::unique(("gelabber-app-line", id)), Sense::hover());
    crate::widgets::announce(&response, role, line);
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

#[cfg(test)]
mod tests {
    use super::description;
    use crate::app::testkit::{FakeApi, launcher, wait_for_label, workspace};
    use crate::platform::paths::testing::TempConfig;
    use crate::platform::update::{
        Notice, Offer, Phase, SetupAsk, SetupView, show_setup, take_ask,
    };
    use crate::platform::{DesktopInfo, show_desktop_info};
    use crate::setup::Launcher;
    use egui::accesskit::{Role, Toggled};
    use egui_kittest::kittest::{NodeT, Queryable};

    type Harness = egui_kittest::Harness<'static, Launcher>;

    const AUTO: &str = "Beim Start und alle 6 Stunden nach Updates suchen";
    const TRAY: &str = "Fenster schließen minimiert in den Infobereich";

    /// The row and the switch in it are both named after the setting.
    fn switch<'h>(harness: &'h Harness, label: &'h str) -> egui_kittest::Node<'h> {
        harness
            .get_all_by_role_and_label(Role::Switch, label)
            .next()
            .expect(label)
    }

    /// Listed once it has something; the switch, the check and the right
    /// install action for how the app came; "Fenster" with a tray only.
    #[test]
    fn the_app_area_shows_the_updates_and_the_window() {
        let _config = TempConfig::new("settings-app");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Darstellung");
        harness.run_steps(3);
        assert!(
            harness.query_by_role_and_label(Role::Link, "App").is_none(),
            "nothing to show, not listed"
        );

        let latest = SetupView {
            current: "0.9.0".into(),
            auto_check: true,
            checked: true,
            ..SetupView::default()
        };
        show_setup(&harness.ctx, Some(latest.clone()));
        harness.run_steps(3);
        harness.get_by_role_and_label(Role::Link, "App").click();
        wait_for_label(&mut harness, "Nach Updates suchen");
        assert_eq!(description(&harness.ctx), "Updates dieser App.");
        assert!(harness.query_by_label(TRAY).is_none());
        let status = harness.get_by_label("Installiert: 0.9.0. Das ist die neueste Version.");
        assert_eq!(status.accesskit_node().role(), Role::Status);
        assert!(harness.query_by_label("Jetzt installieren").is_none());
        switch(&harness, AUTO).click();
        harness.run_steps(3);
        assert_eq!(take_ask(&harness.ctx), Some(SetupAsk::AutoCheck(false)));
        harness
            .get_by_role_and_label(Role::Button, "Nach Updates suchen")
            .click();
        harness.run_steps(3);
        assert_eq!(take_ask(&harness.ctx), Some(SetupAsk::CheckNow));

        // The Windows setup: "Jetzt installieren", and what it does.
        let offer = |offer, phase| SetupView {
            available: Some("0.9.1".into()),
            installs: offer == Offer::Restart,
            notice: Some(Notice {
                phase,
                ..Notice::new("0.9.1", offer)
            }),
            ..latest.clone()
        };
        show_setup(&harness.ctx, Some(offer(Offer::Restart, Phase::Offer)));
        harness.run_steps(3);
        harness.get_by_label("Installiert: 0.9.0. Verfügbar: 0.9.1.");
        harness.get_by_label("Zum Installieren startet die App neu.");
        harness
            .get_by_role_and_label(Role::Button, "Jetzt installieren")
            .click();
        harness.run_steps(3);
        assert_eq!(take_ask(&harness.ctx), Some(SetupAsk::Install));

        // At work: no second check, the install button busy.
        let downloading = Phase::Downloading {
            downloaded: 1,
            total: Some(4),
        };
        show_setup(&harness.ctx, Some(offer(Offer::Restart, downloading)));
        harness.run_steps(3);
        harness.get_by_label("Update wird geladen … 25 %");
        let busy = harness.get_by_role_and_label(Role::Button, "Wird installiert…");
        assert!(busy.accesskit_node().is_disabled());
        let check = harness.get_by_role_and_label(Role::Button, "Nach Updates suchen");
        assert!(check.accesskit_node().is_disabled());
        assert!(
            harness
                .query_by_label("Zum Installieren startet die App neu.")
                .is_none()
        );

        // The tarball links the release; pacman's says where updates come from.
        show_setup(&harness.ctx, Some(offer(Offer::Download, Phase::Offer)));
        harness.run_steps(3);
        assert!(
            harness
                .query_by_role_and_label(Role::Button, "Herunterladen")
                .is_some()
        );
        assert!(harness.query_by_label("Jetzt installieren").is_none());
        show_setup(
            &harness.ctx,
            Some(offer(Offer::PackageManager, Phase::Offer)),
        );
        harness.run_steps(3);
        assert!(harness.query_by_label_contains("über pacman").is_some());
        assert!(harness.query_by_label("Herunterladen").is_none());

        // With a tray: "Fenster" and its switch, moved from Benachrichtigungen.
        show_desktop_info(
            &harness.ctx,
            Some(DesktopInfo {
                tray: true,
                close_to_tray: false,
                system_notifications: true,
            }),
        );
        harness.run_steps(3);
        let tray = switch(&harness, TRAY);
        assert_eq!(tray.accesskit_node().toggled(), Some(Toggled::False));
        assert_eq!(
            description(&harness.ctx),
            "Updates und was beim Schließen des Fensters passiert."
        );
        show_setup(&harness.ctx, None);
        assert_eq!(
            description(&harness.ctx),
            "Was beim Schließen des Fensters passiert."
        );
    }
}
