//! The signed-in workspace. A stub until the shell is built: it splits the
//! window into the web's columns and lets the pages draw theirs, so each
//! page can be compared with the web on its own.

use egui::{Rect, Ui, UiBuilder, pos2};

use crate::app::AppCtx;
use crate::route::Route;
use crate::screens::layout::{self, RAIL_WIDTH, SIDEBAR_WIDTH};
use crate::screens::{chat, server_settings, settings};
use crate::theme::Theme;

pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    let theme = Theme::of(ui.ctx());
    let screen = ui.ctx().content_rect();
    ui.painter().rect_filled(screen, 0.0, theme.c.bg);
    let route = app.nav.route().clone();
    let members =
        matches!(route, Route::Channel(..) | Route::Server(_)) && layout::members_visible(ui.ctx());
    let sidebar = Rect::from_min_max(
        pos2(screen.left() + RAIL_WIDTH, screen.top()),
        pos2(screen.left() + RAIL_WIDTH + SIDEBAR_WIDTH, screen.bottom()),
    );
    let page = layout::page_rect(screen, members);
    let mut side = ui.new_child(UiBuilder::new().max_rect(sidebar));
    let mut main = ui.new_child(UiBuilder::new().max_rect(page));
    match route {
        Route::Channel(s, c) => chat::channel_page(&mut main, app, s, c),
        Route::Dm(c) => chat::dm_page(&mut main, app, c),
        Route::Settings(area) => {
            settings::sidebar(&mut side, app, area.as_deref());
            settings::page(&mut main, app, area.as_deref());
        }
        Route::ServerSettings(s) => {
            server_settings::sidebar(&mut side, app, s);
            server_settings::page(&mut main, app, s);
        }
        _ => {}
    }
}
