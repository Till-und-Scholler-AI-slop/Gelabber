//! The custom tooltip (`.server-tile-tip`, index.css): elevated box,
//! padding 7px 10px, a 600 title line and an optional muted meta line.
//! First hover waits 200ms (tooltipTiming.ts).

use egui::{Align2, Id, Order, Pos2, Rect, Response, Stroke, StrokeKind, Vec2, pos2};

use crate::text::{self, Block};
use crate::theme::{Theme, corner};

const DELAY: f64 = 0.2;

#[must_use]
pub struct Tooltip<'a> {
    title: &'a str,
    detail: Option<&'a str>,
}

/// Where the tooltip sits relative to its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// 12px right of the anchor, vertically centred (server rail).
    Right,
    /// 6px above, horizontally centred.
    Above,
    /// 6px below, horizontally centred.
    Below,
}

impl<'a> Tooltip<'a> {
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            detail: None,
        }
    }

    pub fn detail(mut self, detail: Option<&'a str>) -> Self {
        self.detail = detail;
        self
    }

    /// Shows the tooltip for `response` once it has been hovered for 200ms.
    pub fn on_hover(self, response: &Response, side: Side) {
        let ctx = &response.ctx;
        let id = response.id.with("gelabber-tip");
        let now = ctx.input(|i| i.time);
        if !response.hovered() {
            ctx.data_mut(|d| d.remove::<f64>(id));
            return;
        }
        let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
        if now - since < DELAY {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(DELAY - (now - since)));
            return;
        }
        self.show(ctx, response.rect, side);
    }

    /// Paints the tooltip next to `anchor` on the tooltip layer.
    pub fn show(self, ctx: &egui::Context, anchor: Rect, side: Side) {
        let theme = Theme::of(ctx);
        let c = &theme.c;
        let (pos, align) = match side {
            Side::Right => (
                pos2(anchor.right() + 12.0, anchor.center().y),
                Align2::LEFT_CENTER,
            ),
            Side::Above => (
                pos2(anchor.center().x, anchor.top() - 6.0),
                Align2::CENTER_BOTTOM,
            ),
            Side::Below => (
                pos2(anchor.center().x, anchor.bottom() + 6.0),
                Align2::CENTER_TOP,
            ),
        };
        egui::Area::new(Id::unique((
            "gelabber-tooltip",
            anchor.min.x as i32,
            anchor.min.y as i32,
        )))
        .order(Order::Tooltip)
        .fixed_pos(pos)
        .pivot(align)
        .interactable(false)
        .show(ctx, |ui| {
            let max = 260.0 - 20.0 - 2.0;
            let title = Block::wrapped(ui, self.title, text::UI.weight(600.0), c.text, max);
            let detail = self
                .detail
                .map(|d| Block::wrapped(ui, d, text::META, c.muted, max));
            let inner_w = title.width().max(detail.as_ref().map_or(0.0, Block::width));
            let mut inner_h = title.height();
            if let Some(detail) = &detail {
                inner_h += 2.0 + detail.height();
            }
            let size = Vec2::new(inner_w + 22.0, inner_h + 16.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter();
            painter.add(theme.shadow_1.as_shape(rect, corner(theme.radius.sm)));
            painter.rect(
                rect,
                corner(theme.radius.sm),
                c.elevated,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            let mut y = rect.top() + 8.0;
            title.paint(painter, Pos2::new(rect.left() + 11.0, y));
            y += title.height() + 2.0;
            if let Some(detail) = detail {
                detail.paint(painter, Pos2::new(rect.left() + 11.0, y));
            }
        });
    }
}
