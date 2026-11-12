//! `LiveHint` (voice/LiveHint.tsx, chat.css `.lr-live-pill`, `.lr-live-hint`):
//! who is live where on a server and the way to come along, as a pill in a
//! channel header, or on a narrow window as a strip over the messages. A
//! Live is watched in its voice channel, so the action, "Zuschauen", joins
//! it and watches (or, in another call, opens it).

use egui::{Id, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use gelabber_i18n::voice as texts;
use uuid::Uuid;

use super::{session, summary};
use crate::app::AppCtx;
use crate::screens::shell::common::{self, paint_mid};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};

/// One voice channel with people live in it.
struct Entry {
    channel: Uuid,
    name: Option<String>,
    own: bool,
    who: Vec<String>,
    /// The people live there besides me.
    others: Vec<Uuid>,
    here: bool,
}

/// The voice channels with Lives, whether I may join, and my call.
fn entries(app: &AppCtx, server: Uuid) -> (Vec<Entry>, bool, Option<(Uuid, Uuid)>) {
    let joined = app.call.with_state(session::joined);
    app.client.with_state(|s| {
        let me = s.me().map(|m| m.id);
        let detail = s.detail(server);
        let name_of = |u: &Uuid| {
            detail
                .and_then(|d| d.members.iter().find(|m| m.user_id == *u))
                .map_or(texts::people_someone().to_owned(), |m| m.name.clone())
        };
        let allowed = s
            .server(server)
            .is_none_or(|v| v.can(gelabber_client::Permission::JoinVoice));
        let mut out: Vec<Entry> = s
            .live
            .get(&server)
            .map(|l| {
                l.iter()
                    .filter(|(_, people)| !people.is_empty())
                    .map(|(channel, people)| {
                        let own = me.is_some_and(|m| people.contains(&m));
                        let mut who: Vec<String> = Vec::new();
                        if own {
                            who.push(summary::you_name());
                        }
                        who.extend(people.iter().filter(|u| Some(**u) != me).map(name_of));
                        Entry {
                            channel: *channel,
                            name: s.channel(*channel).map(|c| c.name.clone()),
                            own,
                            who,
                            others: people.iter().filter(|u| Some(**u) != me).copied().collect(),
                            here: joined == Some((server, *channel)),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The channel list's order, for a stable first entry.
        if let Some(d) = detail {
            out.sort_by_key(|e| d.channels.iter().position(|c| c.id == e.channel));
        }
        (out, allowed, joined)
    })
}

/// The pill laid out: its blocks and width.
struct Pill {
    channel: Uuid,
    others: Vec<Uuid>,
    badge: Block,
    link: Block,
    link_text: String,
    title: String,
    action: Option<(&'static str, Block)>,
    width: f32,
}

fn measure(ui: &Ui, app: &AppCtx, server: Uuid, max_width: f32) -> Option<Pill> {
    let (list, allowed, joined) = entries(app, server);
    let first = list.first()?;
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let badge_style = TextStyle::new(10.0, 700.0, 1.5).tracking(0.04);
    let style = TextStyle::new(12.0, 400.0, 1.5);
    let badge = Block::single(ui, "LIVE", badge_style, c.on_live);
    let more = list.len() - 1;
    let who = summary::name_list(&first.who);
    let sentence = summary::live_sentence(&first.who, first.own);
    let (who, title) = match &first.name {
        Some(n) => (texts::hint_in(&who, n), texts::hint_in(&sentence, n)),
        None => (who, sentence),
    };
    let link_text = if more > 0 {
        format!("{who} +{more}")
    } else {
        who
    };
    let action = (!first.here && allowed && !first.others.is_empty()).then_some(watch_label(
        joined,
        server,
        first.channel,
    ));
    let action = action.map(|a| (a, Block::single(ui, a, style.weight(650.0), c.text)));
    let fixed = 1.0
        + 6.0
        + badge.width()
        + 12.0
        + 8.0
        + action.as_ref().map_or(0.0, |(_, b)| 8.0 + b.width() + 20.0)
        + 4.0
        + 1.0;
    let link_max = (max_width - fixed).max(20.0);
    let link = common::line(ui, &link_text, style, c.text, link_max);
    let width = fixed + link.width();
    Some(Pill {
        channel: first.channel,
        others: first.others.clone(),
        title,
        badge,
        link,
        link_text,
        action,
        width,
    })
}

fn paint_pill(ui: &mut Ui, app: &mut AppCtx, server: Uuid, pill: Pill, x: f32, mid: f32) -> f32 {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let style = TextStyle::new(12.0, 400.0, 1.5);
    let rect = Rect::from_min_size(pos2(x, mid - 15.0), vec2(pill.width, 30.0));
    let painter = ui.painter().clone();
    painter.rect(
        rect,
        corner(15.0),
        mix(c.bg, c.live, 0.10),
        Stroke::new(1.0, mix(c.border, c.live, 0.45)),
        StrokeKind::Inside,
    );
    let mut cx = rect.left() + 1.0 + 6.0;
    let b = Rect::from_min_size(pos2(cx, mid - 9.0), vec2(pill.badge.width() + 12.0, 18.0));
    painter.rect_filled(b, corner(9.0), c.live);
    paint_mid(&painter, &pill.badge, b.left() + 6.0, mid);
    cx = b.right() + 8.0;
    let link_rect = Rect::from_min_size(pos2(cx, mid - 9.0), vec2(pill.link.width(), 18.0));
    let response = common::click(ui, link_rect, Id::unique(("live-pill-link", server)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, &pill.link_text));
    let response = response.on_hover_text(&pill.title);
    paint_mid(&painter, &pill.link, cx, mid);
    if response.hovered() {
        painter.hline(
            cx..=cx + pill.link.width(),
            mid + style.baseline() - style.line_px() / 2.0 + 1.5,
            Stroke::new(1.0, c.text),
        );
    }
    let channel = pill.channel;
    if response.clicked() {
        app.nav.navigate(&format!("/s/{server}/c/{channel}"));
    }
    cx += pill.link.width();
    if let Some((label, block)) = pill.action {
        cx += 8.0;
        let r = Rect::from_min_size(pos2(cx, mid - 12.0), vec2(block.width() + 20.0, 24.0));
        come_button(
            ui,
            app,
            server,
            channel,
            &pill.others,
            label,
            &block,
            r,
            mid,
        );
    }
    rect.right()
}

/// The action's word for a Live in `channel` (watchLive.ts `watchLabel`):
/// "Zuschauen" joins and watches, or watches in that call; in another call
/// it only opens the channel, where the room asks before it switches.
pub(crate) fn watch_label(
    joined: Option<(Uuid, Uuid)>,
    server: Uuid,
    channel: Uuid,
) -> &'static str {
    match joined {
        Some(here) if here != (server, channel) => texts::watch_to_channel(),
        _ => texts::watch_watch(),
    }
}

/// Into the voice channel of a Live, as `joinToWatch` (watchLive.ts):
/// outside a call it joins and watches `people` from the start.
pub fn join_to_watch(app: &mut AppCtx, server: Uuid, channel: Uuid, people: &[Uuid]) {
    app.nav.navigate(&format!("/s/{server}/c/{channel}"));
    match app.call.with_state(session::joined) {
        None => {
            app.call.join(server, channel);
            for user in people {
                app.call.watch(*user, true);
            }
        }
        Some(here) if here == (server, channel) => {
            for user in people {
                app.call.watch(*user, true);
            }
        }
        Some(_) => {}
    }
}

/// "Zuschauen" / "Zum Kanal" (`.lr-live-pill > button`): a neutral
/// button, watching is no alarm.
#[allow(clippy::too_many_arguments)]
fn come_button(
    ui: &mut Ui,
    app: &mut AppCtx,
    server: Uuid,
    channel: Uuid,
    people: &[Uuid],
    label: &'static str,
    block: &Block,
    r: Rect,
    mid: f32,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let response = common::click(ui, r, Id::unique(("live-come", server, channel)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = if response.hovered() {
        c.hover
    } else {
        c.surface
    };
    ui.painter().rect(
        r,
        corner(r.height() / 2.0),
        fill,
        Stroke::new(1.0, c.border_strong),
        StrokeKind::Inside,
    );
    paint_mid(ui.painter(), block, r.left() + 10.0, mid);
    if response.clicked() {
        join_to_watch(app, server, channel, people);
    }
}

/// Draws the pill from `x` centred on `mid` when someone is live on
/// `server`; returns its right edge.
pub fn pill(
    ui: &mut Ui,
    app: &mut AppCtx,
    server: Uuid,
    x: f32,
    mid: f32,
    max_right: f32,
) -> Option<f32> {
    let pill = measure(ui, app, server, max_right - x)?;
    Some(paint_pill(ui, app, server, pill, x, mid))
}

/// The pill ending at `right` (`margin-left: auto`, a text channel's
/// header), no further left than `min_left`; returns its left edge.
pub fn pill_right(
    ui: &mut Ui,
    app: &mut AppCtx,
    server: Uuid,
    right: f32,
    mid: f32,
    min_left: f32,
) -> Option<f32> {
    let pill = measure(ui, app, server, right - min_left)?;
    let x = right - pill.width;
    paint_pill(ui, app, server, pill, x, mid);
    Some(x)
}

/// The window is narrow: the strip shows instead of the pill (chat.css,
/// `max-width: 800px`).
pub fn narrow(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() <= 800.0
}

const STRIP: TextStyle = TextStyle::new(13.0, 400.0, 1.5);

/// How tall the strip is for `server`: 0 when nobody is live.
pub fn strip_height(app: &AppCtx, server: Uuid) -> f32 {
    let n = entries(app, server).0.len();
    if n == 0 {
        return 0.0;
    }
    // Padding 6, rows of 28 two apart, the border.
    12.0 + n as f32 * 28.0 + (n - 1) as f32 * 2.0 + 1.0
}

/// `LiveHint` as a strip over the message pane (`.lr-live-hint`): per voice
/// channel, "Mia ist live in Sofa." and the way to come along.
pub fn strip(ui: &mut Ui, app: &mut AppCtx, server: Uuid, rect: Rect) {
    let (list, allowed, joined) = entries(app, server);
    if list.is_empty() {
        return;
    }
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, mix(c.bg, c.live, 0.09));
    crate::widgets::rule_h(&painter, c.border, rect.x_range(), rect.bottom() - 1.0);
    let badge_style = TextStyle::new(10.0, 700.0, 1.5).tracking(0.04);
    let mut y = rect.top() + 6.0;
    for entry in list {
        let mid = y + 14.0;
        let mut x = rect.left() + 20.0;
        let badge = Block::single(ui, "LIVE", badge_style, c.on_live);
        let b = Rect::from_min_size(pos2(x, mid - 9.0 - 1.0), vec2(badge.width() + 12.0, 18.0));
        painter.rect_filled(b, corner(9.0), c.live);
        paint_mid(&painter, &badge, b.left() + 6.0, b.center().y);
        x = b.right() + 8.0;
        let sentence = summary::live_sentence(&entry.who, entry.own);
        // The template is split before the names go in, so a name cannot
        // add a link.
        let template = match &entry.name {
            Some(_) => texts::hint_sentence_in("{sentence}", "{channel}"),
            None => texts::hint_sentence("{sentence}"),
        };
        for piece in gelabber_i18n::pieces(&template) {
            let Some(name) = entry.name.as_ref().filter(|_| piece.tag.is_some()) else {
                let text = piece.text.replace("{sentence}", &sentence);
                let block = Block::single(ui, &text, STRIP, c.text);
                paint_mid(&painter, &block, x, mid);
                x += block.width();
                continue;
            };
            let name = piece.text.replace("{channel}", name);
            let link = Block::single(ui, &name, STRIP.weight(600.0), c.text);
            let r = Rect::from_min_size(pos2(x, mid - 10.0), vec2(link.width(), 20.0));
            let response = common::click(ui, r, Id::unique(("live-strip-link", entry.channel)));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, &name));
            paint_mid(&painter, &link, x, mid);
            // `text-underline-offset: 2px`.
            painter.hline(
                x..=x + link.width(),
                mid + STRIP.baseline() - STRIP.line_px() / 2.0 + 2.0,
                Stroke::new(1.0, c.text),
            );
            if response.clicked() {
                app.nav
                    .navigate(&format!("/s/{server}/c/{}", entry.channel));
            }
            x += link.width();
        }
        if !entry.here && allowed && !entry.others.is_empty() {
            let label = watch_label(joined, server, entry.channel);
            let style = TextStyle::new(12.0, 650.0, 1.5);
            let block = Block::single(ui, label, style, c.text);
            let w = block.width() + 20.0;
            let r = Rect::from_min_size(pos2(rect.right() - 20.0 - w, mid - 12.0), vec2(w, 24.0));
            come_button(
                ui,
                app,
                server,
                entry.channel,
                &entry.others,
                label,
                &block,
                r,
                mid,
            );
        }
        y += 28.0 + 2.0;
    }
}
