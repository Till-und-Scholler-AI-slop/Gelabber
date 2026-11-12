//! The emoji picker (components/EmojiPicker.tsx in a wide `Modal`):
//! search, category, the 8-column grid of 80 at a time, "Weitere Emojis
//! anzeigen". The data is web/src/messages/emoji-data.json; in English the
//! names, search words and categories come from emoji-names.en.json
//! (components/emojiNames.ts), German where it has no entry.

use std::sync::OnceLock;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, TextBuffer, Ui, Vec2, pos2, vec2};
use gelabber_i18n::{Locale, emoji as texts};
use serde_json::Value;

use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Modal, ModalWidth};

struct Entry {
    emoji: String,
    /// Emoji, label and keywords, lower-cased, for the search.
    haystack: String,
    label: String,
    group: usize,
}

struct Data {
    version: String,
    cldr: String,
    groups: Vec<String>,
    entries: Vec<Entry>,
}

/// The picker's data in the interface's language. The entries and groups
/// are in the same order in both, so a chosen group and the drawable
/// check carry over a language switch.
fn data() -> &'static Data {
    static GERMAN: OnceLock<Data> = OnceLock::new();
    static ENGLISH: OnceLock<Data> = OnceLock::new();
    match gelabber_i18n::get() {
        Locale::De => GERMAN.get_or_init(|| load(&Value::Null)),
        Locale::En => ENGLISH.get_or_init(|| {
            let names: Value =
                serde_json::from_str(include_str!("../../../assets/emoji/emoji-names.en.json"))
                    .unwrap_or(Value::Null);
            load(&names)
        }),
    }
}

/// `names`: emoji-names.en.json (`{groups: {de: en}, emoji: {e: [label,
/// keywords]}}`), or null for German.
fn load(names: &Value) -> Data {
    let raw: Value = serde_json::from_str(include_str!("../../../assets/emoji/emoji-data.json"))
        .unwrap_or(Value::Null);
    let mut groups: Vec<String> = Vec::new();
    let mut entries = Vec::new();
    for e in raw["emoji"].as_array().into_iter().flatten() {
        let emoji = e["emoji"].as_str().unwrap_or_default().to_owned();
        let english = &names["emoji"][emoji.as_str()];
        let (label, keywords) = match english.as_array() {
            Some(pair) => (&pair[0], &pair[1]),
            None => (&e["label"], &e["keywords"]),
        };
        let label = label.as_str().unwrap_or_default().to_owned();
        let group_name = e["group"].as_str().unwrap_or_default();
        let group_name = names["groups"][group_name]
            .as_str()
            .unwrap_or(group_name)
            .to_owned();
        let group = match groups.iter().position(|g| *g == group_name) {
            Some(i) => i,
            None => {
                groups.push(group_name);
                groups.len() - 1
            }
        };
        let keywords: Vec<&str> = keywords
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let haystack = format!("{emoji} {label} {}", keywords.join(" ")).to_lowercase();
        entries.push(Entry {
            emoji,
            haystack,
            label,
            group,
        });
    }
    Data {
        version: raw["version"].as_str().unwrap_or_default().to_owned(),
        cldr: raw["cldr"].as_str().unwrap_or_default().to_owned(),
        groups,
        entries,
    }
}

#[derive(Default)]
pub struct PickerState {
    query: String,
    group: Option<usize>,
    limit: usize,
    group_open: bool,
    mounted: bool,
    /// Which entries the fonts here can draw, checked once.
    drawable: Vec<bool>,
}

/// Emoji newer than the system font draws as boxes: those stay out.
fn drawable(ui: &Ui, emoji: &str) -> bool {
    let Some(first) = emoji.chars().next() else {
        return false;
    };
    if !emoji.contains('\u{200d}') && (first as u32) < 0x1f000 {
        return true;
    }
    let font = egui::FontId::new(24.0, egui::FontFamily::Proportional);
    ui.fonts_mut(|f| f.has_glyph(&font, first))
}

/// Tailwind `text-sm font-medium`: 14px / 20px.
const LABEL: TextStyle = TextStyle::new(14.0, 500.0, 20.0 / 14.0);
const INPUT: TextStyle = TextStyle::new(14.0, 400.0, 1.5);

