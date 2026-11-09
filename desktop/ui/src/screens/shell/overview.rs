//! The server's front door, `/s/{id}` (pages/CommunityOverview.tsx,
//! overview.css): "Übersicht" bar, name and member line, the voice room
//! with its people and "Beitreten", and what is unread.

use chrono::{DateTime, Datelike, Local, Utc};
use egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{Channel, ChannelKind, HistoryQuery, Message, Permission, ServerDetail};
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::state::{self, Preview};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::voice;
use crate::task::Pending;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::{Button, IconButton, hairline_h};

/// `UNREAD_SHOWN`.
const UNREAD_SHOWN: usize = 5;

pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, server: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, c.bg);
    let detail = app.client.with_state(|s| s.detail(server).cloned());
    // `ServerPage`: without the server, its error in place of the page.
    if detail.is_none() && load_error(ui, app, rect, server) {
        return;
    }

    // The bar.
    icons::paint(
        &painter,
        Icon::Chat,
        pos2(rect.left() + 24.0, rect.top() + 16.5),
        18.0,
        c.muted,
    );
    let title = Block::single(ui, "Übersicht", text::TITLE_SM.size(15.0), c.text);
    paint_mid(
        &painter,
        &title,
        rect.left() + 24.0 + 18.0 + 10.0,
        rect.top() + 25.5,
    );
    hairline_h(
        &painter,
        &theme,
        rect.left()..=rect.right(),
        rect.top() + 51.0,
    );

    let Some(detail) = detail else {
        skeleton(ui, rect);
        return;
    };
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 52.0), rect.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt(("overview-scroll", server))
        .max_height(body.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let width = (body.width().min(760.0) - 64.0).max(0.0);
            let left = body.left() + 32.0;
            let mut y = body.top() + 32.0;
            y = head(ui, app, &theme, left, y, width, &detail);
            y += 32.0;
            y = voice_room(ui, app, &theme, left, y, width, &detail);
            if let Some(bottom) = unread(ui, app, &theme, left, y + 32.0, width, &detail) {
                y = bottom;
            }
            ui.allocate_exact_size(
                Vec2::new(body.width(), y + 32.0 - body.top()),
                egui::Sense::hover(),
            );
        });

    // A server just created asks for its first invite once its page is up.
    let prompt = state::with(ui.ctx(), |s| {
        s.invite_prompt.take_if(|(id, _)| *id == server)
    });
    if let Some((id, name)) = prompt {
        super::dialogs::open_invite(ui.ctx(), id, &name);
    }
}

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
    let msg = "Der Server konnte nicht geladen werden.";
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
    let label = "Erneut versuchen";
    let bw = Block::single(ui, label, text::UI.weight(600.0), c.text).width() + 26.0;
    let br = Rect::from_min_size(
        pos2(r.left() + 15.0, r.top() + 10.0 + block.height() + 12.0),
        Vec2::new(bw, 32.0),
    );
    let button = Button::new(label)
        .small()
        .pending(pending.then_some("Wird geladen…"));
    if ui.put(br, button).clicked() && !pending {
        app.client.refresh_server(server);
    }
    true
}

