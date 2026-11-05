//! `/` (pages/WorkspaceIndexPage.tsx): back to the last server or DM, or
//! the first server; someone without servers gets the welcome with
//! "Server erstellen" and "Server beitreten".

use egui::{Rect, Stroke, StrokeKind, Ui, Vec2, pos2};

use super::state::{self, Place};
use crate::app::AppCtx;
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, ButtonSize};

/// `startPlace`: where "/" goes; `None` while the lists still load.
pub fn redirect(ui: &Ui, app: &mut AppCtx) -> bool {
    let (servers_loaded, servers, dms_loaded, dms, user) = app.client.with_state(|s| {
        (
            s.servers_loaded,
            s.servers.iter().map(|v| v.id).collect::<Vec<_>>(),
            s.dms_loaded || matches!(s.op("bootstrap"), Some(gelabber_client::OpState::Failed(_))),
            s.dms.iter().map(|d| d.id).collect::<Vec<_>>(),
            s.me().map(|m| m.id),
        )
    });
    if !servers_loaded {
        return true;
    }
    let remembered = state::memory(ui.ctx(), user).place;
    let target = match remembered {
        Some(Place::Server(id)) if servers.contains(&id) => Some(format!("/s/{id}")),
        Some(Place::Dm(id)) if dms.contains(&id) => Some(format!("/d/{id}")),
        Some(Place::Dm(_)) if !dms_loaded => return true,
        _ => servers.first().map(|id| format!("/s/{id}")),
    };
    match target {
        Some(target) => {
            app.nav.replace(&target);
            true
        }
        None => false,
    }
}

/// `.welcome`: padding 40 32 32, the greeting and two 360px cards.
pub fn welcome(ui: &mut Ui, app: &mut AppCtx, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    let name = app.client.with_state(|s| s.me().map(|m| m.name.clone()));
    let title = match name {
        Some(name) => format!("Willkommen, {name}"),
        None => "Willkommen".to_owned(),
    };
    let left = rect.left() + 32.0;
    let width = rect.width() - 64.0;
    let h1 = Block::wrapped(ui, &title, text::HERO.tracking(-0.035), c.text, width);
    h1.paint(ui.painter(), pos2(left, rect.top() + 40.0));
    let top = rect.top() + 40.0 + h1.height() + 32.0;
    let card_w = ((width - 16.0) / 2.0).min(360.0);
    let card_h = 20.0 + 20.8 + 8.0 + 21.0 + 12.0 + 44.0 + 20.0;
    for i in 0..2 {
        let card = Rect::from_min_size(
            pos2(left + i as f32 * (card_w + 16.0), top),
            Vec2::new(card_w, card_h),
        );
        ui.painter().rect(
            card,
            corner(theme.radius.lg),
            c.panel,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
        let inner = card.shrink(21.0);
        let (h2, p) = if i == 0 {
            (
                "Eigenen Server erstellen",
                Some("Mit #allgemein und einem Sprachkanal."),
            )
        } else {
            ("Server beitreten", None)
        };
        let block = Block::single(ui, h2, text::TITLE_SM, c.text);
        block.paint(ui.painter(), inner.min);
        let mut y = inner.top() + block.height() + 8.0;
        if let Some(p) = p {
            let block = Block::wrapped(ui, p, text::BODY, c.muted, inner.width());
            block.paint(ui.painter(), pos2(inner.left(), y));
            y += block.height() + 12.0;
        }
        let action = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 44.0), inner.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(action));
        if i == 0 {
            if child
                .add(
                    Button::new("Server erstellen")
                        .primary()
                        .size(ButtonSize::Lg)
                        .fill(),
                )
                .clicked()
            {
                super::dialogs::open_add(ui.ctx(), state::AddStep::Create);
            }
        } else if child
            .add(Button::new("Server beitreten").size(ButtonSize::Lg).fill())
            .clicked()
        {
            super::dialogs::open_add(ui.ctx(), state::AddStep::Join);
        }
        let _ = y;
    }
}
