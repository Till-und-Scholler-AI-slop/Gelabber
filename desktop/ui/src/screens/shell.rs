//! The signed-in workspace (rail, sidebar, pages). A stub until the shell
//! is built: it paints the page background only.

use egui::Ui;

use crate::app::AppCtx;
use crate::theme::Theme;

pub fn show(ui: &mut Ui, _app: &mut AppCtx) {
    let theme = Theme::of(ui.ctx());
    let rect = ui.ctx().content_rect();
    ui.painter().rect_filled(rect, 0.0, theme.c.bg);
}