/// The channel's name in an unread row, with the pencil of a draft
/// (`channel-row-draft`, "Entwurf"); returns the width taken.
pub fn unread_name(ui: &Ui, rect: Rect, name: &str, draft: bool) -> f32 {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let max = if draft { 120.0 - 13.0 - 6.0 } else { 120.0 };
    let block = line(ui, name, text::BODY.weight(600.0), c.text, max);
    let cy = rect.center().y;
    paint_mid(ui.painter(), &block, rect.left(), cy);
    let mut w = block.width();
    if draft {
        let pencil = Rect::from_min_size(pos2(rect.left() + w + 6.0, cy - 6.5), Vec2::splat(13.0));
        icons::paint(ui.painter(), Icon::Pencil, pencil.min, 13.0, c.muted);
        let response = ui.interact(
            pencil,
            Id::unique(("overview-draft", name)),
            egui::Sense::hover(),
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Entwurf"));
        response.on_hover_text("Entwurf");
        w += 6.0 + 13.0;
    }
    w
}

/// A black scrim fading in from `rect`'s top to `alpha` at its bottom,
/// whose lower corners follow the card's `radius` (`overflow: hidden`).
fn bottom_scrim(painter: &egui::Painter, rect: Rect, radius: f32, alpha: f32) {
    let steps = 24;
    let mut mesh = egui::Mesh::default();
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let y = rect.top() + t * rect.height();
        let dy = y - (rect.bottom() - radius);
        let inset = if dy > 0.0 {
            radius - (radius * radius - dy * dy).max(0.0).sqrt()
        } else {
            0.0
        };
        let color = Color32::from_black_alpha((alpha * t * 255.0).round() as u8);
        mesh.colored_vertex(pos2(rect.left() + inset, y), color);
        mesh.colored_vertex(pos2(rect.right() - inset, y), color);
        if i > 0 {
            let b = 2 * i as u32;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b + 1, b);
        }
    }
    painter.add(mesh);
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

/// The name, "{n} Mitglieder · …", and the invite nudge for a lone owner.
#[allow(clippy::too_many_arguments)]
fn head(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    left: f32,
    top: f32,
    width: f32,
    detail: &ServerDetail,
) -> f32 {
    let c = theme.c;
    let name = Block::wrapped(
        ui,
        &detail.server.name,
        text::PAGE.tracking(-0.01),
        c.text,
        width,
    );
    name.paint(ui.painter(), pos2(left, top));
    let mut y = top + name.height() + 6.0;
    let line_text = overview_line(app, detail);
    let meta = TextStyle::new(13.0, 400.0, 1.4);
    let block = Block::wrapped(ui, &line_text, meta, c.muted, width);
    block.paint(ui.painter(), pos2(left, y));
    y += block.height();
    if detail.members.len() == 1 {
        y += 10.0;
        let alone = Block::single(ui, "Noch keine weiteren Mitglieder.", meta, c.muted);
        let row_h = 32.0;
        paint_mid(ui.painter(), &alone, left, y + row_h / 2.0);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(left + alone.width() + 16.0, y),
            Vec2::new(200.0, row_h),
        )));
        if child.add(Button::new("Einladen").small()).clicked() {
            super::dialogs::open_invite(ui.ctx(), detail.server.id, &detail.server.name);
        }
        y += row_h;
    }
    y
}

/// `overviewLine` (servers/overview.ts).
fn overview_line(app: &AppCtx, detail: &ServerDetail) -> String {
    let server = detail.server.id;
    let count = detail.members.len();
    let members = format!(
        "{count} {}",
        if count == 1 { "Mitglied" } else { "Mitglieder" }
    );
    let rooms: Vec<Uuid> = detail
        .channels
        .iter()
        .filter(|c| c.kind == ChannelKind::Voice)
        .map(|c| c.id)
        .collect();
    let own = voice::session::own_seat(app, server).map(|(ch, _, _)| ch);
    let (inside, spread) = app.client.with_state(|s| {
        let me = s.me().map(|m| m.id);
        let mut where_: Vec<Uuid> = Vec::new();
        for m in &detail.members {
            let channel = if Some(m.user_id) == me {
                own
            } else {
                s.voice
                    .get(&server)
                    .and_then(|r| r.get(&m.user_id))
                    .map(|o| o.channel_id)
            };
            if let Some(ch) = channel.filter(|ch| rooms.contains(ch)) {
                where_.push(ch);
            }
        }
        let mut distinct = where_.clone();
        distinct.sort();
        distinct.dedup();
        (where_.len(), distinct.len() > 1)
    });
    if inside == 0 {
        return members;
    }
    let place = if spread {
        "in Sprachkanälen"
    } else {
        "im Sprachkanal"
    };
    format!("{members} · {inside} {place}")
}

