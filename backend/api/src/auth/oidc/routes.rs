//! The routes of sign-in through a provider (#518):
//!
//! - `GET /api/auth/providers`: the configured providers.
//! - `POST /api/auth/oidc/{provider}/start`: a flow and the provider's
//!   authorization URL, for the browser (bound by the `gelabber_oidc`
//!   cookie) or the desktop app (bound by its own PKCE challenge, with the
//!   result sent to its loopback port).
//! - `GET /api/auth/oidc/{provider}/callback`: the provider sends the person
//!   back; every answer is a `303`.
//! - `POST /api/auth/oidc/pending` and `…/complete`: the name step of a new
//!   account.
//! - `POST /api/auth/oidc/desktop/exchange`: the desktop app trades its
//!   result code.
//!
//! Every session is issued by `auth::signed_in`. Logs name the provider,
//! the account and the outcome, never a state, code, verifier, nonce, token
//! or address.

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

use axum::Router;
use axum::extract::{ConnectInfo, FromRequestParts, Path, Query, State};
use axum::http::header::{CACHE_CONTROL, HOST, LOCATION, REFERRER_POLICY};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{AppendHeaders, IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tracing::{info, warn};
use url::Url;

use super::flow::{self, Desktop, Flow, NewFlow, NewResult, Purpose, Stored};
use super::link::{self, Identity, SignIn};
use super::{Claims, pkce};
use crate::auth::session::{self, CurrentSession};
use crate::auth::{
    Outgoing, Redacted, SessionResponse, send_mail, signed_in_parts, transport_allowed, user,
    validate,
};
use crate::config::OidcProvider;
use crate::cookies::{self, SESSION_COOKIE, SetCookie};
use crate::error::{ApiError, FieldErrors};
use crate::i18n::Locale;
use crate::json::Body;
use crate::limits::{self, MailBudget};
use crate::operator::admission;
use crate::password;
use crate::servers::invite;
use crate::state::AppState;
use crate::token;

/// Binds a web flow to the browser that started it.
pub const BINDING_COOKIE: &str = "gelabber_oidc";
/// The binding cookie goes only to these routes.
const BINDING_PATH: &str = "/api/auth/oidc";
/// Long enough for the provider and the name step after it.
const BINDING_MAX_AGE_SECS: u64 = 30 * 60;
/// A `redirect` longer than this is dropped.
const REDIRECT_MAX: usize = 2048;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/providers", get(providers))
        .route("/api/auth/oidc/{provider}/start", post(start))
        .route("/api/auth/oidc/{provider}/callback", get(callback))
        .route("/api/auth/oidc/pending", post(pending))
        .route("/api/auth/oidc/complete", post(complete))
        .route("/api/auth/oidc/desktop/exchange", post(desktop_exchange))
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
}

impl From<&OidcProvider> for ProviderInfo {
    fn from(provider: &OidcProvider) -> Self {
        Self {
            id: provider.id,
            name: provider.name,
        }
    }
}

#[derive(Debug, Serialize)]
struct ProvidersResponse {
    providers: Vec<ProviderInfo>,
}

/// The configured providers; empty while sign-in through a provider is off.
async fn providers(State(state): State<AppState>) -> Json<ProvidersResponse> {
    let providers = state
        .oidc
        .as_ref()
        .map(|oidc| oidc.config.providers.iter().map(Into::into).collect())
        .unwrap_or_default();
    Json(ProvidersResponse { providers })
}

#[derive(Deserialize)]
pub struct StartBody {
    #[serde(default)]
    pub purpose: String,
    /// A path of the app to land on afterwards; anything else is dropped.
    #[serde(default)]
    pub redirect: Option<String>,
    /// `link` only, when the account has a password.
    #[serde(default)]
    pub current_password: String,
    /// A beta invitation's code for the admission gate of a new account.
    #[serde(default)]
    pub invitation: Option<String>,
    /// Set by the desktop app; the browser leaves it out.
    #[serde(default)]
    pub desktop: Option<DesktopBody>,
}

impl std::fmt::Debug for StartBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartBody")
            .field("purpose", &self.purpose)
            .field("redirect", &self.redirect)
            .field("current_password", &Redacted)
            .field("invitation", &Redacted)
            .field("desktop", &self.desktop.as_ref().map(|_| Redacted))
            .finish()
    }
}

#[derive(Deserialize)]
pub struct DesktopBody {
    #[serde(default)]
    pub port: i64,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub challenge: String,
}

#[derive(Debug, Serialize)]
struct StartResponse {
    url: String,
}

