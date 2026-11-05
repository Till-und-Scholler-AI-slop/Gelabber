//! The history list (MessagePane.tsx `MessageList`, useMessageScroll.ts):
//! rows laid out on demand with their heights cached, pinned to the end
//! while the reader is there, the top row held in place when older pages
//! arrive; day and "Neu" dividers, the history start, the unread bar and
//! the jump pill.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use gelabber_client::{ChannelMessages, Message, ReadState, State};
use uuid::Uuid;

use super::reactions;
use super::row::{self, RowAction, RowCx, SendState};
use super::time;
use crate::icons::{self, Icon};
use crate::images::Images;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::{Button, ButtonVariant};

/// "At the end" within this distance (`useMessageScroll.ts`).
const LATEST_SLACK: f32 = 16.0;
const LOADING_ROW: f32 = 32.0;
const DAY_DIVIDER: f32 = 38.0;
const NEW_DIVIDER: f32 = 24.5;

/// Where the conversation's history begins.
#[derive(Clone, Debug)]
pub enum Start {
    Channel(String),
    Dm {
        name: String,
        avatar: Option<String>,
    },
}

impl Start {
    fn height(&self) -> f32 {
        match self {
            Start::Channel(_) => 24.0 + 25.0 + 4.0 + 17.55 + 8.0,
            Start::Dm { .. } => 24.0 + 64.0 + 8.0 + 4.0 + 25.0 + 4.0 + 17.55 + 8.0,
        }
    }
}

/// The read position when the conversation opened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReadMark {
    pub id: Uuid,
    pub order: Option<i64>,
}

#[derive(Default)]
pub struct PaneState {
    /// Scroll position from the top of the scroll content.
    offset: f32,
    stick: bool,
    initialised: bool,
    /// The first row in view and its distance below the viewport top.
    anchor: Option<(Uuid, f32)>,
    heights: HashMap<Uuid, (u64, f32)>,
    /// `undefined` (None) until the read state is known.
    pub new_after: Option<Option<ReadMark>>,
    pub unread_count: Option<i64>,
    pub opened_at_unread: bool,
    opened: bool,
    away_from: Option<Uuid>,
    pub highlight: Option<(Uuid, f64)>,
    pub editing: Option<(Uuid, String)>,
    pub menu: Option<(Uuid, Rect)>,
    pub picker: Option<Uuid>,
    pub confirm_delete: Option<Uuid>,
    pub revealed: HashMap<Uuid, HashSet<usize>>,
    pub copied: Option<(Uuid, usize, bool, f64)>,
    /// My optimistic reaction votes: (message, emoji) → (add, since).
    pub intents: HashMap<(Uuid, String), (bool, f64)>,
    hover_bar: Option<(Uuid, Rect)>,
    pub last_marked: Option<Uuid>,
    /// A scroll to the end was asked for (sent, jump pill).
    pub to_latest: bool,
    /// A loaded row to scroll to and highlight.
    pub focus: Option<Uuid>,
    scrolled_at: f64,
    pub at_latest: bool,
    /// The editor takes the keyboard on its next frame.
    pub edit_autofocus: bool,
    /// A picture shown large: URL and filename.
    pub lightbox: Option<(String, String)>,
}

impl PaneState {
    pub fn start_edit(&mut self, id: Uuid, draft: String) {
        self.editing = Some((id, draft));
        self.edit_autofocus = true;
        self.menu = None;
    }
}

pub struct ListInput<'a> {
    pub channel: Uuid,
    pub state: &'a State,
    pub me: Uuid,
    pub names: &'a HashMap<String, String>,
    pub can_send: bool,
    pub can_moderate: bool,
    pub start: Start,
    pub now: DateTime<Utc>,
    pub images: &'a Images,
    pub http: &'a gelabber_client::Http,
}

/// What the list asks for once the state lock is released.
pub enum ListAction {
    Row(RowAction),
    LoadOlder,
    MarkRead,
    Reload,
    MarkReadNow,
}

/// A row of the list: a loaded message or my own pending send.
struct Item<'a> {
    msg: std::borrow::Cow<'a, Message>,
    send: Option<SendState>,
    day: Option<String>,
    new_divider: bool,
    start: bool,
    continued: bool,
}