/// `overviewRoom`: the current call, the last visited room, an occupied
/// room, or the first.
fn overview_room<'a>(app: &AppCtx, ui: &Ui, detail: &'a ServerDetail) -> Option<&'a Channel> {
    let server = detail.server.id;
    let rooms: Vec<&Channel> = detail
        .channels
        .iter()
        .filter(|c| c.kind == ChannelKind::Voice)
        .collect();
    let active = voice::session::own_seat(app, server).map(|(ch, _, _)| ch);
    let occupied = app.client.with_state(|s| {
        let occupied: Vec<Uuid> = s
            .voice
            .get(&server)
            .map(|r| {
                r.iter()
                    .filter(|(u, _)| detail.members.iter().any(|m| m.user_id == **u))
                    .map(|(_, o)| o.channel_id)
                    .collect()
            })
            .unwrap_or_default();
        occupied
    });
    let user = app.client.with_state(|s| s.me().map(|m| m.id));
    let remembered = state::memory(ui.ctx(), user).channels.get(&server).copied();
    active
        .and_then(|id| rooms.iter().find(|c| c.id == id).copied())
        .or_else(|| remembered.and_then(|id| rooms.iter().find(|c| c.id == id).copied()))
        .or_else(|| rooms.iter().find(|c| occupied.contains(&c.id)).copied())
        .or_else(|| rooms.first().copied())
}

