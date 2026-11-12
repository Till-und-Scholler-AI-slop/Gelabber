//! The directory's dialogs (components/DirectoryDialogs.tsx): "Beitritt
//! anfragen" with an optional message, and "Server melden" with a reason
//! and details, which go to the operator.

use egui::{Sense, Ui, vec2};
use gelabber_client::{Listing, ReportBody, ReportFiled, ReportReason, RequestAnswer};
use gelabber_i18n::{common, discovery as texts};
use uuid::Uuid;

use super::{Answer, State, patch};
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, field_message};
use crate::screens::shell::overlays::notify::radio_card;
use crate::task::{Pending, poll};
use crate::text::{self, Block};
use crate::theme::{Theme, space};
use crate::widgets::{Button, FormError, Modal, TextField};

/// `MESSAGE_MAX`.
const MESSAGE_MAX: usize = 300;
/// `REPORT_DETAILS_MAX` (operator's `reports::DETAILS_MAX`).
const DETAILS_MAX: usize = 1000;

/// `REPORT_REASONS`, in the order the dialog lists them.
const REASONS: [ReportReason; 5] = [
    ReportReason::Spam,
    ReportReason::Harassment,
    ReportReason::Sexual,
    ReportReason::Illegal,
    ReportReason::Other,
];

fn reason_label(reason: ReportReason) -> &'static str {
    match reason {
        ReportReason::Spam => texts::report_reasons_spam(),
        ReportReason::Harassment => texts::report_reasons_harassment(),
        ReportReason::Sexual => texts::report_reasons_sexual(),
        ReportReason::Illegal => texts::report_reasons_illegal(),
        ReportReason::Other => texts::report_reasons_other(),
    }
}

/// `validateMessage`: one line of at most 300 characters; empty is fine.
pub fn validate_message(raw: &str) -> Option<&'static str> {
    let value = raw.trim();
    if value.chars().count() > MESSAGE_MAX {
        Some("too_long")
    } else if value.chars().any(char::is_control) {
        Some("invalid")
    } else {
        None
    }
}

/// `validateReportDetails`: required for "Sonstiges"; line breaks and
/// tabs are allowed.
pub fn validate_details(raw: &str, reason: ReportReason) -> Option<&'static str> {
    let value = raw.trim();
    if value.chars().count() > DETAILS_MAX {
        Some("too_long")
    } else if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        Some("invalid")
    } else if value.is_empty() && reason == ReportReason::Other {
        Some("required")
    } else {
        None
    }
}

pub struct RequestForm {
    server: Uuid,
    name: String,
    message: String,
    errors: FormErrors,
    pending: Option<Answer<RequestAnswer>>,
}

pub struct ReportForm {
    server: Uuid,
    name: String,
    /// No reason is chosen for the reporter.
    reason: Option<ReportReason>,
    details: String,
    errors: FormErrors,
    pending: Option<Answer<ReportFiled>>,
}

pub(super) fn open_request(s: &mut State, listing: &Listing) {
    s.request = Some(RequestForm {
        server: listing.server_id,
        name: listing.name.clone(),
        message: String::new(),
        errors: FormErrors::default(),
        pending: None,
    });
}

pub(super) fn open_report(s: &mut State, server: Uuid, name: &str) {
    s.report = Some(ReportForm {
        server,
        name: name.to_owned(),
        reason: None,
        details: String::new(),
        errors: FormErrors::default(),
        pending: None,
    });
}

/// The answers of a sent request or report.
pub(super) fn take_answers(_ctx: &egui::Context, app: &mut AppCtx, s: &mut State) {
    let closed = s
        .request_sent
        .as_mut()
        .and_then(|(server, pending)| pending.take().map(|r| (*server, r)));
    if let Some((server, result)) = closed {
        s.request_sent = None;
        match result {
            Ok(answer) => patch(s, server, |l| {
                l.request = Some(answer.request);
                l.retry_at = None;
            }),
            Err(_) => s.list.stale = true,
        }
    }
    let answer = s.request.as_mut().and_then(|f| poll(&mut f.pending));
    if let Some(result) = answer {
        match result {
            Ok(answer) => {
                let server = s.request.take().map(|f| f.server).unwrap_or_default();
                patch(s, server, |l| {
                    l.request = Some(answer.request);
                    l.retry_at = None;
                });
            }
            Err(error) => {
                if let Some(form) = s.request.as_mut() {
                    form.errors
                        .from_failure(&Failure::from_api(&error), &["message"]);
                }
                // The row is out of date (a new mode, a ban, a delist).
                s.list.stale = true;
            }
        }
    }
    let answer = s.report.as_mut().and_then(|f| poll(&mut f.pending));
    if let Some(result) = answer {
        match result {
            Ok(_) => {
                s.report = None;
                app.toasts.notify(texts::report_sent());
            }
            Err(error) => {
                if let Some(form) = s.report.as_mut() {
                    form.errors
                        .from_failure(&Failure::from_api(&error), &["reason", "details"]);
                }
            }
        }
    }
}

