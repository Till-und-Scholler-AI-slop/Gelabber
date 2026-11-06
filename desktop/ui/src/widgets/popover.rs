//! `Popover` (components/ui/Popover.tsx, `.ui-popover`): an elevated panel
//! anchored 6px under (or over) its trigger, min-width 180, padding 8,
//! closed by Esc or a press outside; plus the menu rows inside it.

use egui::{Id, Order, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};

use super::eased;
use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, corner};

#[must_use]
pub struct Popover {
    id: Id,
    anchor: Rect,
    align_end: bool,
    min_width: f32,
}

impl Popover {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, anchor: Rect) -> Self {
        Self {
            id: Id::unique(("gelabber-popover", id)),
            anchor,
            align_end: false,
            min_width: 180.0,
        }
    }

    /// `bottom-end`: right edges aligned.
    pub fn align_end(mut self) -> Self {
        self.align_end = true;
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    /// Shows the panel; returns `None` once it should close (Esc, a press
    /// outside), otherwise what `content` returned.
    pub fn show<R>(self, ctx: &egui::Context, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let theme = Theme::of(ctx);
        let c = theme.c;
        let screen = ctx.content_rect();
        let size_id = self.id.with("size");
        let size: Vec2 = ctx
            .data(|d| d.get_temp(size_id))
            .unwrap_or(Vec2::new(self.min_width, 100.0));
        let below = self.anchor.bottom() + 6.0;
        let top = if below + size.y > screen.bottom() - 8.0 {
            (self.anchor.top() - 6.0 - size.y).max(screen.top() + 8.0)
        } else {
            below
        };
        let left = if self.align_end {
            self.anchor.right() - size.x
        } else {
            self.anchor.left()
        }
        .clamp(
            screen.left() + 8.0,
            (screen.right() - 8.0 - size.x).max(screen.left() + 8.0),
        );
        // `focusInside`: the frame it opens in (all passes of it), so the
        // content can take the focus as the web's autofocus does.
        let frame = ctx.cumulative_frame_nr();
        let seen_id = self.id.with("seen");
        let (opened, seen) = ctx
            .data(|d| d.get_temp::<(u64, u64)>(seen_id))
            .unwrap_or((0, 0));
        let opened = if seen == 0 || seen + 1 < frame {
            frame
        } else {
            opened
        };
        // A select's list inside a popover is a popover too: the outer
        // one's state comes back after it.
        let outer: Option<Focus> = ctx.data(|d| d.get_temp(opening_id()));
        ctx.data_mut(|d| {
            d.insert_temp(seen_id, (opened, frame.max(1)));
            d.insert_temp(
                opening_id(),
                if opened == frame {
                    Focus::Free
                } else {
                    Focus::Shown
                },
            );
        });
        let mut out = None;
        let area = egui::Area::new(self.id)
            .order(Order::Foreground)
            .fixed_pos(pos2(left, top))
            .show(ctx, |ui| {
                let inner_rect = Rect::from_min_size(
                    pos2(left + 9.0, top + 9.0),
                    Vec2::new(
                        (size.x - 18.0).max(self.min_width - 18.0),
                        (size.y - 18.0).max(0.0),
                    ),
                );
                let frame_rect = Rect::from_min_size(pos2(left, top), size);
                let painter = ui.painter();
                painter.add(theme.shadow_1.as_shape(frame_rect, corner(theme.radius.lg)));
                painter.rect(
                    frame_rect,
                    corner(theme.radius.lg),
                    c.elevated,
                    Stroke::new(1.0, c.border),
                    StrokeKind::Inside,
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner_rect));
                out = Some(content(&mut child));
                let used = child.min_rect();
                let measured = Vec2::new(
                    (used.width() + 18.0).max(self.min_width).min(360.0),
                    used.height() + 18.0,
                );
                if (measured - size).length() > 0.5 {
                    ctx.data_mut(|d| d.insert_temp(size_id, measured));
                    ctx.request_discard("popover size");
                }
                ui.allocate_rect(frame_rect, Sense::hover());
            });
        ctx.data_mut(|d| d.insert_temp(opening_id(), outer.unwrap_or(Focus::Shown)));
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !area.response.rect.contains(p) && !self.anchor.contains(p))
        });
        if escape || outside { None } else { out }
    }
}

/// Where the focus of the popover being drawn stands (Popover.tsx
/// `focusInside`: the element with `autoFocus`, else the first control).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// Not opening now, or no popover.
    Shown,
    /// Opening, and no control took the focus yet.
    Free,
    /// Opening, and a control took it.
    Taken,
}

fn opening_id() -> Id {
    Id::unique("gelabber-popover-opening")
}

fn focus_state(ctx: &egui::Context) -> Focus {
    ctx.data(|d| d.get_temp(opening_id()))
        .unwrap_or(Focus::Shown)
}

/// True while a popover draws its content in the frame it opened in.
pub fn opening(ctx: &egui::Context) -> bool {
    focus_state(ctx) != Focus::Shown
}

/// A control in a popover: the first one drawn as it opens takes the
/// focus. Called by the shared controls, so each popover gets it.
pub fn focus_first(response: &Response) {
    if focus_state(&response.ctx) == Focus::Free && response.sense.is_focusable() {
        response.request_focus();
        response
            .ctx
            .data_mut(|d| d.insert_temp(opening_id(), Focus::Taken));
    }
}

/// `autoFocus`: this control takes the focus as its popover opens, over
/// the first one.
pub fn autofocus(response: &Response) {
    if opening(&response.ctx) && response.sense.is_focusable() {
        response.request_focus();
        response
            .ctx
            .data_mut(|d| d.insert_temp(opening_id(), Focus::Taken));
    }
}

/// A row in a popover menu: optional 16px icon, label in `--lr-type-ui`,
/// hover fill, danger tone for destructive items.
pub fn menu_item(ui: &mut Ui, icon: Option<Icon>, label: &str, danger: bool) -> Response {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width().max(160.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    focus_first(&response);
    if ui.is_rect_visible(rect) {
        let fill = eased(
            ui.ctx(),
            response.id.with("hover"),
            response.hovered(),
            egui::Color32::TRANSPARENT,
            c.hover,
        );
        let painter = ui.painter();
        painter.rect(
            rect,
            corner(theme.radius.sm),
            fill,
            Stroke::NONE,
            StrokeKind::Inside,
        );
        let color = if danger { c.danger } else { c.text };
        let mut x = rect.left() + 8.0;
        if let Some(icon) = icon {
            icons::paint(
                painter,
                icon,
                Pos2::new(x, rect.center().y - 8.0),
                16.0,
                color,
            );
            x += 16.0 + 8.0;
        }
        let block = Block::single(ui, label, text::UI, color);
        block.paint(
            painter,
            Pos2::new(x, rect.center().y - block.height() / 2.0),
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}
