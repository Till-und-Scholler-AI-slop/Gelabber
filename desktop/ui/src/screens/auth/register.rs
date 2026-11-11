//! `/register` (pages/RegisterPage.tsx) and its "Postfach prüfen" step.

use egui::Ui;
use gelabber_client::{ApiError, OidcPurpose, OidcStart, RegisterBody, RegistrationMode};
use gelabber_i18n::auth;
use serde_json::{Value, json};

use super::registration::{
    InvitationState, invitation_param, invite_code_of, is_admission_error, shows_invitation_field,
    shows_invitation_only, validate_invitation,
};
use super::{
    RESEND_COOLDOWN_S, Request, banner, focus_invalid, form_error, invite_banner, layout, oidc,
    oidc_error_line, result_lines, result_with_strong, seconds_left, submit_after, wide_button,
};
use crate::app::{AppCtx, is_invite_redirect, with_query};
use crate::rules::{
    Failure, FormErrors, PASSWORD_MIN, error_message, field_message, validate_email, validate_name,
    validate_password,
};
use crate::task::{Pending, poll};
use crate::theme::space;
use crate::widgets::field::field_id;
use crate::widgets::{InlineStatus, StatusTone, TextField};

#[derive(Default)]
pub struct State {
    pub name: String,
    pub email: String,
    pub password: String,
    pub errors: FormErrors,
    request: Request,
    /// Set once the server mailed a confirmation link.
    mailed_to: Option<String>,
    resend: Request,
    resend_until: Option<f64>,
    status: Option<(String, StatusTone)>,
    /// "Über Google anmelden" and its flow through the system browser.
    pub provider: oidc::ProviderSignIn,
    /// The "Einladungscode" field (#517), pre-filled from `?invitation=`.
    pub invitation: String,
    /// `GET /api/auth/registration`, asked once per page.
    mode: Option<RegistrationMode>,
    mode_request: Option<Pending<Result<RegistrationMode, ApiError>>>,
    /// The page was set up from its address (the code, a sign-in refusal).
    started: bool,
    /// The API (or a refused sign-in) asked for a code.
    asked: bool,
}

