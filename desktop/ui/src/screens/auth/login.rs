//! `/login` (pages/LoginPage.tsx).

use egui::{Sense, Ui, Vec2};
use gelabber_client::{ApiError, OidcPurpose, OidcStart};
use gelabber_i18n::auth;
use reqwest::Method;
use serde_json::{Value, json};

use super::{
    RESEND_COOLDOWN_S, Request, banner, fail, focus_invalid, form_error, invite_banner, layout,
    oidc, oidc_error_line, seconds_left, submit_after,
};
use crate::app::{AppCtx, is_invite_redirect, with_query};
use crate::rules::{Failure, FormErrors, error_message, field_message, validate_email};
use crate::task::{Pending, poll};
use crate::text::{self, Block};
use crate::theme::{Theme, space};
use crate::widgets::{Button, FormError, InlineStatus, StatusTone, TextField};

#[derive(Default)]
pub struct State {
    pub email: String,
    pub password: String,
    pub errors: FormErrors,
    request: Request,
    /// "Über Google anmelden" and its flow through the system browser.
    pub provider: oidc::ProviderSignIn,
    /// Why the last sign-in on this page was refused, for the help.
    refusal: Option<String>,
    help: Help,
}

/// `ConfirmationHelp` (GEL-40): the way to a new confirmation link for an
/// address that was never confirmed.
#[derive(Default)]
struct Help {
    request: Request,
    until: Option<f64>,
    status: Option<(String, StatusTone)>,
}

/// `confirmationHelpReason` (web/src/auth/confirmationHelp.ts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpReason {
    /// A password sign-in was refused and sent a link.
    EmailUnverified,
    /// A provider sign-in found the unconfirmed account.
    Provider,
}

/// Which help the page shows: only with account mail; a refusal of the
/// page's own sign-in replaces the provider's `?oidc_error=`.
pub fn help_reason(
    refusal: Option<&str>,
    oidc_error: Option<&str>,
    account_mail: Option<bool>,
) -> Option<HelpReason> {
    if account_mail != Some(true) {
        return None;
    }
    match refusal {
        Some("email_unverified") => Some(HelpReason::EmailUnverified),
        // The exchange's refusal, or the code the flow itself ended with.
        Some("oidc_account_unverified" | "account_unverified") => Some(HelpReason::Provider),
        Some(_) => None,
        None => (oidc_error == Some("account_unverified")).then_some(HelpReason::Provider),
    }
}

