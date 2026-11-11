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
    app.client
        .with_state(|s| s.server(server).filter(|v| v.blocked).cloned())
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

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};

    use crate::app::testkit::{FakeApi, HOME, LENA, Reply, launcher, wait_for, wait_for_label};
    use crate::platform::paths::testing::TempConfig;

    /// Wohnzimmer, blocked, with Lena as `role`.
    fn blocked_api(role: &'static str) -> FakeApi {
        FakeApi::start(true, move |method, path, _| match (method, path) {
            ("GET", "/servers") => Some(Reply::json(serde_json::json!([{
                "id": HOME, "name": "Wohnzimmer", "owner_id": LENA, "icon_url": null,
                "created_at": "2026-01-01T00:00:00Z", "role": role,
                "permissions": [], "member_permissions": [], "blocked": true
            }]))),
            ("GET", p) if p == format!("/servers/{HOME}") => {
                Some(Reply::error(403, "server_blocked"))
            }
            ("DELETE", p) if p == format!("/servers/{HOME}") => {
                Some(Reply::status(204, serde_json::Value::Null))
            }
            _ => None,
        })
    }

    /// The server's page is the notice; the owner deletes it after the
    /// settings' confirmation.
    #[test]
    fn a_blocked_server_opens_to_its_notice() {
        let _config = TempConfig::new("blocked-owner");
        let api = blocked_api("owner");
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        wait_for_label(&mut harness, "Server löschen");
        assert!(harness.query_by_label("Server verlassen").is_none());
        harness
            .get_by_role_and_label(Role::Button, "Server löschen")
            .click();
        wait_for_label(&mut harness, "Server „Wohnzimmer“ endgültig löschen?");
        let confirm = harness
            .query_all_by_label("Server löschen")
            .rfind(|n| n.accesskit_node().role() == Role::Button)
            .expect("the dialog's button");
        confirm.click();
        let delete = format!("DELETE /servers/{HOME}");
        wait_for(&mut harness, "the delete", |_| api.saw(&delete));
    }

    /// A member is offered to leave.
    #[test]
    fn a_member_of_a_blocked_server_may_leave() {
        let _config = TempConfig::new("blocked-member");
        let api = blocked_api("member");
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        wait_for_label(&mut harness, "Server verlassen");
        harness.get_by_role_and_label(Role::Button, "Server verlassen");
        assert!(harness.query_by_label("Server löschen").is_none());
    }
}
