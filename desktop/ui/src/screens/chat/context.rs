//! A message in its context (messages/MessageContext.tsx): opened from a
//! search hit, or from a reply whose original is not loaded. The bar with
//! "Zurück zu den Treffern" (from a search) and "Zurück zum Chat", then the
//! messages around it with the found one framed and in the middle.

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gelabber_client::{ApiError, Client, MessageContext};
use gelabber_i18n::chat as texts;
use uuid::Uuid;

use super::pane::{self, ContextScroll, ListInput};
use super::row::RowAction;
use crate::task::{Pending, poll};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner};

/// The bar's buttons and the state lines: the page's 16px text.
const TEXT: TextStyle = TextStyle::new(16.0, 400.0, 1.5);

pub struct ContextState {
    pub target: Uuid,
    /// The search query whose hit opened it; `None` for a reply jump.
    pub query: Option<String>,
    request: Option<Pending<Result<MessageContext, ApiError>>>,
    result: Option<Result<MessageContext, ApiError>>,
    scroll: ContextScroll,
}

impl ContextState {
    pub fn new(target: Uuid, query: Option<String>) -> Self {
        Self {
            target,
            query,
            request: None,
            result: None,
            scroll: ContextScroll::default(),
        }
    }

    fn load(&mut self, client: &Client, channel: Uuid) {
        let http = client.http().clone();
        let target = self.target;
        self.request = Some(Pending::spawn(client, async move {
            http.message_context(channel, target).await
        }));
    }
}

/// What the bar asks for.
pub enum ContextAction {
    /// "Zurück zu den Treffern".
    Hits,
    /// "Zurück zum Chat".
    Chat,
    Row(RowAction),
}

/// What the view shows for its request (`query.isPending`, `query.error`).
#[derive(Debug, PartialEq)]
pub enum Shown<'a> {
    Loading,
    /// The message is gone or hidden (`not_found`, `forbidden`).
    Missing,
    Failed,
    Messages(&'a MessageContext),
}

impl ContextState {
    pub fn shown(&self) -> Shown<'_> {
        match &self.result {
            None => Shown::Loading,
            Some(Err(e)) if e.is_gone() => Shown::Missing,
            Some(Err(_)) => Shown::Failed,
            Some(Ok(context)) => Shown::Messages(context),
        }
    }
}

/// A `rounded border px-3 py-2` button of the page's text colour.
fn bordered(ui: &mut Ui, at: egui::Pos2, label: &str, id: egui::Id, enabled: bool) -> (Rect, bool) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::single(ui, label, TEXT, c.text);
    let rect = Rect::from_min_size(at, vec2(block.width() + 24.0 + 2.0, 42.0));
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if response.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let alpha = if enabled { 1.0 } else { 0.5 };
    ui.painter().rect_stroke(
        rect,
        corner(4.0),
        Stroke::new(1.0, c.text.gamma_multiply(alpha)),
        StrokeKind::Inside,
    );
    block.paint_tinted(
        ui.painter(),
        pos2(rect.left() + 13.0, rect.top() + 9.0),
        (!enabled).then(|| c.text.gamma_multiply(alpha)),
    );
    (rect, response.clicked())
}

/// Draws the context into `rect` (the list's place).
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    client: &Client,
    channel: Uuid,
    state: &mut ContextState,
    input: &ListInput,
    revealed: &std::collections::HashMap<Uuid, std::collections::HashSet<usize>>,
) -> Vec<ContextAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    if state.result.is_none() && state.request.is_none() {
        state.load(client, channel);
    }
    if let Some(result) = poll(&mut state.request) {
        state.result = Some(result);
    }
    ui.painter().rect_filled(rect, 0.0, c.bg);
    let mut actions = Vec::new();
    // The bar: p-3, the buttons 8 apart, a border in the text colour under it.
    let mut x = rect.left() + 12.0;
    if state.query.is_some() {
        let (r, clicked) = bordered(
            ui,
            pos2(x, rect.top() + 12.0),
            texts::context_back_to_results(),
            egui::Id::unique(("gelabber-context-hits", channel)),
            true,
        );
        if clicked {
            actions.push(ContextAction::Hits);
        }
        x = r.right() + 8.0;
    }
    let (_, clicked) = bordered(
        ui,
        pos2(x, rect.top() + 12.0),
        texts::pane_back_to_chat(),
        egui::Id::unique(("gelabber-context-chat", channel)),
        true,
    );
    if clicked {
        actions.push(ContextAction::Chat);
    }
    let bar_bottom = rect.top() + 12.0 + 42.0 + 12.0;
    ui.painter().rect_filled(
        Rect::from_min_size(pos2(rect.left(), bar_bottom), vec2(rect.width(), 1.0)),
        0.0,
        c.text,
    );
    let body = Rect::from_min_max(pos2(rect.left(), bar_bottom + 1.0), rect.max);
    let line = |ui: &Ui, text: &str| {
        let block = Block::wrapped(ui, text, TEXT, c.text, body.width() - 32.0);
        block.paint(ui.painter(), pos2(body.left() + 16.0, body.top() + 12.0));
        block.height()
    };
    let mut retry = false;
    match state.shown() {
        Shown::Loading => {
            line(ui, texts::context_loading());
        }
        Shown::Missing | Shown::Failed => {
            let text = if state.shown() == Shown::Missing {
                texts::context_missing()
            } else {
                texts::context_failed()
            };
            let h = line(ui, text);
            let (_, clicked) = bordered(
                ui,
                pos2(body.left() + 16.0, body.top() + 12.0 + h + 8.0),
                texts::list_reload(),
                egui::Id::unique(("gelabber-context-retry", channel)),
                state.request.is_none(),
            );
            retry = clicked;
        }
        Shown::Messages(_) => {}
    }
    // As `refetch`: the error stays up, its button off, until the answer.
    if retry {
        state.load(client, channel);
    }
    if let Some(Ok(context)) = &state.result {
        let messages = context.messages.clone();
        let rows = pane::context_rows(
            ui,
            body,
            input,
            &messages,
            state.target,
            &mut state.scroll,
            revealed,
        );
        actions.extend(rows.into_iter().map(ContextAction::Row));
    }
    actions
}
