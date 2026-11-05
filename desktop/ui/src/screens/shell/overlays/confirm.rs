//! `ConfirmHost` (components/ConfirmHost.tsx, confirm.ts): one question at
//! a time over whatever is open, "Abbrechen" + the confirm button in the
//! footer. A destructive question starts on "Abbrechen", so a held Enter
//! cannot confirm it; a neutral one starts on its confirm button.

use egui::Ui;
use uuid::Uuid;

use crate::app::AppCtx;
use crate::text;
use crate::theme::Theme;
use crate::widgets::{Button, Modal};

/// What a "yes" does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmAction {
    Kick {
        server: Uuid,
        user: Uuid,
    },
    Ban {
        server: Uuid,
        user: Uuid,
    },
    DeleteChannel {
        server: Uuid,
        channel: Uuid,
    },
    DeleteCategory {
        server: Uuid,
        category: Uuid,
    },
    /// Signing out ends the call (components/logout.ts).
    Logout,
}

#[derive(Clone, Debug)]
pub struct Question {
    pub title: String,
    pub message: Option<String>,
    pub label: String,
    /// `tone: "neutral"`: a primary button instead of danger.
    pub neutral: bool,
    pub action: ConfirmAction,
}

impl Question {
    pub fn danger(
        title: impl Into<String>,
        message: Option<&str>,
        label: &str,
        action: ConfirmAction,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.map(str::to_owned),
            label: label.into(),
            neutral: false,
            action,
        }
    }

    /// `MemberActions` kick.
    pub fn kick(server: Uuid, user: Uuid, name: &str) -> Self {
        Self::danger(
            format!("{name} vom Server entfernen?"),
            Some("Mit einem neuen Einladungslink ist ein erneuter Beitritt möglich."),
            "Entfernen",
            ConfirmAction::Kick { server, user },
        )
    }

    /// `MemberActions` ban.
    pub fn ban(server: Uuid, user: Uuid, name: &str) -> Self {
        Self::danger(
            format!("{name} sperren?"),
            Some("Sie können nicht wieder beitreten, auch nicht mit einem Einladungslink."),
            "Sperren",
            ConfirmAction::Ban { server, user },
        )
    }
}

/// Draws the question; true once it is answered. `escape` is the Esc the
/// host took away from the dialog underneath.
pub fn show(ui: &mut Ui, app: &mut AppCtx, q: &Question, escape: bool) -> bool {
    let ctx = ui.ctx().clone();
    let answer = dialog(
        &ctx,
        &q.title,
        q.message.as_deref(),
        &q.label,
        q.neutral,
        escape,
    );
    match answer {
        Some(true) => {
            run(app, q.action);
            true
        }
        Some(false) => true,
        None => false,
    }
}

/// The `ConfirmDialog` itself, for every confirm host: the answer once
/// given. Enter answers through the focused button, as in the browser.
pub fn dialog(
    ctx: &egui::Context,
    title: &str,
    message: Option<&str>,
    label: &str,
    neutral: bool,
    escape: bool,
) -> Option<bool> {
    let theme = Theme::of(ctx);
    let first = crate::widgets::field::mounted(ctx, egui::Id::unique(("gelabber-confirm", title)));
    let mut answer = None;
    let output = Modal::new(("confirm", title), title).show(
        ctx,
        |ui| {
            if let Some(m) = message {
                let w = ui.available_width();
                let block = text::Block::wrapped(ui, m, text::BODY, theme.c.muted, w);
                text::show(ui, &block);
            }
        },
        Some(&mut |ui: &mut Ui| {
            // "Abbrechen" first in the Tab order, the pair at the right end.
            let width = |label: &str, style: text::TextStyle| {
                text::Block::single(ui, label, style, theme.c.text).width() + 34.0
            };
            let total = width("Abbrechen", text::UI) + 8.0 + width(label, text::UI.weight(600.0));
            let (no, yes) = ui
                .allocate_ui_with_layout(
                    egui::vec2(total, 36.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let no = ui.add(Button::new("Abbrechen").ghost());
                        let confirm = Button::new(label);
                        let confirm = if neutral {
                            confirm.primary()
                        } else {
                            confirm.danger()
                        };
                        (no, ui.add(confirm))
                    },
                )
                .inner;
            if first {
                if neutral {
                    yes.request_focus();
                } else {
                    no.request_focus();
                }
            }
            if yes.clicked() {
                answer = Some(true);
            }
            if no.clicked() {
                answer = Some(false);
            }
        }),
    );
    if output.close_requested || escape {
        answer.get_or_insert(false);
    }
    answer
}

fn run(app: &mut AppCtx, action: ConfirmAction) {
    match action {
        ConfirmAction::Kick { server, user } => app.client.kick(server, user),
        ConfirmAction::Ban { server, user } => app.client.ban(server, user),
        ConfirmAction::DeleteChannel { server, channel } => {
            app.client.delete_channel(server, channel)
        }
        ConfirmAction::DeleteCategory { server, category } => {
            app.client.delete_category(server, category)
        }
        ConfirmAction::Logout => crate::screens::shell::user_panel::sign_out(app),
    }
}
