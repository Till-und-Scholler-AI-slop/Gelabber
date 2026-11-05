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
    click_stop(ui, rect, id, true)
}

/// Like [`click`]; `stop` says whether Tab reaches it (a roving list's
/// rows other than its stop do not).
pub fn click_stop(ui: &Ui, rect: Rect, id: Id, stop: bool) -> Response {
    let sense = if stop { Sense::click() } else { Sense::CLICK };
    let response = ui.interact(rect, id, sense);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// `:focus-visible` on a shell control: the global ring while focus came
/// by keyboard.
pub fn focus_visible(ui: &Ui, response: &Response, rect: Rect, radius: f32) {
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        let theme = Theme::of(ui.ctx());
        crate::widgets::focus_ring(ui.painter(), rect, radius, &theme);
    }
}

/// One row of a roving list: a stable key and its items (the link, then
/// its action buttons) in order.
pub struct NavRow {
    pub key: Id,
    pub items: Vec<Id>,
}

/// Where a list's Tab stop and focus are this frame (rowKeys.ts).
#[derive(Clone, Copy, Debug, Default)]
pub struct Roved {
    /// (row, item) that Tab reaches.
    pub stop: Option<(usize, usize)>,
    /// The row with focus inside it.
    pub within: Option<usize>,
}

impl Roved {
    /// Whether item `item` of row `row` takes the Tab stop.
    pub fn is_stop(&self, row: usize, item: usize) -> bool {
        self.stop == Some((row, item))
    }

    pub fn has_focus_within(&self, row: usize) -> bool {
        self.within == Some(row)
    }
}

/// The hints the web reads once per list (sr-only).
pub const LIST_HINT: &str = "Pfeiltasten hoch und runter wechseln zwischen den Einträgen.";
pub const ROW_HINT: &str = "Pfeiltasten hoch und runter wechseln zwischen den Einträgen, links und rechts zwischen einem Eintrag und seinen Aktionen. Pos1 und Ende springen an den Anfang und das Ende der Zeile.";

/// A whole list is one Tab stop: ↑/↓ move between its rows, ←/→/Pos1/Ende
/// within a row (`moveWithinList`, `moveWithinRow`, `listStop`). The stop
/// is the row focused last, else `preferred` (the open conversation),
/// else the first. Call before drawing the rows; it moves focus for keys
/// pressed this frame.
pub fn rove(ui: &Ui, list: Id, rows: &[NavRow], preferred: Option<usize>) -> Roved {
    let memory_id = list.with("roving");
    let focused = ui.memory(|m| m.focused());
    let at = focused.and_then(|f| {
        rows.iter()
            .enumerate()
            .find_map(|(r, row)| row.items.iter().position(|i| *i == f).map(|i| (r, i)))
    });
    let mut last: Option<(Id, usize)> = ui.data(|d| d.get_temp(memory_id));
    let mut stop = None;
    if let Some((r, i)) = at {
        // The arrows are ours here, not egui's spatial focus moves.
        if let Some(id) = focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
        }
        let mut target = (r, i);
        ui.ctx().input_mut(|input| {
            let none = egui::Modifiers::NONE;
            let count = rows[r].items.len();
            if input.consume_key(none, egui::Key::ArrowDown) {
                target = ((r + 1).min(rows.len() - 1), 0);
            } else if input.consume_key(none, egui::Key::ArrowUp) {
                target = (r.saturating_sub(1), 0);
            } else if input.consume_key(none, egui::Key::ArrowRight) {
                target = (r, (i + 1).min(count - 1));
            } else if input.consume_key(none, egui::Key::ArrowLeft) {
                target = (r, i.saturating_sub(1));
            } else if count > 1 && i != 0 && input.consume_key(none, egui::Key::Home) {
                target = (r, 0);
            } else if count > 1 && i + 1 != count && input.consume_key(none, egui::Key::End) {
                target = (r, count - 1);
            }
        });
        if target != (r, i) {
            ui.memory_mut(|m| m.request_focus(rows[target.0].items[target.1]));
            crate::widgets::set_keyboard_focus(ui.ctx(), true);
        }
        last = Some((rows[target.0].key, target.1));
        stop = Some(target);
    }
    if stop.is_none()
        && let Some((key, item)) = last
        && let Some(r) = rows.iter().position(|row| row.key == key)
    {
        stop = Some((r, item.min(rows[r].items.len().saturating_sub(1))));
    }
    if stop.is_none() {
        stop = preferred
            .filter(|p| *p < rows.len())
            .or((!rows.is_empty()).then_some(0))
            .map(|r| (r, 0));
    }
    ui.data_mut(|d| match last {
        Some(last) => {
            d.insert_temp(memory_id, last);
        }
        None => d.remove::<(Id, usize)>(memory_id),
    });
    Roved {
        stop,
        within: at.map(|(r, _)| r).and(stop.map(|(r, _)| r)),
    }
}

