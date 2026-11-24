//! The server's front door, `/s/{id}` (pages/CommunityOverview.tsx,
//! overview.css, GEL-69): the "Übersicht" bar with "Springen zu …", a
//! greeting, who is live, what was missed per text channel and what was
//! shared. It shows only what the sidebar, the member panel and the voice
//! page do not.

use std::collections::{HashMap, HashSet};

use chrono::{Local, Timelike, Utc};
use egui::text::LayoutJob;
use egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};
use gelabber_call::VideoKey;
use gelabber_client::{
    Attachment, ChannelKind, HistoryQuery, Message, Permission, ServerDetail, SharedFile,
};
use gelabber_i18n::{chat as chat_texts, servers, shell};
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::state::{self, Missed, SharedFiles};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::images::{Fit, ImageState, cover_uv};
use crate::screens::chat::{self, attachments, content, pictures, time};
use crate::screens::voice;
use crate::task::Pending;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::{Button, hairline_h};

/// `.overview__body`: padding 32px 40px.
const PAD_Y: f32 = 32.0;
const PAD_X: f32 = 40.0;
/// The "Geteilt" column and the gap between the columns and the blocks.
const SIDE: f32 = 320.0;
const GAP: f32 = 28.0;
/// From this page width on, "Geteilt" has its own column (`@container
/// (min-width: 1000px)`); narrower, it follows "Verpasst".
const TWO_COLUMNS: f32 = 1000.0;
/// From this page width on, two Lives side by side; narrower, as many of
/// at least 240 as fit.
const TWO_STREAMS: f32 = 560.0;
/// "Verpasst": at most this many cards.
const MAX_CARDS: usize = 4;
/// The messages fetched per card: the most one shows.
const FETCHED: u32 = 5;
/// "Geteilt": asked for, and shown.
const SHARED_LIMIT: u32 = 12;
const SHARED_SHOWN: usize = 4;
/// The dark chips over pictures: rgba(14, 14, 16, .84).
const CHIP: Color32 = Color32::from_rgba_premultiplied(12, 12, 13, 214);
/// Text on those chips and on the streams' black.
const ON_DARK: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf2);
const H2: TextStyle = TextStyle::new(16.0, 600.0, 1.2);
const NAME: TextStyle = TextStyle::new(14.0, 600.0, 1.3);
const SMALL: TextStyle = TextStyle::new(13.0, 500.0, 1.3);

// ------------------------------------------------------------ pure logic

/// The part of the day the greeting names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Daytime {
    /// 05:00–10:59.
    Morning,
    /// 11:00–17:59.
    Day,
    /// 18:00–04:59.
    Evening,
}

pub fn daytime(hour: u32) -> Daytime {
    match hour {
        5..=10 => Daytime::Morning,
        11..=17 => Daytime::Day,
        _ => Daytime::Evening,
    }
}

/// "Guten Morgen, {name}" / "Guten Tag, {name}" / "Guten Abend, {name}".
pub fn greeting(name: &str, hour: u32) -> String {
    match daytime(hour) {
        Daytime::Morning => servers::overview_greeting_morning(name),
        Daytime::Day => servers::overview_greeting_day(name),
        Daytime::Evening => servers::overview_greeting_evening(name),
    }
}

/// An unread text channel, as its "Verpasst" card needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Unread {
    pub channel: Uuid,
    pub name: String,
    /// Its place in the sidebar, for equal ones.
    pub place: usize,
    pub unread: i64,
    pub mentions: i64,
    pub latest: Option<i64>,
    pub read: Option<i64>,
}

/// The cards: channels with a mention first, then the newest message
/// first, at most [`MAX_CARDS`]; and how many channels were left out.
pub fn cards(mut list: Vec<Unread>) -> (Vec<Unread>, usize) {
    list.sort_by(|a, b| {
        (b.mentions > 0)
            .cmp(&(a.mentions > 0))
            .then(
                b.latest
                    .unwrap_or(i64::MIN)
                    .cmp(&a.latest.unwrap_or(i64::MIN)),
            )
            .then(a.place.cmp(&b.place))
    });
    let more = list.len().saturating_sub(MAX_CARDS);
    list.truncate(MAX_CARDS);
    (list, more)
}

/// How many messages a card shows and whether it is dimmed: 5 when only
/// one channel is unread; else a busy channel (10 or more unread, no
/// mention) shows 1, dimmed, and the others 2.
pub fn card_look(unread: i64, mentions: i64, single: bool) -> (usize, bool) {
    if single {
        (5, false)
    } else if unread >= 10 && mentions == 0 {
        (1, true)
    } else {
        (2, false)
    }
}

/// The newest of `page` (oldest first) after the read position, at most
/// `count`, oldest first.
pub fn unread_tail(page: &[Message], read: Option<i64>, count: usize) -> Vec<Message> {
    let unread: Vec<&Message> = page
        .iter()
        .filter(|m| read.is_none_or(|r| m.created_order > r))
        .collect();
    unread[unread.len().saturating_sub(count)..]
        .iter()
        .map(|m| (*m).clone())
        .collect()
}

/// "Geteilt": the files of messages no card shows, at most
/// [`SHARED_SHOWN`], newest first as the server sends them.
pub fn shared_shown(files: &[SharedFile], in_cards: &HashSet<Uuid>) -> Vec<SharedFile> {
    files
        .iter()
        .filter(|f| !in_cards.contains(&f.message_id))
        .take(SHARED_SHOWN)
        .cloned()
        .collect()
}

/// Whether a message reaches me: named or `@everyone` (`mentionsMe`).
pub fn mentions_me(m: &Message, me: Option<Uuid>) -> bool {
    m.mention_everyone || me.is_some_and(|me| m.mentions.contains(&me))
}

/// A picture the overview draws as one (`isImage`).
fn is_picture(att: &Attachment) -> bool {
    attachments::is_image(&att.content_type)
}

// ------------------------------------------------------------------ page

pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, server: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    let detail = app.client.with_state(|s| s.detail(server).cloned());
    // `ServerPage`: without the server, its error in place of the page.
    if detail.is_none() && load_error(ui, app, rect, server) {
        return;
    }
    bar(ui, &theme, rect);
    let Some(detail) = detail else {
        skeleton(ui, rect);
        return;
    };

    let streams = streams(app, &detail);
    let unread = unread_channels(app, &detail);
    let single = unread.len() == 1;
    let (cards, more_channels) = cards(unread);
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 52.0), rect.max);
    let mut child = ui.new_child(UiBuilder::new().max_rect(body));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt(("overview-scroll", server))
        .max_height(body.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let origin = ui.max_rect().min;
            let width = (body.width() - 2.0 * PAD_X).max(0.0);
            let left = origin.x + PAD_X;
            let top = origin.y + PAD_Y;
            // The cards come first: "Geteilt" leaves out what they show.
            let mut in_cards = HashSet::new();
            let fetched: Vec<Option<Vec<Message>>> = cards
                .iter()
                .map(|card| {
                    let (count, _) = card_look(card.unread, card.mentions, single);
                    let page = missed_messages(ui, app, card.channel, card.latest)?;
                    let shown = unread_tail(&page, card.read, count);
                    in_cards.extend(shown.iter().map(|m| m.id));
                    Some(shown)
                })
                .collect();
            let shared = shared_shown(&shared_files(ui, app, &detail), &in_cards);
            let two = body.width() >= TWO_COLUMNS;
            let left_w = if two { width - SIDE - GAP } else { width };
            let stream_cols = if body.width() >= TWO_STREAMS {
                2
            } else {
                (((left_w + 20.0) / 260.0).floor() as usize).max(1)
            };

            let mut y = greet(ui, app, &theme, left, top, left_w);
            if !streams.is_empty() {
                y = streams_block(
                    ui,
                    app,
                    &theme,
                    server,
                    (left, y + GAP, left_w),
                    stream_cols,
                    &streams,
                );
            }
            y = missed(
                ui,
                app,
                &theme,
                &detail,
                (left, y + GAP, left_w),
                &cards,
                &fetched,
                more_channels,
                single,
            );
            if two && !shared.is_empty() {
                let side = shared_column(
                    ui,
                    app,
                    &theme,
                    &detail,
                    left + left_w + GAP,
                    top,
                    SIDE,
                    &shared,
                );
                y = y.max(side);
            } else if !shared.is_empty() {
                y = shared_column(ui, app, &theme, &detail, left, y + GAP, width, &shared);
            }
            ui.allocate_exact_size(
                Vec2::new(body.width(), y + PAD_Y - origin.y),
                egui::Sense::hover(),
            );
        });

    // A picture of a card shown large, as the chat shows it.
    let mut view = state::with(ui.ctx(), |s| s.lightbox.take());
    if let Some(lightbox) = view.as_mut() {
        let (close, download) = attachments::lightbox(ui.ctx(), app.images, lightbox);
        if let Some(shown) = lightbox.current().filter(|_| download) {
            let url = format!("{}?download=1", shown.url);
            attachments::download(ui.ctx(), app.client, &url, &shown.name);
        }
        if close {
            view = None;
        }
    }
    state::with(ui.ctx(), |s| {
        if s.lightbox.is_none() {
            s.lightbox = view;
        }
    });

    // A server just created asks for its first invite once its page is up.
    let prompt = state::with(ui.ctx(), |s| {
        s.invite_prompt.take_if(|(id, _)| *id == server)
    });
    if let Some((id, name)) = prompt {
        super::dialogs::open_invite(ui.ctx(), id, &name);
    }
}

/// The bar: the chat icon and "Übersicht", then on the right the search
/// field look-alike with the quick switcher's shortcut, which opens it.
fn bar(ui: &mut Ui, theme: &Theme, rect: Rect) {
    let c = theme.c;
    let painter = ui.painter().clone();
    // `.community-overview__bar`: 52 from the left, after the channel
    // list's button.
    icons::paint(
        &painter,
        Icon::Chat,
        pos2(rect.left() + 52.0, rect.top() + 16.5),
        18.0,
        c.muted,
    );
    let title = Block::single(
        ui,
        servers::overview_label(),
        text::TITLE_SM.size(15.0),
        c.text,
    );
    let title_left = rect.left() + 52.0 + 18.0 + 10.0;
    paint_mid(&painter, &title, title_left, rect.top() + 25.5);
    hairline_h(
        &painter,
        theme,
        rect.left()..=rect.right(),
        rect.top() + 51.0,
    );

    // The member list's toggle at the end, as in a channel's header.
    let toggle = Rect::from_min_size(
        pos2(rect.right() - 16.0 - 32.0, rect.top() + 25.5 - 16.0),
        Vec2::splat(32.0),
    );
    chat::members_toggle(ui, toggle, Id::unique("overview-members"));
    // `.overview-jump`: 248 × 32, the panel's fill, a hairline.
    let right = toggle.left() - 10.0;
    let w = (right - (title_left + title.width() + 16.0)).min(248.0);
    if w < 140.0 {
        return;
    }
    let field = Rect::from_min_size(pos2(right - w, rect.top() + 25.5 - 16.0), vec2(w, 32.0));
    let response = common::click(ui, field, Id::unique("overview-search"));
    let label = servers::overview_search();
    crate::widgets::announce(&response, egui::Role::Button, label);
    painter.rect(
        field,
        corner(theme.radius.md),
        c.panel,
        Stroke::new(
            1.0,
            if response.hovered() {
                c.border_strong
            } else {
                c.border
            },
        ),
        StrokeKind::Inside,
    );
    let cy = field.center().y;
    icons::paint(
        &painter,
        Icon::Search,
        pos2(field.left() + 10.0, cy - 7.5),
        15.0,
        c.muted,
    );
    let keys = format!("{} K", shell::keys_ctrl());
    let kbd = kbd(ui, &painter, &keys, field.right() - 6.0, cy, theme);
    let words = line(
        ui,
        label,
        TextStyle::new(13.0, 400.0, 1.3),
        if response.hovered() { c.text } else { c.muted },
        kbd.left() - 8.0 - (field.left() + 33.0),
    );
    paint_mid(&painter, &words, field.left() + 33.0, cy);
    common::focus_visible(ui, &response, field, theme.radius.md);
    if response.clicked() {
        super::overlays::open_switcher(ui.ctx());
    }
}

/// A `<kbd>`: 500 11px, padding 3 5, a strong hairline, radius 5, its
/// right edge at `right`.
fn kbd(ui: &Ui, painter: &egui::Painter, keys: &str, right: f32, cy: f32, theme: &Theme) -> Rect {
    let c = theme.c;
    let block = Block::single(ui, keys, TextStyle::new(11.0, 500.0, 1.0), c.muted);
    let r = Rect::from_min_max(
        pos2(right - block.width() - 12.0, cy - 9.5),
        pos2(right, cy + 9.5),
    );
    painter.rect_stroke(
        r,
        corner(5.0),
        Stroke::new(1.0, c.border_strong),
        StrokeKind::Inside,
    );
    paint_mid(painter, &block, r.left() + 6.0, cy);
    r
}

/// "Guten Abend, Rafi": 700 28px/1.15, -0.02em; returns its bottom.
fn greet(ui: &Ui, app: &AppCtx, theme: &Theme, left: f32, top: f32, width: f32) -> f32 {
    let name = app
        .client
        .with_state(|s| s.me().map(|m| m.name.clone()))
        .unwrap_or_default();
    let words = greeting(&name, Local::now().hour());
    let block = Block::wrapped(ui, &words, text::PAGE.tracking(-0.02), theme.c.text, width);
    block.paint(ui.painter(), pos2(left, top));
    let r = Rect::from_min_size(pos2(left, top), block.size());
    let response = ui.interact(r, Id::unique("overview-greeting"), egui::Sense::hover());
    crate::widgets::announce(&response, egui::Role::Label, &words);
    top + block.height()
}

// --------------------------------------------------------------- streams

/// Someone live with a screen in one of the server's voice channels.
struct Stream {
    owner: Uuid,
    channel: Uuid,
    /// The member's name (initials, the picture's name).
    name: String,
    avatar: Option<String>,
    own: bool,
    /// This client sits in that call.
    here: bool,
    /// The user may join voice channels (`join_voice`).
    allowed: bool,
    viewers: usize,
    key: Option<VideoKey>,
}