pub(super) fn show(ctx: &egui::Context, app: &AppCtx, s: &mut State) {
    if let Some(form) = s.request.as_mut()
        && !request(ctx, app, form)
    {
        // Closed while sending: the answer still reaches the row, as the
        // web's hook-level `onSuccess`/`onError` outlive the dialog.
        if let Some(form) = s.request.take()
            && let Some(pending) = form.pending
        {
            s.request_sent = Some((form.server, pending));
        }
    }
    if let Some(form) = s.report.as_mut()
        && !report(ctx, app, form)
    {
        s.report = None;
    }
}

/// The dialog's lead: 14px muted (`.discover-request-lead`).
fn lead(ui: &mut Ui, s: &str) {
    let theme = Theme::of(ui.ctx());
    let width = ui.available_width();
    let block = Block::wrapped(ui, s, text::BODY, theme.c.muted, width);
    let (rect, response) = ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
    block.paint(ui.painter(), rect.min);
    crate::widgets::announce(&response, egui::Role::Label, s);
}

/// The form's end: "Abbrechen" and the submit button, right-aligned; after
/// a 429 the button waits out `cooldown` seconds, saying so
/// (`SubmitButton`'s `cooldown`). Returns (cancel, submit).
fn buttons(ui: &mut Ui, submit: &str, sending: &str, pending: bool, cooldown: u64) -> (bool, bool) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 36.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = space::S2;
            let sent = if cooldown > 0 {
                // Each second drops one, as useCooldown's timer does.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(250));
                let waiting = gelabber_i18n::app::submit_cooldown(cooldown);
                ui.add(Button::new(&waiting).primary().disabled(true));
                false
            } else {
                ui.add(
                    Button::new(submit)
                        .primary()
                        .pending(pending.then_some(sending)),
                )
                .clicked()
            };
            let cancel = ui.add(Button::new(common::cancel()).ghost()).clicked();
            (cancel, sent)
        },
    )
    .inner
}

fn form_error(ui: &mut Ui, errors: &FormErrors) {
    if let Some(message) = &errors.form {
        let theme = Theme::of(ui.ctx());
        ui.add(FormError::new(message).on(theme.c.elevated));
    }
}

/// "Beitritt anfragen"; false once it closes.
fn request(ctx: &egui::Context, app: &AppCtx, form: &mut RequestForm) -> bool {
    let pending = form.pending.is_some();
    let mut done = false;
    let output = Modal::new(("discover-request", form.server), texts::request_title()).show(
        ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = space::S5;
            lead(ui, &texts::request_lead(&form.name));
            let field = TextField::new("discover-request-message", &mut form.message)
                .label(texts::request_message())
                .hint(Some(texts::request_message_hint()))
                .autofocus(true)
                .char_limit(MESSAGE_MAX)
                .error(form.errors.field("message"))
                .show(ui);
            if field.response.changed() {
                form.errors.clear_field("message");
            }
            form_error(ui, &form.errors);
            let cooldown = form.errors.cooldown();
            let (cancel, sent) = buttons(
                ui,
                texts::request_submit(),
                texts::request_sending(),
                pending,
                cooldown,
            );
            done = cancel;
            if (sent || field.submitted) && !pending && cooldown == 0 {
                match validate_message(&form.message) {
                    Some(code) => form
                        .errors
                        .set_fields(vec![("message", field_message("message", code))]),
                    None => {
                        let http = app.client.http().clone();
                        let (server, message) = (form.server, form.message.trim().to_owned());
                        form.pending = Some(Pending::spawn(app.client, async move {
                            http.request_join(server, &message).await
                        }));
                    }
                }
            }
        },
        None,
    );
    !(output.close_requested || done)
}