/// `isContinued`: same author within 5 minutes, no reply or system line.
pub fn is_continued(previous: Option<&Message>, message: &Message) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if message.reply.is_some() || message.system.is_some() || previous.system.is_some() {
        return false;
    }
    if previous.author.id != message.author.id {
        return false;
    }
    let delta = message.created_at - previous.created_at;
    delta >= chrono::Duration::zero() && delta < chrono::Duration::minutes(5)
}

/// `unreadAnchor`: the index of the first unread row, if one is loaded.
pub fn unread_anchor(items: &[&Message], read: Option<ReadMark>) -> Option<usize> {
    let read = read?;
    let index = match items.iter().position(|m| m.id == read.id) {
        Some(i) => i + 1,
        None => {
            let order = read.order?;
            items.iter().position(|m| m.created_order > order)?
        }
    };
    (index < items.len()).then_some(index)
}

fn pending_message(p: &gelabber_client::PendingMessage, state: &State) -> Message {
    let me = state.me();
    Message {
        id: p.nonce,
        channel_id: p.channel_id,
        author: gelabber_client::Author {
            id: me.map(|u| u.id).unwrap_or_default(),
            name: me.map(|u| u.name.clone()).unwrap_or_default(),
            avatar_url: me.and_then(|u| u.avatar_url.clone()),
        },
        content: p.content.clone(),
        created_at: p.created_at,
        edited_at: None,
        revision: 0,
        created_order: i64::MAX,
        attachments: p
            .files
            .iter()
            .map(|f| gelabber_client::Attachment {
                id: Uuid::nil(),
                filename: f.filename.clone(),
                content_type: f.content_type.clone(),
                size: f.data.len() as i64,
            })
            .collect(),
        reactions: Vec::new(),
        gif: None,
        mentions: Vec::new(),
        mention_everyone: false,
        reply: None,
        previews: Vec::new(),
        system: None,
        nonce: Some(p.nonce),
    }
}

/// The per-row UI state of one frame.
struct Snap {
    intents: HashMap<(Uuid, String), (bool, f64)>,
    revealed: HashMap<Uuid, HashSet<usize>>,
    editing: Option<(Uuid, String)>,
    copied: Option<(Uuid, usize, bool, f64)>,
    highlight: Option<(Uuid, f64)>,
    empty: HashSet<usize>,
    now_t: f64,
}

fn make_cx<'a>(item: &'a Item<'a>, input: &'a ListInput<'a>, snap: &'a Snap) -> RowCx<'a> {
    let m: &Message = &item.msg;
    let mine = m.author.id == input.me;
    let intent = snap
        .intents
        .iter()
        .find(|((id, _), _)| *id == m.id)
        .map(|((_, e), (add, _))| (e.as_str(), *add));
    RowCx {
        msg: m,
        continued: item.continued,
        mine,
        mentioned: !mine && (m.mention_everyone || m.mentions.contains(&input.me)),
        send: item.send.clone(),
        can_send: input.can_send,
        can_delete: mine || input.can_moderate,
        me: input.me,
        names: input.names,
        now: input.now,
        revealed: snap.revealed.get(&m.id).unwrap_or(&snap.empty),
        copied: snap
            .copied
            .filter(|(id, _, _, at)| *id == m.id && snap.now_t - at < 1.5)
            .map(|(_, i, ok, _)| (i, ok)),
        images: input.images,
        reactions: reactions::displayed(&m.reactions, input.me, intent),
        edit: snap
            .editing
            .as_ref()
            .filter(|(id, _)| *id == m.id)
            .map(|(_, d)| d.as_str()),
        highlight: snap
            .highlight
            .filter(|(id, _)| *id == m.id)
            .map(|(_, at)| {
                let age = (snap.now_t - at) as f32;
                if age < 2.0 {
                    1.0
                } else {
                    (1.0 - (age - 2.0) / 0.6).max(0.0)
                }
            })
            .unwrap_or(0.0),
        gif_url: m.gif.as_ref().map(|g| {
            input
                .http
                .gif_media_url(g.still.as_deref().unwrap_or(&g.url))
        }),
    }
}

