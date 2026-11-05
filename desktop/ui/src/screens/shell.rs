//! The signed-in workspace (pages/WorkspaceLayout.tsx): the server rail,
//! the sidebar column with the user panel at its bottom, the page column
//! and the member panel. The shell opens the client's view for every
//! route, remembers where the user was, sends "/" there, and runs the
//! workspace shortcuts; the pages draw into the rects it hands them.

mod channel_sidebar;
pub(crate) mod common;
pub mod dialogs;
mod dm_index;
mod dm_sidebar;
mod home;
mod members;
pub mod overlays;
mod overview;
mod rail;
mod state;
mod user_panel;

use egui::{Key, Modifiers, Rect, Ui, UiBuilder, pos2};
use gelabber_client::{ChannelKind, OpState, View};
use uuid::Uuid;

use crate::app::AppCtx;
use crate::route::Route;
use crate::screens::layout::{self, MEMBERS_WIDTH, RAIL_WIDTH, SIDEBAR_WIDTH};
use crate::screens::{chat, server_settings, settings, voice};
use crate::theme::{Theme, layout::DOCK_HEIGHT};
use crate::widgets::hairline_v;

pub use state::{Memory, Place};

/// `user`'s remembered places, for the desktop app to keep across restarts.
pub fn memory_of(ctx: &egui::Context, user: Uuid) -> Option<Memory> {
    state::with(ctx, |s| s.memory.get(&user).cloned())
}

/// Puts stored places back, unless this run already has some.
pub fn restore_memory(ctx: &egui::Context, user: Uuid, memory: Memory) {
    state::with(ctx, |s| {
        s.memory.entry(user).or_insert(memory);
    });
}

pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    let theme = Theme::of(ui.ctx());
    let screen = ui.ctx().content_rect();
    ui.painter().rect_filled(screen, 0.0, theme.c.bg);
    effects(ui, app);
    let route = app.nav.route().clone();
    overlays::keys(ui, state::with(ui.ctx(), |s| s.dialog.is_some()));
    shortcuts(ui, app, &route);

    // A running call's bar takes the bottom strip next to the user panel;
    // the page and the member panel end above it.
    let bar = voice::call_bar::visible(app);
    let content = if bar {
        Rect::from_min_max(
            screen.min,
            pos2(screen.right(), screen.bottom() - DOCK_HEIGHT),
        )
    } else {
        screen
    };
    if let Route::Channel(s, c) = route
        && is_voice(app, c)
        && voice::page::kino(ui.ctx(), app, s, c)
    {
        // Kino: the stage takes the window but for the call bar.
        let mut main = ui.new_child(UiBuilder::new().max_rect(content));
        voice::show(&mut main, app, s, c);
        call_bar(ui, app, screen, true);
        dialogs::show(ui, app);
        return;
    }

    let rail = Rect::from_min_max(
        screen.min,
        pos2(screen.left() + RAIL_WIDTH, screen.bottom()),
    );
    let column = Rect::from_min_max(
        pos2(rail.right(), screen.top()),
        pos2(rail.right() + SIDEBAR_WIDTH, screen.bottom()),
    );
    // The sidebar's own part ends where the user panel starts.
    let sidebar = Rect::from_min_max(
        column.min,
        pos2(column.right(), column.bottom() - DOCK_HEIGHT),
    );
    let dock = Rect::from_min_max(pos2(column.left(), sidebar.bottom()), column.max);
    ui.painter().rect_filled(column, 0.0, theme.c.panel);
    hairline_v(
        ui.painter(),
        &theme,
        column.right() - 1.0,
        column.top()..=column.bottom(),
    );

    let mut active_server = None;
    let mut dm_active = false;
    let mut context = None;
    match route.clone() {
        Route::Server(s) => {
            active_server = Some(s);
            context = Some(s);
            channel_sidebar::show(ui, app, sidebar, s, None, true);
            let page = layout::page_rect(content, true);
            overview::show(ui, app, page, s);
            members::show(ui, app, members_rect(content), s);
        }
        Route::Channel(s, c) => {
            active_server = Some(s);
            context = Some(s);
            channel_sidebar::show(ui, app, sidebar, s, Some(c), false);
            let voice_room = is_voice(app, c);
            // Next to pictures the member list starts closed.
            layout::set_members_video(ui.ctx(), voice_room && voice::page::staged(app, s, c));
            let visible = layout::members_visible(ui.ctx());
            let page = layout::page_rect(content, visible);
            let mut main = ui.new_child(UiBuilder::new().max_rect(page));
            if voice_room {
                voice::show(&mut main, app, s, c);
            } else {
                chat::channel_page(&mut main, app, s, c);
            }
            if visible {
                members::show(ui, app, members_rect(content), s);
            }
        }
        Route::ServerSettings(s) => {
            active_server = Some(s);
            context = Some(s);
            channel_sidebar::show(ui, app, sidebar, s, None, false);
            let mut side = ui.new_child(UiBuilder::new().max_rect(sidebar));
            server_settings::sidebar(&mut side, app, s);
            let mut main =
                ui.new_child(UiBuilder::new().max_rect(layout::page_rect(content, false)));
            server_settings::page(&mut main, app, s);
        }
        Route::Settings(area) => {
            let mut side = ui.new_child(UiBuilder::new().max_rect(sidebar));
            settings::sidebar(&mut side, app, area.as_deref());
            let mut main =
                ui.new_child(UiBuilder::new().max_rect(layout::page_rect(content, false)));
            settings::page(&mut main, app, area.as_deref());
        }
        Route::Dms => {
            dm_active = true;
            dm_sidebar::show(ui, app, sidebar, None);
            dm_index::show(ui, app, layout::page_rect(content, false));
        }
        Route::Dm(c) => {
            dm_active = true;
            context = Some(c);
            dm_sidebar::show(ui, app, sidebar, Some(c));
            let mut main =
                ui.new_child(UiBuilder::new().max_rect(layout::page_rect(content, false)));
            chat::dm_page(&mut main, app, c);
        }
        _ => {
            // `/`: the brand sidebar; the page sends the user on.
            channel_sidebar::brand(ui, sidebar);
            let empty = app
                .client
                .with_state(|s| s.servers_loaded && s.servers.is_empty());
            if empty {
                let theme = Theme::of(ui.ctx());
                let block = crate::text::Block::wrapped(
                    ui,
                    "Keine Server. Erstelle einen über + in der Serverleiste oder öffne einen Einladungslink.",
                    crate::text::BODY.line_height(1.6),
                    theme.c.muted,
                    sidebar.width() - 1.0 - 44.0,
                );
                block.paint(
                    ui.painter(),
                    pos2(sidebar.left() + 22.0, sidebar.top() + 52.0 + 24.0),
                );
            }
            if !home::redirect(ui, app) {
                home::welcome(ui, app, layout::page_rect(content, false));
            }
        }
    }
    rail::show(ui, app, rail, active_server, dm_active);
    user_panel::show(ui, app, dock, context);
    call_bar(ui, app, screen, false);
    dialogs::show(ui, app);
    overlays::show(ui, app);
}

fn is_voice(app: &AppCtx, channel: Uuid) -> bool {
    app.client
        .with_state(|s| s.channel(channel).map(|ch| ch.kind))
        == Some(ChannelKind::Voice)
}

/// The call bar in the bottom strip, from the sidebar's edge (Kino: from
/// the window's).
fn call_bar(ui: &mut Ui, app: &mut AppCtx, screen: Rect, full: bool) {
    let account = app.client.with_state(|s| s.me().map(|m| m.id));
    voice::volume::sync(ui.ctx(), app.call, account);
    if !voice::call_bar::visible(app) {
        return;
    }
    let left = if full {
        screen.left()
    } else {
        screen.left() + RAIL_WIDTH + SIDEBAR_WIDTH
    };
    let rect = Rect::from_min_max(pos2(left, screen.bottom() - DOCK_HEIGHT), screen.max);
    let mut bar = ui.new_child(UiBuilder::new().max_rect(rect));
    voice::call_bar::show(&mut bar, app, rect);
}

