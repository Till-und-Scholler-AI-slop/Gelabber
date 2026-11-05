//! `MainSkeleton` (components/ui/Skeleton.tsx, `.ui-main-skeleton`): the
//! main column while a page loads. The shapes fade in only after 150ms, so
//! a fast answer never flashes them; screen readers hear "Wird geladen…".

use egui::{Color32, Id, Rect, Sense, Ui, Vec2, pos2};

use crate::theme::{Theme, corner, motion, space};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Eight lines of varying width (`kind="list"`).
    List,
    /// A title bar and two cards (`kind="overview"`).
    Overview,
}

const LINE_WIDTHS: [f32; 6] = [0.38, 0.86, 0.62, 0.74, 0.48, 0.92];
const DELAY: f64 = 0.150;

/// Paints the skeleton into `rect` (padding 24, gap 24).
pub fn main(ui: &Ui, rect: Rect, kind: Kind, key: Id) {
    let ctx = ui.ctx();
    let now = ctx.input(|i| i.time);
    let frame = ctx.cumulative_frame_nr();
    // Shown since: reset when the skeleton was not on the last frame.
    let id = key.with("gelabber-skeleton");
    let (since, _) = ctx
        .data(|d| d.get_temp::<(f64, u64)>(id))
        .filter(|(_, last)| last + 1 >= frame)
        .unwrap_or((now, frame));
    ctx.data_mut(|d| d.insert_temp(id, (since, frame)));
    let response = ui.interact(rect, id.with("region"), Sense::hover());
    super::announce(&response, egui::Role::Status, "Wird geladen…");
    let shown = now - since - DELAY;
    ctx.request_repaint();
    if shown <= 0.0 {
        return;
    }
    let alpha = ((shown as f32) / motion::STANDARD).clamp(0.0, 1.0);
    let theme = Theme::of(ctx);
    let fill = theme.c.surface.gamma_multiply(alpha);
    let inner = rect.shrink(space::S6);
    let painter = ui.painter();
    let block = |r: Rect, color: Color32| painter.rect_filled(r, corner(theme.radius.sm), color);
    match kind {
        Kind::List => {
            let mut y = inner.top();
            for i in 0..8 {
                let w = inner.width() * LINE_WIDTHS[i % 6];
                block(
                    Rect::from_min_size(pos2(inner.left(), y), Vec2::new(w, 10.0)),
                    fill,
                );
                y += 10.0 + space::S4;
            }
        }
        Kind::Overview => {
            let title_w = (inner.width() * 0.7).min(420.0);
            block(
                Rect::from_min_size(inner.min, Vec2::new(title_w, 32.0)),
                fill,
            );
            let top = inner.top() + 32.0 + space::S6;
            let columns = ((inner.width() + space::S4) / (240.0 + space::S4))
                .floor()
                .max(1.0);
            let card_w = (inner.width() - space::S4 * (columns - 1.0)) / columns;
            for i in 0..2 {
                let row = (i as f32 / columns).floor();
                let col = i as f32 % columns;
                let card = Rect::from_min_size(
                    pos2(
                        inner.left() + col * (card_w + space::S4),
                        top + row * (106.0 + space::S4),
                    ),
                    Vec2::new(card_w, 106.0),
                );
                painter.rect(
                    card,
                    corner(theme.radius.lg),
                    theme.c.panel.gamma_multiply(alpha),
                    egui::Stroke::new(1.0, theme.c.border.gamma_multiply(alpha)),
                    egui::StrokeKind::Inside,
                );
                let c = card.shrink(space::S5 + 1.0);
                let mut y = c.top();
                for (w, h) in [(0.45, 14.0), (0.9, 10.0), (0.7, 10.0)] {
                    block(
                        Rect::from_min_size(pos2(c.left(), y), Vec2::new(c.width() * w, h)),
                        fill,
                    );
                    y += h + space::S3;
                }
            }
        }
    }
}
