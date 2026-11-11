//! The screens, one per route of web/src/routes.tsx. Each keeps its form
//! state in [`ScreenState`], which resets whenever the location changes.

pub mod auth;
pub mod chat;
pub mod layout;
pub mod server_settings;
pub mod settings;
pub mod shell;
pub mod voice;

use egui::Ui;

use crate::app::{AppCtx, Location};
use crate::route::Route;

/// Per-screen state for the current location.
pub enum ScreenState {
    None,
    Login(auth::login::State),
    Register(auth::register::State),
    ForgotPassword(auth::forgot_password::State),
    ResetPassword(auth::reset_password::State),
    VerifyEmail(auth::verify_email::State),
    Invite(auth::invite::State),
    SignInName(auth::sign_in_name::State),
}

impl ScreenState {
    pub fn for_location(location: &Location) -> Self {
        match &location.route {
            Route::Login => Self::Login(auth::login::State::default()),
            Route::Register => Self::Register(auth::register::State::default()),
            Route::ForgotPassword => Self::ForgotPassword(auth::forgot_password::State::new(
                location.query("email").unwrap_or_default(),
            )),
            Route::ResetPassword => {
                Self::ResetPassword(auth::reset_password::State::new(&location.fragment))
            }
            Route::VerifyEmail => {
                Self::VerifyEmail(auth::verify_email::State::new(&location.fragment))
            }
            Route::Invite(code) => Self::Invite(auth::invite::State::new(code)),
            Route::SignInName => {
                Self::SignInName(auth::sign_in_name::State::new(&location.fragment))
            }
            _ => Self::None,
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, state: &mut ScreenState) {
    match state {
        ScreenState::Login(s) => auth::login::show(ui, app, s),
        ScreenState::Register(s) => auth::register::show(ui, app, s),
        ScreenState::ForgotPassword(s) => auth::forgot_password::show(ui, app, s),
        ScreenState::ResetPassword(s) => auth::reset_password::show(ui, app, s),
        ScreenState::VerifyEmail(s) => auth::verify_email::show(ui, app, s),
        ScreenState::Invite(s) => auth::invite::show(ui, app, s),
        ScreenState::SignInName(s) => auth::sign_in_name::show(ui, app, s),
        ScreenState::None => match app.nav.route() {
            Route::NotFound(_) => auth::not_found::show(ui, app),
            // Only for an operator: everyone else meets "not found", as on
            // the web.
            Route::Operator(_) if !app.session.operator() => auth::not_found::show(ui, app),
            _ => shell::show(ui, app),
        },
    }
}
