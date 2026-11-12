//! `.ui-button` and `.ui-icon-button` (components/ui/Button.tsx,
//! IconButton.tsx, ui.css).

use egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget};

use super::{eased, focus_ring, keyboard_focus, snap};
use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, control, corner, motion};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
    DangerOutline,
    /// A border and no fill until hovered ("+ Neue Rolle").
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
}

impl ButtonSize {
    fn height(self) -> f32 {
        match self {
            Self::Sm => control::SM,
            Self::Md => control::MD,
            Self::Lg => control::LG,
        }
    }

    fn padding(self) -> f32 {
        match self {
            Self::Sm => 12.0,
            Self::Md => 16.0,
            Self::Lg => 20.0,
        }
    }
}

/// A button as `<Button variant size>`: `ui.add(Button::new("…").primary())`.
#[must_use]
pub struct Button<'a> {
    label: &'a str,
    variant: ButtonVariant,
    size: ButtonSize,
    icon: Option<Icon>,
    fill_width: bool,
    width: Option<f32>,
    height: Option<f32>,
    disabled: bool,
    /// Disabled, yet in the focus order (`aria-disabled`).
    focusable: bool,
    pending: Option<&'a str>,
    text_color: Option<Color32>,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            variant: ButtonVariant::Secondary,
            size: ButtonSize::Md,
            icon: None,
            fill_width: false,
            width: None,
            height: None,
            disabled: false,
            focusable: false,
            pending: None,
            text_color: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn primary(self) -> Self {
        self.variant(ButtonVariant::Primary)
    }

    pub fn ghost(self) -> Self {
        self.variant(ButtonVariant::Ghost)
    }

    pub fn danger(self) -> Self {
        self.variant(ButtonVariant::Danger)
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn small(self) -> Self {
        self.size(ButtonSize::Sm)
    }

    pub fn large(self) -> Self {
        self.size(ButtonSize::Lg)
    }

    /// A leading icon (16px, `gap: 8px`).
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Stretch to the available width (a button in a column flex).
    pub fn fill(mut self) -> Self {
        self.fill_width = true;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// A taller box, as `align-items: stretch` makes it in a flex row.
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Unavailable but focusable (`aria-disabled="true"`): it looks and
    /// acts disabled, yet keyboard focus reaches it, so a tooltip can say
    /// why on focus too.
    pub fn unavailable_focusable(mut self) -> Self {
        self.disabled = true;
        self.focusable = true;
        self
    }

    /// While `Some`, the label is swapped and the button is busy: disabled
    /// but keeping its variant's look (`aria-busy`).
    pub fn pending(mut self, label: Option<&'a str>) -> Self {
        self.pending = label;
        self
    }

    /// Overrides the label colour (toast actions use accent text).
    pub fn text_color(mut self, color: Color32) -> Self {
        self.text_color = Some(color);
        self
    }

    /// The width it takes without `width` or `fill`: the label (or the
    /// pending one), the icon, the padding and the border.
    pub fn natural_width(&self, ui: &Ui) -> f32 {
        let label = self.pending.unwrap_or(self.label);
        let block = Block::single(ui, label, self.style(), Color32::WHITE);
        let icon_w = if self.icon.is_some() { 16.0 + 8.0 } else { 0.0 };
        block.width() + icon_w + 2.0 * self.size.padding() + 2.0
    }

    fn style(&self) -> text::TextStyle {
        let style = match self.size {
            ButtonSize::Lg => text::UI.size(14.0),
            _ => text::UI,
        };
        match self.variant {
            ButtonVariant::Primary | ButtonVariant::Danger => style.weight(600.0),
            _ => style,
        }
    }
}

struct Look {
    fill: Color32,
    hover_fill: Color32,
    border: Color32,
    text: Color32,
}

fn look(theme: &Theme, variant: ButtonVariant) -> Look {
    let c = &theme.c;
    match variant {
        ButtonVariant::Primary => Look {
            fill: c.accent,
            hover_fill: theme.primary_hover,
            border: Color32::TRANSPARENT,
            text: c.accent_ink,
        },
        ButtonVariant::Secondary => Look {
            fill: c.surface,
            hover_fill: c.hover,
            border: c.border_strong,
            text: c.text,
        },
        ButtonVariant::Ghost => Look {
            fill: Color32::TRANSPARENT,
            hover_fill: c.hover,
            border: Color32::TRANSPARENT,
            text: c.text,
        },
        ButtonVariant::Danger => Look {
            fill: c.danger_bg,
            hover_fill: theme.danger_hover,
            border: Color32::TRANSPARENT,
            text: c.on_danger,
        },
        ButtonVariant::DangerOutline => Look {
            fill: Color32::TRANSPARENT,
            hover_fill: c.danger_bg.gamma_multiply(0.14),
            border: c.danger,
            text: c.danger,
        },
        ButtonVariant::Outline => Look {
            fill: Color32::TRANSPARENT,
            hover_fill: c.hover,
            border: c.border_strong,
            text: c.text,
        },
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let style = self.style();
        let label = self.pending.unwrap_or(self.label);
        let block = Block::single(ui, label, style, Color32::WHITE);
        let icon_w = if self.icon.is_some() { 16.0 + 8.0 } else { 0.0 };
        let content_w = block.width() + icon_w;
        let width = if self.fill_width {
            ui.available_width()
        } else {
            self.width
                .unwrap_or(content_w + 2.0 * self.size.padding() + 2.0)
        };
        let height = self.height.unwrap_or_else(|| self.size.height());
        let busy = self.pending.is_some();
        let enabled = !self.disabled && !busy;
        let sense = if enabled {
            Sense::click()
        } else if self.focusable && !busy {
            Sense::focusable_noninteractive()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
        super::popover::focus_first(&response);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let mut look = look(&theme, self.variant);
        let unavailable = self.disabled && !busy;
        if unavailable {
            look = Look {
                fill: theme.c.surface,
                hover_fill: theme.c.surface,
                border: theme.c.border,
                text: theme.c.subtle,
            };
        }
        if let Some(color) = self.text_color.filter(|_| !unavailable) {
            look.text = color;
        }
        let hovered = enabled && response.hovered();
        let fill = eased(
            ui.ctx(),
            response.id.with("hover"),
            hovered,
            look.fill,
            look.hover_fill,
        );
        let pressed = enabled && response.is_pointer_button_down_on();
        let scale = ui.ctx().animate_value_with_time(
            response.id.with("press"),
            if pressed { motion::PRESS_SCALE } else { 1.0 },
            motion::MICRO,
        );
        let shown = Rect::from_center_size(rect.center(), rect.size() * scale);
        let painter = ui.painter();
        let radius = theme.radius.sm;
        let box_rect = snap(ui.ctx(), shown);
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            focus_ring(painter, box_rect, radius, &theme);
        }
        painter.rect(
            box_rect,
            corner(radius),
            fill,
            Stroke::new(1.0, look.border),
            StrokeKind::Inside,
        );
        let line = style.line_px();
        let top = rect.center().y - line / 2.0;
        let mut x = rect.center().x - content_w / 2.0;
        if let Some(icon) = self.icon {
            icons::paint(
                painter,
                icon,
                egui::pos2(x, rect.center().y - 8.0),
                16.0,
                look.text,
            );
            x += icon_w;
        }
        block.paint_tinted(painter, egui::pos2(x, top), Some(look.text));
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        } else if unavailable && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        } else if busy && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Progress);
        }
        response
    }
}