/// The list's key hint for screen readers, once per list.
pub fn list_hint(ui: &Ui, rect: Rect, id: Id, hint: &str) {
    let response = ui.interact(
        Rect::from_min_size(rect.min, Vec2::ZERO),
        id.with("hint"),
        Sense::hover(),
    );
    crate::widgets::announce(&response, egui::Role::Label, hint);
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

#[cfg(test)]
mod tests {
    use crate::app::testkit::{
        ALLGEMEIN, FakeApi, HOME, focused_label, launcher, wait_for_label, workspace,
    };
    use crate::platform::paths::testing::TempConfig;

    /// rowKeys.ts: the rail's server list, the channel list and its rows
    /// are one Tab stop each; the arrows move inside them.
    #[test]
    fn lists_are_one_tab_stop_and_arrows_move_inside() {
        let _config = TempConfig::new("shell-keys");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, &format!("/s/{HOME}/c/{ALLGEMEIN}"));
        wait_for_label(&mut harness, "allgemein");
        let mut order = Vec::new();
        for _ in 0..9 {
            harness.key_press(egui::Key::Tab);
            harness.run_steps(2);
            order.push(focused_label(&harness).unwrap_or_default());
        }
        assert_eq!(
            order,
            [
                "Zum Inhalt springen",
                "Direktnachrichten",
                "Wohnzimmer",
                "Server hinzufügen",
                "Menü für Wohnzimmer",
                "Einladen",
                "Übersicht",
                "allgemein",
                "Lena"
            ]
        );
        // Back into the list, then ↓ to the next row, → into its actions.
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        harness.run_steps(2);
        assert_eq!(focused_label(&harness).as_deref(), Some("allgemein"));
        let mut keys = Vec::new();
        for key in [
            egui::Key::ArrowDown,
            egui::Key::ArrowRight,
            egui::Key::ArrowRight,
            egui::Key::ArrowRight,
            egui::Key::Home,
            egui::Key::End,
            egui::Key::ArrowLeft,
            egui::Key::ArrowDown,
            egui::Key::ArrowUp,
            egui::Key::ArrowUp,
        ] {
            harness.key_press(key);
            harness.run_steps(2);
            keys.push(focused_label(&harness).unwrap_or_default());
        }
        assert_eq!(
            keys,
            [
                "Kanal in dieser Kategorie",
                "Kategorie umbenennen",
                "Kategorie löschen",
                "Kategorie löschen",
                "Kanal in dieser Kategorie",
                "Kategorie löschen",
                "Kategorie umbenennen",
                "Wohnzimmer",
                "Kanal in dieser Kategorie",
                "allgemein"
            ]
        );
        // The stop roves: Tab away and Shift+Tab back lands on the row again.
        harness.key_press(egui::Key::ArrowDown);
        harness.run_steps(2);
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        harness.run_steps(2);
        assert_eq!(
            focused_label(&harness).as_deref(),
            Some("Kanal in dieser Kategorie")
        );
    }
}