fn row_key(cx: &RowCx, item: &Item, width: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    width.to_bits().hash(&mut h);
    item.continued.hash(&mut h);
    item.day.hash(&mut h);
    item.new_divider.hash(&mut h);
    item.start.hash(&mut h);
    cx.msg.revision.hash(&mut h);
    cx.msg.edited_at.hash(&mut h);
    cx.msg.content.hash(&mut h);
    for r in &cx.reactions {
        r.emoji.hash(&mut h);
        r.users.len().hash(&mut h);
    }
    cx.msg.attachments.len().hash(&mut h);
    cx.msg.previews.len().hash(&mut h);
    format!("{:?}", cx.send).hash(&mut h);
    cx.edit.map(|e| e.split('\n').count()).hash(&mut h);
    cx.revealed.len().hash(&mut h);
    h.finish()
}

fn pre_height(item: &Item, start: &Start) -> f32 {
    let mut h = 0.0;
    if item.start {
        h += start.height();
    }
    if item.day.is_some() {
        h += DAY_DIVIDER;
    }
    if item.new_divider {
        h += NEW_DIVIDER;
    }
    h
}

/// Draws the list into `rect`; `overlay` hides it (search shown).
pub fn show(ui: &mut Ui, rect: Rect, input: &ListInput, pane: &mut PaneState) -> Vec<ListAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let now_t = ui.input(|i| i.time);
    let mut actions = Vec::new();
    let history: Option<&ChannelMessages> = input.state.messages(input.channel);
    let loaded = history.is_some_and(|h| h.loaded);
    let has_more = history.is_some_and(|h| h.has_more);
    let loading_older = history.is_some_and(|h| h.loading_older);
    let error = history.and_then(|h| h.error.as_ref());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, c.bg);

    // The read mark the conversation opened with (`newAfter`).
    let read: Option<&ReadState> = input.state.read.get(&input.channel);
    if pane.new_after.is_none() && (input.state.bootstrapped || read.is_some()) {
        let mark = read.filter(|r| r.unread_count > 0).and_then(|r| {
            r.read_message_id.map(|id| ReadMark {
                id,
                order: r.read_order,
            })
        });
        pane.new_after = Some(mark);
        pane.unread_count = mark.and(read.map(|r| r.unread_count));
    }
    if pane.unread_count.is_some() && read.is_some_and(|r| r.unread_count == 0) {
        pane.unread_count = None;
    }

    if error.is_some() && !loaded {
        paint_state(
            ui,
            rect,
            "Nachrichten konnten nicht geladen werden.",
            &mut actions,
        );
        return actions;
    }
    if !loaded {
        paint_skeleton(ui, rect);
        return actions;
    }

    // Items: loaded rows, then my pending sends.
    let history = history.expect("loaded");
    let pending: Vec<Message> = history
        .pending
        .iter()
        .map(|p| pending_message(p, input.state))
        .collect();
    let mut all: Vec<(&Message, Option<SendState>)> =
        history.messages.iter().map(|m| (m, None)).collect();
    for (p, m) in history.pending.iter().zip(pending.iter()) {
        let state = match &p.failed {
            Some(e) => SendState::Failed {
                error: e.german().to_owned(),
            },
            None => SendState::Sending {
                visible: (input.now - p.created_at).num_milliseconds() >= 400,
            },
        };
        if (input.now - p.created_at).num_milliseconds() < 400 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        all.push((m, Some(state)));
    }
    let refs: Vec<&Message> = all.iter().map(|(m, _)| *m).collect();
    let new_index = unread_anchor(&refs, pane.new_after.flatten());
    let mut items: Vec<Item> = Vec::with_capacity(all.len());
    for (i, (m, send)) in all.iter().enumerate() {
        let previous = i.checked_sub(1).map(|p| refs[p]);
        let new_day = time::day_break(previous.map(|p| p.created_at), m.created_at);
        let first_new = new_index == Some(i);
        items.push(Item {
            msg: std::borrow::Cow::Borrowed(*m),
            send: send.clone(),
            day: new_day.then(|| time::day_label(m.created_at, input.now)),
            new_divider: first_new,
            start: i == 0 && !has_more,
            continued: !new_day && !first_new && is_continued(previous, m),
        });
    }

    let width = rect.width();
    // Heights: cached per row, recomputed when the row changed.
    let mut heights = Vec::with_capacity(items.len());
    let mut raw = Vec::with_capacity(items.len());
    let mut layouts: HashMap<usize, row::RowLayout> = HashMap::new();
    // A snapshot of the per-row UI state, so the rows borrow it, not `pane`.
    let snap = Snap {
        intents: pane.intents.clone(),
        revealed: pane.revealed.clone(),
        editing: pane.editing.clone(),
        copied: pane.copied,
        highlight: pane.highlight,
        empty: HashSet::new(),
        now_t,
    };
    let make_cx = |item| make_cx(item, input, &snap);
    for (i, item) in items.iter().enumerate() {
        let cx = make_cx(item);
        let key = row_key(&cx, item, width);
        let h = match pane.heights.get(&item.msg.id) {
            Some((k, h)) if *k == key => *h,
            _ => {
                let layout = row::layout(ui, &cx, width);
                let h = layout.height;
                pane.heights.insert(item.msg.id, (key, h));
                layouts.insert(i, layout);
                h
            }
        };
        // The virtualizer rounds every measured item (`Math.round`), so
        // items start on whole pixels.
        raw.push(h);
        heights.push((pre_height(item, &input.start) + h).round());
    }
    let content_h: f32 = heights.iter().sum();
    let top_row = if !items.is_empty() || has_more || loading_older {
        LOADING_ROW
    } else {
        0.0
    };
    let viewport = rect.height();
    let scroll_h = (top_row + content_h).max(viewport);
    let content_top = scroll_h - content_h;
    let max_offset = scroll_h - viewport;
    let starts: Vec<f32> = heights
        .iter()
        .scan(content_top, |y, h| {
            let s = *y;
            *y += h;
            Some(s)
        })
        .collect();

    // Opening: at the end, or at the first unread row when many follow it.
    if !pane.initialised {
        pane.initialised = true;
        pane.stick = true;
        pane.offset = max_offset;
    }
    if !pane.opened && pane.new_after.is_some() {
        pane.opened = true;
        if let Some(index) = new_index {
            let start = starts[index] - content_top;
            if content_h - start > viewport * 2.0 / 3.0 {
                pane.stick = false;
                pane.offset = (starts[index] - (viewport / 3.0).round()).clamp(0.0, max_offset);
                pane.anchor = None;
                pane.opened_at_unread = true;
            }
        }
    }
    if pane.to_latest {
        pane.to_latest = false;
        pane.stick = true;
    }
    if let Some(target) = pane.focus.take()
        && let Some(i) = items.iter().position(|it| it.msg.id == target)
    {
        pane.stick = false;
        pane.anchor = None;
        pane.offset = (starts[i] + heights[i] / 2.0 - viewport / 2.0).clamp(0.0, max_offset);
        pane.highlight = Some((target, now_t));
    }
    // Hold the row that was on top in place (older pages, rows above
    // changing height); at the end, stay at the end.
    if pane.stick {
        pane.offset = max_offset;
    } else if let Some((id, delta)) = pane.anchor
        && let Some(i) = items.iter().position(|it| it.msg.id == id)
    {
        pane.offset = (starts[i] - delta).clamp(0.0, max_offset);
    }
    // Wheel and keys scroll; the reader takes the position over.
    let pointer_in = ui.rect_contains_pointer(rect);
    if pointer_in {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta != 0.0 {
            pane.offset = (pane.offset - delta).clamp(0.0, max_offset);
            pane.stick = pane.offset >= max_offset - 0.5;
            pane.scrolled_at = now_t;
            ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
        }
    }
    let typing_somewhere = ui.memory(|m| m.focused().is_some());
    if !typing_somewhere {
        let page = viewport * 0.9;
        let step = ui.input(|i| {
            if i.key_pressed(egui::Key::PageUp) {
                -page
            } else if i.key_pressed(egui::Key::PageDown) {
                page
            } else {
                0.0
            }
        });
        if step != 0.0 {
            pane.offset = (pane.offset + step).clamp(0.0, max_offset);
            pane.stick = pane.offset >= max_offset - 0.5;
        }
    }
    let at_latest = pane.offset >= max_offset - LATEST_SLACK;
    pane.at_latest = at_latest;

    // Visible rows.
    let view_top = pane.offset;
    let view_bottom = pane.offset + viewport;
    let first_visible = starts
        .iter()
        .zip(&heights)
        .position(|(s, h)| s + h > view_top);
    pane.anchor = first_visible.map(|i| (items[i].msg.id, starts[i] - view_top));
    // Older history when the top rows come into view.
    if let Some(first) = first_visible
        && first <= 4
        && has_more
        && !loading_older
        && !pane.stick
        && error.is_none()
    {
        actions.push(ListAction::LoadOlder);
    }
    if at_latest && !items.is_empty() {
        let newest = refs
            .iter()
            .rev()
            .find(|m| !history.pending.iter().any(|p| p.nonce == m.id))
            .map(|m| m.id);
        if newest.is_some() && newest != pane.last_marked {
            pane.last_marked = newest;
            actions.push(ListAction::MarkRead);
        }
    }
    // Who arrived while the reader was away from the end.
    let newest_id = refs.last().map(|m| m.id);
    if at_latest {
        pane.away_from = None;
    } else if pane.away_from.is_none() {
        pane.away_from = newest_id;
    }
    let arrived = pane
        .away_from
        .and_then(|id| refs.iter().position(|m| m.id == id))
        .map(|at| {
            refs[at + 1..]
                .iter()
                .filter(|m| m.author.id != input.me && m.nonce.is_none_or(|_| true))
                .filter(|m| !history.pending.iter().any(|p| p.nonce == m.id))
                .count()
        })
        .unwrap_or(0);

    let y0 = rect.top() - pane.offset;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    if loading_older {
        let block = Block::single(
            &child,
            "Ältere Nachrichten…",
            TextStyle::new(12.0, 400.0, 16.0 / 12.0),
            c.muted,
        );
        block.paint(
            child.painter(),
            pos2(rect.center().x - block.width() / 2.0, y0 + 8.0),
        );
    }
    if let Some(error) = error.filter(|_| loaded) {
        let _ = error;
        let r = Rect::from_min_size(pos2(rect.left(), y0), vec2(width, LOADING_ROW + 40.0));
        paint_state(
            &mut child,
            r,
            "Ältere Nachrichten konnten nicht geladen werden.",
            &mut actions,
        );
    }
    // Which row the pointer is on; its bar keeps it hovered over the row above.
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let blocked = pointer
        .and_then(|p| ui.ctx().layer_id_at(p))
        .is_some_and(|l| l != ui.layer_id());
    let mut hovered_id = None;
    if let (Some(p), true, false) = (pointer, pointer_in, blocked) {
        if let Some((id, bar)) = pane.hover_bar
            && bar.contains(p)
        {
            hovered_id = Some(id);
        }
        if hovered_id.is_none() {
            for (i, item) in items.iter().enumerate() {
                let row_top = y0 + starts[i] + pre_height(item, &input.start);
                let row_bottom = y0 + starts[i] + heights[i];
                if p.y >= row_top && p.y < row_bottom {
                    hovered_id = Some(item.msg.id);
                    break;
                }
            }
        }
    }
    let menu_id = pane.menu.map(|(id, _)| id);
    let mut new_bar = None;
    for (i, item) in items.iter().enumerate() {
        let top = starts[i];
        let bottom = top + heights[i];
        if bottom < view_top - 20.0 || top > view_bottom + 20.0 {
            continue;
        }
        let mut y = y0 + top;
        if item.start {
            paint_start(&child, &input.start, pos2(rect.left(), y), input.images);
            y += input.start.height();
        }
        if let Some(label) = &item.day {
            paint_day(&child, label, pos2(rect.left(), y), width);
            y += DAY_DIVIDER;
        }
        if item.new_divider {
            paint_new(&child, pos2(rect.left(), y), width);
            y += NEW_DIVIDER;
        }
        let cx = make_cx(item);
        let layout = layouts
            .remove(&i)
            .unwrap_or_else(|| row::layout(&child, &cx, width));
        if (layout.height - raw[i]).abs() > 0.01 {
            // An image arrived or the row changed under the cache key.
            let key = row_key(&cx, item, width);
            pane.heights.insert(item.msg.id, (key, layout.height));
            ui.ctx().request_repaint();
        }
        let row_rect = Rect::from_min_size(pos2(rect.left(), y), vec2(width, layout.height));
        let hovered = hovered_id == Some(item.msg.id);
        let menu_open = menu_id == Some(item.msg.id);
        let row_resp = child.interact(
            row_rect,
            egui::Id::unique(("gelabber-row-bg", item.msg.id)),
            Sense::click(),
        );
        if row_resp.secondary_clicked()
            && let Some(anchor) = row::more_rect(&cx, row_rect)
        {
            actions.push(ListAction::Row(RowAction::Menu(item.msg.id, anchor)));
        }
        let mut draft = pane
            .editing
            .as_mut()
            .filter(|(id, _)| *id == item.msg.id)
            .map(|(_, d)| d.clone());
        let autofocus = draft.is_some() && std::mem::take(&mut pane.edit_autofocus);
        let painted = row::paint(
            &mut child,
            &cx,
            &layout,
            row_rect,
            row::Look { hovered, menu_open },
            draft
                .as_mut()
                .map(|draft| row::Editing { draft, autofocus }),
        );
        if let (Some(d), Some((_, current))) = (draft, pane.editing.as_mut()) {
            *current = d;
        }
        if hovered || menu_open {
            new_bar = row::bar_rect(&cx, row_rect).map(|b| (item.msg.id, b));
        }
        actions.extend(painted.actions.into_iter().map(ListAction::Row));
    }
    pane.hover_bar = new_bar;
    if let Some((id, at)) = pane.highlight
        && now_t - at < 2.7
    {
        let _ = id;
        ui.ctx().request_repaint();
    }

    // A thin thumb while the reader scrolls.
    if max_offset > 0.0 && now_t - pane.scrolled_at < 1.0 {
        let track = rect.shrink2(vec2(0.0, 2.0));
        let thumb_h = (viewport / scroll_h * track.height()).max(24.0);
        let t = pane.offset / max_offset;
        let thumb = Rect::from_min_size(
            pos2(
                rect.right() - 8.0,
                track.top() + t * (track.height() - thumb_h),
            ),
            vec2(6.0, thumb_h),
        );
        ui.painter()
            .rect_filled(thumb, corner(3.0), mix(c.bg, c.text, 0.25));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    if items.is_empty() {
        paint_empty(ui, rect, &input.start, input.images);
    }

    // The unread bar over the top of the list.
    if pane.unread_count.is_some() && pane.opened_at_unread {
        let count = pane.unread_count.unwrap_or(0);
        let first = new_index.map(|i| refs[i]);
        if let Some(action) = paint_unread_bar(ui, rect, count, first, input.now, pane) {
            actions.push(action);
        }
    }
    // The jump pill.
    if !at_latest && !items.is_empty() && paint_jump(ui, rect, arrived) {
        pane.stick = true;
        pane.offset = max_offset;
    }
    actions
}

