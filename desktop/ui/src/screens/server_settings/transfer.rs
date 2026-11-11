//! "Server übergeben" (web ServerSettingsPage.tsx `TransferDialog`, #519):
//! pick the new owner among the members, then a confirmation names them
//! before anything is sent.

use egui::{Rect, Ui, Vec2};
use gelabber_client::{Member, ServerDetail};
use gelabber_i18n::{common, server_settings as texts};
use uuid::Uuid;

use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors};
use crate::screens::settings::controls::{self, Confirm};
use crate::screens::settings::{op_pending, settled};
use crate::text::{self, Block};
use crate::theme::{Theme, space};
use crate::widgets::field::{layout_unit, note_color};
use crate::widgets::{Button, FormError, Modal};

/// The confirmation's key (`Confirm::danger("transfer-server", …)`).
const CONFIRM: &str = "transfer-server";

/// Who the server can be handed to: every member but the owner
/// (serverSettingsSections.ts `transferCandidates`).
pub(super) fn candidates(detail: &ServerDetail) -> Vec<&Member> {
    detail
        .members
        .iter()
        .filter(|m| m.user_id != detail.server.owner_id)
        .collect()
}

/// The open dialog.
#[derive(Default)]
pub(super) struct Dialog {
    /// The picked member's id; empty until one is picked.
    picked: String,
    errors: FormErrors,
    /// The member the open confirmation names.
    asking: Option<Uuid>,
}

