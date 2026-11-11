//! `/reset-password#token=…` (pages/ResetPasswordPage.tsx).

use egui::Ui;
use gelabber_i18n::auth;
use reqwest::Method;
use serde_json::{Value, json};

use super::{Request, fail, focus_invalid, form_error, layout, result_lines, submit_after};
use crate::app::AppCtx;
use crate::rules::{
    Failure, FormErrors, PASSWORD_MIN, field_message, link_token, validate_password,
};
use crate::task::{Pending, poll};
use crate::theme::space;
use crate::widgets::TextField;

pub struct State {
    token: Option<String>,
    pub password: String,
    pub errors: FormErrors,
    dead_link: bool,
    request: Request,
}

impl State {
    /// `fragment` is the location's `#…` part, read once.
    pub fn new(fragment: &str) -> Self {
        let token = link_token(fragment);
        Self {
            dead_link: token.is_none(),
            token,
            password: String::new(),
            errors: FormErrors::default(),
            request: None,
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    if let Some(result) = poll(&mut s.request) {
        match result {
            Ok(session) => {
                app.session.adopt(&session);
                app.toasts.notify(auth::reset_done());
                app.nav.replace("/");
                return;
            }
            Err(error) if Failure::from_api(&error).code == "token_invalid" => s.dead_link = true,
            Err(error) => fail(&mut s.errors, &error, &["password"]),
        }
    }
    if s.dead_link {
        layout::frame(ui, app, "reset-dead", |ui, app| {
            layout::focused_heading(ui, auth::page_link_expired());
            result_lines(ui, &[(auth::page_link_used(), false)]);
            ui.add_space(space::S6);
            if layout::action_link(ui, auth::reset_request_new()) {
                app.nav.navigate("/forgot-password");
            }
        });
        return;
    }
    let pending = s.request.is_some();
    layout::frame(ui, app, "reset-password", |ui, app| {
        layout::heading(ui, auth::reset_title());
        layout::lead(ui, auth::reset_lead());
        ui.add_space(space::S6);
        let hint = auth::field_error_password_too_short(PASSWORD_MIN);
        let password = TextField::new("password", &mut s.password)
            .label(auth::reset_password_label())
            .password()
            .autofocus(true)
            .error(s.errors.field("password"))
            .hint(Some(&hint))
            .show(ui);
        if password.response.changed() {
            s.errors.clear_field("password");
        }
        ui.add_space(space::S5);
        form_error(ui, &s.errors);
        let cooldown = s.errors.cooldown();
        if (submit_after(
            ui,
            auth::reset_submit(),
            auth::reset_submitting(),
            pending,
            cooldown,
        ) || password.submitted)
            && !pending
            && cooldown == 0
        {
            match validate_password(&s.password) {
                Some(code) => s
                    .errors
                    .set_fields(vec![("password", field_message("password", code))]),
                None => {
                    s.errors = FormErrors::default();
                    let http = app.client.http().clone();
                    let body = json!({ "token": s.token, "password": s.password });
                    s.request = Some(Pending::spawn(app.client, async move {
                        let _: Value = http
                            .send(Method::POST, "/auth/password-reset/confirm", &body)
                            .await?;
                        // A new session: take its user and CSRF token.
                        http.session().await
                    }));
                }
            }
        }
        focus_invalid(ui, &mut s.errors);
    });
}