const ORDER: [&str; 4] = ["name", "email", "password", "invitation"];

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let redirect = app.nav.location().redirect();
    let param = invitation_param(app.nav.location().query("invitation")).map(str::to_owned);
    let oidc_refusal = crate::rules::oidc_error(app.nav.location().query("oidc_error"));
    if !s.started {
        s.started = true;
        s.invitation = param.clone().unwrap_or_default();
        // A refusal for want of a code brings the field up even before
        // (or without) the mode check.
        s.asked = oidc_refusal.is_some_and(is_admission_error);
        let http = app.client.http().clone();
        s.mode_request = Some(Pending::spawn(app.client, async move {
            http.registration_mode().await
        }));
    }
    if let Some(result) = poll(&mut s.mode_request) {
        s.mode = result.ok();
    }
    let code_state = |asked| InvitationState {
        mode: s.mode,
        invitation: param.as_deref(),
        redirect: redirect.as_deref(),
        asked,
    };
    if let Some(result) = poll(&mut s.request) {
        match result {
            // 202 with `user: null`: account mail is on, nobody is signed in.
            Ok(response) if response.get("user").is_none_or(Value::is_null) => {
                s.mailed_to = Some(s.email.trim().to_owned());
                s.resend_until = Some(ui.input(|i| i.time) + RESEND_COOLDOWN_S);
            }
            Ok(response) => {
                app.session.adopt(&response);
                app.nav.replace(redirect.as_deref().unwrap_or("/"));
                return;
            }
            Err(error) => {
                let mut failure = Failure::from_api(&error);
                if is_admission_error(&failure.code) {
                    if shows_invitation_field(code_state(true)) {
                        s.asked = true;
                    } else {
                        // A server invite was followed: no field to point at,
                        // so the code becomes the form's line.
                        failure.fields.remove("invitation");
                    }
                }
                s.errors.from_failure(&failure, &ORDER);
            }
        }
    }
    let ask_code = shows_invitation_field(code_state(s.asked));
    let invitation_only = oidc_refusal.is_none() && shows_invitation_only(code_state(s.asked));
    if s.provider.poll(ui, app, &mut s.errors, &ORDER) {
        return;
    }
    if s.mailed_to.is_some() {
        mail_sent(ui, app, s, redirect.as_deref());
        return;
    }
    let invited = is_invite_redirect(redirect.as_deref());
    let pending = s.request.is_some();

    layout::frame(ui, app, "register", |ui, app| {
        if invited {
            invite_banner(ui);
        }
        // A refused provider sign-in says why at the form, as on /login.
        if invitation_only {
            banner(ui, auth::register_invitation_only(), false);
        }
        layout::heading(ui, auth::register_title());
        if layout::lead_with_link(ui, auth::register_have_account(), auth::register_sign_in()) {
            app.nav.navigate(&with_query(
                "/login",
                &[("redirect", redirect.as_deref().unwrap_or(""))],
            ));
        }
        // A new account on an invitation-only instance needs the code too.
        let typed = Some(s.invitation.trim().to_owned()).filter(|c| ask_code && !c.is_empty());
        let start = || OidcStart {
            redirect: redirect.clone(),
            invitation: typed,
            ..OidcStart::new(OidcPurpose::SignIn)
        };
        if !s.provider.show(ui, app, &mut s.errors, start) {
            ui.add_space(space::S6);
        }
        let name = TextField::new("name", &mut s.name)
            .label(auth::field_label_name())
            .autofocus(true)
            .char_limit(64)
            .error(s.errors.field("name"))
            .hint(Some(auth::register_name_hint()))
            .show(ui);
        if name.response.changed() {
            s.errors.clear_field("name");
        }
        ui.add_space(space::S5);
        let email = TextField::new("email", &mut s.email)
            .label(auth::field_label_email())
            .error(s.errors.field("email"))
            .show(ui);
        if email.response.changed() {
            s.errors.clear_field("email");
        }
        ui.add_space(space::S5);
        let hint = auth::field_error_password_too_short(PASSWORD_MIN);
        let password = TextField::new("password", &mut s.password)
            .label(auth::field_label_password())
            .password()
            .error(s.errors.field("password"))
            .hint(Some(&hint))
            .show(ui);
        if password.response.changed() {
            s.errors.clear_field("password");
        }
        ui.add_space(space::S5);
        let mut invitation_submitted = false;
        if ask_code {
            let invitation = TextField::new("invitation", &mut s.invitation)
                .label(auth::field_label_invitation())
                .char_limit(64)
                .error(s.errors.field("invitation"))
                .show(ui);
            if invitation.response.changed() {
                s.errors.clear_field("invitation");
            }
            invitation_submitted = invitation.submitted;
            ui.add_space(space::S5);
        }
        if !oidc_error_line(ui, app, &s.errors) {
            form_error(ui, &s.errors);
        }
        let cooldown = s.errors.cooldown();
        let clicked = submit_after(
            ui,
            auth::register_submit(),
            auth::register_submitting(),
            pending,
            cooldown,
        );
        if (clicked
            || name.submitted
            || email.submitted
            || password.submitted
            || invitation_submitted)
            && !pending
            && cooldown == 0
        {
            let checks = [
                ("name", validate_name(&s.name)),
                ("email", validate_email(&s.email)),
                ("password", validate_password(&s.password)),
                (
                    "invitation",
                    ask_code
                        .then(|| validate_invitation(&s.invitation, s.mode, redirect.as_deref()))
                        .flatten(),
                ),
            ];
            let fields: Vec<(&str, String)> = checks
                .iter()
                .filter_map(|(f, code)| code.map(|c| (*f, field_message(f, c))))
                .collect();
            if fields.is_empty() {
                s.errors = FormErrors::default();
                s.provider.cancel();
                let http = app.client.http().clone();
                // `admissionCodes`: the typed invitation (when its field
                // shows) and the server invite of an `/invite/{code}` target.
                let body = RegisterBody {
                    invitation: Some(s.invitation.trim().to_owned())
                        .filter(|c| ask_code && !c.is_empty()),
                    server_invite: invite_code_of(redirect.as_deref()).map(str::to_owned),
                    ..RegisterBody::new(&s.email, &s.password, &s.name)
                };
                s.request = Some(Pending::spawn(app.client, async move {
                    let response = http.register_with(&body).await?;
                    if response.user.is_some() {
                        // Signed in: the new session has a new CSRF token.
                        return http.session().await;
                    }
                    Ok(json!({ "user": null, "verification_sent": response.verification_sent }))
                }));
            } else {
                s.errors.set_fields(fields);
            }
        }
        focus_invalid(ui, &mut s.errors);
    });
}