fn paint_state(ui: &mut Ui, rect: Rect, message: &str, actions: &mut Vec<ListAction>) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::single(ui, message, TextStyle::new(13.0, 400.0, 1.5), c.text);
    let y = rect.top() + 12.0;
    block.paint(ui.painter(), pos2(rect.center().x - block.width() / 2.0, y));
    let label = Block::single(ui, "Erneut laden", TextStyle::new(13.0, 400.0, 1.5), c.text);
    let button = Rect::from_min_size(
        pos2(
            rect.center().x - (label.width() + 26.0) / 2.0,
            y + block.height() + 8.0,
        ),
        vec2(label.width() + 26.0, label.height() + 12.0),
    );
    let response = ui.interact(
        button,
        egui::Id::unique(("gelabber-list-retry", rect.top() as i32)),
        Sense::click(),
    );
    let fill = if response.hovered() { c.hover } else { c.panel };
    ui.painter().rect(
        button,
        corner(7.0),
        fill,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    label.paint(ui.painter(), pos2(button.left() + 13.0, button.top() + 6.0));
    if response.clicked() {
        actions.push(ListAction::Reload);
    }
}

fn paint_skeleton(ui: &Ui, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    const WIDTHS: [f32; 6] = [0.38, 0.86, 0.62, 0.74, 0.48, 0.92];
    let block_h = 6.0 * 32.0 + 5.0 * 16.0;
    let mut y = rect.bottom() - 16.0 - block_h;
    for i in 0..6 {
        let x = rect.left() + 20.0;
        ui.painter()
            .circle_filled(pos2(x + 16.0, y + 16.0), 16.0, c.surface);
        let col = rect.width() - 40.0 - 44.0;
        let bar = |w: f32, top: f32| Rect::from_min_size(pos2(x + 44.0, top), vec2(col * w, 10.0));
        ui.painter()
            .rect_filled(bar(0.22, y + 6.0), corner(5.0), c.surface);
        ui.painter().rect_filled(
            bar(WIDTHS[(i + 1) % 6], y + 24.0 - 8.0 + 2.0),
            corner(5.0),
            c.surface,
        );
        y += 48.0;
    }
}

