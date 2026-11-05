//! `Switch` (`.ui-switch`) and `Checkbox` (`.ui-check`).

use egui::{Pos2, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, Widget};

use super::{focus_ring, keyboard_focus, lerp};
use crate::theme::{Theme, corner, motion};

/// A 38×22 switch; toggles `on` when clicked.
#[must_use]
pub struct Switch<'a> {
    on: &'a mut bool,
    disabled: bool,
}

impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool) -> Self {
        Self {
            on,
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Widget for Switch<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let sense = if self.disabled {
            Sense::hover()
        } else {
            Sense::click()
        };
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(38.0, 22.0), sense);
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let t = ui
                .ctx()
                .animate_bool_with_time(response.id, *self.on, motion::STANDARD);
            let alpha = if self.disabled { 0.5 } else { 1.0 };
            let track = lerp(c.surface, c.accent, t).gamma_multiply(alpha);
            let border = lerp(c.border_strong, c.accent, t).gamma_multiply(alpha);
            let knob = lerp(c.muted, c.accent_ink, t).gamma_multiply(alpha);
            let painter = ui.painter();
            if response.has_focus() && keyboard_focus(ui.ctx()) {
                focus_ring(painter, rect, 11.0, &theme);
            }
            painter.rect(
                rect,
                corner(11.0),
                track,
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
            let center = Pos2::new(rect.left() + 1.0 + 3.0 + 7.0 + 16.0 * t, rect.center().y);
            painter.circle_filled(center, 7.0, knob);
        }
        if response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }
}

/// A 20×20 checkbox, or a radio with `radio()`.
#[must_use]
pub struct Checkbox<'a> {
    checked: &'a mut bool,
    radio: bool,
    disabled: bool,
    label: Option<&'a str>,
}

impl<'a> Checkbox<'a> {
    pub fn new(checked: &'a mut bool) -> Self {
        Self {
            checked,
            radio: false,
            disabled: false,
            label: None,
        }
    }

    /// The accessible name, where the text beside it is drawn apart.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn radio(mut self) -> Self {
        self.radio = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// The check mark's `clip-path` polygon in a 12×12 box.
const CHECK: [(f32, f32); 6] = [
    (0.14, 0.52),
    (0.0, 0.66),
    (0.38, 1.0),
    (1.0, 0.22),
    (0.86, 0.08),
    (0.38, 0.72),
];

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let sense = if self.disabled {
            Sense::hover()
        } else {
            Sense::click()
        };
        let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(20.0), sense);
        if response.clicked() {
            *self.checked = if self.radio { true } else { !*self.checked };
            response.mark_changed();
        }
        if let Some(label) = self.label {
            let (checked, enabled) = (*self.checked, !self.disabled);
            let role = if self.radio {
                egui::Role::RadioButton
            } else {
                egui::Role::CheckBox
            };
            response.widget_info(|| egui::WidgetInfo::selected(role, enabled, checked, label));
        }
        if ui.is_rect_visible(rect) {
            let alpha = if self.disabled { 0.5 } else { 1.0 };
            let (fill, border) = if *self.checked {
                (c.accent, c.accent)
            } else {
                (c.bg, c.border_strong)
            };
            let radius = if self.radio { 10.0 } else { theme.radius.xs };
            let painter = ui.painter();
            if response.has_focus() && keyboard_focus(ui.ctx()) {
                focus_ring(painter, rect, radius, &theme);
            }
            painter.rect(
                rect,
                corner(radius),
                fill.gamma_multiply(alpha),
                Stroke::new(1.0, border.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
            let scale = ui
                .ctx()
                .animate_bool_with_time(response.id, *self.checked, motion::FAST);
            if scale > 0.0 {
                let ink = c.accent_ink.gamma_multiply(alpha);
                if self.radio {
                    painter.circle_filled(rect.center(), 4.0 * scale, ink);
                } else {
                    let size = 12.0 * scale;
                    let min = rect.center() - Vec2::splat(size / 2.0);
                    let points: Vec<Pos2> = CHECK
                        .iter()
                        .map(|(x, y)| Pos2::new(min.x + x * size, min.y + y * size))
                        .collect();
                    // The polygon is concave: draw it as two convex halves.
                    painter.add(Shape::convex_polygon(
                        vec![points[0], points[1], points[2], points[5]],
                        ink,
                        Stroke::NONE,
                    ));
                    painter.add(Shape::convex_polygon(
                        vec![points[5], points[2], points[3], points[4]],
                        ink,
                        Stroke::NONE,
                    ));
                }
            }
        }
        if response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }
}
