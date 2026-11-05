//! `/forgot-password` (pages/ForgotPasswordPage.tsx).

use egui::Ui;
use reqwest::Method;
use serde_json::{Value, json};

use super::{
    RESEND_COOLDOWN_S, Request, fail, focus_invalid, form_error, layout, result_lines,
    result_with_strong, seconds_left, submit_after, wide_button,
};
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, error_message, field_message, validate_email};
use crate::task::{Pending, poll};
use crate::theme::space;
use crate::widgets::field::field_id;
use crate::widgets::{Button, InlineStatus, StatusTone, TextField};

pub struct State {
    pub email: String,
    pub errors: FormErrors,
    request: Request,
    sent_to: Option<String>,
    resend: Request,
    resend_until: Option<f64>,
    status: Option<(String, StatusTone)>,
}

impl State {
    /// `?email=` from the sign-in page prefills the field.
    pub fn new(email: &str) -> Self {
        Self {
            email: email.to_owned(),
            errors: FormErrors::default(),
            request: None,
            sent_to: None,
            resend: None,
            resend_until: None,
            status: None,
        }
    }
}

fn request_reset(app: &AppCtx, email: &str) -> Request {
    let http = app.client.http().clone();
    let body = json!({ "email": email });
    Some(Pending::spawn(app.client, async move {
        http.send::<Value, Value>(Method::POST, "/auth/password-reset", &body)
            .await
    }))
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    if let Some(result) = poll(&mut s.request) {
        match result {
            Ok(_) => {
                s.sent_to = Some(s.email.trim().to_owned());
                s.resend_until = Some(ui.input(|i| i.time) + RESEND_COOLDOWN_S);
            }
            Err(error) => fail(&mut s.errors, &error, &["email"]),
        }
    }
    if s.sent_to.is_some() {
        sent(ui, app, s);
        return;
    }
    let pending = s.request.is_some();
    layout::frame(ui, app, "forgot-password", |ui, app| {
        layout::heading(ui, "Passwort vergessen");
        layout::lead(
            ui,
            "Du erhältst einen Link, um ein neues Passwort festzulegen.",
        );
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
        form_error(ui, &s.errors);
        let cooldown = s.errors.cooldown();
        if (submit_after(ui, "Link schicken", "Wird gesendet…", pending, cooldown)
            || email.submitted)
            && !pending
            && cooldown == 0
        {
            match validate_email(&s.email) {
                Some(code) => s
                    .errors
                    .set_fields(vec![("email", field_message("email", code))]),
                None => {
                    s.errors = FormErrors::default();
                    s.request = request_reset(app, &s.email);
                }
            }
        }
        focus_invalid(ui, &mut s.errors);
        ui.add_space(space::S6);
        if layout::link_line(ui, "Zurück zur Anmeldung") {
            app.nav.navigate("/login");
        }
    });
}

/// `ResetMailSent`.
fn sent(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    if let Some(result) = poll(&mut s.resend) {
        match result {
            Ok(_) => {
                s.resend_until = Some(ui.input(|i| i.time) + RESEND_COOLDOWN_S);
                s.status = Some(("Neuer Link gesendet.".into(), StatusTone::Success));
            }
            Err(error) => {
                let failure = Failure::from_api(&error);
                s.status = Some((error_message(&failure.code).into(), StatusTone::Error));
            }
        }
    }
    let email = s.sent_to.clone().unwrap_or_default();
    let seconds = seconds_left(ui, s.resend_until);
    let pending = s.resend.is_some();
    layout::frame(ui, app, "forgot-password-sent", |ui, app| {
        layout::focused_heading(ui, "Postfach prüfen");
        result_with_strong(
            ui,
            "Falls zu ",
            &email,
            " ein Konto existiert, wurde ein Link gesendet. Er gilt eine Stunde.",
        );
        result_lines(
            ui,
            &[(
                "Keine Mail erhalten? Prüfe den Spam-Ordner oder sende den Link erneut.",
                true,
            )],
        );
        ui.add_space(space::S6);
        let label = if seconds > 0 {
            format!("Erneut senden ({seconds} s)")
        } else {
            "Erneut senden".to_owned()
        };
        let resend = ui.add(
            Button::new(&label)
                .large()
                .fill()
                .disabled(seconds > 0)
                .pending(pending.then_some("Wird gesendet…")),
        );
        if resend.clicked() {
            s.resend = request_reset(app, &email);
        }
        ui.add_space(space::S2);
        let status = s.status.as_ref();
        ui.add(InlineStatus::new(
            status.map(|(m, _)| m.as_str()),
            status.map_or(StatusTone::Neutral, |(_, t)| *t),
        ));
        ui.add_space(space::S2);
        if wide_button(ui, "Andere Adresse verwenden", true) {
            s.sent_to = None;
            s.status = None;
            ui.memory_mut(|m| m.request_focus(field_id("email")));
        }
        ui.add_space(space::S6);
        if layout::link_line(ui, "Zurück zur Anmeldung") {
            app.nav.navigate("/login");
        }
    });
}
