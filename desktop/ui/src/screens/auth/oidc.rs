//! Sign-in through a provider (components/ProviderButton.tsx,
//! auth/providers.ts): the instance's providers, one button each and the
//! "oder" divider above a page's form, and the flow through the system
//! browser (gelabber_client::oidc) while the card waits for it. The web
//! leaves the page for the provider; the app stays and shows "Weiter im
//! Browser …" with "Abbrechen", which drops the flow and its loopback port.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::Ui;
use gelabber_client::oidc::{LOOPBACK_TIMEOUT, OidcError};
use gelabber_client::{ApiError, AuthProvider, Client, OidcExchange, OidcStart, SignInMethods};
use gelabber_i18n::auth;
use parking_lot::Mutex;
use serde_json::Value;

use super::layout;
use crate::rules::{Failure, FormErrors, oidc_error_message};
use crate::task::Pending;
use crate::theme::space;

// ------------------------------------------------------------ providers

#[derive(Default)]
enum Known {
    #[default]
    Unknown,
    Loading(Pending<Result<Vec<AuthProvider>, ApiError>>),
    Listed(Vec<AuthProvider>),
}

#[derive(Clone, Default)]
struct Providers(Arc<Mutex<HashMap<String, Known>>>);

fn providers_cache(ctx: &egui::Context) -> Providers {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Providers>(egui::Id::unique("gelabber-auth-providers"))
            .clone()
    })
}

/// `useAuthProviders()`: the server's providers, asked once per server
/// (`staleTime: Infinity`, `retry: false`). Empty while loading, after a
/// failure and while the operator has none, so the page looks as before.
pub fn providers(ctx: &egui::Context, client: &Client) -> Vec<AuthProvider> {
    let cache = providers_cache(ctx);
    let mut map = cache.0.lock();
    let known = map.entry(client.http().base().to_string()).or_default();
    if let Known::Unknown = known {
        let http = client.http().clone();
        *known = Known::Loading(Pending::spawn(client, async move {
            http.auth_providers().await
        }));
    }
    if let Known::Loading(pending) = known
        && let Some(result) = pending.take()
    {
        *known = Known::Listed(result.unwrap_or_default());
    }
    match known {
        Known::Listed(list) => list.clone(),
        _ => Vec::new(),
    }
}

// --------------------------------------------------------------- opener

/// Hands the provider's page to the system browser.
pub type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

fn opener_id() -> egui::Id {
    egui::Id::unique("gelabber-oidc-opener")
}

/// The app's `open_url`, or what a test put in its place.
pub(crate) fn opener(ctx: &egui::Context) -> Opener {
    ctx.data(|d| d.get_temp::<Opener>(opener_id()))
        .unwrap_or_else(|| Arc::new(crate::platform::open_url))
}

/// For tests: a stand-in for the system browser.
#[cfg(test)]
pub fn set_opener(ctx: &egui::Context, open: Opener) {
    ctx.data_mut(|d| d.insert_temp(opener_id(), open));
}

// ------------------------------------------------------------- verifier

#[derive(Clone, Default)]
struct Verifiers(Arc<Mutex<HashMap<String, String>>>);

fn verifiers(ctx: &egui::Context) -> Verifiers {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Verifiers>(egui::Id::unique("gelabber-oidc-verifiers"))
            .clone()
    })
}

/// Keeps a `needs_name` flow's PKCE verifier for `/sign-in/name`, which
/// sends it with `complete`.
pub fn keep_verifier(ctx: &egui::Context, code: &str, verifier: &str) {
    verifiers(ctx)
        .0
        .lock()
        .insert(code.to_lowercase(), verifier.to_owned());
}

/// The verifier kept for the name step's `token`.
pub fn verifier_for(ctx: &egui::Context, token: &str) -> Option<String> {
    verifiers(ctx).0.lock().get(token).cloned()
}

/// Forgets it once the account exists.
pub fn forget_verifier(ctx: &egui::Context, token: &str) {
    verifiers(ctx).0.lock().remove(token);
}

// ----------------------------------------------------------------- flow

/// What a finished flow comes to.
pub enum Done {
    /// Signed in: the session as GET /api/auth/session answers it.
    SignedIn(Value),
    /// A new account: `/sign-in/name#token={code}` with the kept verifier.
    NeedsName { code: String, verifier: String },
    /// Connected to the signed-in account.
    Linked(SignInMethods),
}

/// One flow in progress. Dropping it ends the flow and closes the
/// loopback port (the card's "Abbrechen", or leaving the page).
pub struct Flow {
    pending: Pending<Result<Done, OidcError>>,
    opened: Arc<AtomicBool>,
    pub provider: AuthProvider,
    /// The start's `redirect` and `invitation`, which a refusal for want
    /// of a code carries on to /register (the callback's `web_failure`).
    redirect: Option<String>,
    invitation: Option<String>,
}

impl Flow {
    /// Starts `start` through `provider` in the system browser.
    pub fn start(
        ctx: &egui::Context,
        client: &Client,
        provider: &AuthProvider,
        start: OidcStart,
    ) -> Self {
        let http = client.http().clone();
        let open = opener(ctx);
        let opened = Arc::new(AtomicBool::new(false));
        let seen = opened.clone();
        let repaint = ctx.clone();
        let id = provider.id.clone();
        let (redirect, invitation) = (start.redirect.clone(), start.invitation.clone());
        let pending = Pending::spawn_abortable(client, async move {
            let outcome = http
                .oidc_desktop(&id, &start, LOOPBACK_TIMEOUT, move |url| {
                    let result = open(url);
                    seen.store(result.is_ok(), Ordering::Release);
                    repaint.request_repaint();
                    result
                })
                .await?;
            Ok(match outcome.exchange {
                // A new session has a new CSRF token, and the app wants
                // the session as the login gets it.
                OidcExchange::SignedIn(_) => Done::SignedIn(http.session().await?),
                OidcExchange::NeedsName(_) => Done::NeedsName {
                    code: outcome.code,
                    verifier: outcome.verifier,
                },
                OidcExchange::Linked(methods) => Done::Linked(methods),
            })
        });
        Self {
            pending,
            opened,
            provider: provider.clone(),
            redirect,
            invitation,
        }
    }