impl Dialog {
    /// Draws the dialog; false once it closes.
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        app: &mut AppCtx,
        detail: &ServerDetail,
    ) -> bool {
        let server = detail.server.id;
        let op = format!("transfer:{server}");
        let candidates = candidates(detail);
        // The first member until one is picked, or when the picked one has
        // left.
        let chosen = candidates
            .iter()
            .find(|m| m.user_id.to_string() == self.picked)
            .or(candidates.first())
            .copied();
        if let Some(answer) = controls::answer(ctx, CONFIRM) {
            if let (true, Some(user)) = (answer, self.asking) {
                app.client.transfer_server(server, user);
            }
            self.asking = None;
        }
        match settled(app, &op) {
            Some(Ok(())) => return false,
            Some(Err(e)) => self
                .errors
                .from_failure(&Failure::from_api(&e), &["user_id"]),
            None => {}
        }
        let pending = op_pending(app, &op);
        let options: Vec<(String, String)> = candidates
            .iter()
            .map(|m| (m.user_id.to_string(), m.name.clone()))
            .collect();
        let mut value = chosen.map(|m| m.user_id.to_string()).unwrap_or_default();
        controls::select_takes_escape(ctx, "transfer-member");
        let output = Modal::new(("transfer-server", server), texts::transfer_title()).show(
            ctx,
            |ui| {
                let field = self.member_field(ui, &options, &mut value);
                ui.add_space(20.0);
                if let Some(message) = &self.errors.form {
                    let theme = Theme::of(ui.ctx());
                    ui.add(FormError::new(message).on(theme.c.elevated));
                    ui.add_space(20.0);
                }
                let mut out = (false, false);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 36.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        out.1 = ui
                            .add(
                                Button::new(texts::transfer_submit())
                                    .danger()
                                    .disabled(chosen.is_none())
                                    .pending(pending.then_some(texts::transfer_pending())),
                            )
                            .clicked();
                        out.0 = ui.add(Button::new(common::cancel()).ghost()).clicked();
                    },
                );
                (out.0, out.1, field)
            },
            None,
        );
        let (cancel, submit, changed) = output.inner;
        if changed {
            self.picked = value;
            self.errors.clear_field("user_id");
        }
        // The confirmation sits over the dialog: its Esc is the question's.
        if controls::asking(ctx) {
            return true;
        }
        if cancel || output.close_requested {
            return false;
        }
        if submit
            && !pending
            && let Some(member) = chosen
        {
            self.asking = Some(member.user_id);
            controls::ask(
                ctx,
                Confirm::danger(
                    CONFIRM,
                    texts::transfer_confirm_title(&detail.server.name, &member.name),
                    Some(texts::transfer_confirm_message()),
                    texts::transfer_submit(),
                ),
            );
        }
        if pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        true
    }

    /// `FormField` "Neuer Eigentümer" over the member `Select`; the field's
    /// error under it. True when another member was picked.
    fn member_field(&self, ui: &mut Ui, options: &[(String, String)], value: &mut String) -> bool {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let width = ui.available_width();
        let label = texts::transfer_member();
        let block = Block::single(ui, label, text::UI, c.text);
        let top = ui.cursor().min;
        block.paint(ui.painter(), top);
        ui.advance_cursor_after_rect(Rect::from_min_size(
            top,
            Vec2::new(width, layout_unit(block.height())),
        ));
        ui.add_space(space::S1_5);
        // The select's own response is not handed out; its id is the next
        // automatic one.
        let select_id = ui.next_auto_id();
        let changed = controls::select(ui, "transfer-member", options, value, c.bg);
        let error = self.errors.field("user_id");
        ui.ctx().accesskit_node_builder(select_id, |node| {
            node.set_label(label);
            if let Some(error) = error {
                node.set_description(error);
            }
        });
        // The Modal focuses its first control, the select, as it opens.
        if crate::widgets::field::mounted(ui.ctx(), select_id.with("mounted")) {
            ui.memory_mut(|m| m.request_focus(select_id));
            crate::widgets::set_keyboard_focus(ui.ctx(), true);
        }
        if let Some(error) = error {
            ui.add_space(space::S1_5);
            let block = Block::wrapped(ui, error, text::META, note_color(&theme, true), width);
            let at = ui.cursor().min;
            block.paint(ui.painter(), at);
            ui.advance_cursor_after_rect(Rect::from_min_size(
                at,
                Vec2::new(width, layout_unit(block.height())),
            ));
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    use parking_lot::Mutex;
    use serde_json::{Value, json};

    use crate::app::testkit::{
        FakeApi, HOME, Reply, launcher, wait_for, wait_for_label, workspace,
    };
    use crate::platform::paths::testing::TempConfig;

    const JONAS: &str = "00000000-0000-4000-8000-000000000038";
    const MIRA: &str = "00000000-0000-4000-8000-000000000039";

    fn member(id: &str, name: &str, role: &str) -> Value {
        json!({
            "user_id": id, "name": name, "avatar_url": null,
            "joined_at": "2026-01-02T00:00:00Z", "status_text": null,
            "accent_color": null, "avatar_decoration": null, "role": role
        })
    }

    /// Wohnzimmer with Lena as `role` and the `others` beside her; the
    /// bodies of transfer requests go into `sent`.
    fn api(role: &'static str, others: Vec<Value>, sent: Arc<Mutex<Vec<String>>>) -> FakeApi {
        FakeApi::start(true, move |method, path, body| {
            let detail = format!("/servers/{HOME}");
            match (method, path) {
                ("POST", p) if p == format!("{detail}/transfer") => {
                    sent.lock().push(body.to_owned());
                    let mut server = workspace("GET", "/servers")?.body[0].clone();
                    server["role"] = json!("member");
                    Some(Reply::json(server))
                }
                ("GET", "/servers") => {
                    let mut list = workspace(method, path)?.body;
                    list[0]["role"] = json!(role);
                    Some(Reply::json(list))
                }
                ("GET", p) if p == detail => {
                    let mut server = workspace(method, path)?.body;
                    server["role"] = json!(role);
                    if role != "owner" {
                        server["permissions"] = json!([]);
                    }
                    let members = server["members"].as_array_mut()?;
                    members.extend(others.iter().cloned());
                    Some(Reply::json(server))
                }
                _ => workspace(method, path),
            }
        })
    }

    fn route() -> String {
        format!("/s/{HOME}/settings")
    }

    /// The owner picks the first other member, confirms, and the dialog
    /// closes once the server has it (`transfer_server`).
    #[test]
    fn the_owner_hands_the_server_on() {
        let _config = TempConfig::new("transfer-owner");
        let sent = Arc::default();
        let others = vec![
            member(JONAS, "Jonas", "member"),
            member(MIRA, "Mira", "member"),
        ];
        let api = api("owner", others, Arc::clone(&sent));
        let mut harness = launcher(&api, &route());
        wait_for_label(&mut harness, "Übergeben…");
        harness.get_by_label("Ein anderes Mitglied wird Eigentümer. Du bleibst Mitglied.");
        // Down the page first, as the scene does: a row out of view takes
        // no click.
        harness
            .get_by_role_and_label(Role::Link, "Gefahrenzone")
            .click();
        for _ in 0..30 {
            harness.step();
        }
        harness
            .get_by_role_and_label(Role::Button, "Übergeben…")
            .click();
        wait_for_label(&mut harness, "Neuer Eigentümer");
        harness
            .get_by_role_and_label(Role::Button, "Übergeben")
            .click();
        wait_for_label(&mut harness, "„Wohnzimmer“ an Jonas übergeben?");
        harness.get_by_label("Du kannst das nicht selbst rückgängig machen.");
        assert!(!api.saw(&format!("POST /servers/{HOME}/transfer")));
        let confirm = harness
            .query_all_by_label("Übergeben")
            .rfind(|n| n.accesskit_node().role() == Role::Button)
            .expect("the question's button");
        confirm.click();
        let transfer = format!("POST /servers/{HOME}/transfer");
        wait_for(&mut harness, "the transfer", |_| api.saw(&transfer));
        let body: Value = serde_json::from_str(&sent.lock()[0]).expect("json body");
        assert_eq!(body, json!({ "user_id": JONAS }));
        wait_for(&mut harness, "the dialog to close", |h| {
            h.query_by_label("Neuer Eigentümer").is_none()
        });
    }

    /// Esc on the open member list closes the list, not the dialog, as the
    /// web's native select.
    #[test]
    fn esc_closes_the_open_member_list_first() {
        let _config = TempConfig::new("transfer-esc");
        let others = vec![member(JONAS, "Jonas", "member")];
        let api = api("owner", others, Arc::default());
        let mut harness = launcher(&api, &route());
        wait_for_label(&mut harness, "Übergeben…");
        harness
            .get_by_role_and_label(Role::Link, "Gefahrenzone")
            .click();
        for _ in 0..30 {
            harness.step();
        }
        harness
            .get_by_role_and_label(Role::Button, "Übergeben…")
            .click();
        wait_for_label(&mut harness, "Neuer Eigentümer");
        harness.get_by_label("Neuer Eigentümer").click();
        for _ in 0..5 {
            harness.step();
        }
        assert!(super::controls::select_open(
            &harness.ctx,
            "transfer-member"
        ));
        harness.key_press(egui::Key::Escape);
        for _ in 0..5 {
            harness.step();
        }
        assert!(!super::controls::select_open(
            &harness.ctx,
            "transfer-member"
        ));
        harness.get_by_label("Neuer Eigentümer");
        harness.key_press(egui::Key::Escape);
        wait_for(&mut harness, "the dialog to close", |h| {
            h.query_by_label("Neuer Eigentümer").is_none()
        });
    }

    /// Alone on the server, there is nobody to hand it to.
    #[test]
    fn without_another_member_the_row_waits_for_one() {
        let _config = TempConfig::new("transfer-alone");
        let api = api("owner", Vec::new(), Arc::default());
        let mut harness = launcher(&api, &route());
        wait_for_label(&mut harness, "Übergeben…");
        harness.get_by_label("Lade zuerst jemanden ein, dem du den Server übergeben kannst.");
        let open = harness.get_by_role_and_label(Role::Button, "Übergeben…");
        assert!(open.accesskit_node().is_disabled());
        harness.get_by_role_and_label(Role::Button, "Server löschen");
        // The deletion sentence is the row's hint, not the section's.
        harness.get_by_label(
            "Löscht Kanäle, Kategorien, Nachrichten, Einladungen und Mitgliedschaften endgültig.",
        );
    }

    /// Members only leave; the transfer row is the owner's.
    #[test]
    fn a_member_sees_no_transfer() {
        let _config = TempConfig::new("transfer-member");
        let api = api("member", Vec::new(), Arc::default());
        let mut harness = launcher(&api, &route());
        wait_for_label(&mut harness, "Server verlassen");
        assert!(harness.query_by_label("Übergeben…").is_none());
        assert!(harness.query_by_label("Server übergeben").is_none());
    }
}
