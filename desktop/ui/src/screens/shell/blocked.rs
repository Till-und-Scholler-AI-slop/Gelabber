//! A server an operator blocked (servers/BlockedServerNotice.tsx, #517):
//! its page says so and offers the one thing still possible there,
//! leaving it (members) or deleting it (the owner), through the server
//! settings' confirmations.

use egui::{Rect, Ui, pos2};
use gelabber_client::{Role, Server};
use gelabber_i18n::{auth, server_settings as texts};

use crate::app::AppCtx;
use crate::screens::auth::layout;
use crate::screens::settings::controls::{self, Confirm};
use crate::theme::space;
use crate::widgets::{Button, ButtonVariant};

/// The list entry of `server` when the list marks it blocked.
pub fn blocked(app: &AppCtx, server: uuid::Uuid) -> Option<Server> {
    app.client.with_state(|s| s.server(server).filter(|v| v.blocked).cloned())
}

/// The notice in the page column: `.lr-auth-card mx-auto my-10 w-full
/// max-w-sm` with the server's name, the reason and the action.
pub fn show(ui: &mut Ui, app: &mut AppCtx, page: Rect, server: &Server) {
    let key = format!("blocked-drop:{}", server.id);
    let owner = server.role == Role::Owner;
    if let Some(true) = controls::answer(ui.ctx(), &key) {
        // Optimistic, as in the settings: the server is already off the rail.
        super::leave_server(ui.ctx(), app.client, server.id);
        if owner {
            app.client.delete_server(server.id);
        } else {
            app.client.leave_server(server.id);
        }
        app.nav.replace("/");
        return;
    }
    let width = page.width().min(384.0);
    let min = pos2(page.center().x - width / 2.0, page.top() + space::S10);
    layout::card_at(ui, app, min, width, |ui, _app| {
        layout::heading(ui, &server.name);
        layout::lead(ui, auth::error_server_blocked());
        ui.add_space(space::S6);
        let label = if owner {
            texts::danger_delete()
        } else {
            texts::danger_leave()
        };
        if ui
            .add(
                Button::new(label)
                    .variant(ButtonVariant::DangerOutline)
                    .fill(),
            )
            .clicked()
        {
            let confirm = if owner {
                Confirm::danger(
                    &key,
                    texts::danger_delete_title(&server.name),
                    Some(texts::danger_delete_message()),
                    texts::danger_delete(),
                )
            } else {
                Confirm::danger(
                    &key,
                    texts::danger_leave_title(&server.name),
                    None,
                    texts::danger_leave_confirm(),
                )
            };
            controls::ask(ui.ctx(), confirm);
        }
    });
    controls::show_confirm(ui.ctx());
}
