//! The GIF picker (components/GifPicker.tsx in a wide `Modal`): trending
//! GIFs, a search after 550ms of typing, three columns of 150×112 tiles,
//! "Weitere GIFs anzeigen" and the KLIPY attribution.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, TextBuffer, Ui, pos2, vec2};
use gelabber_client::{ApiError, Client, Gif, GifPage, GifPick};

use crate::images::{ImageState, Images, cover_uv};
use crate::task::{Pending, poll};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Modal, ModalWidth};

const DEBOUNCE: f64 = 0.55;
const LABEL: TextStyle = TextStyle::new(14.0, 500.0, 20.0 / 14.0);
const INPUT: TextStyle = TextStyle::new(14.0, 400.0, 1.5);
const SMALL: TextStyle = TextStyle::new(12.0, 400.0, 16.0 / 12.0);

#[derive(Default)]
pub struct GifState {
    input: String,
    changed_at: f64,
    query: Option<String>,
    gifs: Vec<Gif>,
    next: Option<u32>,
    error: Option<ApiError>,
    request: Option<Pending<Result<GifPage, ApiError>>>,
    more: bool,
    mounted: bool,
}

fn status(state: &GifState) -> String {
    if state.request.is_some() && !state.more {
        return "GIFs werden geladen …".into();
    }
    if let Some(error) = &state.error {
        return match error.code() {
            "rate_limited" | "upstream_quota" | "upstream_unavailable" => error.german().to_owned(),
            _ if state.gifs.is_empty() => "GIFs konnten nicht geladen werden.".into(),
            _ => "Weitere GIFs konnten nicht geladen werden.".into(),
        };
    }
    let query = state.query.clone().unwrap_or_default();
    match (state.gifs.len(), query.is_empty()) {
        (0, _) => "Keine GIFs gefunden.".into(),
        (n, true) => format!("{n} angesagte GIFs"),
        (n, false) => format!("{n} GIFs zu „{query}“"),
    }
}

fn fetch(client: &Client, query: &str, page: Option<u32>) -> Pending<Result<GifPage, ApiError>> {
    let http = client.http().clone();
    let query = query.to_owned();
    Pending::spawn(client, async move {
        if query.is_empty() {
            http.gifs_trending(page).await
        } else {
            http.gifs_search(&query, page).await
        }
    })
}

