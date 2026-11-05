//! The signed-out screens: sign-in, register, the mailed links and the
//! invite landing, all in the [`layout::frame`].

pub mod forgot_password;
pub mod invite;
pub mod layout;
pub mod login;
pub mod not_found;
pub mod recovery;
pub mod register;
pub mod reset_password;
pub mod verify_email;

use egui::{Color32, Rect, Sense, Ui, Vec2, pos2};
use gelabber_client::ApiError;
use serde_json::Value;

use crate::rules::{Failure, FormErrors};
use crate::task::Pending;
use crate::text::{self, Block};
use crate::theme::{Theme, corner, mix, space};
use crate::widgets::field::{field_id, layout_unit};
use crate::widgets::{Button, FormError};

/// A JSON request in flight.
pub type Request = Option<Pending<Result<Value, ApiError>>>;

/// `.auth-banner` (margin-bottom 16): a note above the heading;
/// `invite` gives it the accent bar.
pub fn banner(ui: &mut Ui, message: &str, invite: bool) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let block = Block::wrapped(ui, message, text::UI, c.text, width - 3.0 - 24.0);
    let height = 20.0 + layout_unit(block.height());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    crate::widgets::announce(&response, egui::Role::Status, message);
    let (fill, bar) = if invite {
        (mix(c.panel, c.accent, 0.14), c.accent)
    } else {
        (c.surface, c.border_strong)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, corner(theme.radius.sm), fill);
    painter
        .with_clip_rect(Rect::from_min_max(
            rect.min,
            pos2(rect.left() + 3.0, rect.bottom()),
        ))
        .rect_filled(rect, corner(theme.radius.sm), bar);
    block.paint(painter, pos2(rect.left() + 15.0, rect.top() + 10.0));
    ui.add_space(space::S4);
}

/// The invite redirect banner.
pub fn invite_banner(ui: &mut Ui) {
    banner(
        ui,
        "Einladung erhalten. Melde dich an oder registriere dich, um beizutreten.",
        true,
    );
}

/// `<FormError>` and the 20px form gap after it, when there is a message.
pub fn form_error(ui: &mut Ui, errors: &FormErrors) {
    if let Some(message) = &errors.form {
        let theme = Theme::of(ui.ctx());
        ui.add(FormError::new(message).on(theme.c.panel));
        ui.add_space(space::S5);
    }
}

/// `<SubmitButton size="lg">`: the primary, full-width form action.
pub fn submit(ui: &mut Ui, label: &str, pending_label: &str, pending: bool) -> bool {
    submit_after(ui, label, pending_label, pending, 0)
}

/// `SubmitButton` with `cooldown`: "Erneut versuchen in N s", disabled
/// until the rate limit is over.
pub fn submit_after(
    ui: &mut Ui,
    label: &str,
    pending_label: &str,
    pending: bool,
    cooldown: u64,
) -> bool {
    if cooldown > 0 {
        // Each second drops one, as useCooldown's timer does.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        let waiting = format!("Erneut versuchen in {cooldown} s");
        ui.add(
            Button::new(&waiting)
                .primary()
                .large()
                .fill()
                .disabled(true),
        );
        return false;
    }
    ui.add(
        Button::new(label)
            .primary()
            .large()
            .fill()
            .pending(pending.then_some(pending_label)),
    )
    .clicked()
}

/// A full-width large secondary or ghost button.
pub fn wide_button(ui: &mut Ui, label: &str, ghost: bool) -> bool {
    let button = Button::new(label).large().fill();
    let button = if ghost { button.ghost() } else { button };
    ui.add(button).clicked()
}

/// Moves focus to the first invalid field once errors render.
pub fn focus_invalid(ui: &Ui, errors: &mut FormErrors) {
    if let Some(field) = errors.focus.take() {
        ui.memory_mut(|m| m.request_focus(field_id(field.as_str())));
    }
}

/// Applies a failed request to the form (`fromError`).
pub fn fail(errors: &mut FormErrors, error: &ApiError, order: &[&str]) {
    errors.from_failure(&Failure::from_api(error), order);
}

/// `.auth-result`: column, gap 12, margin-top 20; muted lines are quiet.
pub fn result_lines(ui: &mut Ui, lines: &[(&str, bool)]) {
    let theme = Theme::of(ui.ctx());
    ui.add_space(space::S5);
    for (i, (line, quiet)) in lines.iter().enumerate() {
        if i > 0 {
            ui.add_space(space::S3);
        }
        let color = if *quiet { theme.c.muted } else { theme.c.text };
        let block = Block::wrapped(ui, line, text::BODY, color, ui.available_width());
        layout::show_block(ui, &block);
    }
}

/// A result line with one bold run (`Bestätigungslink an <strong>…`).
pub fn result_with_strong(ui: &mut Ui, before: &str, strong: &str, after: &str) {
    let theme = Theme::of(ui.ctx());
    ui.add_space(space::S5);
    let block = text::Rich::new(text::BODY)
        .push(before, text::BODY, theme.c.text)
        .push(strong, text::BODY.weight(700.0), theme.c.text)
        .push(after, text::BODY, theme.c.text)
        .wrap(ui.available_width())
        .layout(ui);
    layout::show_block(ui, &block);
}

/// Seconds left of a resend cooldown that ends at `until`.
pub fn seconds_left(ui: &Ui, until: Option<f64>) -> u64 {
    let now = ui.input(|i| i.time);
    match until {
        Some(until) if until > now => {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    (until - now).fract().max(0.05),
                ));
            (until - now).ceil() as u64
        }
        _ => 0,
    }
}

/// `RESEND_COOLDOWN_S`.
pub const RESEND_COOLDOWN_S: f64 = 60.0;

/// Text colour for the quiet `.auth-quiet` lines.
pub fn quiet(theme: &Theme) -> Color32 {
    theme.c.muted
}

#[cfg(test)]
mod tests;
