//! `/api/auth/*`: register, login, logout, session bootstrap, password
//! change and signing out every session of the account.
//!
//! Every successful register/login answers with two `Set-Cookie` headers
//! (`gelabber_session`, `gelabber_csrf`; both `HttpOnly; SameSite=Lax`) and a
//! JSON body `{ "user": …, "csrf_token": … }`. The web client keeps the CSRF
//! token in memory and sends it as `X-CSRF-Token` on every mutation.
//!
//! Without `API_COOKIE_SECURE` those cookies would cross the network in clear
//! text, so routes that issue a session then answer only loopback hosts
//! (`http://localhost`), unless `API_ALLOW_PLAIN_HTTP` says the network is
//! trusted ([`transport_allowed`]).

pub mod session;
pub mod user;
pub mod validate;

use axum::Router;
use axum::extract::State;
use axum::http::header::HOST;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{AppendHeaders, IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::cookies::{self, CSRF_COOKIE, SESSION_COOKIE, SetCookie};
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::limits;
use crate::password;
use crate::state::AppState;
use crate::token;

use self::session::{CurrentSession, MaybeUser};
use self::user::User;

/// CSRF cookies outlive sessions on purpose: an anonymous visitor needs one
/// to be allowed to log in at all.
const CSRF_COOKIE_MAX_AGE_SECS: u64 = 365 * 24 * 3600;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/logout-all", post(logout_all))
        .route("/api/auth/password", post(change_password))
        .route("/api/auth/session", get(current_session))
}

/// Request bodies with a password print it as `<redacted>`.
struct Redacted;

impl std::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Deserialize)]
pub struct RegisterBody {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub name: String,
}

impl std::fmt::Debug for RegisterBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterBody")
            .field("email", &self.email)
            .field("password", &Redacted)
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Deserialize)]
pub struct LoginBody {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
}

impl std::fmt::Debug for LoginBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginBody")
            .field("email", &self.email)
            .field("password", &Redacted)
            .finish()
    }
}

/// `password` is the new one, so it shares the register rules and their
/// field code.
#[derive(Deserialize)]
pub struct PasswordBody {
    #[serde(default)]
    pub current_password: String,
    #[serde(default)]
    pub password: String,
}

impl std::fmt::Debug for PasswordBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordBody")
            .field("current_password", &Redacted)
            .field("password", &Redacted)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub user: Option<User>,
    pub csrf_token: String,
}

#[derive(Debug, Serialize)]
pub struct LogoutResponse {
    pub csrf_token: String,
}

async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<RegisterBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let mut errors = FieldErrors::new();
    let email = validate::email(&body.email, &mut errors);
    let name = validate::name(&body.name, &mut errors);
    let pw = validate::password(&body.password, &mut errors);
    validate::finish(errors)?;
    let (email, name, pw) = (
        email.expect("validated"),
        name.expect("validated"),
        pw.expect("validated"),
    );

    let hash = password::hash(pw).await?;
    let user = user::insert(&state.db, &email, &name, &hash).await?;
    info!(user_id = %user.id, "user registered");

    signed_in(&state, &headers, user, StatusCode::CREATED).await
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<LoginBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let mut errors = FieldErrors::new();
    let email = validate::email(&body.email, &mut errors);
    if body.password.is_empty() {
        errors.insert("password", "required");
    }
    validate::finish(errors)?;
    let email = email.expect("validated");
    limits::login_blocked(&state, &email)?;

    // Register never stores anything longer, so this cannot match; fail
    // before the lookup and the hash, with the same answer as a wrong
    // password.
    if body.password.chars().count() > validate::PASSWORD_MAX {
        limits::login_failed(&state, &email);
        return Err(ApiError::InvalidCredentials);
    }

    // Unknown address: `verify` runs Argon2 against a dummy hash anyway so
    // the response time does not reveal whether the account exists.
    let found = user::credentials_by_email(&state.db, &email).await?;
    let stored = found.as_ref().map(|c| c.password_hash.clone());
    let ok = password::verify(body.password, stored).await?;

    let Some(credentials) = found.filter(|_| ok) else {
        limits::login_failed(&state, &email);
        return Err(ApiError::InvalidCredentials);
    };
    let user = session::reload(&state.db, credentials.id).await?;
    info!(user_id = %user.id, "user logged in");

    signed_in(&state, &headers, user, StatusCode::OK).await
}