/// Who is live in the server's voice channels (the roster's live map, the
/// own Live from the call), in the sidebar's order.
fn streams(app: &AppCtx, detail: &ServerDetail) -> Vec<Stream> {
    let server = detail.server.id;
    let call = app.call.state();
    let rooms: Vec<Uuid> = super::channel_sidebar::channel_order(detail)
        .into_iter()
        .filter(|id| {
            detail
                .channels
                .iter()
                .any(|c| c.id == *id && c.kind == ChannelKind::Voice)
        })
        .collect();
    app.client.with_state(|s| {
        let mut out = Vec::new();
        for channel in rooms {
            let Some(room) = voice::session::build(s, &call, server, channel) else {
                continue;
            };
            for p in room.occupants.iter().filter(|p| p.live) {
                let share = room.shares.iter().find(|pic| pic.id.user == p.id);
                let key = share
                    .and_then(|pic| pic.key)
                    .or_else(|| (p.me && room.here).then(|| VideoKey::live(p.id)));
                out.push(Stream {
                    owner: p.id,
                    channel,
                    name: p.name.clone(),
                    avatar: p.avatar.clone(),
                    own: p.me,
                    here: room.here,
                    allowed: room.allowed,
                    viewers: voice::watchers::count(&room, p.id),
                    key,
                });
            }
        }
        out
    })
}

/// `.overview-streams`: `cols` per row, gap 20. Each the 16:9 picture
/// (the video where the call has it, else the owner's avatar), LIVE and the
/// viewers on it, the avatar and name under it. `at` is the column's left,
/// top and width; returns the bottom.
fn streams_block(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    server: Uuid,
    at: (f32, f32, f32),
    cols: usize,
    streams: &[Stream],
) -> f32 {
    let c = theme.c;
    let (left, top, width) = at;
    let radius = theme.radius.lg;
    let gaps = (cols - 1) as f32 * 20.0;
    let item_w = ((width - gaps) / cols as f32).max(0.0);
    let pic_h = (item_w * 9.0 / 16.0).round();
    let item_h = pic_h + 10.0 + 28.0;
    for (i, st) in streams.iter().enumerate() {
        let at = pos2(
            left + (i % cols) as f32 * (item_w + 20.0),
            top + (i / cols) as f32 * (item_h + 20.0),
        );
        let item = Rect::from_min_size(at, vec2(item_w, item_h));
        let pic = Rect::from_min_size(at, vec2(item_w, pic_h));
        let id = Id::unique(("overview-stream", st.owner));
        let response = common::click(ui, item, id);
        let label = if st.own {
            servers::overview_own_screen().to_owned()
        } else {
            servers::overview_watch(&st.name)
        };
        crate::widgets::announce(&response, egui::Role::Button, &label);
        let painter = ui.painter().clone();
        // The picture only where the call already has it: the own screen,
        // or a Live watched in the call this client is in.
        let frame = st
            .key
            .filter(|_| st.here)
            .and_then(|key| app.call.frame(key).map(|f| (key, f)));
        match frame {
            Some((key, frame)) => {
                painter.rect_filled(pic, corner(radius), Color32::BLACK);
                let fit = voice::video::Fit::Cover;
                voice::video::request_fit(ui, app.call, key, pic, Some(&frame), fit, 15);
                voice::video::paint(ui, id.with("video"), pic, frame, fit, false, radius);
            }
            None => {
                // `.overview-stream__stand-in`: the large avatar on the panel.
                painter.rect_filled(pic, corner(radius), c.panel);
                let avatar = Rect::from_center_size(pic.center(), Vec2::splat(64.0));
                common::avatar(
                    ui,
                    app.images,
                    avatar,
                    &st.name,
                    st.avatar.as_deref(),
                    20.0,
                    1.0,
                    c.panel,
                );
            }
        }
        painter.rect_stroke(
            pic,
            corner(radius),
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
        // LIVE: the live fill, 700 11px, 24 high, radius 6.
        let live = Block::single(
            ui,
            "LIVE",
            TextStyle::new(11.0, 700.0, 1.0).tracking(0.04),
            c.on_live,
        );
        let tag = Rect::from_min_size(
            pos2(pic.left() + 12.0, pic.top() + 12.0),
            vec2(live.width() + 16.0, 24.0),
        );
        painter.rect_filled(tag, corner(6.0), c.live);
        paint_mid(&painter, &live, tag.left() + 8.0, tag.center().y);
        // The viewers: the eye and the count on a dark chip.
        let count = Block::single(
            ui,
            &st.viewers.to_string(),
            TextStyle::new(12.0, 600.0, 1.0),
            ON_DARK,
        );
        let chip = Rect::from_min_max(
            pos2(
                pic.right() - 12.0 - (8.0 + 14.0 + 5.0 + count.width() + 8.0),
                pic.top() + 12.0,
            ),
            pos2(pic.right() - 12.0, pic.top() + 36.0),
        );
        painter.rect_filled(chip, corner(6.0), CHIP);
        icons::paint(
            &painter,
            Icon::Eye,
            pos2(chip.left() + 8.0, chip.center().y - 7.0),
            14.0,
            ON_DARK,
        );
        paint_mid(&painter, &count, chip.left() + 27.0, chip.center().y);
        let viewers = servers::overview_viewers(st.viewers as u64);
        ui.interact(chip, id.with("viewers"), egui::Sense::hover())
            .on_hover_text(&viewers)
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &viewers));
        // Under it: the avatar and the name ("Du" for the own).
        let cy = pic.bottom() + 10.0 + 14.0;
        let small = Rect::from_min_size(pos2(at.x, cy - 14.0), Vec2::splat(28.0));
        common::avatar(
            ui,
            app.images,
            small,
            &st.name,
            st.avatar.as_deref(),
            12.0,
            1.0,
            c.bg,
        );
        let shown = if st.own {
            servers::overview_you()
        } else {
            st.name.as_str()
        };
        let name = line(ui, shown, NAME, c.text, item_w - 38.0);
        paint_mid(&painter, &name, at.x + 38.0, cy);
        common::focus_visible(ui, &response, pic, radius);
        if response.clicked() {
            // Outside a call this joins it with the Live watched from the
            // start; in another call only the page opens.
            if !st.own && !st.here && st.allowed {
                voice::hint::join_to_watch(app, server, st.channel, &[st.owner]);
            } else {
                app.nav.navigate(&format!("/s/{server}/c/{}", st.channel));
            }
        }
    }
    let rows = streams.len().div_ceil(cols) as f32;
    top + rows * item_h + (rows - 1.0).max(0.0) * 20.0
}

// -------------------------------------------------------------- verpasst

/// The server's unread text channels, as the sidebar marks them.
fn unread_channels(app: &AppCtx, detail: &ServerDetail) -> Vec<Unread> {
    let now = Utc::now();
    let order = super::channel_sidebar::channel_order(detail);
    app.client.with_state(|s| {
        detail
            .channels
            .iter()
            .filter(|ch| ch.kind == ChannelKind::Text)
            .filter_map(|ch| {
                let badge = s.channel_badge(ch.id, now);
                if badge.count == 0 && !badge.dot {
                    return None;
                }
                let read = s.read.get(&ch.id)?;
                Some(Unread {
                    channel: ch.id,
                    name: ch.name.clone(),
                    place: order
                        .iter()
                        .position(|id| *id == ch.id)
                        .unwrap_or(usize::MAX),
                    unread: read.unread_count.max(0),
                    // The sidebar's "@": a server channel counts only mentions.
                    mentions: badge.count,
                    latest: read.latest_order,
                    read: read.read_order,
                })
            })
            .collect()
    })
}