/// Whether a shell dialog is open (shortcuts keep off then).
pub fn dialog_open(ctx: &egui::Context) -> bool {
    state::with(ctx, |s| s.dialog.is_some())
}

fn members_rect(screen: Rect) -> Rect {
    Rect::from_min_max(
        pos2(screen.right() - MEMBERS_WIDTH, screen.top()),
        screen.max,
    )
}

/// What a route does besides drawing: the client's view (topics, history,
/// read marks), the remembered places, and leaving a server that is gone.
fn effects(ui: &Ui, app: &mut AppCtx) {
    let route = app.nav.route().clone();
    let href = app.nav.location().href();
    let first = state::with(ui.ctx(), |s| {
        let first = s.effects_for.as_deref() != Some(href.as_str());
        s.effects_for = Some(href.clone());
        if first {
            s.server_menu = false;
            s.user_menu = false;
            s.row_menu = None;
        }
        first
    });
    // Kino belongs to the room's page: another page leaves it.
    if first {
        voice::stage::set_kino(ui.ctx(), false);
    }
    let user = app.client.with_state(|s| s.me().map(|m| m.id));
    match &route {
        Route::Server(s) => {
            app.client.open_server(*s);
            if first {
                state::remember(ui.ctx(), user, |m| m.place = Some(Place::Server(*s)));
            }
        }
        Route::Channel(s, c) => {
            let kind = app.client.with_state(|st| st.channel(*c).map(|ch| ch.kind));
            match kind {
                Some(ChannelKind::Text) => app.client.open_server_channel(*s, *c),
                // A voice room has no history of its own; the server's view
                // keeps its topics (the call subscribes the room).
                _ => app.client.open_server(*s),
            }
            if first || kind.is_some() {
                state::remember(ui.ctx(), user, |m| {
                    m.place = Some(Place::Server(*s));
                    m.channels.insert(*s, *c);
                });
            }
        }
        Route::ServerSettings(s) => app.client.open_server(*s),
        Route::Dm(c) => {
            app.client.open_dm(*c);
            if first {
                state::remember(ui.ctx(), user, |m| {
                    m.place = Some(Place::Dm(*c));
                    m.dm = Some(*c);
                });
            }
        }
        _ => {
            if first && app.client.with_state(|s| s.view != View::default()) {
                app.client.close_view();
            }
        }
    }
    // SelectedServer: a server that is gone or forbidden leaves for "/".
    if let Route::Server(s) | Route::Channel(s, _) | Route::ServerSettings(s) = route {
        let gone = app
            .client
            .with_state(|st| match st.op(&format!("server:{s}")) {
                Some(OpState::Failed(e)) => matches!(e.code(), "not_found" | "forbidden"),
                _ => false,
            });
        if gone {
            app.toasts
                .notify("Server nicht gefunden oder kein Zugriff.");
            app.client.clear_op(format!("server:{s}"));
            state::remember(ui.ctx(), user, |m| {
                m.channels.remove(&s);
                if m.place == Some(Place::Server(s)) {
                    m.place = None;
                }
            });
            app.nav.replace("/");
        }
    }
}

