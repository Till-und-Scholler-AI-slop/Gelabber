//! "Neuen Link erstellen" (components/InviteDialog.tsx `InviteLinkPanel`,
//! servers/inviteTerms.ts, invite.css): a link made as the dialog opens,
//! copy, and the expiry and use chips that make a new one.

use egui::{Id, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use gelabber_client::{ApiError, Invite, Server};
use gelabber_i18n::{Text, common, server_settings as texts};
use uuid::Uuid;

use crate::app::AppCtx;
use crate::rules::{Failure, error_message};
use crate::screens::settings::controls::{self, Status};
use crate::task::{Pending, poll};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, InlineStatus, Modal, StatusTone};

const EXPIRY: [(Text, Option<u32>); 4] = [
    (texts::terms_expiry_hour, Some(1)),
    (texts::terms_expiry_day, Some(24)),
    (texts::terms_expiry_week, Some(168)),
    (texts::terms_expiry_never, None),
];
const USES: [(Text, Option<u32>); 4] = [
    (|| "1", Some(1)),
    (|| "5", Some(5)),
    (|| "25", Some(25)),
    (texts::terms_uses_unlimited, None),
];

/// `termsSummary`: "gültig 7 Tage · unbegrenzt nutzbar".
fn summary(expiry: usize, uses: usize) -> String {
    let e = match EXPIRY[expiry].1 {
        None => texts::terms_valid_forever().to_owned(),
        Some(_) => texts::terms_valid_for(EXPIRY[expiry].0()),
    };
    let u = match USES[uses].1 {
        None => texts::terms_uses_unlimited_summary().to_owned(),
        Some(n) => texts::terms_uses_limited(n),
    };
    format!("{e} · {u}")
}

pub struct Dialog {
    server: Uuid,
    expiry: usize,
    uses: usize,
    /// The link shown and the terms it was made with.
    current: Option<(Invite, usize, usize)>,
    creating: Option<Pending<Result<Invite, ApiError>>>,
    error: Option<String>,
    handed_out: bool,
    status: Status,
    open_terms: bool,
}

impl Dialog {
    pub fn new(app: &AppCtx, server: Uuid) -> Self {
        let mut d = Self {
            server,
            expiry: 2,
            uses: 3,
            current: None,
            creating: None,
            error: None,
            handed_out: false,
            status: Status::default(),
            open_terms: false,
        };
        d.make(app);
        d
    }

    fn make(&mut self, app: &AppCtx) {
        let http = app.client.http().clone();
        let (server, uses, hours) = (self.server, USES[self.uses].1, EXPIRY[self.expiry].1);
        self.error = None;
        self.creating = Some(Pending::spawn(app.client, async move {
            http.create_invite(server, uses, hours).await
        }));
    }

    fn revoke(app: &AppCtx, code: String) {
        let http = app.client.http().clone();
        app.client.spawn(async move {
            let _ = http.revoke_invite(&code).await;
        });
    }

