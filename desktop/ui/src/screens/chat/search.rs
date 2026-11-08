//! The header search's hits over the list (messages/MessageSearch.tsx,
//! chat.css `.lr-search-*`): after 250ms of typing, newest first, 25 a
//! page with "Ältere Treffer" for the next; a hit opens in its context.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gelabber_client::{ApiError, Client, MessagePage};
use uuid::Uuid;

use super::content::{self, Mentions};
use super::row::readable_mentions;
use super::time;
use crate::task::{Pending, poll};
use crate::text::{Block, Rich, TextStyle};
use crate::theme::{Theme, corner};

const DEBOUNCE: f64 = 0.25;
/// `limit: "25"` (messages/search.ts).
const PAGE: u32 = 25;

type Request = Option<Pending<Result<MessagePage, ApiError>>>;

#[derive(Default)]
pub struct SearchState {
    pub text: String,
    changed_at: f64,
    query: String,
    request: Request,
    /// The first page's answer, or its error.
    result: Option<Result<(), String>>,
    /// Loaded pages, newest first; each page oldest → newest.
    pages: Vec<MessagePage>,
    /// The next page on its way (`isFetchingNextPage`).
    more: Request,
    /// The next page failed (`isFetchNextPageError`).
    more_failed: bool,
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

    /// `getNextPageParam`: `<created_at>|<id>` of the oldest hit of the
    /// last page, while the server has more.
    pub fn next_cursor(&self) -> Option<String> {
        let last = self.pages.last()?;
        let oldest = last.messages.first()?;
        last.has_more.then(|| oldest.cursor())
    }

    /// The button under the hits: its label and whether it can be pressed,
    /// `None` without a next page.
    pub fn more_button(&self) -> Option<(&'static str, bool)> {
        self.next_cursor()?;
        Some(if self.more.is_some() {
            ("Lädt…", false)
        } else {
            ("Ältere Treffer", true)
        })
    }

    fn fetch(&self, client: &Client, channel: Uuid, before: Option<String>) -> Request {
        let http = client.http().clone();
        let q = self.query.clone();
        Some(Pending::spawn(client, async move {
            http.search_messages(channel, &q, before, Some(PAGE)).await
        }))
    }
}

pub enum SearchAction {
    Close,
    Open(Uuid),
}