/// Starts a sign-in or a link: stores the flow and answers the provider's
/// authorization URL. The browser variant must be opened at the instance's
/// public address (`409 oidc_wrong_origin`), where the binding cookie is
/// set; the desktop variant is bound by its own challenge instead.
async fn start(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    Body(body): Body<StartBody>,
) -> Result<Response, ApiError> {
    let oidc = state.oidc.clone().ok_or(ApiError::NotFound)?;
    let provider = oidc.provider(&provider_id).ok_or(ApiError::NotFound)?;
    transport_allowed(&state, &headers)?;

    let mut errors = FieldErrors::new();
    let purpose = Purpose::parse(&body.purpose);
    if purpose.is_none() {
        let code = if body.purpose.is_empty() {
            "required"
        } else {
            "invalid"
        };
        errors.insert("purpose", code);
    }
    let desktop = body
        .desktop
        .as_ref()
        .and_then(|desktop| desktop_fields(desktop, &mut errors));
    let invitation = match present(body.invitation.as_deref()) {
        None => None,
        Some(raw) => {
            let code = invite::normalise_code(raw);
            if code.is_none() {
                errors.insert("invitation", "invalid");
            }
            code
        }
    };
    validate::finish(errors)?;
    let purpose = purpose.expect("validated");
    let redirect = body.redirect.as_deref().and_then(safe_redirect);

    let link_user_id = match purpose {
        Purpose::SignIn => None,
        Purpose::Link => {
            let current = current_session(&state, &headers)
                .await?
                .ok_or(ApiError::Unauthenticated)?;
            link_allowed(&state, provider, &current, body.current_password).await?;
            Some(current.user.id)
        }
    };
    if desktop.is_none() {
        same_origin(&oidc.config.public_url, &headers)?;
    }
    let doc = oidc.discovery(provider).await?;

    let (flow_state, nonce, verifier) = (token::generate(), token::generate(), pkce::verifier());
    let binding = desktop.is_none().then(token::generate);
    flow::insert_flow(
        &state.db,
        &flow_state,
        &NewFlow {
            provider: provider.id,
            purpose,
            link_user_id,
            nonce: &nonce,
            pkce_verifier: &verifier,
            binding: binding.as_deref(),
            desktop: desktop.as_ref(),
            redirect: redirect.as_deref(),
            invitation: invitation.as_deref(),
        },
    )
    .await?;

    let mut url = doc.authorization_endpoint.clone();
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &provider.client_id)
            .append_pair("redirect_uri", &oidc.redirect_uri(provider))
            .append_pair("scope", provider.scopes)
            .append_pair("state", &flow_state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &pkce::challenge(&verifier))
            .append_pair("code_challenge_method", "S256");
        for (name, value) in provider.extra_auth_params {
            query.append_pair(name, value);
        }
    }
    info!(
        provider = provider.id,
        purpose = purpose.as_str(),
        desktop = desktop.is_some(),
        "oidc flow started"
    );
    let body = Json(StartResponse { url: url.into() });
    Ok(match binding {
        Some(binding) => {
            let cookie = SetCookie::new(BINDING_COOKIE, binding, BINDING_MAX_AGE_SECS)
                .path(BINDING_PATH)
                .secure(state.cookie_secure);
            (AppendHeaders([cookie.header()]), body).into_response()
        }
        None => body.into_response(),
    })
}

/// The desktop app's port, state and challenge, or field errors.
fn desktop_fields(body: &DesktopBody, errors: &mut FieldErrors) -> Option<Desktop> {
    let port = u16::try_from(body.port).ok().filter(|port| *port >= 1024);
    if port.is_none() {
        errors.insert("desktop.port", "invalid");
    }
    let state_ok = (43..=128).contains(&body.state.len())
        && body
            .state
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'-'));
    if !state_ok {
        errors.insert("desktop.state", "invalid");
    }
    let challenge_ok = body.challenge.len() == 43
        && body
            .challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    if !challenge_ok {
        errors.insert("desktop.challenge", "invalid");
    }
    let port = port.filter(|_| state_ok && challenge_ok)?;
    Some(Desktop {
        port,
        state: body.state.clone(),
        challenge: body.challenge.clone(),
    })
}