    /// The system browser has the provider's page.
    pub fn in_browser(&self) -> bool {
        self.opened.load(Ordering::Acquire)
    }

    /// The pending label of the flow's button: "Weiter zu Google…" until
    /// the browser has the page, then "Weiter im Browser …".
    pub fn label(&self) -> String {
        if self.in_browser() {
            auth::oidc_waiting().to_owned()
        } else {
            auth::oidc_redirecting(&self.provider.name)
        }
    }
}

/// Polls `flow`: its result once, clearing the slot.
pub fn poll_flow(flow: &mut Option<Flow>) -> Option<Result<Done, OidcError>> {
    let result = flow.as_ref()?.pending.take()?;
    *flow = None;
    Some(result)
}

/// A failed flow on a form: an API refusal as `fromError` reads it, else
/// the `oidc_error` copy.
pub fn fail(errors: &mut FormErrors, error: &OidcError, order: &[&str]) {
    match error {
        OidcError::Api(error) => errors.from_failure(&Failure::from_api(error), order),
        other => {
            *errors = FormErrors {
                form: Some(oidc_error_message(other.code()).to_owned()),
                ..FormErrors::default()
            }
        }
    }
}

/// The copy of a failed flow, for a row's status line.
pub fn message(error: &OidcError) -> &'static str {
    match error {
        OidcError::Api(error) => crate::rules::error_message(&Failure::from_api(error).code),
        other => oidc_error_message(other.code()),
    }
}

// -------------------------------------------------------- sign-in block

/// `<ProviderSignIn>` on /login and /register: the flow it runs.
#[derive(Default)]
pub struct ProviderSignIn {
    flow: Option<Flow>,
}

impl ProviderSignIn {
    /// Polls the flow and applies a finished one: signed in, on to the
    /// name step, or the form's error.
    pub fn poll(
        &mut self,
        ui: &Ui,
        app: &mut crate::app::AppCtx,
        errors: &mut FormErrors,
        order: &[&str],
    ) -> bool {
        let Some(flow) = self.flow.as_ref() else {
            return false;
        };
        let (redirect, invitation) = (flow.redirect.clone(), flow.invitation.clone());
        let Some(result) = poll_flow(&mut self.flow) else {
            return false;
        };
        match result {
            Ok(Done::SignedIn(session)) => {
                app.session.adopt(&session);
                let redirect = app.nav.location().redirect();
                app.nav.replace(redirect.as_deref().unwrap_or("/"));
                return true;
            }
            Ok(Done::NeedsName { code, verifier }) => {
                keep_verifier(ui.ctx(), &code, &verifier);
                app.nav.navigate(&format!("/sign-in/name#token={code}"));
                return true;
            }
            // Not asked for here; nothing to show.
            Ok(Done::Linked(_)) => {}
            // A new account wants a code: on to /register, which says why
            // and asks for it, as the web's callback sends the browser.
            Err(OidcError::Api(error))
                if super::registration::is_admission_error(&Failure::from_api(&error).code) =>
            {
                let code = Failure::from_api(&error).code;
                app.nav.navigate(&crate::app::with_query(
                    "/register",
                    &[
                        ("oidc_error", code.as_str()),
                        ("invitation", invitation.as_deref().unwrap_or("")),
                        ("redirect", redirect.as_deref().unwrap_or("")),
                    ],
                ));
                return true;
            }
            Err(error) => fail(errors, &error, order),
        }
        false
    }

    /// Whether a flow is running.
    pub fn running(&self) -> bool {
        self.flow.is_some()
    }

    /// Ends the running flow (a sign-in by password went first).
    pub fn cancel(&mut self) {
        self.flow = None;
    }

    /// The buttons and the divider, 24px under the lead; true when drawn
    /// (the form then follows 16px under the divider instead of 24px under
    /// the lead). `start` makes the flow's start body when one is clicked.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        app: &mut crate::app::AppCtx,
        errors: &mut FormErrors,
        start: impl FnOnce() -> OidcStart,
    ) -> bool {
        let providers = providers(ui.ctx(), app.client);
        if providers.is_empty() {
            return false;
        }
        ui.add_space(space::S6);
        let mut clicked = None;
        for (i, provider) in providers.iter().enumerate() {
            if i > 0 {
                ui.add_space(space::S2);
            }
            let label = self
                .flow
                .as_ref()
                .filter(|flow| flow.provider.id == provider.id)
                .map(Flow::label);
            let response =
                layout::provider_button(ui, provider, label.as_deref(), self.flow.is_some());
            if response.clicked() && self.flow.is_none() {
                clicked = Some(provider.clone());
            }
        }
        if self.flow.is_some() {
            ui.add_space(space::S2);
            if super::wide_button(ui, gelabber_i18n::common::cancel(), true) {
                self.flow = None;
            }
        }
        if let Some(provider) = clicked {
            *errors = FormErrors::default();
            self.flow = Some(Flow::start(ui.ctx(), app.client, &provider, start()));
        }
        ui.add_space(space::S4);
        layout::divider(ui);
        ui.add_space(space::S4);
        true
    }
}
