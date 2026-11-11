//! `/register` (pages/RegisterPage.tsx) and its "Postfach prüfen" step.

use egui::Ui;
use gelabber_i18n::auth;
use reqwest::Method;
use serde_json::{Value, json};

use super::{
    RESEND_COOLDOWN_S, Request, fail, focus_invalid, form_error, invite_banner, layout,
    result_lines, result_with_strong, seconds_left, submit_after, wide_button,
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
}

const ORDER: [&str; 3] = ["name", "email", "password"];

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let redirect = app.nav.location().redirect();
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
            Err(error) => fail(&mut s.errors, &error, &ORDER),
        }
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
        layout::heading(ui, auth::register_title());
        if layout::lead_with_link(ui, auth::register_have_account(), auth::register_sign_in()) {
            app.nav.navigate(&with_query(
                "/login",
                &[("redirect", redirect.as_deref().unwrap_or(""))],
            ));
        }
        ui.add_space(space::S6);
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
        form_error(ui, &s.errors);
        let cooldown = s.errors.cooldown();
        let clicked = submit_after(
            ui,
            auth::register_submit(),
            auth::register_submitting(),
            pending,
            cooldown,
        );
        if (clicked || name.submitted || email.submitted || password.submitted)
            && !pending
            && cooldown == 0
        {
            let checks = [
                ("name", validate_name(&s.name)),
                ("email", validate_email(&s.email)),
                ("password", validate_password(&s.password)),
            ];
            let fields: Vec<(&str, String)> = checks
                .iter()
                .filter_map(|(f, code)| code.map(|c| (*f, field_message(f, c))))
                .collect();
            if fields.is_empty() {
                s.errors = FormErrors::default();
                let http = app.client.http().clone();
                let body = json!({ "email": s.email, "password": s.password, "name": s.name });
                s.request = Some(Pending::spawn(app.client, async move {
                    let response: Value = http.send(Method::POST, "/auth/register", &body).await?;
                    if response.get("user").is_some_and(|u| !u.is_null()) {
                        // Signed in: the new session has a new CSRF token.
                        return http.session().await;
                    }
                    Ok(response)
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