/// Shows the picker; `Some(Some(pick))` chosen, `Some(None)` closed.
pub fn show(
    ctx: &egui::Context,
    client: &Client,
    images: &Images,
    state: &mut GifState,
) -> Option<Option<GifPick>> {
    let theme = Theme::of(ctx);
    let c = theme.c;
    let now = ctx.input(|i| i.time);
    let wanted = state.input.trim().to_owned();
    if state.query.as_deref() != Some(wanted.as_str()) {
        if state.query.is_none() || now - state.changed_at >= DEBOUNCE {
            state.query = Some(wanted.clone());
            state.gifs.clear();
            state.next = None;
            state.error = None;
            state.more = false;
            state.request = Some(fetch(client, &wanted, None));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(DEBOUNCE));
        }
    }
    if let Some(result) = poll(&mut state.request) {
        match result {
            Ok(page) => {
                state.gifs.extend(page.gifs);
                state.next = page.next_page;
                state.error = None;
            }
            Err(e) => state.error = Some(e),
        }
        state.more = false;
    }
    let mut picked = None;
    let output = Modal::new("gif-picker", "GIF senden")
        .width(ModalWidth::Wide)
        .show(
            ctx,
            |ui| {
                let width = ui.available_width();
                ui.spacing_mut().item_spacing.y = 12.0;
                crate::text::label(ui, "GIF suchen", LABEL, c.text);
                let input_id = egui::Id::unique("gelabber-gif-search");
                let (r, _) = ui.allocate_exact_size(vec2(width, 39.0), Sense::hover());
                let border = if theme.is_dark() { c.muted } else { c.border };
                ui.painter().rect(
                    r,
                    corner(4.0),
                    Color32::TRANSPARENT,
                    Stroke::new(1.0, border),
                    StrokeKind::Inside,
                );
                if ui.memory(|m| m.has_focus(input_id)) {
                    crate::widgets::focus_ring(ui.painter(), r, 4.0, &theme);
                }
                let color = c.text;
                let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, _w: f32| {
                    let mut job = crate::text::job(buf.as_str(), INPUT, color, None);
                    job.keep_trailing_whitespace = true;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let before = state.input.clone();
                ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(1.0)), |ui| {
                    egui::TextEdit::singleline(&mut state.input)
                        .id(input_id)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(8)))
                        .desired_width(r.width() - 2.0)
                        .char_limit(100)
                        .layouter(&mut layouter)
                        .show(ui)
                });
                if state.input.is_empty() {
                    Block::single(ui, "Search KLIPY", INPUT, c.subtle)
                        .paint(ui.painter(), pos2(r.left() + 9.0, r.top() + 9.0));
                }
                if state.input != before {
                    state.changed_at = now;
                }
                if !state.mounted {
                    state.mounted = true;
                    ui.memory_mut(|m| m.request_focus(input_id));
                }
                crate::text::label(ui, &status(state), SMALL, c.muted);
                if state.error.is_some()
                    && state.gifs.is_empty()
                    && retry_button(ui, width, "Erneut versuchen")
                {
                    let q = state.query.clone().unwrap_or_default();
                    state.error = None;
                    state.request = Some(fetch(client, &q, None));
                }
                let tile = vec2((width - 12.0) / 3.0, 112.0);
                let rows = state.gifs.len().div_ceil(3);
                let grid_h = rows as f32 * 112.0 + rows.saturating_sub(1) as f32 * 6.0;
                let max_h = (ctx.content_rect().height() * 0.5).floor();
                egui::ScrollArea::vertical()
                    .id_salt("gelabber-gif-grid")
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        let (grid, _) = ui.allocate_exact_size(vec2(width, grid_h), Sense::hover());
                        for (i, gif) in state.gifs.iter().enumerate() {
                            let r = Rect::from_min_size(
                                pos2(
                                    grid.left() + (i % 3) as f32 * (tile.x + 6.0),
                                    grid.top() + (i / 3) as f32 * 118.0,
                                ),
                                tile,
                            );
                            if !ui.is_rect_visible(r) {
                                continue;
                            }
                            ui.painter().rect_filled(r, corner(7.0), c.panel);
                            let url = client
                                .http()
                                .gif_media_url(gif.still.as_deref().unwrap_or(&gif.url));
                            let resp = ui.interact(
                                r,
                                egui::Id::unique(("gelabber-gif", i)),
                                Sense::click(),
                            );
                            if let ImageState::Ready(t) = images.get(ui.ctx(), &url) {
                                let tint = if resp.hovered() {
                                    Color32::WHITE
                                } else {
                                    Color32::from_gray(235)
                                };
                                ui.painter().add(
                                    egui::epaint::RectShape::filled(
                                        r,
                                        egui::CornerRadius::same(7),
                                        tint,
                                    )
                                    .with_texture(t.id(), cover_uv(t.size_vec2(), r.size())),
                                );
                            }
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if !gif.title.is_empty() {
                                crate::widgets::Tooltip::new(&gif.title)
                                    .on_hover(&resp, crate::widgets::tooltip::Side::Above);
                            }
                            if resp.clicked() {
                                picked = Some(GifPick {
                                    slug: gif.slug.clone(),
                                    query: state.query.clone().unwrap_or_default(),
                                });
                            }
                        }
                    });
                if let Some(page) = state.next {
                    let label = if state.more {
                        "Lädt …"
                    } else {
                        "Weitere GIFs anzeigen"
                    };
                    if retry_button(ui, width, label) && !state.more {
                        let q = state.query.clone().unwrap_or_default();
                        state.more = true;
                        state.request = Some(fetch(client, &q, Some(page)));
                    }
                }
                let note = Block::single(
                    ui,
                    "Powered by KLIPY",
                    TextStyle::new(11.0, 400.0, 1.5),
                    c.muted,
                );
                let (r, _) = ui.allocate_exact_size(vec2(width, note.height()), Sense::hover());
                note.paint(ui.painter(), pos2(r.right() - note.width(), r.top()));
            },
            None,
        );
    if picked.is_some() {
        return Some(picked);
    }
    output.close_requested.then_some(None)
}

fn retry_button(ui: &mut Ui, width: f32, label: &str) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::single(ui, label, TextStyle::new(14.0, 400.0, 20.0 / 14.0), c.text);
    let (r, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    ui.painter().rect(
        r,
        corner(4.0),
        Color32::TRANSPARENT,
        Stroke::new(1.0, c.text),
        StrokeKind::Inside,
    );
    block.paint(
        ui.painter(),
        pos2(r.center().x - block.width() / 2.0, r.top() + 9.0),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}