/// Linking needs the current password when the account has one, counted
/// like a password change: a stolen session alone cannot add a way in.
async fn link_allowed(
    state: &AppState,
    provider: &OidcProvider,
    current: &CurrentSession,
    current_password: String,
) -> Result<(), ApiError> {
    let connected: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_identities WHERE user_id = $1 AND provider = $2)",
    )
    .bind(current.user.id)
    .bind(provider.id)
    .fetch_one(&state.db)
    .await?;
    if connected {
        return Err(ApiError::IdentityTaken);
    }
    let stored = user::password_hash_by_id(&state.db, current.user.id).await?;
    if stored.is_none() {
        return Ok(());
    }
    if current_password.is_empty() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "current_password",
            "required",
        )])));
    }
    let key = validate::canonical(&current.user.email);
    limits::login_attempt(state, &key)?;
    let ok = current_password.chars().count() <= validate::PASSWORD_MAX
        && password::verify(current_password, stored).await?;
    if !ok {
        return Err(ApiError::Validation(FieldErrors::from([(
            "current_password",
            "wrong",
        )])));
    }
    limits::login_succeeded(state, &key);
    Ok(())
}

/// The browser variant must run at `APP_PUBLIC_URL`: the provider sends the
/// person back there, and only there does the binding cookie exist.
fn same_origin(public_url: &Url, headers: &HeaderMap) -> Result<(), ApiError> {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::OidcWrongOrigin)?;
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(':') && port.bytes().all(|b| b.is_ascii_digit()) => {
            (name, port.parse::<u16>().ok())
        }
        _ => (host, None),
    };
    // Without a port the request went to the scheme's default one.
    let default_port = match public_url.scheme() {
        "https" => Some(443),
        _ => Some(80),
    };
    let same = public_url
        .host_str()
        .is_some_and(|expected| expected.eq_ignore_ascii_case(name))
        && port.or(default_port) == public_url.port_or_known_default();
    if same {
        Ok(())
    } else {
        Err(ApiError::OidcWrongOrigin)
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The client's address as the server saw it, for the rate limit.
pub struct Peer(pub Option<IpAddr>);

impl FromRequestParts<AppState> for Peer {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, Infallible> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0.ip()),
        ))
    }
}

/// Where the provider sends the person back. The flow is taken once; a web
/// flow must come back to the browser that started it (the binding cookie),
/// which stops login CSRF and injected callbacks. A web flow resolves the
/// account now; a desktop flow leaves a result for the app's exchange and
/// sends the browser to the app's loopback port.
async fn callback(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
    Peer(peer): Peer,
) -> Result<Response, ApiError> {
    limits::anonymous_hit(&state, &headers, peer)?;
    let flow = match query.state.as_deref() {
        Some(raw) if !raw.is_empty() => flow::take_flow(&state.db, raw).await?,
        _ => None,
    };
    let Some(flow) = flow.filter(|flow| flow.provider == provider_id) else {
        return Ok(web_failure(&state, Purpose::SignIn, "expired", None));
    };
    let fail = |code: &'static str| failure(&state, &flow, code);

    if flow.desktop().is_none() {
        let bound = flow
            .binding_hash
            .as_deref()
            .is_some_and(|expected| binding_matches(&headers, expected));
        if !bound {
            return Ok(fail("expired"));
        }
    }
    let Some(oidc) = state.oidc.clone() else {
        return Ok(fail("failed"));
    };
    let Some(provider) = oidc.provider(&flow.provider) else {
        return Ok(fail("failed"));
    };
    if let Some(error) = query.error.as_deref() {
        let code = if error == "access_denied" {
            "cancelled"
        } else {
            "failed"
        };
        info!(provider = provider.id, outcome = code, "oidc callback");
        return Ok(fail(code));
    }
    let Some(code) = query.code.as_deref().filter(|code| !code.is_empty()) else {
        return Ok(fail("failed"));
    };
    let claims = match oidc
        .exchange(provider, code, &flow.pkce_verifier, &flow.nonce)
        .await
    {
        Ok(claims) => claims,
        Err(err) => {
            if let ApiError::OidcUnavailable(detail) = &err {
                warn!(provider = provider.id, error = %detail, "oidc callback failed");
            }
            return Ok(fail("failed"));
        }
    };

    if let Some((port, desktop_state)) = flow.desktop() {
        let result = NewResult {
            provider: provider.id,
            subject: &claims.subject,
            email: claims.email.as_deref().unwrap_or_default(),
            email_verified: claims.email_verified,
            name_hint: claims.name.as_deref(),
            purpose: flow.purpose(),
            link_user_id: flow.link_user_id,
            binding_hash: None,
            desktop_challenge: flow.desktop_challenge.as_deref(),
            redirect: flow.redirect.as_deref(),
            invitation: flow.invitation.as_deref(),
            signup: false,
        };
        let result_code = flow::insert_result(&state.db, &result).await?;
        info!(
            provider = provider.id,
            outcome = "desktop_result",
            "oidc callback"
        );
        return Ok(see_other(
            &loopback(port, desktop_state, ("code", &result_code)),
            [],
        ));
    }

    if transport_allowed(&state, &headers).is_err() {
        return Ok(fail("insecure_transport"));
    }
    let outcome = match flow.purpose() {
        Purpose::Link => web_link(&state, provider, &flow, &claims, &headers).await,
        Purpose::SignIn => web_sign_in(&state, provider, &flow, &claims, &headers).await,
    };
    match outcome {
        Ok(response) => Ok(response),
        Err(err) => match oidc_error_code(&err) {
            Some(code) => {
                info!(provider = provider.id, outcome = code, "oidc callback");
                Ok(fail(code))
            }
            None => Err(err),
        },
    }
}