    /// Draws the dialog; false once it closed.
    pub fn show(&mut self, ctx: &egui::Context, app: &AppCtx, server: &Server) -> bool {
        if let Some(result) = poll(&mut self.creating) {
            match result {
                Ok(invite) => {
                    let previous = self.current.take();
                    self.current = Some((invite, self.expiry, self.uses));
                    if let Some((old, ..)) = previous {
                        if !self.handed_out {
                            Self::revoke(app, old.code);
                        }
                        let text = texts::invite_new_link(&summary(self.expiry, self.uses));
                        self.status.line = Some((text, StatusTone::Neutral, ctx.input(|i| i.time)));
                    }
                    self.handed_out = false;
                }
                Err(e) => {
                    let code = Failure::from_api(&e).code;
                    self.error = Some(error_message(&code).to_owned());
                    if let Some((_, e, u)) = &self.current {
                        self.expiry = *e;
                        self.uses = *u;
                    }
                }
            }
        }
        let theme = Theme::of(ctx);
        let c = theme.c;
        let title = texts::invite_title(&server.name);
        let mut close = false;
        let out = Modal::new(("invite", self.server), &title).show(
            ctx,
            |ui| {
                let width = ui.available_width();
                if self.current.is_none() && self.creating.is_none() {
                    let msg = texts::invite_create_failed(&self.error.clone().unwrap_or_default());
                    controls::paragraph(ui, &msg, text::BODY, c.text, width);
                    ui.add_space(12.0);
                    if ui.add(Button::new(common::try_again()).small()).clicked() {
                        self.make(app);
                    }
                    return;
                }
                let link = self
                    .current
                    .as_ref()
                    .map(|(i, ..)| app.client.http().invite_link(&i.code))
                    .unwrap_or_default();
                let shown = if self.creating.is_some() {
                    texts::invite_making().to_owned()
                } else {
                    link.clone()
                };
                controls::row(ui, 8.0, 37.0, |ui| {
                    let copy_label = texts::invite_copy();
                    let bw = Block::single(ui, copy_label, text::UI.weight(600.0), c.text).width()
                        + 34.0
                        + 24.0;
                    let field_w = ui.available_width() - 8.0 - bw;
                    let (rect, _) = ui.allocate_exact_size(vec2(field_w, 37.0), Sense::hover());
                    ui.painter().rect(
                        rect,
                        corner(theme.radius.sm),
                        c.bg,
                        Stroke::new(1.0, c.border_strong),
                        StrokeKind::Inside,
                    );
                    let mono = TextStyle::new(13.0, 400.0, 1.5);
                    let mut f = mono.format(c.text);
                    f.font_id = egui::FontId::monospace(13.0);
                    let b = Block::new(
                        ui,
                        egui::text::LayoutJob::single_section(shown.clone(), f),
                        mono,
                    );
                    b.paint(
                        &ui.painter().with_clip_rect(rect.shrink(11.0)),
                        pos2(rect.left() + 12.0, rect.center().y - b.height() / 2.0),
                    );
                    let copy = ui.add(
                        Button::new(copy_label)
                            .primary()
                            .icon(crate::icons::Icon::Copy),
                    );
                    if copy.clicked() {
                        if self.creating.is_some() {
                            self.status
                                .show(ui, texts::invite_making(), StatusTone::Neutral);
                        } else if !link.is_empty() {
                            ui.ctx().copy_text(link.clone());
                            self.handed_out = true;
                            let sum = self
                                .current
                                .as_ref()
                                .map(|(_, e, u)| summary(*e, *u))
                                .unwrap_or_default();
                            self.status.success(ui, texts::invite_copied(&sum));
                        }
                    }
                });
                ui.add_space(12.0);
                match self.status.current(ui) {
                    Some((m, t)) => {
                        ui.add(InlineStatus::new(Some(&m), t));
                    }
                    None => {
                        let sum = self
                            .current
                            .as_ref()
                            .map(|(_, e, u)| summary(*e, *u))
                            .unwrap_or_else(|| summary(self.expiry, self.uses));
                        let top = ui.cursor().min;
                        controls::paint(ui, pos2(top.x, top.y + 1.6), &sum, text::META, c.muted);
                        ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
                    }
                }
                if self.error.is_some() && self.current.is_some() {
                    ui.add_space(12.0);
                    controls::paragraph(
                        ui,
                        texts::invite_replace_failed(),
                        text::META,
                        c.danger,
                        width,
                    );
                }
                ui.add_space(12.0 + 4.0);
                let top = ui.cursor().min;
                crate::widgets::rule_h(ui.painter(), c.border, top.x..=top.x + width, top.y);
                ui.add_space(13.0);
                let label = texts::invite_terms_editor();
                let lb = Block::single(ui, label, text::UI, c.muted);
                let r = Rect::from_min_size(ui.cursor().min, vec2(lb.width() + 4.0 + 14.0, 32.0));
                let resp = ui.interact(
                    r,
                    Id::unique(("gelabber-invite-terms", self.server)),
                    Sense::click(),
                );
                let color = if resp.hovered() { c.text } else { c.muted };
                lb.paint_tinted(
                    ui.painter(),
                    pos2(r.left(), r.center().y - lb.height() / 2.0),
                    Some(color),
                );
                // The chevron points right while closed, down when open.
                let cx = r.left() + lb.width() + 4.0 + 7.0;
                let cy = r.center().y;
                let pts = if self.open_terms {
                    [
                        pos2(cx - 3.5, cy - 1.75),
                        pos2(cx, cy + 1.75),
                        pos2(cx + 3.5, cy - 1.75),
                    ]
                } else {
                    [
                        pos2(cx - 1.75, cy - 3.5),
                        pos2(cx + 1.75, cy),
                        pos2(cx - 1.75, cy + 3.5),
                    ]
                };
                ui.painter().line(pts.to_vec(), Stroke::new(1.6, color));
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    self.open_terms = !self.open_terms;
                }
                ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
                if self.open_terms {
                    let mut changed = false;
                    for (caps, opts, which) in [
                        (texts::invite_expires_after(), EXPIRY.map(|e| e.0()), 0),
                        (texts::invite_uses_label(), USES.map(|u| u.0()), 1),
                    ] {
                        let caps = caps.to_uppercase();
                        ui.add_space(8.0 + 8.0);
                        controls::paint(ui, ui.cursor().min, &caps, text::CAPS, c.muted);
                        ui.add_space(text::CAPS.line_px() + 8.0);
                        let sel = if which == 0 { self.expiry } else { self.uses };
                        if let Some(i) =
                            controls::chips(ui, ("invite-terms", which), &opts, sel, 32.0, None)
                        {
                            if which == 0 {
                                self.expiry = i;
                            } else {
                                self.uses = i;
                            }
                            changed = true;
                        }
                    }
                    if changed {
                        self.make(app);
                    }
                }
            },
            None,
        );
        if out.close_requested {
            close = true;
        }
        !close
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries() {
        assert_eq!(summary(2, 3), "gültig 7 Tage · unbegrenzt nutzbar");
        assert_eq!(summary(3, 1), "unbegrenzt gültig · 5-mal nutzbar");
    }
}