/// shellShortcuts.ts: Alt+↑/↓ the previous or next channel, with Umschalt
/// the previous or next unread one, with Strg the previous or next server.
fn shortcuts(ui: &Ui, app: &mut AppCtx, route: &Route) {
    if state::with(ui.ctx(), |s| s.dialog.is_some()) || overlays::is_open(ui.ctx()) {
        return;
    }
    let mut step = 0i32;
    let mut kind = 0u8;
    ui.ctx().input_mut(|i| {
        i.events.retain(|event| {
            let egui::Event::Key {
                key,
                modifiers,
                pressed: true,
                repeat: false,
                ..
            } = event
            else {
                return true;
            };
            let dir = match key {
                Key::ArrowDown => 1,
                Key::ArrowUp => -1,
                _ => return true,
            };
            let m: &Modifiers = modifiers;
            let which = match (m.alt, m.shift, m.command) {
                (true, false, false) => 0,
                (true, true, false) => 1,
                (true, false, true) => 2,
                _ => return true,
            };
            step = dir;
            kind = which;
            false
        });
    });
    if step == 0 {
        return;
    }
    let server = match route {
        Route::Server(s) | Route::Channel(s, _) | Route::ServerSettings(s) => Some(*s),
        _ => None,
    };
    let current = match route {
        Route::Channel(_, c) | Route::Dm(c) => Some(*c),
        _ => None,
    };
    if kind == 2 {
        let list = app
            .client
            .with_state(|s| s.servers.iter().map(|v| v.id).collect::<Vec<_>>());
        let index = server.and_then(|s| list.iter().position(|x| *x == s));
        let Some(next) = step_index(list.len(), index, step).map(|i| list[i]) else {
            return;
        };
        let user = app.client.with_state(|s| s.me().map(|m| m.id));
        let in_voice = app
            .client
            .with_state(|s| s.voice.get(&next).is_some_and(|r| !r.is_empty()));
        let target = (!in_voice)
            .then(|| state::memory(ui.ctx(), user).channels.get(&next).copied())
            .flatten();
        match target {
            Some(c) => app.nav.navigate(&format!("/s/{next}/c/{c}")),
            None => app.nav.navigate(&format!("/s/{next}")),
        }
        return;
    }
    let now = chrono::Utc::now();
    let ids: Vec<Uuid> = app.client.with_state(|s| match server {
        Some(server) => s
            .detail(server)
            .map(channel_sidebar::channel_order)
            .unwrap_or_default(),
        None if matches!(route, Route::Dms | Route::Dm(_)) => s.dms.iter().map(|d| d.id).collect(),
        None => Vec::new(),
    });
    let index = current.and_then(|c| ids.iter().position(|x| *x == c));
    let at = if kind == 0 {
        step_index(ids.len(), index, step)
    } else {
        let unread: Vec<bool> = app.client.with_state(|s| {
            ids.iter()
                .map(|id| {
                    let b = s.channel_badge(*id, now);
                    b.count > 0 || b.dot
                })
                .collect()
        });
        next_unread(&unread, index, step)
    };
    let Some(next) = at.map(|i| ids[i]) else {
        return;
    };
    match server {
        Some(server) => app.nav.navigate(&format!("/s/{server}/c/{next}")),
        None => app.nav.navigate(&format!("/d/{next}")),
    }
}

/// `stepIndex`: the neighbour, or `None` at the end; from outside the
/// list the first step goes to either end.
fn step_index(count: usize, index: Option<usize>, step: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match index {
        None => Some(if step > 0 { 0 } else { count - 1 }),
        Some(i) => {
            let next = i as i64 + i64::from(step);
            (0..count as i64).contains(&next).then_some(next as usize)
        }
    }
}

/// `nextUnread`: round the end of the list.
fn next_unread(unread: &[bool], index: Option<usize>, step: i32) -> Option<usize> {
    let count = unread.len() as i64;
    if count == 0 {
        return None;
    }
    let from = index.map_or(-1, |i| i as i64);
    for offset in 1..=count {
        let at = ((from + i64::from(step) * offset) % count + count) % count;
        if Some(at as usize) != index && unread[at as usize] {
            return Some(at as usize);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_like_the_web() {
        assert_eq!(step_index(3, None, 1), Some(0));
        assert_eq!(step_index(3, None, -1), Some(2));
        assert_eq!(step_index(3, Some(2), 1), None);
        assert_eq!(step_index(3, Some(1), -1), Some(0));
        assert_eq!(next_unread(&[true, false, true], Some(2), 1), Some(0));
        assert_eq!(next_unread(&[false, false, true], Some(2), 1), None);
    }
}