fn paint_start(ui: &Ui, start: &Start, at: egui::Pos2, images: &Images) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let x = at.x + 20.0;
    let mut y = at.y + 24.0;
    match start {
        Start::Channel(name) => {
            icons::paint(ui.painter(), Icon::Hash, pos2(x, y + 2.5), 20.0, c.muted);
            Block::single(ui, name, text::TITLE, c.text).paint(ui.painter(), pos2(x + 26.0, y));
        }
        Start::Dm { name, avatar } => {
            row::paint_avatar(
                ui,
                images,
                Rect::from_min_size(pos2(x, y), Vec2::splat(64.0)),
                name,
                avatar.as_deref(),
                20.0,
            );
            y += 64.0 + 8.0 + 4.0;
            Block::single(ui, name, text::TITLE, c.text).paint(ui.painter(), pos2(x, y));
        }
    }
    y += 25.0 + 4.0;
    let label = match start {
        Start::Channel(_) => "Anfang des Kanals.",
        Start::Dm { .. } => "Anfang der Unterhaltung.",
    };
    Block::single(ui, label, text::UI.weight(400.0), c.muted).paint(ui.painter(), pos2(x, y));
}

fn paint_day(ui: &Ui, label: &str, at: egui::Pos2, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::single(ui, label, TextStyle::new(12.0, 600.0, 1.5), c.muted);
    let line_y = (at.y + 14.0 + 9.0 - 0.5).floor();
    let text_x = at.x + (width - block.width()) / 2.0;
    let left = at.x + 20.0;
    let right = at.x + width - 20.0;
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(left, line_y), pos2(text_x - 12.0, line_y + 1.0)),
        0.0,
        c.border,
    );
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(text_x + block.width() + 12.0, line_y),
            pos2(right, line_y + 1.0),
        ),
        0.0,
        c.border,
    );
    block.paint(ui.painter(), pos2(text_x, at.y + 14.0));
}