/// `.lr-button-secondary`: 13px, padding 5 12, the panel fill; disabled
/// like the shared button, a readable subtle label.
fn secondary(ui: &mut Ui, at: egui::Pos2, label: &str, enabled: bool, id: egui::Id) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let style = TextStyle::new(13.0, 400.0, 1.5);
    let block = Block::single(ui, label, style, c.text);
    let rect = Rect::from_min_size(at, vec2(block.width() + 26.0, block.height() + 12.0));
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if !enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
    }
    let fill = if enabled && response.hovered() {
        c.hover
    } else {
        c.panel
    };
    ui.painter().rect(
        rect,
        corner(theme.radius.md),
        fill,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    block.paint_tinted(
        ui.painter(),
        pos2(rect.left() + 13.0, rect.top() + 6.0),
        (!enabled).then_some(c.subtle),
    );
    enabled && response.clicked()
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
        // Typing on keeps the last hits until the new ones are there.
        state.query = wanted;
        state.more = None;
        state.more_failed = false;
        state.request = state.fetch(client, channel, None);
    }
    if let Some(result) = poll(&mut state.request) {
        match result {
            Ok(page) => {
                state.pages = vec![page];
                state.result = Some(Ok(()));
                state.scroll = 0.0;
            }
            Err(e) => {
                state.pages.clear();
                state.result = Some(Err(e.german().to_owned()));
            }
        }
    }
    if let Some(result) = poll(&mut state.more) {
        match result {
            Ok(page) => {
                state.pages.push(page);
                state.more_failed = false;
            }
            Err(_) => state.more_failed = true,
        }
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
    let line_style = TextStyle::new(14.0, 400.0, 1.5);
    if ui.rect_contains_pointer(body) {
        state.scroll = (state.scroll - ui.input(|i| i.smooth_scroll_delta.y)).max(0.0);
    }
    let clip = ui.painter().with_clip_rect(body.intersect(ui.clip_rect()));
    let mut y = body.top() - state.scroll;
    // `<p>` lines: padding 6 8.
    let state_line = |ui: &Ui, y: &mut f32, text: &str| {
        let block = Block::single(ui, text, line_style, c.muted);
        block.paint(&clip, pos2(body.left() + 8.0, *y + 6.0));
        *y += 6.0 + block.height() + 6.0;
    };
    let failed = matches!(state.result, Some(Err(_))) || state.more_failed;
    if state.result.is_none() {
        state_line(ui, &mut y, "Suche läuft…");
    }
    if failed {
        state_line(ui, &mut y, "Die Suche konnte nicht geladen werden.");
        let retry = secondary(
            ui,
            pos2(body.left() + 8.0, y - 6.0),
            "Erneut laden",
            true,
            egui::Id::unique(("gelabber-search-retry", channel)),
        );
        y += 32.0;
        if retry {
            if state.more_failed {
                state.more_failed = false;
                state.more = state.fetch(client, channel, state.next_cursor());
            } else {
                state.request = state.fetch(client, channel, None);
            }
        }
    }
    let hits: Vec<&gelabber_client::Message> = state
        .pages
        .iter()
        .flat_map(|page| page.messages.iter().rev())
        .collect();
    if matches!(state.result, Some(Ok(()))) && hits.is_empty() {
        state_line(ui, &mut y, "Keine Nachrichten gefunden.");
    }
    let width = body.width() - 24.0;
    for m in hits {
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
            let text = readable_mentions(&m.content, names);
            let label = format!(
                "Zur Nachricht von {}: {}",
                m.author.name,
                if text.is_empty() { "Datei" } else { &text }
            );
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
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
    if let Some((label, enabled)) = state.more_button() {
        let at = pos2(body.left(), y + 8.0);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        let id = egui::Id::unique(("gelabber-search-more", channel));
        if secondary(&mut child, at, label, enabled, id) {
            state.more = state.fetch(client, channel, state.next_cursor());
        }
    }
    let _ = now;
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(ids: &[u128], has_more: bool) -> MessagePage {
        let messages: Vec<serde_json::Value> = ids
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": Uuid::from_u128(*n),
                    "channel_id": Uuid::nil(),
                    "author": {"id": Uuid::nil(), "name": "A", "avatar_url": null},
                    "content": "Snacks",
                    "created_at": format!("2026-11-05T16:{:02}:00Z", *n),
                    "edited_at": null,
                    "revision": 0,
                    "created_order": *n,
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({"messages": messages, "has_more": has_more}))
            .unwrap()
    }

    #[test]
    fn older_hits_page_from_the_oldest_hit() {
        let mut s = SearchState::default();
        assert_eq!(s.more_button(), None);
        s.pages = vec![page(&[30, 31, 32], true)];
        assert_eq!(
            s.next_cursor().as_deref(),
            Some("2026-11-05T16:30:00Z|00000000-0000-0000-0000-00000000001e")
        );
        assert_eq!(s.more_button(), Some(("Ältere Treffer", true)));
        // The next page from its own oldest; the last one ends the paging.
        s.pages.push(page(&[10, 11], true));
        assert!(
            s.next_cursor()
                .unwrap()
                .starts_with("2026-11-05T16:10:00Z|")
        );
        s.pages.push(page(&[5], false));
        assert_eq!(s.next_cursor(), None);
        assert_eq!(s.more_button(), None);
    }

    #[test]
    fn a_running_page_shows_loading() {
        let client = Client::new("http://127.0.0.1:9").expect("client");
        let mut s = SearchState {
            query: "Snacks".into(),
            pages: vec![page(&[30], true)],
            ..SearchState::default()
        };
        s.more = s.fetch(&client, Uuid::nil(), s.next_cursor());
        assert_eq!(s.more_button(), Some(("Lädt…", false)));
    }
}