/// A web sign-in: an existing account is signed in at once, a new one goes
/// to the name step (`/sign-in/name#token=…`) once the gate admitted it.
async fn web_sign_in(
    state: &AppState,
    provider: &OidcProvider,
    flow: &Flow,
    claims: &Claims,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let identity = identity_of(provider, claims);
    let mut tx = state.db.begin().await?;
    match link::sign_in(&mut tx, &identity).await? {
        resolved @ (SignIn::Known(user_id) | SignIn::Joined(user_id)) => {
            tx.commit().await?;
            let user = session::reload(&state.db, user_id).await?;
            if matches!(resolved, SignIn::Joined(_)) {
                notify_connected(state, &user, provider);
            }
            let (cookies, _) = signed_in_parts(state, headers, user).await?;
            info!(provider = provider.id, %user_id, outcome = outcome_of(resolved), "oidc callback");
            let [session_cookie, csrf_cookie] = cookies;
            Ok(see_other(
                flow.redirect.as_deref().unwrap_or("/"),
                [session_cookie, csrf_cookie, clear_binding(state)],
            ))
        }
        SignIn::New { .. } => {
            drop(tx);
            let email = identity.email.ok_or(ApiError::OidcEmailUnverified)?;
            {
                let mut conn = state.db.acquire().await?;
                admission::admit(
                    state,
                    &mut conn,
                    email,
                    flow.invitation.as_deref(),
                    invite_of(flow.redirect.as_deref()),
                )
                .await?;
            }
            let code = flow::insert_result(
                &state.db,
                &NewResult {
                    provider: provider.id,
                    subject: identity.subject,
                    email,
                    email_verified: identity.email_verified,
                    name_hint: claims.name.as_deref(),
                    purpose: Purpose::SignIn,
                    link_user_id: None,
                    binding_hash: flow.binding_hash.as_deref(),
                    desktop_challenge: None,
                    redirect: flow.redirect.as_deref(),
                    invitation: flow.invitation.as_deref(),
                    signup: true,
                },
            )
            .await?;
            info!(
                provider = provider.id,
                outcome = "needs_name",
                "oidc callback"
            );
            Ok(see_other(&format!("/sign-in/name#token={code}"), []))
        }
    }
}

/// A web link: the browser must still be signed in to the account that
/// started it (`session_changed` otherwise).
async fn web_link(
    state: &AppState,
    provider: &OidcProvider,
    flow: &Flow,
    claims: &Claims,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let current = current_session(state, headers).await?;
    let Some(current) = current.filter(|current| Some(current.user.id) == flow.link_user_id) else {
        return Err(ApiError::SessionChanged);
    };
    let identity = identity_of(provider, claims);
    let mut tx = state.db.begin().await?;
    let attached = link::link(&mut tx, current.user.id, &identity).await?;
    tx.commit().await?;
    if attached {
        notify_connected(state, &current.user, provider);
    }
    info!(provider = provider.id, user_id = %current.user.id, outcome = "linked", "oidc callback");
    let location = format!("/settings/konto?connected={}", provider.id);
    Ok(see_other(&location, [clear_binding(state)]))
}

#[derive(Deserialize)]
pub struct TokenBody {
    #[serde(default)]
    pub token: String,
}

impl std::fmt::Debug for TokenBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBody")
            .field("token", &Redacted)
            .finish()
    }
}

#[derive(Debug, Serialize)]
struct PendingResponse {
    provider: ProviderInfo,
    email: String,
    name: Option<String>,
}

/// What the name step shows: the provider, its address and the name it
/// suggests. Leaves the result in place and needs no binding, so the page
/// can render; only `complete` checks it.
async fn pending(
    State(state): State<AppState>,
    Body(body): Body<TokenBody>,
) -> Result<Json<PendingResponse>, ApiError> {
    let (stored, provider) = signup_result(&state, &body.token).await?;
    Ok(Json(PendingResponse {
        provider,
        email: stored.email,
        name: stored.name_hint,
    }))
}