const ORDER: [&str; 2] = ["email", "password"];

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let redirect = app.nav.location().redirect();
    if let Some(result) = poll(&mut s.request) {
        match result {
            Ok(session) => {
                app.session.adopt(&session);
                app.nav.replace(redirect.as_deref().unwrap_or("/"));
                return;
            }
            Err(error) => {
                let code = Failure::from_api(&error).code;
                if code == "email_unverified" {
                    // The refusal sent a link just now.
                    s.help.until = Some(ui.input(|i| i.time) + RESEND_COOLDOWN_S);
                }
                s.refusal = Some(code);
                fail(&mut s.errors, &error, &ORDER);
            }
        }
    }
    if s.provider.poll(ui, app, &mut s.errors, &ORDER) {
        return;
    }
    if let Some(code) = s.provider.take_refusal() {
        s.refusal = Some(code);
    }
    let help = help_reason(
        s.refusal.as_deref(),
        app.nav.location().query("oidc_error"),
        app.account_mail,
    );
    let invited = is_invite_redirect(redirect.as_deref());
    let reason = app.nav.location().query("reason");
    let ended = reason.and_then(session_end_notice);
    let pending = s.request.is_some();

    layout::frame(ui, app, "login", |ui, app| {
        if invited {
            invite_banner(ui);
        }
        if let Some(notice) = ended {
            banner(ui, notice, false);
        }
        layout::heading(ui, auth::login_title());
        let register = with_query(
            "/register",
            &[("redirect", redirect.as_deref().unwrap_or(""))],
        );
        if invited {
            layout::lead(ui, auth::login_new_here());
            ui.add_space(space::S3);
            if ui
                .add(Button::new(auth::login_create_account()).large().fill())
                .clicked()
            {
                app.nav.navigate(&register);
            }
        } else if layout::lead_with_link(ui, auth::login_no_account(), auth::login_register()) {
            app.nav.navigate(&register);
        }
        let start = || OidcStart {
            redirect: redirect.clone(),
            ..OidcStart::new(OidcPurpose::SignIn)
        };
        if !s.provider.show(ui, app, &mut s.errors, start) {
            ui.add_space(space::S6);
        }
        let email = TextField::new("email", &mut s.email)
            .label(auth::field_label_email())
            .autofocus(true)
            .error(s.errors.field("email"))
            .show(ui);
        if email.response.changed() {
            s.errors.clear_field("email");
        }
        ui.add_space(space::S5);
        let password = TextField::new("password", &mut s.password)
            .label(auth::field_label_password())
            .password()
            .error(s.errors.field("password"))
            .show(ui);
        if password.response.changed() {
            s.errors.clear_field("password");
        }
        ui.add_space(space::S5);
        if help == Some(HelpReason::Provider) && (s.refusal.is_some() || s.errors.form.is_none()) {
            // With mail on the refusal names the way out, below.
            let theme = Theme::of(ui.ctx());
            ui.add(FormError::new(auth::login_oidc_account_unverified()).on(theme.c.panel));
            ui.add_space(space::S5);
        } else if !oidc_error_line(ui, app, &s.errors) {
            form_error(ui, &s.errors);
        }
        if let Some(reason) = help {
            confirmation_help(ui, app, s, reason);
            ui.add_space(space::S5);
        }
        let cooldown = s.errors.cooldown();
        let clicked = submit_after(
            ui,
            auth::login_submit(),
            auth::login_submitting(),
            pending,
            cooldown,
        );
        if (clicked || email.submitted || password.submitted) && !pending && cooldown == 0 {
            let mut fields = Vec::new();
            if let Some(code) = validate_email(&s.email) {
                fields.push(("email", field_message("email", code)));
            }
            if s.password.is_empty() {
                fields.push(("password", field_message("password", "required")));
            }
            s.refusal = None;
            if fields.is_empty() {
                s.errors = FormErrors::default();
                // The password goes first; the browser's sign-in is dropped.
                s.provider.cancel();
                let http = app.client.http().clone();
                let (email, password) = (s.email.clone(), s.password.clone());
                s.request = Some(Pending::spawn(app.client, async move {
                    http.login(&email, &password).await
                }));
            } else {
                s.errors.set_fields(fields);
            }
        }
        focus_invalid(ui, &mut s.errors);

        ui.add_space(space::S5);
        recovery(ui, app, &s.email);
    });
}