/// Shows the picker; `Some(Some(e))` picked, `Some(None)` closed.
pub fn show(
    ctx: &egui::Context,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    title: &str,
    state: &mut PickerState,
) -> Option<Option<String>> {
    let theme = Theme::of(ctx);
    let c = theme.c;
    if state.limit == 0 {
        state.limit = 80;
    }
    let mut picked = None;
    let output = Modal::new(("emoji", id), title)
        .width(ModalWidth::Wide)
        .show(
            ctx,
            |ui| {
                let d = data();
                let width = ui.available_width();
                ui.spacing_mut().item_spacing.y = 12.0;
                crate::text::label(ui, texts::picker_search(), LABEL, c.text);
                let input_id = egui::Id::unique(("gelabber-emoji-search", id));
                let (r, _) = ui.allocate_exact_size(vec2(width, 39.0), Sense::hover());
                let border = if theme.is_dark() { c.muted } else { c.border };
                ui.painter().rect(
                    r,
                    corner(4.0),
                    Color32::TRANSPARENT,
                    Stroke::new(1.0, border),
                    StrokeKind::Inside,
                );
                let color = c.text;
                let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, _w: f32| {
                    let mut job = crate::text::job(buf.as_str(), INPUT, color, None);
                    job.keep_trailing_whitespace = true;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let before = state.query.clone();
                ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(1.0)), |ui| {
                    egui::TextEdit::singleline(&mut state.query)
                        .id(input_id)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(8)))
                        .desired_width(r.width() - 2.0)
                        .layouter(&mut layouter)
                        .show(ui)
                        .response
                        .response
                        // The label above names the field (`<label for>`).
                        .accessible_name(texts::picker_search())
                });
                if ui.memory(|m| m.has_focus(input_id)) {
                    crate::widgets::focus_ring(ui.painter(), r, 4.0, &theme);
                }
                if !state.mounted {
                    state.mounted = true;
                    ui.memory_mut(|m| m.request_focus(input_id));
                }
                if state.query != before {
                    state.limit = 80;
                }
                crate::text::label(ui, texts::picker_group(), LABEL, c.text);
                let (sel, _) = ui.allocate_exact_size(vec2(width, 37.0), Sense::hover());
                let resp = ui.interact(
                    sel,
                    egui::Id::unique(("gelabber-emoji-group", id)),
                    Sense::click(),
                );
                ui.painter().rect(
                    sel,
                    corner(4.0),
                    Color32::TRANSPARENT,
                    Stroke::new(1.0, border),
                    StrokeKind::Inside,
                );
                let current = state
                    .group
                    .map_or(texts::picker_all_groups(), |g| d.groups[g].as_str());
                Block::single(ui, current, INPUT, c.text)
                    .paint(ui.painter(), pos2(sel.left() + 13.0, sel.top() + 8.0));
                crate::icons::paint(
                    ui.painter(),
                    crate::icons::Icon::Chevron,
                    pos2(sel.right() - 18.0, sel.center().y - 6.0),
                    12.0,
                    c.text,
                );
                if resp.clicked() {
                    state.group_open = !state.group_open;
                }
                if state.group_open {
                    let mut choose = None;
                    let options: Vec<Option<usize>> = std::iter::once(None)
                        .chain((0..d.groups.len()).map(Some))
                        .collect();
                    if let Some(()) = crate::widgets::Popover::new(("emoji-group", id), sel)
                        .min_width(width)
                        .select_list()
                        .show(ctx, |ui| {
                            for option in &options {
                                let label = option
                                    .map_or(texts::picker_all_groups(), |g| d.groups[g].as_str());
                                if crate::widgets::menu_item(ui, None, label, false).clicked() {
                                    choose = Some(*option);
                                }
                            }
                        })
                    {
                    } else {
                        state.group_open = false;
                    }
                    if let Some(g) = choose {
                        state.group = g;
                        state.limit = 80;
                        state.group_open = false;
                    }
                }
                if state.drawable.len() != d.entries.len() {
                    state.drawable = d.entries.iter().map(|e| drawable(ui, &e.emoji)).collect();
                }
                let terms: Vec<String> = state
                    .query
                    .trim()
                    .to_lowercase()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                let matches: Vec<&Entry> = d
                    .entries
                    .iter()
                    .zip(&state.drawable)
                    .filter(|(_, ok)| **ok)
                    .map(|(e, _)| e)
                    .filter(|e| state.group.is_none_or(|g| e.group == g))
                    .filter(|e| terms.iter().all(|t| e.haystack.contains(t.as_str())))
                    .collect();
                crate::text::label(
                    ui,
                    &texts::picker_found(matches.len() as u64),
                    TextStyle::new(12.0, 400.0, 16.0 / 12.0),
                    c.muted,
                );
                let cols = 8usize;
                let cell = vec2((width - 7.0 * 4.0) / cols as f32, 44.0);
                let shown = &matches[..matches.len().min(state.limit)];
                let rows = shown.len().div_ceil(cols);
                let grid_h = rows as f32 * 44.0 + rows.saturating_sub(1) as f32 * 4.0;
                let max_h = (ctx.content_rect().height() * 0.4).floor();
                egui::ScrollArea::vertical()
                    .id_salt(("gelabber-emoji-grid", id))
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        let (grid, _) = ui.allocate_exact_size(vec2(width, grid_h), Sense::hover());
                        for (i, e) in shown.iter().enumerate() {
                            let col = (i % cols) as f32;
                            let row = (i / cols) as f32;
                            let r = Rect::from_min_size(
                                pos2(grid.left() + col * (cell.x + 4.0), grid.top() + row * 48.0),
                                cell,
                            );
                            if !ui.is_rect_visible(r) {
                                continue;
                            }
                            let resp = ui.interact(
                                r,
                                egui::Id::unique(("gelabber-emoji", id, i)),
                                Sense::click(),
                            );
                            if resp.hovered() {
                                let fill = if theme.is_dark() { c.surface } else { c.panel };
                                ui.painter().rect_filled(r, corner(4.0), fill);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let glyph = Block::single(
                                ui,
                                &e.emoji,
                                TextStyle::new(24.0, 400.0, 32.0 / 24.0),
                                c.text,
                            );
                            glyph.paint(ui.painter(), r.center() - glyph.size() / 2.0);
                            let resp = resp.on_hover_text(&e.label);
                            if resp.clicked() {
                                picked = Some(e.emoji.clone());
                            }
                        }
                    });
                if matches.len() > state.limit {
                    let label = Block::single(
                        ui,
                        texts::picker_more(),
                        TextStyle::new(14.0, 400.0, 20.0 / 14.0),
                        c.text,
                    );
                    let (r, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
                    ui.painter().rect(
                        r,
                        corner(4.0),
                        Color32::TRANSPARENT,
                        Stroke::new(1.0, c.text),
                        StrokeKind::Inside,
                    );
                    label.paint(
                        ui.painter(),
                        pos2(r.center().x - label.width() / 2.0, r.top() + 9.0),
                    );
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        state.limit += 80;
                    }
                }
                let note = texts::picker_licence(&d.version, &d.cldr);
                let block =
                    Block::single(ui, &note, TextStyle::new(12.0, 400.0, 16.0 / 12.0), c.muted);
                let (r, resp) = ui
                    .allocate_exact_size(Vec2::new(block.width(), block.height()), Sense::click());
                block.paint(ui.painter(), r.min);
                crate::widgets::link::underline(
                    ui.painter(),
                    TextStyle::new(12.0, 400.0, 16.0 / 12.0),
                    r.top(),
                    r.left(),
                    r.right(),
                    c.muted,
                );
                if resp.clicked() {
                    crate::platform::open_url_logged("https://www.unicode.org/license.txt");
                }
            },
            None,
        );
    if let Some(e) = picked {
        return Some(Some(e));
    }
    if output.close_requested && !state.group_open {
        return Some(None);
    }
    None
}

#[cfg(test)]
mod tests {
    use gelabber_i18n::{Locale, on_this_thread};

    use super::data;

    /// In English the names, search words and categories come from
    /// emoji-names.en.json, in the same order as the German ones.
    #[test]
    fn english_names_and_search_words() {
        let (de_groups, de_label) = {
            let _german = on_this_thread(Locale::De);
            let d = data();
            (d.groups.clone(), d.entries[0].label.clone())
        };
        let _english = on_this_thread(Locale::En);
        let d = data();
        assert_eq!(de_groups[0], "Gesichter und Gefühle");
        assert_eq!(d.groups[0], "Smileys & Emotion");
        assert_eq!(d.groups.len(), de_groups.len());
        assert_eq!(d.entries[0].emoji, "😀");
        assert_eq!(de_label, "grinsendes Gesicht");
        assert_eq!(d.entries[0].label, "grinning face");
        assert!(d.entries[0].haystack.contains("happy"));
    }
}