/// The live name-step result behind `token` and its provider, while that
/// provider is still configured; `410 token_invalid` otherwise.
async fn signup_result(state: &AppState, token: &str) -> Result<(Stored, ProviderInfo), ApiError> {
    let oidc = state.oidc.as_ref().ok_or(ApiError::TokenInvalid)?;
    let stored = flow::peek_result(&state.db, token)
        .await?
        .filter(|stored| stored.signup && stored.purpose() == Purpose::SignIn)
        .ok_or(ApiError::TokenInvalid)?;
    let provider = oidc
        .provider(&stored.provider)
        .ok_or(ApiError::TokenInvalid)?;
    Ok((stored, provider.into()))
}

#[derive(Deserialize)]
pub struct CompleteBody {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub name: String,
    /// The desktop app's PKCE verifier; the browser leaves it out.
    #[serde(default)]
    pub verifier: Option<String>,
    /// `de` or `en`, as at registration.
    #[serde(default)]
    pub locale: Option<String>,
}

impl std::fmt::Debug for CompleteBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompleteBody")
            .field("token", &Redacted)
            .field("name", &self.name)
            .field("verifier", &self.verifier.as_ref().map(|_| Redacted))
            .field("locale", &self.locale)
            .finish()
    }
}

#[derive(Debug, Serialize)]
struct CompleteResponse {
    #[serde(flatten)]
    session: SessionResponse,
    redirect: Option<String>,
}

/// Creates the account of a first sign-in after its name step and signs it
/// in. Resolution runs again in the transaction that creates it: a
/// confirmed account that appeared meanwhile is joined instead, and the
/// admission gate runs again, its invitation redeemed with the insert.
async fn complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<CompleteBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let (stored, provider) = signup_result(&state, &body.token).await?;
    let bound = match (&stored.binding_hash, &stored.desktop_challenge) {
        (Some(expected), _) => binding_matches(&headers, expected),
        (None, Some(challenge)) => body
            .verifier
            .as_deref()
            .is_some_and(|verifier| pkce::matches(verifier, challenge)),
        (None, None) => false,
    };
    if !bound {
        return Err(ApiError::TokenInvalid);
    }
    let mut errors = FieldErrors::new();
    let name = validate::name(&body.name, &mut errors);
    let locale = locale_of(body.locale.as_deref(), &headers, &mut errors);
    validate::finish(errors)?;
    let (name, locale) = (name.expect("validated"), locale.expect("validated"));

    let identity = Identity {
        provider: provider.id,
        subject: &stored.subject,
        email: stored.email(),
        email_verified: stored.email_verified,
    };
    let mut tx = state.db.begin().await?;
    flow::take_result(&mut tx, &body.token)
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    let resolved = link::sign_in(&mut tx, &identity).await?;
    let (user_id, revoked) = match resolved {
        SignIn::Known(user_id) | SignIn::Joined(user_id) => (user_id, Vec::new()),
        SignIn::New { takeover } => {
            let email = identity.email.ok_or(ApiError::OidcEmailUnverified)?;
            let admitted = admission::admit(
                &state,
                &mut tx,
                email,
                stored.invitation.as_deref(),
                invite_of(stored.redirect.as_deref()),
            )
            .await?;
            let (account, revoked) =
                link::create(&mut tx, &identity, takeover, &name, locale).await?;
            admission::redeem(&mut tx, &admitted, account.id).await?;
            (account.id, revoked)
        }
    };
    tx.commit().await?;
    session::disconnect(&state, &revoked).await;

    let user = session::reload(&state.db, user_id).await?;
    if matches!(resolved, SignIn::Joined(_)) {
        notify_connected(&state, &user, provider_config(&state, provider.id)?);
    }
    let (cookies, session) = signed_in_parts(&state, &headers, user).await?;
    info!(provider = provider.id, %user_id, outcome = outcome_of(resolved), "oidc complete");
    let status = match resolved {
        SignIn::New { .. } => StatusCode::CREATED,
        _ => StatusCode::OK,
    };
    let body = Json(CompleteResponse {
        session,
        redirect: stored.redirect,
    });
    let mut response = (status, AppendHeaders(cookies), body).into_response();
    if stored.binding_hash.is_some() {
        let (name, value) = clear_binding(&state);
        response.headers_mut().append(name, value);
    }
    Ok(response)
}

#[derive(Deserialize)]
pub struct ExchangeBody {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub verifier: String,
}

impl std::fmt::Debug for ExchangeBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExchangeBody")
            .field("code", &Redacted)
            .field("verifier", &Redacted)
            .finish()
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum ExchangeResponse {
    SignedIn {
        #[serde(flatten)]
        session: Box<SessionResponse>,
    },
    NeedsName {
        provider: ProviderInfo,
        email: String,
        name: Option<String>,
    },
    Linked,
}