fn paint_new(ui: &Ui, at: egui::Pos2, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let style = TextStyle::new(11.0, 700.0, 1.5).tracking(0.4 / 11.0);
    let block = Block::single(ui, "NEU", style, c.accent_text);
    let right = at.x + width - 20.0;
    let text_x = right - block.width();
    let line_y = (at.y + 6.0 + 8.25 - 0.5).floor();
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(at.x + 20.0, line_y), pos2(text_x - 12.0, line_y + 1.0)),
        0.0,
        Color32::from_rgba_unmultiplied(c.accent.r(), c.accent.g(), c.accent.b(), 153),
    );
    block.paint(ui.painter(), pos2(text_x, at.y + 6.0));
}

fn paint_empty(ui: &Ui, rect: Rect, start: &Start, images: &Images) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let note = Block::single(
        ui,
        "Noch keine Nachrichten. Schreib die erste.",
        TextStyle::new(13.0, 400.0, 1.5),
        c.muted,
    );
    let bottom = rect.bottom() - 16.0;
    let note_top = bottom - note.height();
    note.paint(ui.painter(), pos2(rect.left() + 20.0, note_top));
    paint_start(
        ui,
        start,
        pos2(rect.left(), note_top - start.height()),
        images,
    );
}

fn paint_unread_bar(
    ui: &mut Ui,
    rect: Rect,
    count: i64,
    first: Option<&Message>,
    now: DateTime<Utc>,
    pane: &mut PaneState,
) -> Option<ListAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 37.0));
    ui.painter().rect_filled(bar, 0.0, c.selected);
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(bar.left(), bar.bottom() - 1.0), bar.max),
        0.0,
        mix(c.border, c.accent, 0.35),
    );
    let lead = if count == 1 {
        "1 neue Nachricht".to_owned()
    } else if count > 99 {
        "99+ neue Nachrichten".to_owned()
    } else {
        format!("{count} neue Nachrichten")
    };
    let style = TextStyle::new(13.0, 400.0, 1.35);
    let mut rich = text::Rich::new(style).push(&lead, style.weight(600.0), c.text);
    if let Some(first) = first {
        rich = rich.push(
            &format!(" seit {}", time::format_when(first.created_at, now)),
            style,
            c.text,
        );
    }
    let block = rich.layout(ui);
    block.paint(
        ui.painter(),
        pos2(
            bar.left() + 20.0,
            bar.top() + 2.0 + 16.0 - style.line_px() / 2.0,
        ),
    );
    let mut out = None;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(bar.left() + 20.0, bar.top() + 2.0),
                pos2(bar.right() - 12.0, bar.top() + 34.0),
            ))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 8.0;
    if child
        .add(
            Button::new("Als gelesen markieren  Esc")
                .small()
                .variant(ButtonVariant::Ghost),
        )
        .clicked()
    {
        out = Some(ListAction::MarkReadNow);
    }
    if child.add(Button::new("Springen").small()).clicked() {
        pane.opened = false;
        pane.stick = false;
    }
    out
}

