//! Drawing helpers the shell's parts share: one-line text with an
//! ellipsis, clickable rows, avatars with a chosen initials size, presence
//! dots and the accent edge bars.

use egui::text::TextWrapping;
use egui::{Color32, CornerRadius, Id, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, pos2};

use crate::images::{ImageState, Images};
use crate::initials::{initials, person_tone};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, mix};
use crate::widgets::Presence;

/// One line, cut with `…` at `max_width` (`text-overflow: ellipsis`).
pub fn line(ui: &Ui, text: &str, style: TextStyle, color: Color32, max_width: f32) -> Block {
    let mut job = text::job(text, style, color, None);
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    Block::new(ui, job, style)
}

/// Paints `block` with its line box centred on `center_y`.
pub fn paint_mid(painter: &Painter, block: &Block, x: f32, center_y: f32) {
    block.paint(painter, pos2(x, center_y - block.height() / 2.0));
}

/// The top of a line box in `style` whose baseline meets that of a line
/// box in `base` starting at `base_top` (inline runs of another size).
pub fn baseline_top(base: TextStyle, base_top: f32, style: TextStyle) -> f32 {
    base_top + base.baseline() - style.baseline()
}

/// A clickable area with the pointer cursor while hovered.
pub fn click(ui: &Ui, rect: Rect, id: Id) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// `.channel-row.is-unread::before`: 3×16 accent bar at the column edge,
/// rounded on the right.
pub fn edge_bar(painter: &Painter, theme: &Theme, x: f32, center_y: f32) {
    let rect = Rect::from_min_size(pos2(x, center_y - 8.0), Vec2::new(3.0, 16.0));
    painter.rect_filled(
        rect,
        CornerRadius {
            nw: 0,
            sw: 0,
            ne: 3,
            se: 3,
        },
        theme.c.accent,
    );
}

/// An avatar circle: the picture, or the initials (600, `font` px) on the
/// person's tint. `opacity` fades it over `under` (offline members).
#[allow(clippy::too_many_arguments)]
pub fn avatar(
    ui: &Ui,
    images: &Images,
    rect: Rect,
    name: &str,
    url: Option<&str>,
    font: f32,
    opacity: f32,
    under: Color32,
) {
    let theme = Theme::of(ui.ctx());
    let painter = ui.painter();
    let radius = rect.width() / 2.0;
    let texture = url
        .filter(|u| !u.is_empty())
        .and_then(|url| match images.get(ui.ctx(), url) {
            ImageState::Ready(texture) => Some(texture),
            _ => None,
        });
    let fade = |c: Color32| {
        if opacity >= 1.0 {
            c
        } else {
            mix(under, c, f64::from(opacity))
        }
    };
    if let Some(texture) = texture {
        let uv = crate::images::cover_uv(texture.size_vec2(), rect.size());
        let shape = egui::epaint::RectShape::filled(
            rect,
            CornerRadius::same(radius.round() as u8),
            Color32::WHITE.gamma_multiply(opacity),
        )
        .with_texture(texture.id(), uv);
        painter.add(shape);
        return;
    }
    painter.circle_filled(rect.center(), radius, fade(theme.tile(person_tone(name))));
    let style = TextStyle::new(font, 600.0, 1.5);
    let block = Block::single(ui, &initials(name), style, fade(theme.c.text));
    block.paint(
        painter,
        Pos2::new(
            rect.center().x - block.width() / 2.0,
            rect.center().y - block.height() / 2.0,
        ),
    );
}

/// `.gel-presence-dot`: at the avatar's bottom-right, max(23 %, 10px),
/// a 2px ring in `--lr-bg` around the status colour.
pub fn presence_dot(painter: &Painter, theme: &Theme, avatar: Rect, presence: Presence) {
    let size = (avatar.width() * 0.23).max(10.0);
    let center = avatar.max - Vec2::splat(size / 2.0);
    let fill = match presence {
        Presence::Online => theme.c.online,
        Presence::Away => theme.c.warning,
        Presence::Offline => theme.c.muted,
    };
    painter.circle_filled(center, size / 2.0, theme.c.bg);
    painter.circle_filled(center, size / 2.0 - 2.0, fill);
}

/// The wire presence as the widget's.
pub fn presence(status: gelabber_client::PresenceStatus) -> Presence {
    match status {
        gelabber_client::PresenceStatus::Online => Presence::Online,
        gelabber_client::PresenceStatus::Idle => Presence::Away,
        gelabber_client::PresenceStatus::Offline => Presence::Offline,
    }
}

/// `filled_rect` with a uniform CSS radius.
pub fn fill(painter: &Painter, rect: Rect, radius: f32, color: Color32) {
    painter.rect_filled(rect, crate::theme::corner(radius), color);
}