/// The desktop app trades the code its loopback receiver got, proving with
/// its verifier that it started the flow. A sign-in signs the app in, a new
/// account goes to the name step (the same code then serves `complete`),
/// and a link needs the app's session to be the account that started it.
async fn desktop_exchange(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<ExchangeBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let oidc = state.oidc.clone().ok_or(ApiError::TokenInvalid)?;
    let stored = flow::peek_result(&state.db, &body.code)
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    let bound = stored
        .desktop_challenge
        .as_deref()
        .is_some_and(|challenge| pkce::matches(&body.verifier, challenge));
    if !bound {
        return Err(ApiError::TokenInvalid);
    }
    let provider = oidc
        .provider(&stored.provider)
        .ok_or(ApiError::TokenInvalid)?;
    let identity = Identity {
        provider: provider.id,
        subject: &stored.subject,
        email: stored.email(),
        email_verified: stored.email_verified,
    };
    let needs_name = |stored: Stored| ExchangeResponse::NeedsName {
        provider: provider.into(),
        email: stored.email,
        name: stored.name_hint,
    };

    if stored.purpose() == Purpose::Link {
        let current = current_session(&state, &headers).await?;
        let Some(current) = current.filter(|current| Some(current.user.id) == stored.link_user_id)
        else {
            return Err(ApiError::SessionChanged);
        };
        let mut tx = state.db.begin().await?;
        flow::take_result(&mut tx, &body.code)
            .await?
            .ok_or(ApiError::TokenInvalid)?;
        let attached = link::link(&mut tx, current.user.id, &identity).await?;
        tx.commit().await?;
        if attached {
            notify_connected(&state, &current.user, provider);
        }
        info!(provider = provider.id, user_id = %current.user.id, outcome = "linked", "oidc exchange");
        return Ok(Json(ExchangeResponse::Linked).into_response());
    }
    if stored.signup {
        // Asked again before the name step finished.
        return Ok(Json(needs_name(stored)).into_response());
    }

    let mut tx = state.db.begin().await?;
    match link::sign_in(&mut tx, &identity).await? {
        resolved @ (SignIn::Known(user_id) | SignIn::Joined(user_id)) => {
            flow::take_result(&mut tx, &body.code)
                .await?
                .ok_or(ApiError::TokenInvalid)?;
            tx.commit().await?;
            let user = session::reload(&state.db, user_id).await?;
            if matches!(resolved, SignIn::Joined(_)) {
                notify_connected(&state, &user, provider);
            }
            let (cookies, session) = signed_in_parts(&state, &headers, user).await?;
            info!(provider = provider.id, %user_id, outcome = outcome_of(resolved), "oidc exchange");
            Ok((
                AppendHeaders(cookies),
                Json(ExchangeResponse::SignedIn {
                    session: Box::new(session),
                }),
            )
                .into_response())
        }
        SignIn::New { .. } => {
            drop(tx);
            let email = identity.email.ok_or(ApiError::OidcEmailUnverified)?;
            {
                let mut conn = state.db.acquire().await?;
                admission::admit(
                    &state,
                    &mut conn,
                    email,
                    stored.invitation.as_deref(),
                    invite_of(stored.redirect.as_deref()),
                )
                .await?;
            }
            if !flow::await_name(&state.db, &body.code).await? {
                return Err(ApiError::TokenInvalid);
            }
            info!(
                provider = provider.id,
                outcome = "needs_name",
                "oidc exchange"
            );
            Ok(Json(needs_name(stored)).into_response())
        }
    }
}

/// The request's live session, if its cookie has one.
async fn current_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<CurrentSession>, ApiError> {
    match cookies::get(headers, SESSION_COOKIE) {
        Some(raw) => session::lookup(&state.db, &raw).await,
        None => Ok(None),
    }
}

fn identity_of<'a>(provider: &'a OidcProvider, claims: &'a Claims) -> Identity<'a> {
    Identity {
        provider: provider.id,
        subject: &claims.subject,
        email: claims.email.as_deref(),
        email_verified: claims.email_verified,
    }
}

fn provider_config<'a>(state: &'a AppState, id: &str) -> Result<&'a OidcProvider, ApiError> {
    state
        .oidc
        .as_ref()
        .and_then(|oidc| oidc.provider(id))
        .ok_or(ApiError::TokenInvalid)
}