/// The voice room part (`VoiceRoom overview`): heading, the people as
/// 168×136 tiles, then "Beitreten" with the microphone test and settings.
#[allow(clippy::too_many_arguments)]
fn voice_room(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    left: f32,
    top: f32,
    width: f32,
    detail: &ServerDetail,
) -> f32 {
    let c = theme.c;
    let server = detail.server.id;
    let Some(room) = overview_room(app, ui, detail).cloned() else {
        // "Kein Sprachkanal".
        icons::paint(
            ui.painter(),
            Icon::Speaker,
            pos2(left, top + 5.0),
            18.0,
            c.muted,
        );
        let h2 = Block::single(ui, "Kein Sprachkanal", text::TITLE_SM, c.text);
        paint_mid(ui.painter(), &h2, left + 28.0, top + 14.0);
        let mut y = top + 28.0;
        let can = app.client.with_state(|s| {
            s.server(server)
                .is_some_and(|v| v.can(Permission::ManageChannels))
        });
        if can {
            y += 6.0;
            let p = Block::wrapped(
                ui,
                "Erstelle einen Sprachkanal in der Seitenleiste.",
                TextStyle::new(13.0, 400.0, 1.4),
                c.muted,
                width,
            );
            p.paint(ui.painter(), pos2(left, y));
            y += p.height();
        }
        return y;
    };
    let call = app.call.state();
    let joined = voice::session::joined(&call);
    let (here, elsewhere, occupants, live_any, allowed, me, elsewhere_name) =
        app.client.with_state(|s| {
            let here = joined == Some((server, room.id));
            let elsewhere = joined.is_some() && !here;
            let mut occupants = voice::session::occupants(s, &call, server, room.id, false);
            // The roster's order: the server lists seats by user id.
            occupants.sort_by_key(|(u, _)| *u);
            let live_any = s
                .live
                .get(&server)
                .and_then(|l| l.get(&room.id))
                .is_some_and(|u| !u.is_empty());
            let allowed = s
                .server(server)
                .is_none_or(|v| v.can(Permission::JoinVoice));
            let elsewhere_name = joined.and_then(|(_, ch)| s.channel(ch).map(|c| c.name.clone()));
            (
                here,
                elsewhere,
                occupants,
                live_any,
                allowed,
                s.me().map(|m| m.id),
                elsewhere_name,
            )
        });

    // Heading row (min-height 28).
    let center = top + 14.0;
    icons::paint(
        ui.painter(),
        Icon::Speaker,
        pos2(left, center - 9.0),
        18.0,
        if here { c.online } else { c.muted },
    );
    let h2 = line(ui, &room.name, text::TITLE_SM, c.text, width - 28.0);
    paint_mid(ui.painter(), &h2, left + 28.0, center);
    let mut x = left + 28.0 + h2.width() + 10.0;
    if here {
        let style = text::META.weight(500.0);
        let label = Block::single(ui, "Verbunden", style, c.online_text);
        let pill = Rect::from_min_size(
            pos2(x, center - 11.0),
            Vec2::new(8.0 + 6.0 + 6.0 + label.width() + 8.0, 22.0),
        );
        ui.painter()
            .rect_filled(pill, corner(theme.radius.full), mix(c.bg, c.online, 0.14));
        ui.painter()
            .circle_filled(pos2(pill.left() + 11.0, center), 3.0, c.online_text);
        paint_mid(ui.painter(), &label, pill.left() + 20.0, center);
        x = pill.right() + 10.0;
    }
    if live_any {
        let label = Block::single(ui, "LIVE", text::CAPS, c.on_live);
        let badge =
            Rect::from_min_size(pos2(x, center - 9.0), Vec2::new(label.width() + 12.0, 18.0));
        ui.painter()
            .rect_filled(badge, corner(theme.radius.xs), c.live);
        paint_mid(ui.painter(), &label, badge.left() + 6.0, center);
    }
    let mut y = top + 28.0;

    // People: wrapped tiles, gap 12.
    if !occupants.is_empty() {
        y += 12.0;
        let per_row = (((width + 12.0) / 180.0).floor() as usize).max(1);
        for (i, (user, occupant)) in occupants.iter().enumerate() {
            let col = i % per_row;
            let row = i / per_row;
            let tile = Rect::from_min_size(
                pos2(left + col as f32 * 180.0, y + row as f32 * 148.0),
                Vec2::new(168.0, 136.0),
            );
            let member = detail.members.iter().find(|m| m.user_id == *user);
            let name = member.map_or("Jemand", |m| m.name.as_str());
            let speaking =
                voice::session::speaking(&call, *user, occupant.muted, occupant.deafened);
            if speaking {
                ui.painter().rect_stroke(
                    tile.expand(0.5),
                    corner(theme.radius.lg + 0.5),
                    Stroke::new(1.0, c.speaking),
                    StrokeKind::Middle,
                );
            }
            ui.painter().rect(
                tile,
                corner(theme.radius.lg),
                c.surface,
                Stroke::new(1.0, if speaking { c.speaking } else { c.border }),
                StrokeKind::Inside,
            );
            // In the call a camera fills its owner's tile, the name over it.
            let camera = here
                .then(|| {
                    call.participants
                        .iter()
                        .find(|p| p.user == *user)
                        .and_then(|p| p.camera)
                })
                .flatten()
                .and_then(|key| app.call.frame(key).map(|f| (key, f)));
            if let Some((key, frame)) = camera {
                // `.has-video`: no border, the picture to the card's edge.
                let inner = tile;
                ui.painter().rect_filled(
                    tile,
                    corner(theme.radius.lg),
                    Color32::from_rgb(0x0b, 0x0d, 0x0e),
                );
                voice::video::request(
                    ui,
                    app.call,
                    key,
                    voice::video::drawn_rect(inner, &frame, voice::video::Fit::Cover),
                    15,
                );
                voice::video::paint(
                    ui,
                    Id::unique(("overview-camera", *user)),
                    inner,
                    frame,
                    voice::video::Fit::Cover,
                    me == Some(*user),
                    theme.radius.lg,
                );
                // Name and status over a scrim at the bottom (padding 18 44 6 10).
                let white = Color32::WHITE;
                let soft = Color32::from_white_alpha((0.82 * 255.0) as u8);
                let name_b = line(ui, name, text::UI.weight(600.0), white, 168.0 - 54.0);
                let self_b =
                    (me == Some(*user)).then(|| line(ui, " (du)", text::UI, white, 168.0 - 54.0));
                let status = if occupant.deafened {
                    "Taub"
                } else if occupant.muted {
                    "Stumm"
                } else if speaking {
                    "Spricht"
                } else {
                    "Im Raum"
                };
                let status_b = line(
                    ui,
                    &format!("{status} · Kamera an"),
                    text::META,
                    soft,
                    168.0 - 54.0,
                );
                let h = 18.0 + name_b.height() + status_b.height() + 6.0;
                let scrim = Rect::from_min_max(pos2(tile.left(), tile.bottom() - h), tile.max);
                bottom_scrim(ui.painter(), scrim, theme.radius.lg, 0.7);
                let ny = scrim.top() + 18.0;
                name_b.paint(ui.painter(), pos2(tile.left() + 10.0, ny));
                if let Some(b) = self_b {
                    // The overview's `.voice-room-self` stays muted over a picture too.
                    b.paint_tinted(
                        ui.painter(),
                        pos2(tile.left() + 10.0 + name_b.width(), ny),
                        Some(c.muted),
                    );
                }
                status_b.paint(ui.painter(), pos2(tile.left() + 10.0, ny + name_b.height()));
                let mut mx = tile.left() + 8.0;
                for (on, icon) in [
                    (occupant.muted || occupant.deafened, Icon::MicOff),
                    (occupant.deafened, Icon::HeadsetOff),
                ] {
                    if on {
                        icons::paint(ui.painter(), icon, pos2(mx, tile.top() + 8.0), 14.0, soft);
                        mx += 16.0;
                    }
                }
                let _ = common::click(ui, tile, Id::unique(("overview-person", *user)));
                continue;
            }
            let label_style = text::UI.weight(600.0);
            let name_block = line(ui, name, label_style, c.text, 168.0 - 24.0);
            let content_h = 56.0 + 8.0 + name_block.height();
            let ctop = tile.center().y - content_h / 2.0;
            let avatar = Rect::from_min_size(pos2(tile.center().x - 28.0, ctop), Vec2::splat(56.0));
            if speaking {
                voice::look::speaking_ring(ui, &ui.painter().clone(), avatar, true);
            }
            common::avatar(
                ui,
                app.images,
                avatar,
                name,
                member.and_then(|m| m.avatar_url.as_deref()),
                20.0,
                1.0,
                c.surface,
            );
            // `.voice-room-self`: " (du)" regular and muted after the name.
            let self_b = (me == Some(*user)).then(|| {
                line(
                    ui,
                    " (du)",
                    text::UI,
                    c.muted,
                    (168.0 - 24.0 - name_block.width()).max(0.0),
                )
            });
            let total = name_block.width() + self_b.as_ref().map_or(0.0, |b| b.width());
            let lx = tile.center().x - total / 2.0;
            name_block.paint(ui.painter(), pos2(lx, avatar.bottom() + 8.0));
            if let Some(b) = self_b {
                b.paint(
                    ui.painter(),
                    pos2(lx + name_block.width(), avatar.bottom() + 8.0),
                );
            }
            let mut mx = tile.left() + 8.0;
            if occupant.muted || occupant.deafened {
                icons::paint(
                    ui.painter(),
                    Icon::MicOff,
                    pos2(mx, tile.top() + 8.0),
                    14.0,
                    c.muted,
                );
                mx += 16.0;
            }
            if occupant.deafened {
                icons::paint(
                    ui.painter(),
                    Icon::HeadsetOff,
                    pos2(mx, tile.top() + 8.0),
                    14.0,
                    c.muted,
                );
            }
            let _ = common::click(ui, tile, Id::unique(("overview-person", *user))).on_hover_text(
                if me == Some(*user) {
                    "Dein Profil"
                } else {
                    "Profil und Lautstärke"
                },
            );
        }
        let rows = occupants.len().div_ceil(per_row);
        y += rows as f32 * 136.0 + (rows.saturating_sub(1)) as f32 * 12.0;
    }

    if !here && allowed && elsewhere {
        y += 12.0;
        let note = Block::wrapped(
            ui,
            &format!(
                "Du verlässt dabei {}.",
                elsewhere_name.as_deref().unwrap_or("deinen Sprachkanal")
            ),
            text::META,
            c.muted,
            width,
        );
        note.paint(ui.painter(), pos2(left, y));
        y += note.height();
    }
    if here {
        return y;
    }

    // Actions: margin-top 16, gap 8.
    y += 16.0;
    let row = Rect::from_min_size(pos2(left, y), Vec2::new(width, 44.0));
    let mut x = left;
    if allowed {
        let label = if elsewhere {
            "Hierher wechseln"
        } else {
            "Beitreten"
        };
        let join = voice::chips::join_button(ui, theme, row, label, Id::unique("overview-join"));
        if join.clicked() {
            app.call.join(server, room.id);
        }
        x = join.rect.right() + 8.0;
    } else {
        let p = Block::single(
            ui,
            "Keine Berechtigung für Sprachkanäle.",
            text::UI,
            c.muted,
        );
        paint_mid(ui.painter(), &p, x, row.center().y);
        x += p.width() + 8.0;
    }
    let mic = Rect::from_min_size(pos2(x, row.center().y - 18.0), Vec2::splat(36.0));
    if ui
        .put(
            mic,
            IconButton::new(Icon::Mic, "Mikrofon testen")
                .secondary()
                .icon_size(18.0),
        )
        .clicked()
    {
        voice::dialogs::open_mic_test(ui.ctx());
    }
    let gear = mic.translate(Vec2::new(36.0 + 8.0, 0.0));
    let response = ui.put(
        gear,
        IconButton::new(Icon::Gear, "Voice-Einstellungen")
            .secondary()
            .icon_size(17.0),
    );
    voice::quick::button(ui, app, response, gear);
    y + 44.0
}

