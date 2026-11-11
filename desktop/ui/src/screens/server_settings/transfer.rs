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