/// The pill; true when clicked.
fn paint_jump(ui: &mut Ui, rect: Rect, arrived: usize) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let label = match arrived {
        0 => "Zu den neuesten Nachrichten".to_owned(),
        1 => "1 neue Nachricht".to_owned(),
        n => format!("{n} neue Nachrichten"),
    };
    let news = arrived > 0;
    let fg = if news { c.accent_ink } else { c.text };
    let block = Block::single(ui, &label, TextStyle::new(13.0, 600.0, 1.5), fg);
    let w = 1.0 + 14.0 + 15.0 + 7.0 + block.width() + 14.0 + 1.0;
    let pill = Rect::from_min_size(
        pos2(
            (rect.center().x - w / 2.0).round(),
            rect.bottom() - 10.0 - 32.0,
        ),
        vec2(w.round(), 32.0),
    );
    let response = ui.interact(
        pill,
        egui::Id::unique("gelabber-jump-latest"),
        Sense::click(),
    );
    let hovered = response.hovered();
    let fill = match (news, hovered) {
        (true, false) => c.accent,
        (true, true) => crate::theme::brightness(c.accent, 1.08),
        (false, true) => c.hover,
        (false, false) => c.surface,
    };
    let shadow = egui::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(61),
    };
    ui.painter().add(shadow.as_shape(pill, corner(16.0)));
    ui.painter().rect(
        pill,
        corner(16.0),
        fill,
        Stroke::new(1.0, if news { Color32::TRANSPARENT } else { c.border }),
        StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Icon::ArrowDown,
        pos2(pill.left() + 15.0, pill.center().y - 7.5),
        15.0,
        fg,
    );
    block.paint(
        ui.painter(),
        pos2(
            pill.left() + 15.0 + 15.0 + 7.0,
            pill.center().y - block.height() / 2.0,
        ),
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(author: u128, minutes: i64) -> Message {
        let at = DateTime::<Utc>::from_timestamp(1_760_000_000 + minutes * 60, 0).unwrap();
        serde_json::from_value(serde_json::json!({
            "id": Uuid::from_u128(minutes as u128 + 100),
            "channel_id": Uuid::nil(),
            "author": {"id": Uuid::from_u128(author), "name": "A", "avatar_url": null},
            "content": "x",
            "created_at": at,
            "edited_at": null,
            "revision": 0,
            "created_order": minutes,
        }))
        .unwrap()
    }

    #[test]
    fn grouping() {
        let a = message(1, 0);
        assert!(is_continued(Some(&a), &message(1, 4)));
        assert!(!is_continued(Some(&a), &message(1, 5)));
        assert!(!is_continued(Some(&a), &message(2, 1)));
        assert!(!is_continued(None, &a));
    }

    #[test]
    fn unread() {
        let rows = [message(1, 0), message(1, 1), message(2, 2)];
        let refs: Vec<&Message> = rows.iter().collect();
        let mark = |id: Uuid, order| Some(ReadMark { id, order });
        assert_eq!(unread_anchor(&refs, mark(rows[0].id, Some(0))), Some(1));
        assert_eq!(unread_anchor(&refs, mark(rows[2].id, Some(2))), None);
        // The read message was deleted: the first row after its order.
        assert_eq!(unread_anchor(&refs, mark(Uuid::nil(), Some(1))), Some(2));
        assert_eq!(unread_anchor(&refs, None), None);
    }
}