/// The newest messages of `channel`, oldest first: from a loaded history
/// that has the newest one, else fetched once per `latest` order.
fn missed_messages(
    ui: &Ui,
    app: &AppCtx,
    channel: Uuid,
    latest: Option<i64>,
) -> Option<Vec<Message>> {
    let loaded = app.client.with_state(|s| {
        s.messages(channel)
            .filter(|h| h.loaded)
            .filter(|h| {
                latest.is_none_or(|l| h.messages.last().is_some_and(|m| m.created_order >= l))
            })
            .map(|h| {
                let from = h.messages.len().saturating_sub(FETCHED as usize);
                h.messages[from..].to_vec()
            })
    });
    if loaded.is_some() {
        return loaded;
    }
    state::with(ui.ctx(), |s| {
        let entry = s.missed.entry(channel).or_insert_with(|| Missed {
            order: latest,
            shown: None,
            pending: Some(fetch_messages(app, channel)),
        });
        if let Some(page) = entry.pending.as_ref().and_then(Pending::take) {
            // A failed fetch keeps what was there.
            if page.is_some() {
                entry.shown = page;
            }
            entry.pending = None;
        }
        // A new message changes the order: ask again, the old ones stay.
        if entry.order != latest && entry.pending.is_none() {
            entry.order = latest;
            entry.pending = Some(fetch_messages(app, channel));
        }
        entry.shown.clone()
    })
}

fn fetch_messages(app: &AppCtx, channel: Uuid) -> Pending<Option<Vec<Message>>> {
    let http = app.client.http().clone();
    Pending::spawn(app.client, async move {
        http.messages(channel, &HistoryQuery::latest(FETCHED))
            .await
            .ok()
            .map(|page| page.messages)
    })
}

/// "Verpasst": its heading, the cards (or "Alles gelesen."), and how many
/// channels more are unread. `at` is the column's left, top and width;
/// returns the bottom.
#[allow(clippy::too_many_arguments)]
fn missed(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    detail: &ServerDetail,
    at: (f32, f32, f32),
    cards: &[Unread],
    fetched: &[Option<Vec<Message>>],
    more_channels: usize,
    single: bool,
) -> f32 {
    let c = theme.c;
    let (left, top, width) = at;
    let h2 = Block::single(ui, servers::overview_missed(), H2, c.text);
    h2.paint(ui.painter(), pos2(left, top));
    let mut y = top + h2.height() + 12.0;
    if cards.is_empty() {
        return quiet_card(ui, theme, detail, left, y, width);
    }
    let names: HashMap<String, String> = detail
        .members
        .iter()
        .map(|m| (m.user_id.to_string(), m.name.clone()))
        .collect();
    let me = app.client.with_state(|s| s.me().map(|m| m.id));
    for (i, card) in cards.iter().enumerate() {
        let messages = fetched.get(i).cloned().flatten();
        y = missed_card(
            ui,
            app,
            theme,
            detail.server.id,
            (left, y, width),
            card,
            messages.as_deref(),
            i == 0 && detail.server.can(Permission::SendMessages),
            single,
            (&names, me),
        );
        y += 14.0;
    }
    y -= 14.0;
    if more_channels > 0 {
        y += 14.0;
        let words = Block::single(
            ui,
            &servers::overview_more_channels(more_channels as u64),
            SMALL.line_height(1.4),
            c.muted,
        );
        words.paint(ui.painter(), pos2(left, y));
        y += words.height();
    }
    y
}

/// `.overview-quiet`: "Alles gelesen.", and for a server of one "Noch
/// keine weiteren Mitglieder." with "Einladen"; wrapping, gap 8 16, padding
/// 18 20. Returns the bottom.
fn quiet_card(
    ui: &mut Ui,
    theme: &Theme,
    detail: &ServerDetail,
    left: f32,
    top: f32,
    width: f32,
) -> f32 {
    let c = theme.c;
    let style = TextStyle::new(14.0, 400.0, 1.45);
    let alone = detail.members.len() == 1;
    let fill = ui.painter().add(egui::Shape::Noop);
    let inner = (left + 20.0, left + width - 20.0);
    let mut words = vec![servers::overview_all_read()];
    if alone {
        words.push(servers::overview_alone());
    }
    // The pieces in rows, as `flex-wrap` lays them out.
    let blocks: Vec<Block> = words
        .iter()
        .map(|text| Block::wrapped(ui, text, style, c.muted, inner.1 - inner.0))
        .collect();
    let mut sizes: Vec<Vec2> = blocks.iter().map(|b| b.size()).collect();
    if alone {
        let bw = Button::new(servers::overview_invite())
            .small()
            .natural_width(ui);
        sizes.push(vec2(bw, 32.0));
    }
    let mut rects = Vec::new();
    let (mut x, mut y, mut row_h) = (inner.0, top + 18.0, 0.0_f32);
    for size in &sizes {
        if x > inner.0 && x + size.x > inner.1 {
            x = inner.0;
            y += row_h + 8.0;
            row_h = 0.0;
        }
        rects.push(Rect::from_min_size(pos2(x, y), *size));
        x += size.x + 16.0;
        row_h = row_h.max(size.y);
    }
    let bottom = y + row_h + 18.0;
    for (n, block) in blocks.iter().enumerate() {
        let r = rects[n];
        // On the row's middle, as `align-items: center` puts it.
        let at = pos2(
            r.left(),
            r.top() + (32.0_f32.min(row_h) - r.height()).max(0.0) / 2.0,
        );
        block.paint(ui.painter(), at);
        let response = ui.interact(
            Rect::from_min_size(at, r.size()),
            Id::unique(("overview-quiet", n)),
            egui::Sense::hover(),
        );
        crate::widgets::announce(&response, egui::Role::Label, words[n]);
    }
    if alone
        && let Some(r) = rects.last()
        && ui
            .put(*r, Button::new(servers::overview_invite()).small())
            .clicked()
    {
        super::dialogs::open_invite(ui.ctx(), detail.server.id, &detail.server.name);
    }
    let rect = Rect::from_min_max(pos2(left, top), pos2(left + width, bottom));
    ui.painter().set(
        fill,
        egui::epaint::RectShape::new(
            rect,
            corner(theme.radius.lg),
            c.surface,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        ),
    );
    bottom
}