/// Issues a session + fresh CSRF token for `user` and builds the response.
/// A session the browser already carries is revoked first: the cookie is
/// about to be overwritten, so its row would otherwise linger until TTL.
async fn signed_in(
    state: &AppState,
    headers: &HeaderMap,
    user: User,
    status: StatusCode,
) -> Result<Response, ApiError> {
    let previous = cookies::get(headers, SESSION_COOKIE);
    session::revoke_current(state, previous.as_deref()).await?;
    // Expired-row housekeeping is separate from the targeted logout.
    session::revoke(&state.db, None).await?;
    let session_token = session::create(&state.db, user.id, state.session_ttl).await?;
    let csrf_token = token::generate();

    let cookies = AppendHeaders([
        SetCookie::new(SESSION_COOKIE, session_token, state.session_ttl.as_secs())
            .secure(state.cookie_secure)
            .header(),
        csrf_cookie(state, &csrf_token).header(),
    ]);
    let body = SessionResponse {
        user: Some(user),
        csrf_token,
    };
    Ok((status, cookies, Json(body)).into_response())
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let previous = cookies::get(&headers, SESSION_COOKIE);
    session::revoke_current(&state, previous.as_deref()).await?;
    Ok(signed_out(&state))
}

/// Clears the session cookie and rotates the CSRF token.
fn signed_out(state: &AppState) -> Response {
    let csrf_token = token::generate();
    let cookies = AppendHeaders([
        SetCookie::clear(SESSION_COOKIE)
            .secure(state.cookie_secure)
            .header(),
        csrf_cookie(state, &csrf_token).header(),
    ]);
    (StatusCode::OK, cookies, Json(LogoutResponse { csrf_token })).into_response()
}

/// Sets a new password after checking the current one. Every session of the
/// account ends, this one included, and the caller gets a fresh one: a
/// stolen cookie dies with the old password. Wrong guesses count against the
/// account like failed logins, so a hijacked session cannot brute-force it.
async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    current: CurrentSession,
    Body(body): Body<PasswordBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let mut errors = FieldErrors::new();
    if body.current_password.is_empty() {
        errors.insert("current_password", "required");
    }
    let new_password = validate::password(&body.password, &mut errors);
    validate::finish(errors)?;
    let new_password = new_password.expect("validated");

    let user = current.user;
    limits::login_blocked(&state, &user.email)?;
    let stored = user::password_hash_by_id(&state.db, user.id).await?;
    let ok = body.current_password.chars().count() <= validate::PASSWORD_MAX
        && password::verify(body.current_password, stored).await?;
    if !ok {
        limits::login_failed(&state, &user.email);
        return Err(ApiError::Validation(FieldErrors::from([(
            "current_password",
            "wrong",
        )])));
    }

    let hash = password::hash(new_password).await?;
    let revoked = user::set_password_hash(&state.db, user.id, &hash).await?;
    session::disconnect(&state, &revoked).await;
    info!(user_id = %user.id, sessions = revoked.len(), "password changed");

    let user = session::reload(&state.db, user.id).await?;
    signed_in(&state, &headers, user, StatusCode::OK).await
}

/// Ends every session of the signed-in account, on every device, and clears
/// this browser's cookie like `logout`.
async fn logout_all(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<Response, ApiError> {
    session::revoke_all(&state, current.user.id).await?;
    info!(user_id = %current.user.id, "signed out everywhere");
    Ok(signed_out(&state))
}

/// Without `Secure` cookies a session may only be issued to a loopback host,
/// where nothing crosses a network. A request without `Host` is no browser
/// and has no cookie jar to protect.
fn transport_allowed(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if state.cookie_secure || state.allow_plain_http {
        return Ok(());
    }
    match headers.get(HOST).and_then(|value| value.to_str().ok()) {
        Some(host) if !cookies::is_loopback_host(host) => Err(ApiError::InsecureTransport),
        _ => Ok(()),
    }
}

/// Bootstrap for the web client: who am I (if anyone) and which CSRF token
/// do I send. Issues the CSRF cookie when the browser has none yet.
async fn current_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    MaybeUser(user): MaybeUser,
) -> Response {
    let existing = cookies::get(&headers, CSRF_COOKIE).filter(|value| token::is_valid(value));
    let (csrf_token, set_cookie) = match existing {
        Some(value) => (value, None),
        None => {
            let fresh = token::generate();
            let cookie = csrf_cookie(&state, &fresh);
            (fresh, Some(cookie))
        }
    };

    let body = Json(SessionResponse { user, csrf_token });
    match set_cookie {
        Some(cookie) => (AppendHeaders([cookie.header()]), body).into_response(),
        None => body.into_response(),
    }
}

fn csrf_cookie(state: &AppState, value: &str) -> SetCookie {
    SetCookie::new(CSRF_COOKIE, value, CSRF_COOKIE_MAX_AGE_SECS).secure(state.cookie_secure)
}