fn outcome_of(resolved: SignIn) -> &'static str {
    match resolved {
        SignIn::Known(_) => "signed_in",
        SignIn::Joined(_) => "joined",
        SignIn::New { takeover: None } => "created",
        SignIn::New { takeover: Some(_) } => "taken_over",
    }
}

/// Tells the account's owner that a provider can now sign in to it, when
/// account mail is on and the address has mail left this hour.
fn notify_connected(state: &AppState, account: &user::User, provider: &OidcProvider) {
    let Some(mailer) = state.mail.as_ref() else {
        return;
    };
    let email = validate::canonical(&account.email);
    if limits::mail_allowed(state, MailBudget::Registration, &email) {
        send_mail(
            state,
            mailer,
            MailBudget::Registration,
            account.id,
            &email,
            Outgoing::ProviderConnected(provider.name),
        );
    }
}

/// The `oidc_error` code a web callback reports for `err`; `None` for an
/// error that is no outcome of the flow (the database failed).
fn oidc_error_code(err: &ApiError) -> Option<&'static str> {
    Some(match err {
        ApiError::OidcUnavailable(_) => "failed",
        ApiError::OidcEmailUnverified => "email_unverified",
        ApiError::OidcAccountUnverified => "account_unverified",
        ApiError::IdentityTaken => "identity_taken",
        ApiError::InvitationRequired => "invitation_required",
        ApiError::InvitationInvalid => "invitation_invalid",
        ApiError::AccountDisabled => "account_disabled",
        ApiError::InsecureTransport => "insecure_transport",
        ApiError::SessionChanged => "session_changed",
        _ => return None,
    })
}

/// A failed flow: the desktop app's loopback port gets the code, a browser
/// is sent to the page that started it.
fn failure(state: &AppState, flow: &Flow, code: &'static str) -> Response {
    match flow.desktop() {
        Some((port, desktop_state)) => {
            see_other(&loopback(port, desktop_state, ("error", code)), [])
        }
        None => web_failure(state, flow.purpose(), code, Some(flow)),
    }
}

/// `/login?oidc_error=…`, `/settings/konto?oidc_error=…` for a link, or
/// `/register?oidc_error=…` (with the invitation and target kept) when the
/// admission gate refused a new account.
fn web_failure(state: &AppState, purpose: Purpose, code: &str, flow: Option<&Flow>) -> Response {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("oidc_error", code);
    let page = match purpose {
        Purpose::Link => "/settings/konto",
        Purpose::SignIn if matches!(code, "invitation_required" | "invitation_invalid") => {
            if let Some(invitation) = flow.and_then(|flow| flow.invitation.as_deref()) {
                query.append_pair("invitation", invitation);
            }
            if let Some(redirect) = flow.and_then(|flow| flow.redirect.as_deref()) {
                query.append_pair("redirect", redirect);
            }
            "/register"
        }
        Purpose::SignIn => "/login",
    };
    see_other(
        &format!("{page}?{}", query.finish()),
        [clear_binding(state)],
    )
}

/// `http://127.0.0.1:{port}/callback?state=…&{name}={value}`: the desktop
/// app's loopback receiver, never `localhost`.
fn loopback(port: u16, desktop_state: &str, (name, value): (&str, &str)) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("state", desktop_state)
        .append_pair(name, value)
        .finish();
    format!("http://127.0.0.1:{port}/callback?{query}")
}

/// A `303` that nothing caches and that leaks no URL to the next page.
fn see_other(
    location: &str,
    cookies: impl IntoIterator<Item = (HeaderName, HeaderValue)>,
) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    headers.insert(
        LOCATION,
        HeaderValue::from_str(location).unwrap_or_else(|_| HeaderValue::from_static("/")),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    for (name, value) in cookies {
        headers.append(name, value);
    }
    response
}

fn clear_binding(state: &AppState) -> (HeaderName, HeaderValue) {
    SetCookie::clear(BINDING_COOKIE)
        .path(BINDING_PATH)
        .secure(state.cookie_secure)
        .header()
}

/// Whether the request's binding cookie hashes to `expected`, compared in
/// constant time.
fn binding_matches(headers: &HeaderMap, expected: &[u8]) -> bool {
    cookies::get(headers, BINDING_COOKIE)
        .filter(|raw| token::is_valid(raw))
        .is_some_and(|raw| token::hash(&raw).ct_eq(expected).into())
}

/// The language of a new account: the body's, else `Accept-Language`, else
/// German, as at registration.
fn locale_of(raw: Option<&str>, headers: &HeaderMap, errors: &mut FieldErrors) -> Option<Locale> {
    match raw {
        Some(raw) => Locale::parse(raw).or_else(|| {
            errors.insert("locale", "invalid");
            None
        }),
        None => Some(
            headers
                .get(axum::http::header::ACCEPT_LANGUAGE)
                .and_then(Locale::from_accept_language)
                .unwrap_or(Locale::De),
        ),
    }
}