/// One card: the channel's head with "Gelesen" and "Öffnen", its newest
/// unread messages, "x weitere" and, on the first card, the reply field.
#[allow(clippy::too_many_arguments)]
fn missed_card(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    server: Uuid,
    at: (f32, f32, f32),
    card: &Unread,
    messages: Option<&[Message]>,
    first: bool,
    single: bool,
    people: (&HashMap<String, String>, Option<Uuid>),
) -> f32 {
    let c = theme.c;
    let (left, top, width) = at;
    let channel = card.channel;
    // The fill goes under what follows; its height is known at the end.
    let fill = ui.painter().add(egui::Shape::Noop);
    let inner_left = left + 20.0;
    let inner_right = left + width - 20.0;
    let mut y = top + 6.0;
    let open = format!("/s/{server}/c/{channel}");

    // Head (42 high): "# name", "@", "x neu", then the actions.
    let cy = y + 21.0;
    let open_label = servers::overview_open();
    let read_label = servers::overview_mark_read();
    let open_w = ghost_width(ui, open_label);
    let open_r = Rect::from_min_size(pos2(inner_right - open_w, cy - 15.0), vec2(open_w, 30.0));
    let read_w = ghost_width(ui, read_label);
    let read_r = Rect::from_min_size(
        pos2(open_r.left() - 2.0 - read_w, cy - 15.0),
        vec2(read_w, 30.0),
    );
    if ghost(
        ui,
        theme,
        read_r,
        Id::unique(("overview-read", channel)),
        read_label,
        Icon::Check,
        false,
    )
    .clicked()
    {
        app.client.mark_conversation_read(channel);
    }
    if ghost(
        ui,
        theme,
        open_r,
        Id::unique(("overview-open", channel)),
        open_label,
        Icon::ArrowRight,
        true,
    )
    .clicked()
    {
        app.nav.navigate(&open);
    }
    let new = Block::single(
        ui,
        &servers::overview_new(card.unread as u64),
        SMALL,
        c.muted,
    );
    let at_pill = (card.mentions > 0)
        .then(|| Block::single(ui, "@", TextStyle::new(11.0, 700.0, 1.0), c.accent_ink));
    let extras = 8.0 + new.width() + at_pill.as_ref().map_or(0.0, |b| 8.0 + b.width() + 14.0);
    let name_max = (read_r.left() - 12.0 - extras - (inner_left + 24.0)).max(24.0);
    let name = line(ui, &card.name, NAME, c.text, name_max);
    let name_r = Rect::from_min_max(
        pos2(inner_left - 4.0, cy - 15.0),
        pos2(inner_left + 24.0 + name.width() + 4.0, cy + 15.0),
    );
    let name_response = common::click(ui, name_r, Id::unique(("overview-channel", channel)));
    crate::widgets::announce(&name_response, egui::Role::Link, &format!("#{}", card.name));
    icons::paint(
        ui.painter(),
        Icon::Hash,
        pos2(inner_left, cy - 8.0),
        16.0,
        c.muted,
    );
    paint_mid(ui.painter(), &name, inner_left + 24.0, cy);
    common::focus_visible(ui, &name_response, name_r, 6.0);
    if name_response.clicked() {
        app.nav.navigate(&open);
    }
    let mut x = name_r.right() - 4.0 + 8.0;
    if let Some(pill) = &at_pill {
        let r = Rect::from_min_size(pos2(x, cy - 10.0), vec2(pill.width() + 14.0, 20.0));
        ui.painter().rect_filled(r, corner(10.0), c.accent);
        paint_mid(ui.painter(), pill, r.left() + 7.0, cy);
        let mark = ui.interact(
            r,
            Id::unique(("overview-at", channel)),
            egui::Sense::hover(),
        );
        crate::widgets::announce(&mark, egui::Role::Image, servers::overview_mentioned());
        x = r.right() + 8.0;
    }
    paint_mid(ui.painter(), &new, x, cy);
    y += 42.0;

    // The messages, as the chat draws them.
    let (count, dim) = card_look(card.unread, card.mentions, single);
    let shown = messages.map_or(0, |m| m.len().min(count));
    if let Some(messages) = messages {
        for m in messages.iter().take(count) {
            let (bottom, media) = message_row(
                ui,
                app,
                theme,
                (inner_left, y, inner_right - inner_left),
                m,
                dim,
                people,
            );
            y = bottom;
            if let Some(media) = media {
                media_action(ui, app, media);
            }
        }
    }

    // "x weitere →" ("x ältere anzeigen" with one channel): to the first
    // unread message.
    let rest = (card.unread - shown as i64).max(0) as u64;
    if rest > 0 {
        y += 4.0;
        let words = if single {
            servers::overview_older(rest)
        } else {
            servers::overview_more(rest)
        };
        let block = Block::single(ui, &words, SMALL, c.muted);
        let r = Rect::from_min_size(pos2(inner_left, y), vec2(block.width() + 6.0 + 14.0, 20.0));
        let response = common::click(ui, r, Id::unique(("overview-more", channel)));
        crate::widgets::announce(&response, egui::Role::Link, &words);
        let color = if response.hovered() { c.text } else { c.muted };
        block.paint_tinted(
            ui.painter(),
            pos2(r.left(), r.center().y - block.height() / 2.0),
            Some(color),
        );
        icons::paint(
            ui.painter(),
            Icon::ArrowRight,
            pos2(r.left() + block.width() + 6.0, r.center().y - 7.0),
            14.0,
            color,
        );
        common::focus_visible(ui, &response, r, 4.0);
        if response.clicked() {
            app.nav.navigate(&open);
        }
        y = r.bottom();
    }

    // The first card answers in place: Enter sends.
    if first {
        y += 12.0;
        y = reply_field(
            ui,
            app,
            theme,
            channel,
            &card.name,
            Rect::from_min_size(pos2(inner_left, y), vec2(inner_right - inner_left, 42.0)),
        );
    }
    y += 14.0;
    let rect = Rect::from_min_max(pos2(left, top), pos2(left + width, y));
    ui.painter().set(
        fill,
        egui::epaint::RectShape::new(
            rect,
            corner(theme.radius.lg),
            c.surface,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        ),
    );
    y
}

/// A message as the chat has it, compact: avatar 32, name and time, the
/// text with its mentions as chips, pictures and files. One that mentions
/// me has the chat's mention fill. Returns the bottom.
fn message_row(
    ui: &mut Ui,
    app: &AppCtx,
    theme: &Theme,
    at: (f32, f32, f32),
    m: &Message,
    dim: bool,
    people: (&HashMap<String, String>, Option<Uuid>),
) -> (f32, Option<attachments::MediaAction>) {
    let c = theme.c;
    let (left, top, width) = at;
    let (names, me) = people;
    let mut action = None;
    let fill = ui.painter().add(egui::Shape::Noop);
    let col_left = left + 32.0 + 12.0;
    let col_w = (width - 44.0).max(40.0);
    let mut y = top + 8.0;
    common::avatar(
        ui,
        app.images,
        Rect::from_min_size(pos2(left, y), Vec2::splat(32.0)),
        &m.author.name,
        m.author.avatar_url.as_deref(),
        12.0,
        1.0,
        c.surface,
    );
    let when = time::format_when(m.created_at, Utc::now());
    let stamp = Block::single(ui, &when, TextStyle::new(12.0, 400.0, 1.3), c.muted);
    let name = line(
        ui,
        &m.author.name,
        NAME,
        c.text,
        col_w - stamp.width() - 8.0,
    );
    name.paint(ui.painter(), pos2(col_left, y));
    stamp.paint(
        ui.painter(),
        pos2(
            col_left + name.width() + 8.0,
            common::baseline_top(NAME, y, TextStyle::new(12.0, 400.0, 1.3)),
        ),
    );
    y += name.height();
    let color = if dim { c.muted } else { c.text };
    if let Some(system) = system_text(m) {
        y += 3.0;
        let block = Block::wrapped(ui, &system, text::BODY, c.muted, col_w);
        block.paint(ui.painter(), pos2(col_left, y));
        y += block.height();
    } else if !m.content.trim().is_empty() {
        y += 3.0;
        let mentions = content::Mentions {
            names,
            me,
            everyone: m.mention_everyone,
            mentions: &m.mentions,
        };
        let laid = content::layout_plain(ui, &m.content, col_w, &mentions, color);
        laid.paint(
            ui,
            pos2(col_left, y),
            Id::unique(("overview-text", m.id)),
            false,
            None,
        );
        y += laid.height;
    } else if m.gif.is_some() {
        y += 3.0;
        let block = Block::single(ui, "GIF", text::BODY, c.muted);
        block.paint(ui.painter(), pos2(col_left, y));
        y += block.height();
    }
    // `AttachmentList`: the chat's pictures and files.
    if let Some(media) = attachments::layout(ui, app.images, &m.attachments, col_w, None, None) {
        y += 8.0;
        let mut text = None;
        action = media.paint(
            ui,
            app.images,
            pos2(col_left, y),
            Id::unique(("overview-media", m.id)),
            &mut text,
        );
        y += media.height;
    }
    y = y.max(top + 8.0 + 32.0) + 8.0;
    if mentions_me(m, me) {
        // `[data-mentioned]`: the chat's mention colour over the card, 10
        // past the column on both sides.
        let row = Rect::from_min_max(pos2(left - 10.0, top), pos2(left + width + 10.0, y));
        ui.painter().set(
            fill,
            egui::epaint::RectShape::filled(
                row,
                corner(theme.radius.md),
                mix(c.surface, c.warning, 0.13),
            ),
        );
    }
    (y, action)
}

