//! `.auth-link` and inline links: accent text, weight 500, a 1px
//! underline 2px under the baseline (`text-underline-offset: 2px`).

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, Widget};

use super::{focus_ring, keyboard_focus};
use crate::text::{self, Block, TextStyle};
use crate::theme::Theme;

/// A text link. Inside running text, lay the sentence out with
/// [`text::Rich`] and call [`underline`] and [`Link::interact`] on the
/// link's [`Block::section_rect`].
#[must_use]
pub struct Link<'a> {
    label: &'a str,
    style: TextStyle,
    disabled: bool,
    color: Option<Color32>,
}

impl<'a> Link<'a> {
    /// `.auth-link` at body size.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            style: text::BODY.weight(500.0),
            disabled: false,
            color: None,
        }
    }

    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// `.auth-link:disabled`: subtle, no underline.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Hover, cursor and focus ring for a link drawn as part of a paragraph.
    pub fn interact(ui: &mut Ui, rect: Rect, id: egui::Id, label: &str) -> Response {
        let response = ui.interact(rect, id, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, label));
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            let theme = Theme::of(ui.ctx());
            focus_ring(ui.painter(), rect, 0.0, &theme);
        }
        response
    }
}

/// The underline of a link run whose line box starts at `line_top`.
pub fn underline(
    painter: &Painter,
    style: TextStyle,
    line_top: f32,
    left: f32,
    right: f32,
    color: Color32,
) {
    let ppp = painter.ctx().pixels_per_point();
    let baseline = ((line_top + style.baseline()) * ppp).round() / ppp;
    let y = baseline + 2.0;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(left, y), Pos2::new(right, y + 1.0 / ppp)),
        0.0,
        color,
    );
}

impl Widget for Link<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let color = if self.disabled {
            theme.c.subtle
        } else {
            self.color.unwrap_or(theme.c.accent_text)
        };
        let block = Block::single(ui, self.label, self.style, color);
        let sense = if self.disabled {
            Sense::hover()
        } else {
            Sense::click()
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(block.width(), block.height()), sense);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Link, !self.disabled, self.label)
        });
        if ui.is_rect_visible(rect) {
            block.paint(ui.painter(), rect.min);
            if !self.disabled {
                underline(
                    ui.painter(),
                    self.style,
                    rect.top(),
                    rect.left(),
                    rect.right(),
                    color,
                );
            }
            if response.has_focus() && keyboard_focus(ui.ctx()) {
                focus_ring(ui.painter(), rect, 0.0, &theme);
            }
        }
        if response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }
}
