//! The header search's hits over the list (messages/MessageSearch.tsx,
//! chat.css `.lr-search-*`): after 250ms of typing, newest first; a hit
//! goes back to the chat at that message when it is loaded.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use egui::{Rect, Sense, Ui, pos2, vec2};
use gelabber_client::{ApiError, Client, MessagePage};
use uuid::Uuid;

use super::content::{self, Mentions};
use super::time;
use crate::task::{Pending, poll};
use crate::text::{Block, Rich, TextStyle};
use crate::theme::{Theme, corner};

const DEBOUNCE: f64 = 0.25;

#[derive(Default)]
pub struct SearchState {
    pub text: String,
    changed_at: f64,
    query: String,
    request: Option<Pending<Result<MessagePage, ApiError>>>,
    result: Option<Result<MessagePage, String>>,
    scroll: f32,
}

impl SearchState {
    pub fn changed(&mut self, now: f64) {
        self.changed_at = now;
    }

    /// Whether the hits replace the list (typing paused with text).
    pub fn active(&mut self, now: f64) -> bool {
        !self.text.trim().is_empty()
            && (now - self.changed_at >= DEBOUNCE || !self.query.is_empty())
    }
}

pub enum SearchAction {
    Close,
    Open(Uuid),
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    client: &Client,
    channel: Uuid,
    state: &mut SearchState,
    names: &HashMap<String, String>,
    me: Uuid,
    now: DateTime<Utc>,
) -> Option<SearchAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let t = ui.input(|i| i.time);
    let wanted = state.text.trim().to_owned();
    if t - state.changed_at < DEBOUNCE {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(DEBOUNCE));
    } else if wanted != state.query {
        state.query = wanted.clone();
        state.result = None;
        state.scroll = 0.0;
        let http = client.http().clone();
        state.request = Some(Pending::spawn(client, async move {
            http.search_messages(channel, &wanted, None, Some(25)).await
        }));
    }
    if let Some(result) = poll(&mut state.request) {
        state.result = Some(result.map_err(|e| e.german().to_owned()));
    }
    ui.painter().rect_filled(rect, 0.0, c.bg);
    let mut action = None;
    // Summary line: padding 10 20 0.
    let summary = Block::single(
        ui,
        "Treffer in dieser Unterhaltung",
        TextStyle::new(12.0, 400.0, 1.5),
        c.muted,
    );
    summary.paint(
        ui.painter(),
        pos2(rect.left() + 20.0, rect.top() + 10.0 + 4.0),
    );
    let back = Block::single(
        ui,
        "Zurück zum Chat",
        TextStyle::new(12.0, 600.0, 1.5),
        c.muted,
    );
    let back_rect = Rect::from_min_size(
        pos2(rect.right() - 20.0 - back.width() - 16.0, rect.top() + 10.0),
        vec2(back.width() + 16.0, 26.0),
    );
    let resp = ui.interact(
        back_rect,
        egui::Id::unique(("gelabber-search-back", channel)),
        Sense::click(),
    );
    if resp.hovered() {
        ui.painter().rect_filled(back_rect, corner(7.0), c.hover);
    }
    back.paint_tinted(
        ui.painter(),
        pos2(back_rect.left() + 8.0, back_rect.top() + 4.0),
        resp.hovered().then_some(c.text),
    );
    if resp.clicked() {
        action = Some(SearchAction::Close);
    }
    let body = Rect::from_min_max(
        pos2(rect.left() + 12.0, back_rect.bottom() + 10.0),
        pos2(rect.right() - 12.0, rect.bottom()),
    );
    let state_line = |ui: &Ui, text: &str| {
        Block::single(ui, text, TextStyle::new(14.0, 400.0, 1.5), c.muted)
            .paint(ui.painter(), pos2(body.left() + 8.0, body.top() + 6.0));
    };
    match &state.result {
        None => state_line(ui, "Suche läuft…"),
        Some(Err(_)) => state_line(ui, "Die Suche konnte nicht geladen werden."),
        Some(Ok(page)) if page.messages.is_empty() => state_line(ui, "Keine Nachrichten gefunden."),
        Some(Ok(page)) => {
            if ui.rect_contains_pointer(body) {
                state.scroll = (state.scroll - ui.input(|i| i.smooth_scroll_delta.y)).max(0.0);
            }
            let clip = ui.painter().with_clip_rect(body.intersect(ui.clip_rect()));
            let mut y = body.top() - state.scroll;
            let width = body.width() - 24.0;
            for m in page.messages.iter().rev() {
                // `p:first-child`: the name and, 8 apart on its baseline, the time.
                let style = TextStyle::new(13.0, 400.0, 1.5);
                let name = Block::single(ui, &m.author.name, style.weight(700.0), c.text);
                let when = Rich::new(style)
                    .push(
                        &time::format_search(m.created_at),
                        TextStyle::new(11.0, 400.0, 1.5),
                        c.muted,
                    )
                    .layout(ui);
                // The message's markdown, as MessageContent draws it here.
                let mentions = Mentions {
                    names,
                    me: Some(me),
                    everyone: m.mention_everyone,
                    mentions: &m.mentions,
                };
                let laid = content::layout_hit(ui, &m.content, width, &mentions);
                let open = Block::single(
                    ui,
                    "Nachricht im Kontext öffnen",
                    TextStyle::new(12.0, 400.0, 1.5),
                    c.muted,
                );
                // Padding 10, the head line, the text after its top margin,
                // the link's line after the text's bottom margin and its own 4.
                let text_top = 10.0 + name.height() + laid.margin_top;
                let open_top = text_top + laid.height + laid.margin_bottom + 4.0;
                let h = open_top + open.height() + 10.0;
                let hit = Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), h));
                if hit.bottom() > body.top() && hit.top() < body.bottom() {
                    let resp = ui.interact(
                        hit.intersect(body),
                        egui::Id::unique(("gelabber-search-hit", m.id)),
                        Sense::click(),
                    );
                    if resp.hovered() {
                        clip.rect_filled(hit, corner(7.0), c.hover);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let x = hit.left() + 12.0;
                    name.paint(&clip, pos2(x, y + 10.0));
                    when.paint(&clip, pos2(x + name.width() + 8.0, y + 10.0));
                    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(body));
                    inner.set_clip_rect(body.intersect(ui.clip_rect()));
                    let _ = laid.paint(
                        &mut inner,
                        pos2(x, y + text_top),
                        egui::Id::unique(("gelabber-search-text", m.id)),
                        false,
                        None,
                    );
                    open.paint_tinted(
                        &clip,
                        pos2(x, y + open_top),
                        resp.hovered().then_some(c.accent_text),
                    );
                    if resp.clicked() {
                        action = Some(SearchAction::Open(m.id));
                    }
                }
                y += h;
            }
            let _ = now;
        }
    }
    action
}