/// What a card's picture or file asked for: a picture large, a download,
/// a link in the browser.
fn media_action(ui: &Ui, app: &AppCtx, action: attachments::MediaAction) {
    use attachments::{Lightbox, MediaAction};
    let lightbox = match action {
        MediaAction::Open(url, name) => Some(Lightbox::single(url, name)),
        MediaAction::Gallery(items, index) => Some(Lightbox::new(items, index)),
        MediaAction::Download { url, name } => {
            attachments::download(ui.ctx(), app.client, &url, &name);
            None
        }
        MediaAction::External(url) => {
            let full = if url.starts_with('/') {
                app.client.http().url(&url)
            } else {
                url
            };
            crate::platform::open_url_logged(&full);
            None
        }
        // Only the own messages' alt texts, which the chat edits.
        MediaAction::EditAlt { .. } => None,
    };
    if lightbox.is_some() {
        state::with(ui.ctx(), |s| s.lightbox = lightbox);
    }
}

/// A system message's line ("… ist live gegangen").
fn system_text(m: &Message) -> Option<String> {
    let system = m.system.as_ref()?;
    Some(match system.kind.as_str() {
        "live_started" => chat_texts::system_live_started(
            &m.author.name,
            system
                .params
                .get("voice_name")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        ),
        _ => chat_texts::system_unknown(&m.author.name),
    })
}

/// A shared picture in `rect`, cut to it (`object-fit: cover`): the
/// server's 400 thumbnail when it has one, its colour until it arrives.
fn picture(ui: &Ui, app: &AppCtx, theme: &Theme, att: &Attachment, rect: Rect, radius: f32) {
    let c = theme.c;
    let painter = ui.painter();
    let url = if att.thumbnail {
        format!("{}?size=400", pictures::original_url(att))
    } else {
        pictures::original_url(att)
    };
    match app.images.shown(ui.ctx(), &url, Fit::Inline) {
        ImageState::Ready(texture) => {
            painter.add(
                egui::epaint::RectShape::filled(rect, corner(radius), Color32::WHITE)
                    .with_texture(texture.id(), cover_uv(texture.size_vec2(), rect.size())),
            );
        }
        _ => {
            let under = pictures::placeholder(att).unwrap_or(c.panel);
            painter.rect_filled(rect, corner(radius), under);
        }
    }
    painter.rect_stroke(
        rect,
        corner(radius),
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
}

/// A file's icon (accent) and its two lines, inside `rect`'s 10 12 padding.
fn file_lines(ui: &Ui, theme: &Theme, rect: Rect, name: &str, second: &str) {
    let c = theme.c;
    let painter = ui.painter();
    let cy = rect.center().y;
    icons::paint(
        painter,
        Icon::File,
        pos2(rect.left() + 12.0, cy - 10.0),
        20.0,
        c.accent_text,
    );
    let x = rect.left() + 12.0 + 20.0 + 10.0;
    let w = rect.right() - 12.0 - x;
    let top = line(ui, name, TextStyle::new(13.0, 600.0, 1.3), c.text, w);
    let bottom = line(ui, second, TextStyle::new(12.0, 400.0, 1.3), c.muted, w);
    let y = cy - (top.height() + bottom.height()) / 2.0;
    top.paint(painter, pos2(x, y));
    bottom.paint(painter, pos2(x, y + top.height()));
}

/// "Antworten in #name" with "Enter": a single line whose Enter sends to
/// the channel. Returns the bottom.
fn reply_field(
    ui: &mut Ui,
    app: &AppCtx,
    theme: &Theme,
    channel: Uuid,
    name: &str,
    rect: Rect,
) -> f32 {
    let c = theme.c;
    let id = Id::unique(("overview-reply", channel));
    let focused = ui.memory(|m| m.has_focus(id));
    let painter = ui.painter().clone();
    // The field shell's frame and focus ring (#561).
    let f = theme.field;
    let border = if focused { f.border_focus } else { f.border };
    painter.rect(
        rect,
        corner(f.radius),
        f.bg,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    if focused {
        crate::widgets::field_focus_ring(&painter, rect, f.radius, border);
    }
    let kbd = kbd(
        ui,
        &painter,
        "Enter",
        rect.right() - 12.0,
        rect.center().y,
        theme,
    );
    let placeholder = servers::overview_reply(name);
    let mut draft = state::with(ui.ctx(), |s| {
        s.replies.get(&channel).cloned().unwrap_or_default()
    });
    let color = c.text;
    let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        let mut job = LayoutJob::single_section(buf.as_str().to_owned(), text::BODY.format(color));
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let inner = Rect::from_min_max(
        pos2(rect.left() + 12.0, rect.top() + 1.0),
        pos2(kbd.left() - 8.0, rect.bottom() - 1.0),
    );
    let edit = egui::TextEdit::singleline(&mut draft)
        .id(id)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 9)))
        .desired_width(inner.width())
        .min_size(inner.size())
        .text_color(color)
        .layouter(&mut layouter)
        .char_limit(4000)
        .hint_text(LayoutJob::single_section(
            placeholder.clone(),
            text::BODY.format(c.muted),
        ));
    let output = ui
        .scope_builder(UiBuilder::new().max_rect(inner), |ui| edit.show(ui))
        .inner;
    let response = output
        .response
        .response
        .accessible_name(placeholder.as_str());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    let sent = response.lost_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    if sent && !draft.trim().is_empty() {
        app.client.send_message(channel, draft.trim(), None);
        draft.clear();
    }
    state::with(ui.ctx(), |s| {
        if draft.is_empty() {
            s.replies.remove(&channel);
        } else {
            s.replies.insert(channel, draft);
        }
    });
    rect.bottom()
}

/// A ghost button's width: padding 10, a 15px icon, gap 6, 500 13px.
fn ghost_width(ui: &Ui, label: &str) -> f32 {
    Block::single(ui, label, SMALL, Color32::WHITE).width() + 20.0 + 15.0 + 6.0
}