/// `MailSent`: what happens next, a resend, and a way back to fix the
/// address.
fn mail_sent(ui: &mut Ui, app: &mut AppCtx, s: &mut State, redirect: Option<&str>) {
    if let Some(result) = poll(&mut s.resend) {
        match result {
            Ok(response) => {
                // Confirmed meanwhile on another device: signed in now.
                app.session.adopt(&response);
                app.nav.replace(redirect.unwrap_or("/"));
                return;
            }
            Err(error) => {
                let failure = Failure::from_api(&error);
                if failure.code == "email_unverified" {
                    s.resend_until = Some(ui.input(|i| i.time) + RESEND_COOLDOWN_S);
                    s.status = Some((auth::page_new_link_sent().into(), StatusTone::Success));
                } else {
                    s.status = Some((error_message(&failure.code).into(), StatusTone::Error));
                }
            }
        }
    }
    let email = s.mailed_to.clone().unwrap_or_default();
    let seconds = seconds_left(ui, s.resend_until);
    let pending = s.resend.is_some();
    layout::frame(ui, app, "register-mail", |ui, app| {
        layout::focused_heading(ui, auth::page_check_inbox());
        result_with_strong(ui, auth::register_mail_sent(), &email);
        if is_invite_redirect(redirect) {
            result_lines(ui, &[(auth::register_then_invite(), false)]);
        }
        result_lines(ui, &[(auth::page_no_mail(), true)]);
        ui.add_space(space::S6);
        let label = if seconds > 0 {
            auth::page_resend_in(seconds)
        } else {
            auth::register_resend().to_owned()
        };
        let resend = ui.add(
            crate::widgets::Button::new(&label)
                .large()
                .fill()
                .disabled(seconds > 0)
                .pending(pending.then_some(auth::page_sending())),
        );
        if resend.clicked() {
            // Signing in to an unconfirmed account sends a fresh link.
            let http = app.client.http().clone();
            let password = s.password.clone();
            let email = email.clone();
            s.resend = Some(Pending::spawn(app.client, async move {
                http.login(&email, &password).await
            }));
        }
        ui.add_space(space::S2);
        let status = s.status.as_ref();
        ui.add(InlineStatus::new(
            status.map(|(m, _)| m.as_str()),
            status.map_or(StatusTone::Neutral, |(_, t)| *t),
        ));
        // The status line is always mounted: both gaps of the column stay.
        ui.add_space(space::S2);
        if wide_button(ui, auth::page_other_address(), true) {
            s.mailed_to = None;
            s.status = None;
            ui.memory_mut(|m| m.request_focus(field_id("email")));
        }
        ui.add_space(space::S6);
        if layout::link_line(ui, auth::page_to_sign_in()) {
            app.nav.navigate(&with_query(
                "/login",
                &[("redirect", redirect.unwrap_or(""))],
            ));
        }
    });
}