/// The "Ungelesen" list; `None` when nothing is unread.
#[allow(clippy::too_many_arguments)]
fn unread(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    left: f32,
    top: f32,
    width: f32,
    detail: &ServerDetail,
) -> Option<f32> {
    let c = theme.c;
    let now = Utc::now();
    let server = detail.server.id;
    let mut channels: Vec<(usize, Channel, Option<i64>, gelabber_client::Badge)> =
        app.client.with_state(|s| {
            detail
                .channels
                .iter()
                .enumerate()
                .filter(|(_, ch)| ch.kind == ChannelKind::Text)
                .filter_map(|(i, ch)| {
                    let badge = s.channel_badge(ch.id, now);
                    (badge.count > 0 || badge.dot).then(|| {
                        (
                            i,
                            ch.clone(),
                            s.read.get(&ch.id).and_then(|r| r.latest_order),
                            badge,
                        )
                    })
                })
                .collect()
        });
    if channels.is_empty() {
        return None;
    }
    channels.sort_by(|a, b| {
        b.2.unwrap_or(i64::MIN)
            .cmp(&a.2.unwrap_or(i64::MIN))
            .then(a.0.cmp(&b.0))
    });
    channels.truncate(UNREAD_SHOWN);

    let title = Block::single(ui, "UNGELESEN", text::CAPS, c.muted);
    title.paint(ui.painter(), pos2(left, top));
    let list_top = top + title.height() + 10.0;
    let list = Rect::from_min_size(
        pos2(left, list_top),
        Vec2::new(
            width,
            2.0 + 52.0 * channels.len() as f32 + (channels.len() - 1) as f32,
        ),
    );
    ui.painter().rect(
        list,
        corner(theme.radius.lg),
        c.surface,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let members: Vec<(Uuid, String)> = detail
        .members
        .iter()
        .map(|m| (m.user_id, m.name.clone()))
        .collect();
    let me = app.client.with_state(|s| s.me().map(|m| m.id));
    let drafts = crate::screens::chat::drafts(ui.ctx());
    let mut y = list.top() + 1.0;
    let count = channels.len();
    for (i, (_, ch, latest, badge)) in channels.into_iter().enumerate() {
        let row = Rect::from_min_size(pos2(list.left() + 1.0, y), Vec2::new(width - 2.0, 52.0));
        let response = common::click(ui, row, Id::unique(("overview-unread", ch.id)));
        if response.hovered() {
            // The list clips its rows to its rounded corners.
            let r = theme.radius.lg as u8 - 1;
            let rounding = egui::CornerRadius {
                nw: if i == 0 { r } else { 0 },
                ne: if i == 0 { r } else { 0 },
                sw: if i + 1 == count { r } else { 0 },
                se: if i + 1 == count { r } else { 0 },
            };
            ui.painter().rect_filled(row, rounding, c.hover);
        }
        let inner_left = row.left() + 14.0;
        let inner_right = row.right() - 14.0;
        let cy = row.center().y;
        icons::paint(
            ui.painter(),
            Icon::Hash,
            pos2(inner_left, cy - 9.0),
            18.0,
            c.muted,
        );
        let name_rect = Rect::from_min_size(
            pos2(inner_left + 30.0, row.top()),
            Vec2::new(120.0, row.height()),
        );
        unread_name(ui, name_rect, &ch.name, drafts.contains_key(&ch.id));
        // Badge column: min 18, the dot or the count right-aligned.
        let badge_w = if badge.count > 0 {
            let label = if badge.count > 99 {
                "99+".to_owned()
            } else {
                badge.count.to_string()
            };
            let b = Block::single(ui, &label, TextStyle::new(11.0, 700.0, 1.0), Color32::WHITE);
            (b.width() + 10.0).max(18.0)
        } else {
            18.0
        };
        if badge.count > 0 {
            crate::widgets::count_badge(
                ui.painter(),
                ui,
                pos2(inner_right, cy + 9.0),
                badge.count.min(u32::MAX as i64) as u32,
                None,
            );
        } else if badge.dot {
            ui.painter()
                .circle_filled(pos2(inner_right - 4.0, cy), 4.0, c.accent);
        }
        let message = preview(ui, app, ch.id, latest);
        let mut preview_right = inner_right - badge_w - 12.0;
        if let Some(message) = &message {
            let when = short_when(message.created_at, Local::now());
            let time = Block::single(ui, &when, TextStyle::new(12.0, 400.0, 1.5), c.subtle);
            paint_mid(ui.painter(), &time, preview_right - time.width(), cy);
            preview_right -= time.width() + 12.0;
            let (author, body) = message_preview(message, &members, me);
            let preview_left = inner_left + 18.0 + 12.0 + 120.0 + 12.0;
            let style = TextStyle::new(13.0, 400.0, 1.5);
            let mut job = egui::text::LayoutJob::default();
            if let Some(author) = author {
                job.append(&format!("{author}: "), 0.0, style.format(c.text));
            }
            job.append(&body, 0.0, style.format(c.muted));
            job.wrap = egui::text::TextWrapping {
                max_width: (preview_right - preview_left).max(1.0),
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let block = Block::new(ui, job, style);
            paint_mid(ui.painter(), &block, preview_left, cy);
        } else {
            // Kept the place while the newest message loads.
            let _ = preview_right;
        }
        if i + 1 < count {
            hairline_h(ui.painter(), theme, row.left()..=row.right(), row.bottom());
        }
        if response.clicked() {
            app.nav.navigate(&format!("/s/{server}/c/{}", ch.id));
        }
        y += 53.0;
    }
    Some(list.bottom())
}

/// The newest message of `channel`: from a loaded history, else fetched
/// once per `latest` order (the list endpoint's smallest page).
fn preview(ui: &Ui, app: &AppCtx, channel: Uuid, latest: Option<i64>) -> Option<Message> {
    if let Some(message) = app.client.with_state(|s| {
        s.messages(channel)
            .filter(|h| h.loaded)
            .and_then(|h| h.messages.last().cloned())
    }) {
        return Some(message);
    }
    state::with(ui.ctx(), |s| {
        let entry = s.previews.entry(channel).or_insert_with(|| Preview {
            order: latest,
            shown: None,
            pending: Some(fetch(app, channel)),
        });
        if let Some(message) = entry.pending.as_ref().and_then(Pending::take) {
            entry.shown = message;
            entry.pending = None;
        }
        // A new message changes the order: ask again, the old line stays.
        if entry.order != latest && entry.pending.is_none() {
            entry.order = latest;
            entry.pending = Some(fetch(app, channel));
        }
        entry.shown.clone()
    })
}

fn fetch(app: &AppCtx, channel: Uuid) -> Pending<Option<Message>> {
    let http = app.client.http().clone();
    Pending::spawn(app.client, async move {
        http.messages(channel, &HistoryQuery::latest(1))
            .await
            .ok()
            .and_then(|page| page.messages.last().cloned())
    })
}

/// `messagePreview`: author ("Du" for yourself) and the excerpt with
/// mentions as names; "GIF"/"Anhang" when there is no text.
fn message_preview(
    message: &Message,
    members: &[(Uuid, String)],
    me: Option<Uuid>,
) -> (Option<String>, String) {
    if let Some(system) = &message.system {
        let text = match system.kind.as_str() {
            "live_started" => format!(
                "{} ist live in {}.",
                message.author.name,
                system
                    .params
                    .get("voice_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            ),
            _ => format!("Systemnachricht von {}", message.author.name),
        };
        return (None, text);
    }
    let mut text = readable_mentions(&gelabber_client::store::excerpt(&message.content), members);
    if text.is_empty() {
        text = if message.gif.is_some() {
            "GIF".into()
        } else if !message.attachments.is_empty() {
            "Anhang".into()
        } else {
            String::new()
        };
    }
    let author = if Some(message.author.id) == me {
        "Du".to_owned()
    } else {
        message.author.name.clone()
    };
    (Some(author), text)
}

/// `readableMentions`: `<@uuid>` as `@Name`, unknown ones `@Unbekannt`.
fn readable_mentions(content: &str, members: &[(Uuid, String)]) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let token = after
            .get(..36)
            .filter(|_| after.get(36..37) == Some(">"))
            .and_then(|id| {
                Uuid::parse_str(id)
                    .ok()
                    .filter(|_| id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()))
            });
        match token {
            Some(id) => {
                let name = members
                    .iter()
                    .find(|(m, _)| *m == id)
                    .map_or("Unbekannt", |(_, n)| n.as_str());
                out.push('@');
                out.push_str(name);
                rest = &after[37..];
            }
            None => {
                out.push_str("<@");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `shortWhen`: "18:28" today, "Gestern", then "21.10." ("21.10.25" in
/// another year).
pub fn short_when(at: DateTime<Utc>, now: DateTime<Local>) -> String {
    let local = at.with_timezone(&Local);
    if local.date_naive() == now.date_naive() {
        return local.format("%H:%M").to_string();
    }
    if Some(local.date_naive()) == now.date_naive().pred_opt() {
        return "Gestern".into();
    }
    if local.year() == now.year() {
        local.format("%d.%m.").to_string()
    } else {
        local.format("%d.%m.%y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_read_as_names() {
        let id = Uuid::parse_str("3796349f-6f8f-4bcf-826f-d2237e2fe9d3").unwrap();
        let members = vec![(id, "Lena".to_owned())];
        assert_eq!(
            readable_mentions("hi <@3796349f-6f8f-4bcf-826f-d2237e2fe9d3>!", &members),
            "hi @Lena!"
        );
        assert_eq!(
            readable_mentions("<@00000000-0000-0000-0000-000000000000>", &members),
            "@Unbekannt"
        );
        assert_eq!(readable_mentions("a <@ b", &members), "a <@ b");
    }

    #[test]
    fn short_times() {
        let now = Local::now();
        let at = now.with_timezone(&Utc);
        assert_eq!(short_when(at, now), now.format("%H:%M").to_string());
        let yesterday = (now - chrono::Duration::days(1)).with_timezone(&Utc);
        assert_eq!(short_when(yesterday, now), "Gestern");
    }
}
