//! `SessionCheckFailed` (components/SessionRecoveryNotice.tsx): the
//! session check failed; retries count down, and one button retries now.

use egui::Ui;
use gelabber_i18n::app;

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
        layout::heading(ui, app::recovery_no_answer());
        layout::lead(ui, app::recovery_no_answer_lead());
        ui.add_space(space::S3);
        let status = if checking {
            app::recovery_checking().to_owned()
        } else if seconds > 0 {
            app::recovery_retry_in(seconds)
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
            Button::new(app::recovery_retry_now())
                .primary()
                .large()
                .fill()
                .pending(checking.then_some(app::recovery_checking())),
        );
        if retry.clicked() {
            app.session.check(app.client);
        }
    });
}
