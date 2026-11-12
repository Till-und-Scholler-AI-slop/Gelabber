//! `/d` without a conversation (pages/DmIndexPage.tsx): back to the last
//! DM while it is listed, else the hint with "Neue Nachricht".

use egui::{Rect, Ui, Vec2, pos2};
use gelabber_i18n::dms;

use super::state;
use crate::app::AppCtx;
use crate::icons::Icon;
use crate::text::{self, Block};
use crate::theme::Theme;
use crate::widgets::Button;

pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    let (loaded, dms, user, bootstrap) = app.client.with_state(|s| {
        (
            s.dms_loaded,
            s.dms.iter().map(|d| d.id).collect::<Vec<_>>(),
            s.me().map(|m| m.id),
            s.op("bootstrap").cloned(),
        )
    });
    let failed = super::home::bootstrap_failed(ui.ctx(), app.client, loaded);
    let last = state::memory(ui.ctx(), user).dm;
    if let Some(last) = last {
        if dms.contains(&last) {
            app.nav.replace(&format!("/d/{last}"));
            return;
        }
        if loaded {
            state::remember(ui.ctx(), user, |m| m.dm = None);
        }
    }
    if !loaded && failed {
        // `.p-6` around LoadError.
        let area = rect.shrink(24.0);
        let mut box_ui = ui.new_child(egui::UiBuilder::new().max_rect(area));
        let pending = matches!(bootstrap, Some(gelabber_client::OpState::Pending));
        if crate::widgets::load_error(&mut box_ui, dms::index_load_failed(), pending) {
            app.client.bootstrap();
        }
        return;
    }
    if !loaded {
        crate::widgets::skeleton::main(
            ui,
            rect,
            crate::widgets::skeleton::Kind::List,
            egui::Id::unique("dm-index-skeleton"),
        );
        return;
    }
    let any = !dms.is_empty();
    let first = if any {
        Block::single(ui, dms::index_pick(), text::BODY.size(16.0), c.muted)
    } else {
        Block::single(ui, dms::index_empty(), text::TITLE, c.text)
    };
    let label = dms::new_message();
    let button_w = Block::single(ui, label, text::UI, c.text).width() + 16.0 + 8.0 + 32.0 + 2.0;
    let total = first.height() + 14.0 + 36.0;
    let top = rect.center().y - total / 2.0;
    first.paint(
        ui.painter(),
        pos2(rect.center().x - first.width() / 2.0, top),
    );
    let button = Rect::from_min_size(
        pos2(
            rect.center().x - button_w / 2.0,
            top + first.height() + 14.0,
        ),
        Vec2::new(button_w, 36.0),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
    let mut b = Button::new(label).icon(Icon::Plus);
    if !any {
        b = b.primary();
    }
    if child.add(b).clicked() {
        super::dialogs::open_new_message(ui.ctx());
    }
}