/// A path of this app, by the web client's `safeRedirect` rule; anything
/// else (another origin, `//host`, `/\host`, control characters) is `None`.
pub fn safe_redirect(value: &str) -> Option<String> {
    let safe = value.starts_with('/')
        && !value.starts_with("//")
        && !value.starts_with("/\\")
        && value.len() <= REDIRECT_MAX
        && !value.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}');
    safe.then(|| value.to_owned())
}

/// The server invite code of an `/invite/{code}` target, which admits a new
/// account on an invitation-only instance as well.
pub fn invite_of(redirect: Option<&str>) -> Option<&str> {
    let rest = redirect?.strip_prefix("/invite/")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(&rest[..end]).filter(|code| !code.is_empty())
}

/// A code that is absent, empty or blank is no code.
fn present(code: Option<&str>) -> Option<&str> {
    code.map(str::trim).filter(|code| !code.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_follow_the_web_rule() {
        for ok in ["/", "/invite/abc", "/channels/1?x=y#z"] {
            assert_eq!(safe_redirect(ok).as_deref(), Some(ok));
        }
        for bad in [
            "",
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "/\t/evil.example",
            "relative",
        ] {
            assert_eq!(safe_redirect(bad), None, "{bad:?}");
        }
        assert_eq!(
            safe_redirect(&format!("/{}", "a".repeat(REDIRECT_MAX))),
            None
        );
    }

    #[test]
    fn invite_codes_come_from_invite_targets_only() {
        assert_eq!(invite_of(Some("/invite/abc123")), Some("abc123"));
        assert_eq!(invite_of(Some("/invite/abc123/x?y")), Some("abc123"));
        assert_eq!(invite_of(Some("/invite/")), None);
        assert_eq!(invite_of(Some("/servers/abc")), None);
        assert_eq!(invite_of(None), None);
    }

    #[test]
    fn the_loopback_redirect_names_127_0_0_1() {
        assert_eq!(
            loopback(49152, "s.t~a-t_e", ("code", "c0de")),
            "http://127.0.0.1:49152/callback?state=s.t%7Ea-t_e&code=c0de"
        );
    }

    #[test]
    fn the_web_variant_must_run_at_the_public_url() {
        let public = Url::parse("https://chat.example.org").unwrap();
        let host = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(HOST, HeaderValue::from_str(value).unwrap());
            headers
        };
        for ok in ["chat.example.org", "Chat.Example.org:443"] {
            assert!(same_origin(&public, &host(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "chat.example.org:8443",
            "evil.example",
            "chat.example.org.evil",
        ] {
            assert!(same_origin(&public, &host(bad)).is_err(), "{bad}");
        }
        assert!(same_origin(&public, &HeaderMap::new()).is_err());
        let local = Url::parse("http://localhost:5180").unwrap();
        assert!(same_origin(&local, &host("localhost:5180")).is_ok());
        assert!(same_origin(&local, &host("localhost")).is_err());
        let v6 = Url::parse("http://[::1]:5180").unwrap();
        assert!(same_origin(&v6, &host("[::1]:5180")).is_ok());
    }

    #[test]
    fn desktop_fields_are_checked() {
        let good = DesktopBody {
            port: 49152,
            state: "a".repeat(43),
            challenge: "b".repeat(43),
        };
        let mut errors = FieldErrors::new();
        assert!(desktop_fields(&good, &mut errors).is_some());
        assert!(errors.is_empty());
        for bad in [
            DesktopBody {
                port: 80,
                ..clone(&good)
            },
            DesktopBody {
                port: 70000,
                ..clone(&good)
            },
            DesktopBody {
                state: "a".repeat(42),
                ..clone(&good)
            },
            DesktopBody {
                state: "a".repeat(129),
                ..clone(&good)
            },
            DesktopBody {
                state: format!("{}/", "a".repeat(43)),
                ..clone(&good)
            },
            DesktopBody {
                challenge: "b".repeat(44),
                ..clone(&good)
            },
            DesktopBody {
                challenge: format!("{}=", "b".repeat(42)),
                ..clone(&good)
            },
        ] {
            let mut errors = FieldErrors::new();
            assert!(desktop_fields(&bad, &mut errors).is_none());
            assert!(!errors.is_empty());
        }
    }

    fn clone(body: &DesktopBody) -> DesktopBody {
        DesktopBody {
            port: body.port,
            state: body.state.clone(),
            challenge: body.challenge.clone(),
        }
    }
}
