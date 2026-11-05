//! `/login` (pages/LoginPage.tsx).

use egui::{Sense, Ui, Vec2};
use gelabber_client::ApiError;

use super::{
    Request, banner, fail, focus_invalid, form_error, invite_banner, layout, submit_after,
};
use crate::app::{AppCtx, is_invite_redirect, with_query};
use crate::rules::{FormErrors, field_message, validate_email};
use crate::task::{Pending, poll};
use crate::text::{self, Block};
use crate::theme::{Theme, space};
use crate::widgets::{Button, TextField};

#[derive(Default)]
pub struct State {
    pub email: String,
    pub password: String,
    pub errors: FormErrors,
    request: Request,
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
            Err(error) => fail(&mut s.errors, &error, &ORDER),
        }
    }
    let invited = is_invite_redirect(redirect.as_deref());
    let expired = app.nav.location().query("reason") == Some("expired");
    let pending = s.request.is_some();

    layout::frame(ui, app, "login", |ui, app| {
        if invited {
            invite_banner(ui);
        }
        if expired {
            banner(ui, "Sitzung abgelaufen. Melde dich erneut an.", false);
        }
        layout::heading(ui, "Anmelden");
        let register = with_query(
            "/register",
            &[("redirect", redirect.as_deref().unwrap_or(""))],
        );
        if invited {
            layout::lead(ui, "Neu bei Gelabber?");
            ui.add_space(space::S3);
            if ui
                .add(Button::new("Konto anlegen").large().fill())
                .clicked()
            {
                app.nav.navigate(&register);
            }
        } else if layout::lead_with_link(ui, "Noch kein Konto?", "Registrieren") {
            app.nav.navigate(&register);
        }

        ui.add_space(space::S6);
        let email = TextField::new("email", &mut s.email)
            .label("E-Mail-Adresse")
            .autofocus(true)
            .error(s.errors.field("email"))
            .show(ui);
        if email.response.changed() {
            s.errors.clear_field("email");
        }
        ui.add_space(space::S5);
        let password = TextField::new("password", &mut s.password)
            .label("Passwort")
            .password()
            .error(s.errors.field("password"))
            .show(ui);
        if password.response.changed() {
            s.errors.clear_field("password");
        }
        ui.add_space(space::S5);
        form_error(ui, &s.errors);
        let cooldown = s.errors.cooldown();
        let clicked = submit_after(ui, "Anmelden", "Anmelden…", pending, cooldown);
        if (clicked || email.submitted || password.submitted) && !pending && cooldown == 0 {
            let mut fields = Vec::new();
            if let Some(code) = validate_email(&s.email) {
                fields.push(("email", field_message("email", code)));
            }
            if s.password.is_empty() {
                fields.push(("password", field_message("password", "required")));
            }
            if fields.is_empty() {
                s.errors = FormErrors::default();
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

/// `.auth-recovery`: the "Passwort vergessen?" link when the server sends
/// mail, the hint when it does not, an empty 20px line while unknown.
fn recovery(ui: &mut Ui, app: &mut AppCtx, email: &str) {
    let theme = Theme::of(ui.ctx());
    let style = text::META.size(13.0);
    match app.account_mail {
        Some(true) => {
            let block = text::Rich::new(style)
                .push(
                    "Passwort vergessen?",
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
                "Passwort vergessen?",
            ) {
                let typed = email.trim();
                app.nav
                    .navigate(&with_query("/forgot-password", &[("email", typed)]));
            }
        }
        Some(false) => {
            let block = Block::wrapped(
                ui,
                "Passwort vergessen? Frag die Person, die diesen Gelabber-Server betreibt.",
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
