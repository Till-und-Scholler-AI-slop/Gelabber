//! `LiveHint variant="pill"` (voice/LiveHint.tsx, chat.css `.lr-live-pill`):
//! in a channel header, who is live where, and the way to come along. A
//! Live is watched in its voice channel, so the action joins it (or, in
//! another call, opens it).

use egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
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
    others: bool,
    here: bool,
}

fn entries(app: &AppCtx, server: Uuid) -> (Vec<Entry>, bool, bool) {
    let joined = app.call.with_state(session::joined);
    app.client.with_state(|s| {
        let me = s.me().map(|m| m.id);
        let detail = s.detail(server);
        let name_of = |u: &Uuid| {
            detail
                .and_then(|d| d.members.iter().find(|m| m.user_id == *u))
                .map_or("Jemand".to_owned(), |m| m.name.clone())
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
                            who.push("Du".into());
                        }
                        who.extend(people.iter().filter(|u| Some(**u) != me).map(name_of));
                        Entry {
                            channel: *channel,
                            name: s.channel(*channel).map(|c| c.name.clone()),
                            own,
                            who,
                            others: people.iter().any(|u| Some(*u) != me),
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
        (out, allowed, joined.is_some())
    })
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
    let (list, allowed, in_call) = entries(app, server);
    let first = list.first()?;
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let badge_style = TextStyle::new(10.0, 700.0, 1.5).tracking(0.04);
    let style = TextStyle::new(12.0, 400.0, 1.5);
    let badge = Block::single(ui, "LIVE", badge_style, Color32::WHITE);
    let more = list.len() - 1;
    let place = first
        .name
        .as_ref()
        .map(|n| format!(" in {n}"))
        .unwrap_or_default();
    let link_text = format!(
        "{}{place}{}",
        summary::name_list(&first.who),
        if more > 0 {
            format!(" +{more}")
        } else {
            String::new()
        }
    );
    let action = (!first.here && allowed && first.others).then_some(if in_call {
        "Zum Kanal"
    } else {
        "Beitreten"
    });
    let action_block = action.map(|a| Block::single(ui, a, style.weight(650.0), Color32::WHITE));
    let fixed = 1.0
        + 6.0
        + badge.width()
        + 12.0
        + 8.0
        + action_block
            .as_ref()
            .map_or(0.0, |b| 8.0 + b.width() + 20.0)
        + 4.0
        + 1.0;
    let link_max = (max_right - x - fixed).max(20.0);
    let link = common::line(ui, &link_text, style, c.text, link_max);
    let w = fixed + link.width();
    let rect = Rect::from_min_size(pos2(x, mid - 15.0), vec2(w, 30.0));
    let painter = ui.painter().clone();
    painter.rect(
        rect,
        corner(15.0),
        mix(c.bg, c.danger_bg, 0.10),
        Stroke::new(1.0, mix(c.border, c.danger_bg, 0.45)),
        StrokeKind::Inside,
    );
    let mut cx = rect.left() + 1.0 + 6.0;
    let b = Rect::from_min_size(pos2(cx, mid - 9.0), vec2(badge.width() + 12.0, 18.0));
    painter.rect_filled(b, corner(9.0), c.danger_bg);
    paint_mid(&painter, &badge, b.left() + 6.0, mid);
    cx = b.right() + 8.0;
    let link_rect = Rect::from_min_size(pos2(cx, mid - 9.0), vec2(link.width(), 18.0));
    let response = common::click(ui, link_rect, Id::unique(("live-pill-link", server)));
    let title = format!("{}{place}", summary::live_sentence(&first.who, first.own));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, &link_text));
    let response = response.on_hover_text(&title);
    paint_mid(&painter, &link, cx, mid);
    if response.hovered() {
        painter.hline(
            cx..=cx + link.width(),
            mid + style.baseline() - style.line_px() / 2.0 + 1.5,
            Stroke::new(1.0, c.text),
        );
    }
    let channel = first.channel;
    if response.clicked() {
        app.nav.navigate(&format!("/s/{server}/c/{channel}"));
    }
    cx += link.width();
    if let (Some(label), Some(block)) = (action, action_block) {
        cx += 8.0;
        let r = Rect::from_min_size(pos2(cx, mid - 12.0), vec2(block.width() + 20.0, 24.0));
        let response = common::click(ui, r, Id::unique(("live-pill-action", server)));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        let fill = if response.hovered() {
            mix(c.danger_bg, Color32::WHITE, 0.1)
        } else {
            c.danger_bg
        };
        painter.rect_filled(r, corner(12.0), fill);
        paint_mid(&painter, &block, r.left() + 10.0, mid);
        if response.clicked() {
            app.nav.navigate(&format!("/s/{server}/c/{channel}"));
            if !in_call {
                app.call.join(server, channel);
            }
        }
    }
    Some(rect.right())
}
