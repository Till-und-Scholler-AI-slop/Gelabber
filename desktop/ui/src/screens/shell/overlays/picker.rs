//! `Picker` (components/Picker.tsx, shell.css `.shell-picker`): a search
//! field over a list to pick from. ↑ and ↓ move the highlight, Enter picks
//! it, a click picks any row; the caller filters the rows by the query.

use egui::{Color32, Id, Key, Modifiers, Rect, Sense, Stroke, Ui, Vec2, pos2};

use crate::icons::{self, Icon};
use crate::images::{ImageState, Images};
use crate::screens::shell::common::{self, line};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{TextField, count_badge};

/// What sits in a row's 28px icon box.
pub enum RowIcon {
    /// A person: picture or initials (11px) on their tint.
    Avatar { name: String, url: Option<String> },
    /// An 18px icon in muted.
    Glyph(Icon),
    /// A server tile: radius 5, tint, 600 12px initials (or its icon).
    Tile {
        initials: String,
        tint: Color32,
        url: Option<String>,
    },
}

pub struct Row {
    pub label: String,
    pub detail: Option<String>,
    pub icon: RowIcon,
    /// The unread mark after the name: a count pill or a dot.
    pub badge: gelabber_client::Badge,
}

#[derive(Default)]
pub struct PickerState {
    pub query: String,
    active: usize,
}

/// `.shell-picker-list`: `min(360px, 55vh)`.
const LIST_MAX: f32 = 360.0;
const ROW_MIN: f32 = 44.0;

/// Draws the field and the list (or `empty`); returns the row picked.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    images: &Images,
    id: &str,
    state: &mut PickerState,
    label: &str,
    placeholder: &str,
    rows: &[Row],
    empty: &str,
    busy: bool,
) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    let field_id = crate::widgets::field::field_id(id);
    let focused = ctx.memory(|m| m.has_focus(field_id));
    let current = state.active.min(rows.len().saturating_sub(1));
    // The field keeps ↑, ↓ and Enter for the list.
    let (mut step, mut enter) = (0i32, false);
    if focused {
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                step += 1;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                step -= 1;
            }
            enter = i.consume_key(Modifiers::NONE, Key::Enter);
        });
    }
    let mut picked = None;
    let mut active = current;
    if step != 0 && !rows.is_empty() {
        active = (current as i32 + step).clamp(0, rows.len() as i32 - 1) as usize;
    }
    if enter && !rows.is_empty() && !busy {
        picked = Some(current);
    }

    let before = state.query.clone();
    let field = TextField::new(id, &mut state.query)
        .placeholder(placeholder)
        .autofocus(true)
        .show(ui);
    field.response.widget_info(|| {
        egui::WidgetInfo::text_edit(true, before.clone(), state.query.clone(), label)
    });
    if state.query != before {
        active = 0;
    }
    // Chromium's search field: a ✕ that clears it while it has text.
    if !state.query.is_empty() && field.response.has_focus() {
        let field_rect = Rect::from_min_size(field.rect.min, Vec2::new(field.rect.width(), 37.0));
        let center = pos2(field_rect.right() - 21.0, field_rect.center().y);
        let hit = Rect::from_center_size(center, Vec2::splat(14.0));
        let clear = ui.interact(hit, Id::unique((id, "clear")), Sense::click());
        let r = 4.0;
        let stroke = Stroke::new(2.0, c.text);
        let p = ui.painter();
        p.line_segment([center - Vec2::splat(r), center + Vec2::splat(r)], stroke);
        p.line_segment(
            [center + Vec2::new(-r, r), center + Vec2::new(r, -r)],
            stroke,
        );
        if clear.clicked() {
            state.query.clear();
            active = 0;
            field.response.request_focus();
        }
    }

    if rows.is_empty() {
        ui.add_space(8.0);
        let width = ui.available_width();
        let block = Block::wrapped(ui, empty, text::UI, c.muted, width);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(width, block.height() + 32.0), Sense::hover());
        block.paint(
            ui.painter(),
            pos2(rect.center().x - block.width() / 2.0, rect.top() + 16.0),
        );
        state.active = 0;
        return None;
    }

    ui.add_space(8.0);
    // The list bleeds 8px into the dialog's padding on both sides.
    let outer = ui.available_rect_before_wrap();
    let width = outer.width() + 16.0;
    let total = rows.len() as f32 * ROW_MIN;
    let height = total.min(LIST_MAX);
    let (list, _) = ui.allocate_exact_size(Vec2::new(outer.width(), height), Sense::hover());
    let list = Rect::from_min_size(
        pos2(list.left() - 8.0, list.top()),
        Vec2::new(width, height),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let moved = ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
    egui::ScrollArea::vertical()
        .id_salt((id, "list"))
        .max_height(height)
        .auto_shrink([false, true])
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, row) in rows.iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, ROW_MIN), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::ListBoxOption,
                        true,
                        i == active,
                        &row.label,
                    )
                });
                if response.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    if moved {
                        active = i;
                    }
                }
                if response.clicked() && !busy {
                    picked = Some(i);
                }
                if step != 0 && i == active {
                    ui.scroll_to_rect(rect, None);
                }
                paint_row(ui, images, &theme, rect, row, i == active);
            }
        });
    state.active = active;
    picked
}