/// `.overview-card__action`: muted, the card's hover fill under the
/// pointer; the icon before the label, or after it (`after`).
fn ghost(
    ui: &Ui,
    theme: &Theme,
    rect: Rect,
    id: Id,
    label: &str,
    icon: Icon,
    after: bool,
) -> egui::Response {
    let c = theme.c;
    let response = common::click(ui, rect, id);
    crate::widgets::announce(&response, egui::Role::Button, label);
    let hot = response.hovered() || response.has_focus();
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, corner(theme.radius.sm), c.hover);
    }
    let color = if hot { c.text } else { c.muted };
    let block = Block::single(ui, label, SMALL, color);
    let cy = rect.center().y;
    if after {
        paint_mid(ui.painter(), &block, rect.left() + 10.0, cy);
        icons::paint(
            ui.painter(),
            icon,
            pos2(rect.left() + 10.0 + block.width() + 6.0, cy - 7.5),
            15.0,
            color,
        );
    } else {
        icons::paint(
            ui.painter(),
            icon,
            pos2(rect.left() + 10.0, cy - 7.5),
            15.0,
            color,
        );
        paint_mid(ui.painter(), &block, rect.left() + 10.0 + 15.0 + 6.0, cy);
    }
    common::focus_visible(ui, &response, rect, theme.radius.sm);
    response
}

// --------------------------------------------------------------- geteilt

/// The server's newest shared files, fetched again once a channel of it
/// has a newer message.
fn shared_files(ui: &Ui, app: &AppCtx, detail: &ServerDetail) -> Vec<SharedFile> {
    let server = detail.server.id;
    let key = app.client.with_state(|s| {
        detail
            .channels
            .iter()
            .filter_map(|ch| s.read.get(&ch.id).and_then(|r| r.latest_order))
            .max()
    });
    state::with(ui.ctx(), |s| {
        let entry = s.shared.entry(server).or_insert_with(|| SharedFiles {
            key,
            shown: Vec::new(),
            pending: Some(fetch_shared(app, server)),
        });
        if let Some(files) = entry.pending.as_ref().and_then(Pending::take) {
            if let Some(files) = files {
                entry.shown = files;
            }
            entry.pending = None;
        }
        if entry.key != key && entry.pending.is_none() {
            entry.key = key;
            entry.pending = Some(fetch_shared(app, server));
        }
        entry.shown.clone()
    })
}

fn fetch_shared(app: &AppCtx, server: Uuid) -> Pending<Option<Vec<SharedFile>>> {
    let http = app.client.http().clone();
    Pending::spawn(app.client, async move {
        http.shared_files(server, SHARED_LIMIT).await.ok()
    })
}

/// "Geteilt": pictures as 16:9 thumbnails with the sender (and the
/// channel, when the server has more than one text channel), other files
/// as rows. A click opens the channel at the message. Returns the bottom.
#[allow(clippy::too_many_arguments)]
fn shared_column(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    detail: &ServerDetail,
    left: f32,
    top: f32,
    width: f32,
    files: &[SharedFile],
) -> f32 {
    let c = theme.c;
    let server = detail.server.id;
    let h2 = Block::single(ui, servers::overview_shared(), H2, c.text);
    h2.paint(ui.painter(), pos2(left, top));
    let mut y = top + h2.height() + 12.0;
    let texts: Vec<(Uuid, &str)> = detail
        .channels
        .iter()
        .filter(|ch| ch.kind == ChannelKind::Text)
        .map(|ch| (ch.id, ch.name.as_str()))
        .collect();
    let several = texts.len() > 1;
    let channel_name = |id: Uuid| texts.iter().find(|(c, _)| *c == id).map(|(_, n)| *n);
    for f in files {
        let id = Id::unique(("overview-shared", f.attachment.id));
        let rect = if is_picture(&f.attachment) {
            let r = Rect::from_min_size(pos2(left, y), vec2(width, (width * 9.0 / 16.0).round()));
            let response = common::click(ui, r, id);
            crate::widgets::announce(
                &response,
                egui::Role::Button,
                &f.attachment
                    .description
                    .clone()
                    .unwrap_or_else(|| servers::overview_picture_by(&f.author.name)),
            );
            picture(ui, app, theme, &f.attachment, r, theme.radius.lg);
            // The sender: avatar 20 and name on a dark chip, bottom left.
            let name = line(
                ui,
                &f.author.name,
                TextStyle::new(12.0, 600.0, 1.0),
                ON_DARK,
                width - 60.0,
            );
            let chip = Rect::from_min_max(
                pos2(r.left() + 8.0, r.bottom() - 8.0 - 26.0),
                pos2(
                    r.left() + 8.0 + 3.0 + 20.0 + 6.0 + name.width() + 8.0,
                    r.bottom() - 8.0,
                ),
            );
            ui.painter().rect_filled(chip, corner(13.0), CHIP);
            common::avatar(
                ui,
                app.images,
                Rect::from_min_size(pos2(chip.left() + 3.0, chip.top() + 3.0), Vec2::splat(20.0)),
                &f.author.name,
                f.author.avatar_url.as_deref(),
                9.0,
                1.0,
                CHIP,
            );
            paint_mid(ui.painter(), &name, chip.left() + 29.0, chip.center().y);
            // The channel, top right, when there is more than one.
            if several && let Some(channel) = channel_name(f.channel_id) {
                let tag = line(
                    ui,
                    &format!("#{channel}"),
                    TextStyle::new(11.0, 600.0, 1.0),
                    ON_DARK,
                    width / 2.0,
                );
                let t = Rect::from_min_max(
                    pos2(r.right() - 8.0 - tag.width() - 14.0, r.top() + 8.0),
                    pos2(r.right() - 8.0, r.top() + 8.0 + 18.0),
                );
                ui.painter().rect_filled(t, corner(6.0), CHIP);
                paint_mid(ui.painter(), &tag, t.left() + 7.0, t.center().y);
            }
            common::focus_visible(ui, &response, r, theme.radius.lg);
            if response.clicked() {
                open_message(ui, app, server, f.channel_id, f.message_id);
            }
            r
        } else {
            let r = Rect::from_min_size(pos2(left, y), vec2(width, 54.0));
            let response = common::click(ui, r, id);
            crate::widgets::announce(&response, egui::Role::Button, &f.attachment.filename);
            ui.painter().rect(
                r,
                corner(theme.radius.lg),
                if response.hovered() {
                    c.hover
                } else {
                    c.surface
                },
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            let second = match channel_name(f.channel_id).filter(|_| several) {
                Some(channel) => format!("{} · #{channel}", f.author.name),
                None => format!(
                    "{} · {}",
                    f.author.name,
                    attachments::format_size(f.attachment.size)
                ),
            };
            file_lines(ui, theme, r, &f.attachment.filename, &second);
            common::focus_visible(ui, &response, r, theme.radius.lg);
            if response.clicked() {
                open_message(ui, app, server, f.channel_id, f.message_id);
            }
            r
        };
        y = rect.bottom() + 10.0;
    }
    y - 10.0
}

/// The channel's page at `message`.
fn open_message(ui: &Ui, app: &mut AppCtx, server: Uuid, channel: Uuid, message: Uuid) {
    chat::open_at(ui.ctx(), channel, message);
    app.nav.navigate(&format!("/s/{server}/c/{channel}"));
}

// ----------------------------------------------------------- the rest

fn failed_id(server: Uuid) -> Id {
    Id::unique(("gelabber-overview-failed", server))
}

/// `LoadError` "Der Server konnte nicht geladen werden." with "Erneut
/// versuchen" ("Wird geladen…" while it runs) at the page's 24px padding;
/// false while the server just loads.
fn load_error(ui: &mut Ui, app: &AppCtx, rect: Rect, server: Uuid) -> bool {
    let op = app
        .client
        .with_state(|s| s.op(&format!("server:{server}")).cloned());
    let ctx = ui.ctx().clone();
    let failed_before: bool = ctx.data(|d| d.get_temp(failed_id(server))).unwrap_or(false);
    let failed = matches!(op, Some(gelabber_client::OpState::Failed(_)));
    let pending = matches!(op, Some(gelabber_client::OpState::Pending));
    if failed != failed_before && !pending {
        ctx.data_mut(|d| d.insert_temp(failed_id(server), failed));
    }
    // The error stays while the retry runs, as a query keeps its error.
    if !failed && !(pending && failed_before) {
        return false;
    }
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let area = Rect::from_min_size(
        pos2(rect.left() + 24.0, rect.top() + 24.0),
        Vec2::new(rect.width() - 48.0, 200.0),
    );
    let msg = servers::load_server();
    let block = Block::wrapped(ui, msg, text::BODY, c.text, area.width() - 30.0);
    let h = 20.0 + block.height() + 12.0 + 32.0;
    let r = Rect::from_min_size(area.min, Vec2::new(area.width(), h));
    ui.painter()
        .rect_filled(r, corner(theme.radius.sm), mix(c.panel, c.danger_bg, 0.18));
    let bar = Rect::from_min_max(r.min, pos2(r.left() + 3.0, r.bottom()));
    ui.painter()
        .with_clip_rect(bar)
        .rect_filled(r, corner(theme.radius.sm), c.danger);
    let text_rect = Rect::from_min_size(pos2(r.left() + 15.0, r.top() + 10.0), block.size());
    block.paint(ui.painter(), text_rect.min);
    ui.interact(
        text_rect,
        Id::unique(("overview-load-error", server)),
        egui::Sense::hover(),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, msg));
    let label = gelabber_i18n::common::try_again();
    let bw = Block::single(ui, label, text::UI.weight(600.0), c.text).width() + 26.0;
    let br = Rect::from_min_size(
        pos2(r.left() + 15.0, r.top() + 10.0 + block.height() + 12.0),
        Vec2::new(bw, 32.0),
    );
    let button = Button::new(label)
        .small()
        .pending(pending.then_some(gelabber_i18n::common::loading()));
    if ui.put(br, button).clicked() && !pending {
        app.client.refresh_server(server);
    }
    true
}

