//! The server rail (components/ServerRail.tsx, index.css `.server-rail`):
//! the DM button, one tile per server with its edge pill, unread count and
//! tooltip, and the dashed "Server hinzufügen" button under the list.

use egui::{Color32, CornerRadius, Id, Rect, Shape, Stroke, Ui, Vec2, pos2};
use gelabber_i18n::shell;
use uuid::Uuid;

use super::common;
use super::row_menu::{self, Owner};
use super::state::{self, AddStep};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::images::ImageState;
use crate::initials::{tile_initials, tile_tone};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, motion};
use crate::widgets::count_badge;
use crate::widgets::tooltip::{Side, Tooltip};

/// `TILE_PX`: one server row.
const ROW: f32 = 60.0;
const TILE: f32 = 44.0;

pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, active: Option<Uuid>, dm_active: bool) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, c.rail);
    crate::widgets::hairline_v(
        &painter,
        &theme,
        rect.right() - 1.0,
        rect.top()..=rect.bottom(),
    );
    let now = chrono::Utc::now();
    let user = app.client.with_state(|s| s.me().map(|m| m.id));
    let memory = state::memory(ui.ctx(), user);

    // Home / DM button: 44×40 in the 52px header line.
    let home = Rect::from_min_size(
        pos2(rect.left() + 10.0, rect.top() + 6.0),
        Vec2::new(TILE, 40.0),
    );
    let (dm_badge, last_dm) = app.client.with_state(|s| {
        let last = memory.dm.filter(|id| s.dm(*id).is_some());
        (s.dms_badge(now), last)
    });
    let home_id = Id::unique("rail-home");
    let response = common::click(ui, home, home_id).on_hover_text(shell::rail_dms());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, shell::rail_dms()));
    common::describe_unread(ui, &response, dm_badge);
    // Its menu marks every conversation read.
    if response.secondary_clicked() || row_menu::menu_key(ui, &response) {
        row_menu::open(ui.ctx(), Owner::AllDms, home, Some(home_id));
    }
    let lit = response.hovered() || dm_active;
    if lit {
        common::fill(&painter, home, theme.radius.md, c.hover);
    }
    common::focus_visible(ui, &response, home, theme.radius.md);
    icons::paint_centered(
        &painter,
        Icon::Chat,
        home,
        27.0,
        if lit { c.accent_text } else { c.text },
    );
    pill(
        ui,
        &painter,
        &theme,
        Id::unique("rail-home-pill"),
        rect.left(),
        home.center().y,
        dm_active,
        response.hovered(),
        dm_badge.count > 0 || dm_badge.dot,
    );
    if dm_badge.count > 0 {
        count_badge(
            &painter,
            ui,
            home.max + Vec2::splat(5.0),
            dm_badge.count.min(u32::MAX as i64) as u32,
            Some(c.rail),
        );
    }
    if response.clicked() {
        match last_dm {
            Some(id) => app.nav.navigate(&format!("/d/{id}")),
            None => app.nav.navigate("/d"),
        }
    }

    // Servers: 8px padding, 60px rows; the footer follows the list.
    let servers = app.client.with_state(|s| {
        s.servers
            .iter()
            .map(|server| {
                let badge = s.server_badge(server.id, now);
                let muted = s.notification_settings.iter().any(|n| {
                    n.scope == gelabber_client::SettingScope::Server
                        && n.id == server.id
                        && n.muted
                        && n.muted_until.is_none_or(|until| until > now)
                });
                let in_voice = s.voice.get(&server.id).is_some_and(|r| !r.is_empty());
                (
                    server.id,
                    server.name.clone(),
                    server.icon_url.clone(),
                    badge,
                    muted,
                    in_voice,
                )
            })
            .collect::<Vec<_>>()
    });
    let loaded = app.client.with_state(|s| s.servers_loaded);
    // A failed load shows its error on the page: the placeholders rest.
    let failed = !loaded && super::home::bootstrap_failed(ui.ctx(), app.client, loaded);
    let list_top = rect.top() + 52.0;
    let footer_h = 5.0 + TILE + 16.0;
    let count = if loaded { servers.len() } else { 3 };
    let content_h = 16.0 + ROW * count as f32;
    let list_h = content_h.min(rect.bottom() - list_top - footer_h).max(0.0);
    let list = Rect::from_min_size(pos2(rect.left(), list_top), Vec2::new(rect.width(), list_h));
    let mut clicked: Option<(Uuid, Option<Uuid>)> = None;
    // The server list is one Tab stop; ↑ and ↓ move between servers.
    let nav: Vec<common::NavRow> = servers
        .iter()
        .map(|(id, ..)| {
            let item = Id::unique(("rail-server", *id));
            common::NavRow {
                key: item,
                items: vec![item],
            }
        })
        .collect();
    let preferred = servers.iter().position(|(id, ..)| active == Some(*id));
    let roved = if loaded {
        common::rove(ui, Id::unique("rail-list"), &nav, preferred)
    } else {
        common::Roved::default()
    };
    common::list_hint(ui, list, Id::unique("rail-list"), common::list_hint_text());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    child.set_clip_rect(list);
    egui::ScrollArea::vertical()
        .id_salt("rail-scroll")
        .max_height(list_h)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(&mut child, |ui| {
            let (area, _) =
                ui.allocate_exact_size(Vec2::new(rect.width(), content_h), egui::Sense::hover());
            if !loaded {
                let t = ui.input(|i| i.time) as f32;
                // `lr-placeholder-pulse`: 1 → .45, 1.4s alternate.
                let phase = (t / 1.4) % 2.0;
                let k = if phase < 1.0 { phase } else { 2.0 - phase };
                let alpha = if failed {
                    1.0
                } else {
                    1.0 - 0.55 * (0.5 - 0.5 * (k * std::f32::consts::PI).cos())
                };
                for i in 0..3 {
                    let tile = Rect::from_min_size(
                        pos2(area.left() + 10.0, area.top() + 16.0 + ROW * i as f32),
                        Vec2::splat(TILE),
                    );
                    ui.painter().circle_filled(
                        tile.center(),
                        TILE / 2.0,
                        c.surface.gamma_multiply(alpha),
                    );
                }
                if !failed {
                    ui.ctx().request_repaint();
                }
                return;
            }
            for (i, (id, name, icon, badge, muted, in_voice)) in servers.iter().enumerate() {
                let tile = Rect::from_min_size(
                    pos2(area.left() + 10.0, area.top() + 16.0 + ROW * i as f32),
                    Vec2::splat(TILE),
                );
                let is_active = active == Some(*id);
                let item = Id::unique(("rail-server", *id));
                let stop = roved.is_stop(i, 0) || ui.memory(|m| m.has_focus(item));
                let response = common::click_stop(ui, tile, item, stop);
                if roved.has_focus_within(i)
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowUp)
                    })
                {
                    ui.scroll_to_rect(tile, None);
                }
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, name));
                common::describe_unread(ui, &response, *badge);
                // Its menu: mark read, mute, notifications.
                if response.secondary_clicked() || row_menu::menu_key(ui, &response) {
                    row_menu::open(ui.ctx(), Owner::Server(*id, name.clone()), tile, Some(item));
                }
                let summary = if badge.count > 0 {
                    Some(if badge.count > 99 {
                        shell::rail_mentions_over99().to_owned()
                    } else {
                        shell::rail_mentions(badge.count as u64)
                    })
                } else if badge.dot {
                    Some(shell::rail_unread().to_owned())
                } else if *muted {
                    Some(shell::server_menu_muted().to_owned())
                } else {
                    None
                };
                // The open menu takes the tooltip's place.
                if !row_menu::is_open_for(ui.ctx(), *id) {
                    Tooltip::new(name)
                        .detail(summary.as_deref())
                        .on_hover_or_focus(&response, Side::Right);
                }
                server_tile(
                    ui,
                    app,
                    &theme,
                    tile,
                    *id,
                    name,
                    icon.as_deref(),
                    is_active,
                    response.hovered(),
                    *muted,
                );
                pill(
                    ui,
                    ui.painter(),
                    &theme,
                    Id::unique(("rail-pill", *id)),
                    rect.left(),
                    tile.center().y,
                    is_active,
                    response.hovered(),
                    badge.count > 0 || badge.dot,
                );
                if badge.count > 0 {
                    count_badge(
                        ui.painter(),
                        ui,
                        tile.max + Vec2::splat(5.0),
                        badge.count.min(u32::MAX as i64) as u32,
                        Some(c.rail),
                    );
                }
                let radius = if is_active || response.hovered() {
                    theme.radius.lg
                } else {
                    TILE / 2.0
                };
                common::focus_visible(ui, &response, tile, radius);
                if response.clicked() {
                    // railChannel: the overview while someone talks there.
                    let target = if *in_voice {
                        None
                    } else {
                        memory.channels.get(id).copied()
                    };
                    clicked = Some((*id, target));
                }
            }
        });
    if let Some((server, channel)) = clicked {
        let still = channel.filter(|c| {
            app.client.with_state(|s| {
                s.detail(server)
                    .is_none_or(|d| d.channels.iter().any(|x| x.id == *c))
            })
        });
        match still {
            Some(channel) => app.nav.navigate(&format!("/s/{server}/c/{channel}")),
            None => app.nav.navigate(&format!("/s/{server}")),
        }
    }

    // "Server hinzufügen".
    let add = Rect::from_min_size(
        pos2(rect.left() + 10.0, list.bottom() + 5.0),
        Vec2::splat(TILE),
    );
    let response =
        common::click(ui, add, Id::unique("rail-add")).on_hover_text(shell::rail_add_server());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, shell::rail_add_server())
    });
    if response.hovered() {
        painter.circle_filled(add.center(), TILE / 2.0, c.hover);
        painter.circle_stroke(
            add.center(),
            TILE / 2.0 - 0.5,
            Stroke::new(1.0, c.border_strong),
        );
    } else {
        dashed_circle(&painter, add.center(), TILE / 2.0 - 0.5, c.border_strong);
    }
    icons::paint_centered(&painter, Icon::Plus, add, 20.0, c.online_text);
    common::focus_visible(ui, &response, add, TILE / 2.0);
    if response.clicked() {
        super::dialogs::open_add(ui.ctx(), AddStep::Choose);
    }
}

