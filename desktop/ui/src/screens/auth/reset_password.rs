//! `/reset-password#token=…` (pages/ResetPasswordPage.tsx).

use egui::Ui;
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
                app.toasts
                    .notify("Passwort geändert. Andere Geräte sind abgemeldet.");
                app.nav.replace("/");
                return;
            }
            Err(error) if Failure::from_api(&error).code == "token_invalid" => s.dead_link = true,
            Err(error) => fail(&mut s.errors, &error, &["password"]),
        }
    }
    if s.dead_link {
        layout::frame(ui, app, "reset-dead", |ui, app| {
            layout::focused_heading(ui, "Link abgelaufen");
            result_lines(
                ui,
                &[(
                    "Dieser Link ist abgelaufen oder wurde schon benutzt.",
                    false,
                )],
            );
            ui.add_space(space::S6);
            if layout::action_link(ui, "Neuen Link anfordern") {
                app.nav.navigate("/forgot-password");
            }
        });
        return;
    }
    let pending = s.request.is_some();
    layout::frame(ui, app, "reset-password", |ui, app| {
        layout::heading(ui, "Neues Passwort");
        layout::lead(ui, "Danach bist du auf allen anderen Geräten abgemeldet.");
        ui.add_space(space::S6);
        let hint = format!("Mindestens {PASSWORD_MIN} Zeichen.");
        let password = TextField::new("password", &mut s.password)
            .label("Neues Passwort")
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
            "Passwort speichern",
            "Wird gespeichert…",
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