fn paint_row(ui: &Ui, images: &Images, theme: &Theme, rect: Rect, row: &Row, active: bool) {
    let c = &theme.c;
    let painter = ui.painter();
    if active {
        common::fill(painter, rect, theme.radius.sm, c.selected);
    }
    let icon = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.center().y - 14.0),
        Vec2::splat(28.0),
    );
    match &row.icon {
        RowIcon::Avatar { name, url } => {
            common::avatar(
                ui,
                images,
                icon,
                name,
                url.as_deref(),
                11.0,
                1.0,
                c.elevated,
            );
        }
        RowIcon::Glyph(glyph) => icons::paint_centered(painter, *glyph, icon, 18.0, c.muted),
        RowIcon::Tile {
            initials,
            tint,
            url,
        } => {
            let texture = url.as_deref().filter(|u| !u.is_empty()).and_then(|u| {
                match images.get(ui.ctx(), u) {
                    ImageState::Ready(t) => Some(t),
                    _ => None,
                }
            });
            match texture {
                Some(t) => {
                    let uv = crate::images::cover_uv(t.size_vec2(), icon.size());
                    painter.add(
                        egui::epaint::RectShape::filled(
                            icon,
                            corner(theme.radius.sm),
                            Color32::WHITE,
                        )
                        .with_texture(t.id(), uv),
                    );
                }
                None => {
                    common::fill(painter, icon, theme.radius.sm, *tint);
                    let block =
                        Block::single(ui, initials, TextStyle::new(12.0, 600.0, 1.0), c.text);
                    block.paint(
                        painter,
                        pos2(
                            icon.center().x - block.width() / 2.0,
                            icon.center().y - block.height() / 2.0,
                        ),
                    );
                }
            }
        }
    }
    // The trailing badge first: the text column ends before it.
    let mut right = rect.right() - 8.0;
    let badge = row.badge;
    if badge.count > 0 {
        let pill = count_badge(
            painter,
            ui,
            pos2(right, rect.center().y + 9.0),
            badge.count.min(u32::MAX as i64) as u32,
            None,
        );
        right = pill.left() - 10.0;
    } else if badge.dot {
        painter.circle_filled(pos2(right - 4.0, rect.center().y), 4.0, c.accent);
        right -= 8.0 + 10.0;
    }
    let left = icon.right() + 10.0;
    let width = (right - left).max(0.0);
    let label = line(ui, &row.label, text::UI, c.text, width);
    let detail = row
        .detail
        .as_deref()
        .map(|d| line(ui, d, text::META, c.muted, width));
    let h = label.height() + detail.as_ref().map_or(0.0, Block::height);
    let top = rect.center().y - h / 2.0;
    label.paint(painter, pos2(left, top));
    if let Some(d) = detail {
        d.paint(painter, pos2(left, top + label.height()));
    }
}

/// `normalise` of quickSwitch.ts: lower case, accents off, without a
/// leading "#" or "@".
pub fn normalise(text: &str) -> String {
    let folded: String = text
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ä' | 'á' | 'à' | 'â' | 'ã' | 'å' => 'a',
            'ö' | 'ó' | 'ò' | 'ô' | 'õ' => 'o',
            'ü' | 'ú' | 'ù' | 'û' => 'u',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect();
    let trimmed = folded
        .strip_prefix('#')
        .or_else(|| folded.strip_prefix('@'))
        .unwrap_or(&folded);
    trimmed.trim().to_owned()
}

/// `matchScore`: the start of the name 4, the start of a word 3, inside
/// 2, the letters in order 1, else `None`.
pub fn match_score(query: &str, label: &str) -> Option<u8> {
    let q = normalise(query);
    let text = normalise(label);
    if q.is_empty() {
        return Some(0);
    }
    if text.starts_with(&q) {
        return Some(4);
    }
    let word_start = text.match_indices(&q).any(|(at, _)| {
        text[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_whitespace() || matches!(c, '_' | '-' | '.'))
    });
    if word_start {
        return Some(3);
    }
    if text.contains(&q) {
        return Some(2);
    }
    let mut rest = text.as_str();
    for ch in q.chars() {
        let at = rest.find(ch)?;
        rest = &rest[at + ch.len_utf8()..];
    }
    Some(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_like_the_web() {
        assert_eq!(match_score("", "x"), Some(0));
        assert_eq!(match_score("all", "#allgemein"), Some(4));
        assert_eq!(match_score("abend", "Spiele abend"), Some(3));
        assert_eq!(match_score("ge", "allgemein"), Some(2));
        assert_eq!(match_score("agm", "allgemein"), Some(1));
        assert_eq!(match_score("zz", "allgemein"), None);
        assert_eq!(match_score("ub", "Über"), Some(4));
    }
}
