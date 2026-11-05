//! `SessionCheckFailed` (components/SessionRecoveryNotice.tsx): the
//! session check failed; retries count down, and one button retries now.

use egui::Ui;

use super::layout;
use crate::app::AppCtx;
use crate::text::{self, Block};
use crate::theme::{Theme, space};
use crate::widgets::Button;

pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    let now = ui.input(|i| i.time);
    let checking = app.session.checking();
    let seconds = app.session.retry_in(now).map_or(0, |s| s.ceil() as u64);
    if seconds > 0 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }
    layout::frame(ui, app, "session-recovery", |ui, app| {
        let theme = Theme::of(ui.ctx());
        layout::heading(ui, "Server antwortet nicht");
        layout::lead(
            ui,
            "Gelabber ist nicht erreichbar. Möglicherweise startet der Server neu.",
        );
        ui.add_space(space::S3);
        let status = if checking {
            "Wird geprüft…".to_owned()
        } else if seconds > 0 {
            format!("Neuer Versuch in {seconds} s")
        } else {
            String::new()
        };
        let block = Block::single(ui, &status, text::UI, theme.c.muted);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), block.height().max(20.0)),
            egui::Sense::hover(),
        );
        block.paint(ui.painter(), rect.min);
        ui.add_space(space::S6);
        let retry = ui.add(
            Button::new("Jetzt erneut versuchen")
                .primary()
                .large()
                .fill()
                .pending(checking.then_some("Wird geprüft…")),
        );
        if retry.clicked() {
            app.session.check(app.client);
        }
    });
}