/// `MainSkeleton kind="overview"` while the server loads: a few quiet bars.
fn skeleton(ui: &Ui, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let t = ui.input(|i| i.time) as f32;
    let k = 0.75 + 0.25 * (t * std::f32::consts::PI).cos();
    let left = rect.left() + 32.0;
    let bars = [
        (84.0, 260.0, 32.0),
        (124.0, 120.0, 16.0),
        (172.0, 680.0, 120.0),
    ];
    for (top, w, h) in bars {
        let bar = Rect::from_min_size(pos2(left, rect.top() + top), Vec2::new(w, h));
        ui.painter().rect_filled(
            bar,
            corner(theme.radius.md),
            theme.c.surface.gamma_multiply(k),
        );
    }
    ui.ctx().request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unread(n: u128, mentions: i64, latest: i64) -> Unread {
        Unread {
            channel: Uuid::from_u128(n),
            name: format!("c{n}"),
            place: n as usize,
            unread: 3,
            mentions,
            latest: Some(latest),
            read: Some(0),
        }
    }

    fn message(order: i64) -> Message {
        serde_json::from_value(serde_json::json!({
            "id": Uuid::from_u128(order as u128),
            "channel_id": Uuid::from_u128(1),
            "author": {"id": Uuid::from_u128(7), "name": "tino", "avatar_url": null},
            "content": format!("m{order}"),
            "created_at": "2026-11-23T22:00:00Z",
            "edited_at": null,
            "revision": 0,
            "created_order": order,
        }))
        .expect("message")
    }

    fn file(n: u128, message: u128) -> SharedFile {
        serde_json::from_value(serde_json::json!({
            "id": Uuid::from_u128(n), "filename": format!("f{n}.pdf"),
            "content_type": "application/pdf", "size": 1,
            "message_id": Uuid::from_u128(message), "channel_id": Uuid::from_u128(1),
            "author": {"id": Uuid::from_u128(7), "name": "tino", "avatar_url": null},
            "created_at": "2026-11-23T22:00:00Z",
        }))
        .expect("file")
    }

    #[test]
    fn the_greeting_follows_the_hour() {
        for (hour, part) in [
            (0, Daytime::Evening),
            (4, Daytime::Evening),
            (5, Daytime::Morning),
            (10, Daytime::Morning),
            (11, Daytime::Day),
            (17, Daytime::Day),
            (18, Daytime::Evening),
            (23, Daytime::Evening),
        ] {
            assert_eq!(daytime(hour), part, "{hour}:00");
        }
        assert_eq!(greeting("Rafi", 5), "Guten Morgen, Rafi");
        assert_eq!(greeting("Rafi", 12), "Guten Tag, Rafi");
        assert_eq!(greeting("Rafi", 22), "Guten Abend, Rafi");
    }

    #[test]
    fn cards_put_mentions_first_then_the_newest_and_stop_at_four() {
        let list = vec![
            unread(1, 0, 10),
            unread(2, 0, 50),
            unread(3, 2, 5),
            unread(4, 0, 30),
            unread(5, 1, 40),
            unread(6, 0, 20),
        ];
        let (shown, more) = cards(list);
        let ids: Vec<u128> = shown.iter().map(|u| u.channel.as_u128()).collect();
        assert_eq!(ids, [5, 3, 2, 4]);
        assert_eq!(more, 2);
        let (shown, more) = cards(vec![unread(1, 0, 10)]);
        assert_eq!(shown.len(), 1);
        assert_eq!(more, 0);
    }

    #[test]
    fn a_card_shows_two_one_when_busy_and_five_alone() {
        assert_eq!(card_look(3, 0, false), (2, false));
        assert_eq!(card_look(10, 0, false), (1, true));
        assert_eq!(
            card_look(10, 1, false),
            (2, false),
            "a mention is never dimmed"
        );
        assert_eq!(card_look(9, 0, false), (2, false));
        assert_eq!(card_look(40, 0, true), (5, false));
    }

    #[test]
    fn a_card_takes_the_newest_unread_oldest_first() {
        let page: Vec<Message> = (1..=5).map(message).collect();
        let orders = |list: Vec<Message>| list.iter().map(|m| m.created_order).collect::<Vec<_>>();
        assert_eq!(orders(unread_tail(&page, Some(2), 2)), [4, 5]);
        assert_eq!(orders(unread_tail(&page, Some(3), 5)), [4, 5]);
        assert_eq!(orders(unread_tail(&page, None, 3)), [3, 4, 5]);
        assert!(unread_tail(&page, Some(5), 2).is_empty());
    }

    #[test]
    fn shared_leaves_out_the_cards_messages_and_stops_at_four() {
        let files: Vec<SharedFile> = (1..=7).map(|n| file(n, 100 + n)).collect();
        let none = HashSet::new();
        assert_eq!(shared_shown(&files, &none).len(), 4);
        let in_cards: HashSet<Uuid> = [Uuid::from_u128(101), Uuid::from_u128(103)].into();
        let ids: Vec<u128> = shared_shown(&files, &in_cards)
            .iter()
            .map(|f| f.attachment.id.as_u128())
            .collect();
        assert_eq!(ids, [2, 4, 5, 6]);
    }
}
