//! The primitives of web/src/components/ui (ui.css), drawn by hand so every
//! size, radius, border and state matches the CSS: buttons, fields, links,
//! avatars, badges, tooltips, modal, popover, switch, checkbox, separators.

pub mod avatar;
pub mod badge;
pub mod button;
pub mod field;
pub mod link;
pub mod modal;
pub mod opener;
pub mod popover;
pub mod separator;
pub mod status;
pub mod switch;
pub mod tooltip;

pub use avatar::{Avatar, AvatarSize, Presence};
pub use badge::{Badge, BadgeTone, count_badge};
pub use button::{Button, ButtonSize, ButtonVariant, IconButton};
pub use field::{FieldOutput, TextField};
pub use link::Link;
pub use modal::{Modal, ModalOutput, ModalWidth};
pub use popover::{Popover, menu_item};
pub use separator::{hairline_h, hairline_v};
pub use status::{FormError, InlineStatus, StatusTone};
pub use switch::{Checkbox, Switch};
pub use tooltip::Tooltip;

use egui::{Color32, Painter, Rect, Stroke, StrokeKind};

use crate::theme::{Theme, corner};

/// Gives a drawn text its accessibility node (`role="alert"`, `"status"`,
/// a plain label), so screen readers hear it and tests can find it.
pub fn announce(response: &egui::Response, role: egui::Role, text: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(role, true, text));
}

/// Whether focus moves by keyboard right now: `:focus-visible` shows the
/// ring for buttons and links only then; text fields always show it.
pub fn keyboard_focus(ctx: &egui::Context) -> bool {
    let id = egui::Id::unique("gelabber-keyboard-focus");
    let (tab, pointer) = ctx.input(|i| (i.key_pressed(egui::Key::Tab), i.pointer.any_pressed()));
    let mut state = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if tab {
        state = true;
    } else if pointer {
        state = false;
    }
    ctx.data_mut(|d| d.insert_temp(id, state));
    state
}

/// Marks focus as moved by keyboard (or by pointer), as a shortcut that
/// opens a dialog does for :focus-visible in the browser.
pub fn set_keyboard_focus(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("gelabber-keyboard-focus"), on));
}

/// The global focus ring (index.css): a 2px page-coloured gap around the
/// box, then a 2px `--lr-focus` outline that follows its rounding.
pub fn focus_ring(painter: &Painter, rect: Rect, radius: f32, theme: &Theme) {
    painter.rect_stroke(
        rect.expand(1.0),
        corner(radius + 1.0),
        Stroke::new(2.0, theme.c.bg),
        StrokeKind::Middle,
    );
    painter.rect_stroke(
        rect.expand(3.0),
        corner(radius + 3.0),
        Stroke::new(2.0, theme.c.focus),
        StrokeKind::Middle,
    );
}

/// An inside-the-box `outline-offset: -2px` ring (list rows, tabs).
pub fn focus_ring_inset(painter: &Painter, rect: Rect, radius: f32, theme: &Theme) {
    painter.rect_stroke(
        rect.shrink(3.0),
        corner((radius - 3.0).max(0.0)),
        Stroke::new(2.0, theme.c.focus),
        StrokeKind::Middle,
    );
}

/// A colour change eased over `--lr-dur-fast`, like the CSS transitions on
/// hover and pressed states.
pub fn eased(ctx: &egui::Context, id: egui::Id, on: bool, from: Color32, to: Color32) -> Color32 {
    let t = ctx.animate_bool_with_time(id, on, crate::theme::motion::FAST);
    lerp(from, to, t)
}

/// Per-channel lerp including alpha (premultiplied).
pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        ch(a.r(), b.r()),
        ch(a.g(), b.g()),
        ch(a.b(), b.b()),
        ch(a.a(), b.a()),
    )
}

/// Snaps a CSS length to device pixels, as Blink does for box edges.
pub fn snap(ctx: &egui::Context, rect: Rect) -> Rect {
    let ppp = ctx.pixels_per_point();
    let r = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_max(
        egui::pos2(r(rect.min.x), r(rect.min.y)),
        egui::pos2(r(rect.max.x), r(rect.max.y)),
    )
}
