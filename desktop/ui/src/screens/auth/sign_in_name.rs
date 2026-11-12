//! `/sign-in/name#token=…` (pages/SignInNamePage.tsx): the last step of a
//! first sign-in through a provider, the new account's display name. The
//! desktop flow keeps its PKCE verifier for the token ([`super::oidc`]);
//! `complete` sends it, and the account exists only after "Konto anlegen".

use egui::Ui;
use gelabber_client::{ApiError, OidcCompleteResponse, OidcPending};
use gelabber_i18n::auth;
use serde_json::Value;

use super::{fail, focus_invalid, form_error, layout, oidc, result_lines, submit_after};
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, NAME_MAX, error_message, field_message, validate_name};
use crate::task::{Pending, poll};
use crate::text;
use crate::theme::{Theme, space};
use crate::widgets::TextField;

type Completion = Option<Pending<Result<(OidcCompleteResponse, Value), ApiError>>>;

pub struct State {
    token: Option<String>,
    load: Option<Pending<Result<OidcPending, ApiError>>>,
    pending: Option<Result<OidcPending, Failure>>,
    pub name: String,
    pub errors: FormErrors,
    dead_link: bool,
    request: Completion,
}

impl State {
    pub fn new(fragment: &str) -> Self {
        let token = crate::rules::link_token(fragment);
        Self {
            dead_link: token.is_none(),
            token,
            load: None,
            pending: None,
            name: String::new(),
            errors: FormErrors::default(),
            request: None,
        }
    }
}

const ORDER: [&str; 1] = ["name"];

/// `isDeadToken`: `410 token_invalid`.
fn is_dead(error: &ApiError) -> bool {
    Failure::from_api(error).code == "token_invalid"
}

fn load(app: &AppCtx, token: &str) -> Option<Pending<Result<OidcPending, ApiError>>> {
    let http = app.client.http().clone();
    let token = token.to_owned();
    Some(Pending::spawn(app.client, async move {
        http.oidc_pending(&token).await
    }))
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let Some(token) = s.token.clone() else {
        return dead(ui, app);
    };
    if s.pending.is_none() && s.load.is_none() && !s.dead_link {
        s.load = load(app, &token);
    }
    if let Some(result) = poll(&mut s.load) {
        match result {
            Ok(pending) => {
                s.name = pending.name.clone().unwrap_or_default();
                s.pending = Some(Ok(pending));
            }
            Err(error) if is_dead(&error) => s.dead_link = true,
            Err(error) => s.pending = Some(Err(Failure::from_api(&error))),
        }
    }
    if let Some(result) = poll(&mut s.request) {
        match result {
            Ok((done, session)) => {
                oidc::forget_verifier(ui.ctx(), &token);
                app.session.adopt(&session);
                let href = done
                    .redirect
                    .as_deref()
                    .and_then(crate::app::safe_redirect)
                    .unwrap_or_else(|| "/".to_owned());
                app.nav.replace(&href);
                return;
            }
            Err(error) if is_dead(&error) => s.dead_link = true,
            Err(error) => fail(&mut s.errors, &error, &ORDER),
        }
    }
    if s.dead_link {
        return dead(ui, app);
    }
    match s.pending.clone() {
        None => super::invite::skeleton(ui, app),
        Some(Err(failure)) => {
            let reloading = s.load.is_some();
            let mut retry = false;
            layout::frame(ui, app, "sign-in-name-error", |ui, _app| {
                layout::heading(ui, auth::sign_in_name_title());
                ui.add_space(space::S5);
                retry = crate::widgets::load_error(ui, error_message(&failure.code), reloading);
            });
            if retry && !reloading {
                s.load = load(app, &token);
            }
        }
        Some(Ok(pending)) => form(ui, app, s, &token, &pending),
    }
}

fn form(ui: &mut Ui, app: &mut AppCtx, s: &mut State, token: &str, pending: &OidcPending) {
    let busy = s.request.is_some();
    layout::frame(ui, app, "sign-in-name", |ui, app| {
        let theme = Theme::of(ui.ctx());
        layout::heading(ui, auth::sign_in_name_title());
        layout::lead(ui, auth::sign_in_name_lead());
        // "Angemeldet mit {provider} als <b>{email}</b>": split first, then
        // the names go in, so they cannot add tags.
        ui.add_space(space::S1);
        let mut rich = text::Rich::new(text::BODY);
        for piece in gelabber_i18n::pieces(auth::sign_in_name_signed_in_as()) {
            let shown = piece
                .text
                .replace("{provider}", &pending.provider.name)
                .replace("{email}", &pending.email);
            let style = if piece.tag == Some("b") {
                text::BODY.weight(700.0)
            } else {
                text::BODY
            };
            rich = rich.push(&shown, style, theme.c.muted);
        }
        let block = rich.wrap(ui.available_width()).layout(ui);
        layout::show_block(ui, &block);

        ui.add_space(space::S6);
        let name = TextField::new("name", &mut s.name)
            .label(auth::sign_in_name_name_label())
            .autofocus(true)
            .char_limit(NAME_MAX)
            .error(s.errors.field("name"))
            .hint(Some(auth::register_name_hint()))
            .show(ui);
        if name.response.changed() {
            s.errors.clear_field("name");
        }
        ui.add_space(space::S5);
        form_error(ui, &s.errors);
        let cooldown = s.errors.cooldown();
        let clicked = submit_after(
            ui,
            auth::sign_in_name_submit(),
            auth::sign_in_name_submitting(),
            busy,
            cooldown,
        );
        if (clicked || name.submitted) && !busy && cooldown == 0 {
            if let Some(code) = validate_name(&s.name) {
                s.errors
                    .set_fields(vec![("name", field_message("name", code))]);
            } else {
                s.errors = FormErrors::default();
                let http = app.client.http().clone();
                let token = token.to_owned();
                let name = s.name.trim().to_owned();
                // Without the flow's verifier (another browser's step) the
                // API answers 410 and the page says the sign-in expired.
                let verifier = oidc::verifier_for(ui.ctx(), &token).unwrap_or_default();
                s.request = Some(Pending::spawn(app.client, async move {
                    let done = http.oidc_complete(&token, &name, &verifier).await?;
                    // A new session has a new CSRF token.
                    let session = http.session().await?;
                    Ok((done, session))
                }));
            }
        }
        focus_invalid(ui, &mut s.errors);
    });
}

/// `DeadSignIn`: the step's token is used, expired or never there.
fn dead(ui: &mut Ui, app: &mut AppCtx) {
    layout::frame(ui, app, "sign-in-name-dead", |ui, app| {
        layout::focused_heading(ui, auth::sign_in_name_expired_title());
        result_lines(ui, &[(auth::sign_in_name_expired(), false)]);
        ui.add_space(space::S6);
        if layout::action_link(ui, auth::page_to_sign_in()) {
            app.nav.navigate("/login");
        }
    });
}
