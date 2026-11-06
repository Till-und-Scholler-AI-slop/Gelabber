//! The emoji picker (components/EmojiPicker.tsx in a wide `Modal`):
//! search, category, the 8-column grid of 80 at a time, "Weitere Emojis
//! anzeigen". The data is web/src/messages/emoji-data.json.

use std::sync::OnceLock;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, TextBuffer, Ui, Vec2, pos2, vec2};
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

fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| {
        let raw: Value =
            serde_json::from_str(include_str!("../../../assets/emoji/emoji-data.json"))
                .unwrap_or(Value::Null);
        let mut groups: Vec<String> = Vec::new();
        let mut entries = Vec::new();
        for e in raw["emoji"].as_array().into_iter().flatten() {
            let emoji = e["emoji"].as_str().unwrap_or_default().to_owned();
            let label = e["label"].as_str().unwrap_or_default().to_owned();
            let group_name = e["group"].as_str().unwrap_or_default().to_owned();
            let group = match groups.iter().position(|g| *g == group_name) {
                Some(i) => i,
                None => {
                    groups.push(group_name);
                    groups.len() - 1
                }
            };
            let keywords: Vec<&str> = e["keywords"]
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
    })
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
                crate::text::label(ui, "Emoji suchen", LABEL, c.text);
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
                crate::text::label(ui, "Kategorie", LABEL, c.text);
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
                    .map_or("Alle Kategorien", |g| d.groups[g].as_str());
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
                                let label =
                                    option.map_or("Alle Kategorien", |g| d.groups[g].as_str());
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
                    &format!("{} Emojis gefunden", matches.len()),
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
                        "Weitere Emojis anzeigen",
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
                let note = format!("Unicode {} / CLDR {} – Lizenz", d.version, d.cldr);
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
                    ui.ctx().open_url(egui::OpenUrl::new_tab(
                        "https://www.unicode.org/license.txt",
                    ));
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
