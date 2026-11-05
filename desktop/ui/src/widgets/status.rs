//! `FormError` (`.ui-form-error`) and `InlineStatus` (`.ui-inline-status`).

use egui::{Color32, Rect, Response, Sense, Ui, Vec2, Widget, pos2};

use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, corner, over};

/// The form-level error box: a 3px danger bar on the left, danger-bg 18%.
#[must_use]
pub struct FormError<'a> {
    message: &'a str,
    /// The surface it sits on, for the 18% tint (panel in auth cards,
    /// elevated in modals, bg on pages).
    on: Option<Color32>,
}

impl<'a> FormError<'a> {
    pub fn new(message: &'a str) -> Self {
        Self { message, on: None }
    }

    pub fn on(mut self, surface: Color32) -> Self {
        self.on = Some(surface);
        self
    }
}

impl Widget for FormError<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let width = ui.available_width();
        // padding 10px 12px, plus the 3px left border.
        let block = Block::wrapped(ui, self.message, text::BODY, c.danger, width - 3.0 - 24.0);
        let height = 20.0 + super::field::layout_unit(block.height());
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        // role="alert": screen readers (and tests) read the message.
        super::announce(&response, egui::Role::Alert, self.message);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let surface = self.on.unwrap_or(c.panel);
            let box_rect = super::snap(ui.ctx(), rect);
            painter.rect_filled(
                box_rect,
                corner(theme.radius.sm),
                over(surface, c.danger_bg, 0.18),
            );
            // The left border of a rounded box: its corners follow the radius.
            painter
                .with_clip_rect(Rect::from_min_max(
                    box_rect.min,
                    pos2(box_rect.left() + 3.0, box_rect.bottom()),
                ))
                .rect_filled(box_rect, corner(theme.radius.sm), c.danger);
            block.paint(painter, pos2(rect.left() + 15.0, rect.top() + 10.0));
        }
        response
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Neutral,
    Success,
    Error,
}

/// A short status line after an action ("Neuer Link gesendet.").
#[must_use]
pub struct InlineStatus<'a> {
    message: Option<&'a str>,
    tone: StatusTone,
}

impl<'a> InlineStatus<'a> {
    pub fn new(message: Option<&'a str>, tone: StatusTone) -> Self {
        Self { message, tone }
    }
}

impl Widget for InlineStatus<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let Some(message) = self.message.filter(|m| !m.is_empty()) else {
            return ui
                .allocate_exact_size(Vec2::new(ui.available_width(), 0.0), Sense::hover())
                .1;
        };
        let (color, icon) = match self.tone {
            StatusTone::Neutral => (c.muted, None),
            StatusTone::Success => (c.online_text, Some(Icon::Check)),
            StatusTone::Error => (c.danger, Some(Icon::Warning)),
        };
        let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
        let block = Block::wrapped(
            ui,
            message,
            text::META,
            color,
            ui.available_width() - icon_w,
        );
        let height = block.height().max(20.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        super::announce(&response, egui::Role::Status, message);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if let Some(icon) = icon {
                icons::paint(
                    painter,
                    icon,
                    pos2(rect.left(), rect.center().y - 7.0),
                    14.0,
                    color,
                );
            }
            block.paint(
                painter,
                pos2(rect.left() + icon_w, rect.center().y - block.height() / 2.0),
            );
        }
        response
    }
}

/// `LoadError` (`.ui-load-error`): the message and "Erneut versuchen"
/// under it, in the danger box. True when the button was pressed.
pub fn load_error(ui: &mut Ui, message: &str, pending: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    // padding 10px 12px after the 3px bar; the button 12px under the text.
    let block = Block::wrapped(ui, message, text::BODY, c.text, width - 3.0 - 24.0);
    let text_h = super::field::layout_unit(block.height());
    let height = 10.0 + text_h + 12.0 + 32.0 + 10.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    super::announce(&response, egui::Role::Alert, message);
    let painter = ui.painter().clone();
    let box_rect = super::snap(ui.ctx(), rect);
    painter.rect_filled(
        box_rect,
        corner(theme.radius.sm),
        over(c.bg, c.danger_bg, 0.18),
    );
    painter
        .with_clip_rect(Rect::from_min_max(
            box_rect.min,
            pos2(box_rect.left() + 3.0, box_rect.bottom()),
        ))
        .rect_filled(box_rect, corner(theme.radius.sm), c.danger);
    block.paint(&painter, pos2(rect.left() + 15.0, rect.top() + 10.0));
    let button = super::Button::new("Erneut versuchen")
        .small()
        .pending(pending.then_some("Wird geladen…"));
    let at = pos2(rect.left() + 15.0, rect.top() + 10.0 + text_h + 12.0);
    let clicked = ui
        .put(
            Rect::from_min_size(at, Vec2::new(button_width(ui, "Erneut versuchen"), 32.0)),
            button,
        )
        .clicked();
    clicked && !pending
}

/// A small button's width for `label` (padding 12px, a 1px border).
fn button_width(ui: &Ui, label: &str) -> f32 {
    Block::single(ui, label, text::UI, Color32::WHITE).width() + 26.0
}