/// `.ui-icon-button`: a square, transparent button with a muted icon.
#[must_use]
pub struct IconButton<'a> {
    icon: Icon,
    label: &'a str,
    size: f32,
    icon_size: f32,
    secondary: bool,
    pressed: bool,
    disabled: bool,
    rect: Option<Rect>,
    /// The password toggle keeps a transparent fill on hover and pressed.
    plain: bool,
}

impl<'a> IconButton<'a> {
    /// `label` is the accessible name and the tooltip.
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            size: control::MD,
            icon_size: 16.0,
            secondary: false,
            pressed: false,
            disabled: false,
            rect: None,
            plain: false,
        }
    }

    /// `xs` 28, `sm` 32, `md` 36.
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = size;
        self
    }

    pub fn secondary(mut self) -> Self {
        self.secondary = true;
        self
    }

    /// `aria-pressed`: an accent fill.
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draw into this rect instead of allocating a square.
    pub fn at(mut self, rect: Rect) -> Self {
        self.rect = Some(rect);
        self
    }

    pub fn plain(mut self) -> Self {
        self.plain = true;
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let sense = if self.disabled {
            Sense::hover()
        } else {
            Sense::click()
        };
        let (rect, response) = match self.rect {
            Some(rect) => {
                let id = ui.next_auto_id();
                ui.skip_ahead_auto_ids(1);
                (rect, ui.interact(rect, id, sense))
            }
            None => ui.allocate_exact_size(Vec2::splat(self.size), sense),
        };
        let response = response.on_hover_text(self.label);
        super::popover::focus_first(&response);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, !self.disabled, self.label)
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let hovered = !self.disabled && response.hovered();
        let (rest_fill, rest_fg, border) = if self.secondary {
            (c.surface, c.text, c.border_strong)
        } else {
            (Color32::TRANSPARENT, c.muted, Color32::TRANSPARENT)
        };
        let (mut fill, mut fg) = (rest_fill, rest_fg);
        if hovered {
            fg = c.text;
            if !self.plain {
                fill = c.hover;
            }
        }
        if self.pressed {
            if self.plain {
                fg = c.text;
            } else {
                fill = c.accent;
                fg = c.accent_ink;
            }
        }
        if self.disabled {
            fg = c.subtle;
        }
        let fill = eased(
            ui.ctx(),
            response.id.with("fill"),
            fill != rest_fill,
            rest_fill,
            fill,
        );
        let pressed = !self.disabled && response.is_pointer_button_down_on();
        let scale = ui.ctx().animate_value_with_time(
            response.id.with("press"),
            if pressed { motion::PRESS_SCALE } else { 1.0 },
            motion::MICRO,
        );
        let shown = snap(
            ui.ctx(),
            Rect::from_center_size(rect.center(), rect.size() * scale),
        );
        let painter = ui.painter();
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            focus_ring(painter, shown, theme.radius.sm, &theme);
        }
        painter.rect(
            shown,
            corner(theme.radius.sm),
            fill,
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        icons::paint_centered(painter, self.icon, rect, self.icon_size * scale, fg);
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }
}