/// "Server melden"; false once it closes.
fn report(ctx: &egui::Context, app: &AppCtx, form: &mut ReportForm) -> bool {
    let pending = form.pending.is_some();
    let mut done = false;
    let output = Modal::new(("discover-report", form.server), texts::report_title()).show(
        ctx,
        |ui| {
            let theme = Theme::of(ui.ctx());
            let c = &theme.c;
            ui.spacing_mut().item_spacing.y = space::S5;
            lead(ui, &texts::report_lead(&form.name));
            // The reasons: the legend, 8px, the cards 8px apart.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = space::S2;
                let width = ui.available_width();
                let legend = Block::single(ui, texts::report_reason(), text::UI, c.text);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(width, legend.height()), Sense::hover());
                legend.paint(ui.painter(), rect.min);
                crate::widgets::announce(&response, egui::Role::Label, texts::report_reason());
                for reason in REASONS {
                    let checked = form.reason == Some(reason);
                    if radio_card(ui, reason_label(reason), None, checked, false) {
                        form.reason = Some(reason);
                        form.errors.clear_field("details");
                        form.errors.clear_field("reason");
                    }
                }
                if let Some(error) = form.errors.field("reason") {
                    let block = Block::wrapped(ui, error, text::META, c.danger, width);
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
                    block.paint(ui.painter(), rect.min);
                    crate::widgets::announce(&response, egui::Role::Alert, error);
                }
            });
            // "Details": the label, the textarea (rows 3), the hint or error.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = space::S1_5;
                let width = ui.available_width();
                let label = Block::single(ui, texts::report_details(), text::UI, c.text);
                let (rect, _) = ui.allocate_exact_size(vec2(width, label.height()), Sense::hover());
                label.paint(ui.painter(), rect.min);
                let error = form.errors.field("details").map(str::to_owned);
                let response = crate::screens::settings::controls::textarea(
                    ui,
                    "discover-report-details",
                    texts::report_details(),
                    &mut form.details,
                    88.0,
                    error.is_some(),
                );
                if response.changed() {
                    if form.details.chars().count() > DETAILS_MAX {
                        form.details = form.details.chars().take(DETAILS_MAX).collect();
                    }
                    form.errors.clear_field("details");
                }
                let (note, color) = match &error {
                    Some(error) => (error.as_str(), c.danger),
                    None => (texts::report_details_hint(), c.muted),
                };
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_description(note));
                let block = Block::wrapped(ui, note, text::META, color, width);
                let (rect, _) = ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
                block.paint(ui.painter(), rect.min);
            });
            form_error(ui, &form.errors);
            let cooldown = form.errors.cooldown();
            let (cancel, sent) = buttons(
                ui,
                texts::report_submit(),
                texts::report_sending(),
                pending,
                cooldown,
            );
            done = cancel;
            if sent && !pending {
                submit_report(app, form);
            }
        },
        None,
    );
    !(output.close_requested || done)
}

fn submit_report(app: &AppCtx, form: &mut ReportForm) {
    let Some(reason) = form.reason else {
        form.errors
            .set_fields(vec![("reason", texts::report_reason_required().to_owned())]);
        return;
    };
    if let Some(code) = validate_details(&form.details, reason) {
        let message = if code == "required" {
            texts::report_details_required().to_owned()
        } else {
            field_message("details", code)
        };
        form.errors.set_fields(vec![("details", message)]);
        return;
    }
    let details = form.details.trim();
    let body = ReportBody {
        reason,
        details: (!details.is_empty()).then(|| details.to_owned()),
    };
    let http = app.client.http().clone();
    let server = form.server;
    form.pending = Some(Pending::spawn(app.client, async move {
        http.report_listing(server, &body).await
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_like_the_web() {
        assert_eq!(validate_message(""), None);
        assert_eq!(validate_message("  Hallo!  "), None);
        assert_eq!(validate_message(&"a".repeat(301)), Some("too_long"));
        assert_eq!(validate_message(" a\u{7}b "), Some("invalid"));
        assert_eq!(validate_details("", ReportReason::Spam), None);
        assert_eq!(
            validate_details("  ", ReportReason::Other),
            Some("required")
        );
        assert_eq!(
            validate_details("Zeile 1\nZeile 2", ReportReason::Other),
            None
        );
        assert_eq!(
            validate_details(&"x".repeat(1001), ReportReason::Spam),
            Some("too_long")
        );
        assert_eq!(
            validate_details("a\u{0}", ReportReason::Spam),
            Some("invalid")
        );
    }
}