/// One server tile: tint and letters (or the icon), round at rest and the
/// theme's corner while hovered or active. The edge pill marks the active
/// one; an accent ring would read as the keyboard focus ring.
#[allow(clippy::too_many_arguments)]
fn server_tile(
    ui: &Ui,
    app: &AppCtx,
    theme: &Theme,
    tile: Rect,
    id: Uuid,
    name: &str,
    icon: Option<&str>,
    active: bool,
    hovered: bool,
    muted: bool,
) {
    let c = theme.c;
    let painter = ui.painter();
    let t = ui.ctx().animate_bool_with_time(
        Id::unique(("rail-radius", id)),
        active || hovered,
        motion::FAST,
    );
    let round = theme.radius.lg * 2.2;
    let radius = round + (theme.radius.lg - round) * t;
    let opacity = if muted { 0.6 } else { 1.0 };
    let texture = icon.and_then(|url| match app.images.get(ui.ctx(), url) {
        ImageState::Ready(texture) => Some(texture),
        _ => None,
    });
    if let Some(texture) = texture {
        let uv = crate::images::cover_uv(texture.size_vec2(), tile.size());
        painter.add(
            egui::epaint::RectShape::filled(
                tile,
                corner(radius),
                Color32::WHITE.gamma_multiply(opacity),
            )
            .with_texture(texture.id(), uv),
        );
        return;
    }
    let tint = theme.tile(tile_tone(&id.to_string()));
    let fade = |x: Color32| crate::theme::mix(c.rail, x, f64::from(opacity));
    painter.rect_filled(tile, corner(radius), fade(tint));
    let style = TextStyle::new(15.0, 600.0, 1.0).tracking(0.01);
    let block = Block::single(ui, &tile_initials(name), style, fade(c.text));
    block.paint(
        painter,
        pos2(
            tile.center().x - block.width() / 2.0,
            tile.center().y - block.height() / 2.0,
        ),
    );
}