/// `.auth-confirm-help`: what to do, the button that mails a new link
/// (once a minute) and its status line, and after a provider's refusal the
/// way to a new password, which confirms the address too.
fn confirmation_help(ui: &mut Ui, app: &mut AppCtx, s: &mut State, reason: HelpReason) {
    if let Some(result) = poll(&mut s.help.request) {
        s.help.status = Some(match result {
            Ok(_) => {
                s.help.until = Some(ui.input(|i| i.time) + RESEND_COOLDOWN_S);
                (auth::confirm_help_sent().to_owned(), StatusTone::Success)
            }
            Err(error) => {
                let failure = Failure::from_api(&error);
                if let Some(wait) = failure.retry_after.filter(|w| *w > 0.0) {
                    s.help.until = Some(ui.input(|i| i.time) + wait.ceil());
                }
                (error_message(&failure.code).to_owned(), StatusTone::Error)
            }
        });
    }
    let theme = Theme::of(ui.ctx());
    let lead = match reason {
        HelpReason::Provider => auth::confirm_help_provider_lead(),
        HelpReason::EmailUnverified => auth::page_no_mail(),
    };
    let block = Block::wrapped(ui, lead, text::META, theme.c.muted, ui.available_width());
    let size = Vec2::new(
        ui.available_width(),
        crate::widgets::field::layout_unit(block.height()),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    crate::widgets::announce(&response, egui::Role::Label, lead);
    block.paint(ui.painter(), rect.min);
    ui.add_space(space::S2);

    let seconds = seconds_left(ui, s.help.until);
    let label = if seconds > 0 {
        auth::page_resend_in(seconds)
    } else if reason == HelpReason::Provider {
        auth::confirm_help_send().to_owned()
    } else {
        auth::register_resend().to_owned()
    };
    let pending = s.help.request.is_some();
    let send = ui.add(
        Button::new(&label)
            .fill()
            .disabled(seconds > 0)
            .pending(pending.then_some(auth::page_sending())),
    );
    if send.clicked() && !pending && seconds == 0 {
        match validate_email(&s.email) {
            Some(code) => s
                .errors
                .set_fields(vec![("email", field_message("email", code))]),
            None => {
                s.help.status = None;
                let http = app.client.http().clone();
                let body = json!({ "email": s.email.trim() });
                s.help.request = Some(Pending::spawn(app.client, async move {
                    http.send::<Value, Value>(Method::POST, "/auth/verify-email/resend", &body)
                        .await
                }));
            }
        }
    }
    ui.add_space(space::S2);
    let status = s.help.status.as_ref();
    ui.add(InlineStatus::new(
        status.map(|(m, _)| m.as_str()),
        status.map_or(StatusTone::Neutral, |(_, t)| *t),
    ));
    if reason == HelpReason::Provider {
        ui.add_space(space::S2);
        if ui
            .add(Button::new(auth::page_forgot_password()).ghost().fill())
            .clicked()
        {
            let typed = s.email.trim().to_owned();
            app.nav
                .navigate(&with_query("/forgot-password", &[("email", &typed)]));
        }
    }
}

/// `.auth-recovery`: the "Passwort vergessen?" link when the server sends
/// mail, the hint when it does not, an empty 20px line while unknown.
fn recovery(ui: &mut Ui, app: &mut AppCtx, email: &str) {
    let theme = Theme::of(ui.ctx());
    let style = text::META.size(13.0);
    match app.account_mail {
        Some(true) => {
            let block = text::Rich::new(style)
                .push(
                    auth::page_forgot_password(),
                    style.weight(500.0),
                    theme.c.accent_text,
                )
                .layout(ui);
            let top_left = ui.cursor().min;
            layout::show_block(ui, &block);
            if layout::paragraph_link(
                ui,
                &block,
                top_left,
                0,
                style.weight(500.0),
                auth::page_forgot_password(),
            ) {
                let typed = email.trim();
                app.nav
                    .navigate(&with_query("/forgot-password", &[("email", typed)]));
            }
        }
        Some(false) => {
            let block = Block::wrapped(
                ui,
                auth::login_forgot_password_no_mail(),
                style,
                theme.c.muted,
                ui.available_width(),
            );
            layout::show_block(ui, &block);
        }
        None => {
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
        }
    }
}

/// For tests and tools: the error a failed login leaves on the form.
pub fn errors_after(error: &ApiError) -> FormErrors {
    let mut errors = FormErrors::default();
    fail(&mut errors, error, &ORDER);
    errors
}

/// The sign-in page's notice for `reason=`: why the session ended (GEL-83),
/// as on the web's login page.
fn session_end_notice(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "expired" => auth::login_session_expired(),
        "signed_out" => auth::login_signed_out(),
        "password_changed" => auth::login_password_changed(),
        "disabled" => auth::login_account_disabled(),
        "deleted" => auth::login_account_deleted(),
        _ => return None,
    })
}
