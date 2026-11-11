//! `/verify-email#token=…` (pages/VerifyEmailPage.tsx): confirming takes
//! the account's password.

use egui::Ui;
use gelabber_i18n::auth;
use reqwest::Method;
use serde_json::{Value, json};

use super::{Request, fail, focus_invalid, form_error, layout, result_lines, submit_after};
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, field_message, link_token};
use crate::task::{Pending, poll};
use crate::text;
use crate::theme::{Theme, space};
use crate::widgets::TextField;

pub struct State {
    token: Option<String>,
    pub password: String,
    pub errors: FormErrors,
    dead_link: bool,
    request: Request,
}

impl State {
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
                app.toasts.notify(auth::verify_done());
                app.nav.replace("/");
                return;
            }
            Err(error) if Failure::from_api(&error).code == "token_invalid" => s.dead_link = true,
            Err(error) => fail(&mut s.errors, &error, &["password"]),
        }
    }
    if s.dead_link {
        layout::frame(ui, app, "verify-dead", |ui, app| {
            layout::focused_heading(ui, auth::page_link_expired());
            result_lines(
                ui,
                &[
                    (auth::page_link_used(), false),
                    (auth::verify_sign_in_for_new(), true),
                ],
            );
            ui.add_space(space::S6);
            if layout::action_link(ui, auth::page_to_sign_in()) {
                app.nav.navigate("/login");
            }
        });
        return;
    }
    let pending = s.request.is_some();
    layout::frame(ui, app, "verify-email", |ui, app| {
        layout::heading(ui, auth::verify_title());
        layout::lead(ui, auth::verify_lead());
        ui.add_space(space::S6);
        let password = TextField::new("password", &mut s.password)
            .label(auth::field_label_password())
            .password()
            .autofocus(true)
            .error(s.errors.field("password"))
            .show(ui);
        if password.response.changed() {
            s.errors.clear_field("password");
        }
        ui.add_space(space::S5);
        form_error(ui, &s.errors);
        let cooldown = s.errors.cooldown();
        if (submit_after(
            ui,
            auth::verify_submit(),
            auth::verify_submitting(),
            pending,
            cooldown,
        ) || password.submitted)
            && !pending
            && cooldown == 0
        {
            if s.password.is_empty() {
                s.errors
                    .set_fields(vec![("password", field_message("password", "required"))]);
            } else {
                s.errors = FormErrors::default();
                let http = app.client.http().clone();
                let body = json!({ "token": s.token, "password": s.password });
                s.request = Some(Pending::spawn(app.client, async move {
                    let _: Value = http.send(Method::POST, "/auth/verify-email", &body).await?;
                    http.session().await
                }));
            }
        }
        focus_invalid(ui, &mut s.errors);
        ui.add_space(space::S5);
        let theme = Theme::of(ui.ctx());
        let style = text::META.size(13.0);
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
            app.nav.navigate("/forgot-password");
        }
    });
}