/// The edge pill: 8px accent for unread, 16px on hover, 28px in the text
/// colour where you are; grows and shrinks over 120ms.
#[allow(clippy::too_many_arguments)]
fn pill(
    ui: &Ui,
    painter: &egui::Painter,
    theme: &Theme,
    id: Id,
    left: f32,
    center_y: f32,
    active: bool,
    hovered: bool,
    unread: bool,
) {
    let c = theme.c;
    let (height, color) = if active {
        (28.0, c.text)
    } else if hovered {
        (16.0, if unread { c.accent } else { c.muted })
    } else if unread {
        (8.0, c.accent)
    } else {
        (0.0, c.muted)
    };
    let shown = ui.ctx().animate_value_with_time(id, height, motion::FAST);
    if shown < 0.5 {
        return;
    }
    let rect = Rect::from_center_size(pos2(left + 2.0, center_y), Vec2::new(4.0, shown));
    painter.rect_filled(
        rect,
        CornerRadius {
            nw: 0,
            sw: 0,
            ne: 4,
            se: 4,
        },
        color,
    );
}

/// A 1px dashed circle (`border: 1px dashed`), about 3px dashes and 2px
/// gaps as Chromium draws them.
fn dashed_circle(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    let circumference = std::f32::consts::TAU * radius;
    let dashes = (circumference / 5.0).round().max(4.0) as usize;
    let period = std::f32::consts::TAU / dashes as f32;
    let dash = period * 0.6;
    for i in 0..dashes {
        let start = i as f32 * period - std::f32::consts::FRAC_PI_2 - dash / 2.0;
        let points: Vec<egui::Pos2> = (0..=4)
            .map(|k| {
                let a = start + dash * k as f32 / 4.0;
                center + Vec2::new(a.cos(), a.sin()) * radius
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(1.0, color)));
    }
}
