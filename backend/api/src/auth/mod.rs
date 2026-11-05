//! `/api/auth/*`: register, login, logout, session bootstrap, password
//! change and signing out every session of the account; with account mail
//! (#195) also address confirmation and password reset by mailed link.
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
//!
//! With account mail on (`SMTP_HOST`, [`crate::mail`]) registration answers
//! `202` with `verification_sent` and no session, the same whether or not
//! the address already has an account; the mail tells the two apart. An
//! unconfirmed account cannot sign in (`403 email_unverified`). Without it,
//! registration signs in at once and `409 email_taken` names a known
//! address, as before.

pub mod session;
pub mod tokens;
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
use crate::mail::{Mail, Mailer};
use crate::password;
use crate::state::AppState;
use crate::token;

use self::session::{CurrentSession, MaybeUser};
use self::tokens::Purpose;
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
        .route("/api/auth/mail", get(mail_status))
        .route("/api/auth/verify-email", post(verify_email))
        .route("/api/auth/password-reset", post(request_password_reset))
        .route(
            "/api/auth/password-reset/confirm",
            post(confirm_password_reset),
        )
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

/// A confirmation link's token (secret like a password) and the account's
/// password, which proves the person confirming is the one who registered.
#[derive(Deserialize)]
pub struct VerifyBody {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub password: String,
}

impl std::fmt::Debug for VerifyBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifyBody")
            .field("token", &Redacted)
            .field("password", &Redacted)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub struct ResetRequestBody {
    #[serde(default)]
    pub email: String,
}

/// `password` is the new one, with the register rules.
#[derive(Deserialize)]
pub struct ResetConfirmBody {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub password: String,
}

impl std::fmt::Debug for ResetConfirmBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResetConfirmBody")
            .field("token", &Redacted)
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

/// Registration with account mail on: nobody is signed in (`user` is
/// always `null`) until the address is confirmed.
#[derive(Debug, Serialize)]
pub struct VerificationSentResponse {
    pub user: Option<User>,
    pub csrf_token: String,
    pub verification_sent: bool,
}

#[derive(Debug, Serialize)]
pub struct MailStatus {
    pub enabled: bool,
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
    let Some(mailer) = state.mail.as_ref() else {
        let user = user::insert(&state.db, &email, &name, &hash).await?;
        info!(user_id = %user.id, "user registered");
        return signed_in(&state, &headers, user, StatusCode::CREATED).await;
    };

    // Every branch answers alike; only the mail differs. A confirmed
    // address keeps its account untouched, whoever typed it; a pending one
    // now carries this registrant's credentials. Once the address had its
    // hourly mails nothing is written either: a takeover without a new
    // link would leave the owner without a working one.
    if limits::mail_allowed(&state, &email) {
        match user::insert_unverified(&state.db, &email, &name, &hash).await? {
            Some(user_id) => {
                info!(%user_id, "user registered, awaiting confirmation");
                mail_confirmation(&state, mailer, user_id, &email).await?;
            }
            None => match user::verified_by_email(&state.db, &email).await? {
                Some((user_id, true)) => mailer.spawn(user_id, &email, Mail::AlreadyRegistered),
                Some((user_id, false)) => {
                    mail_confirmation(&state, mailer, user_id, &email).await?;
                }
                None => {}
            },
        }
    }

    // The browser is left signed out, as for a fresh visitor.
    let previous = cookies::get(&headers, SESSION_COOKIE);
    session::revoke_current(&state, previous.as_deref()).await?;
    let (cookies, csrf_token) = signed_out_cookies(&state);
    let body = VerificationSentResponse {
        user: None,
        csrf_token,
        verification_sent: true,
    };
    Ok((StatusCode::ACCEPTED, cookies, Json(body)).into_response())
}

/// Mails a fresh confirmation link; the caller counted it against the
/// address's hourly mails ([`limits::mail_allowed`]).
async fn mail_confirmation(
    state: &AppState,
    mailer: &Mailer,
    user_id: uuid::Uuid,
    email: &str,
) -> Result<(), ApiError> {
    let token = tokens::issue(&state.db, user_id, Purpose::VerifyEmail).await?;
    mailer.spawn(user_id, email, Mail::ConfirmEmail { token });
    Ok(())
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
    // Only after the password matched, so the answer tells nothing to
    // someone who does not know it.
    if !credentials.email_verified
        && let Some(mailer) = state.mail.as_ref()
    {
        if limits::mail_allowed(&state, &email) {
            mail_confirmation(&state, mailer, credentials.id, &email).await?;
        }
        return Err(ApiError::EmailUnverified);
    }
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
    // The row starts with the idle window and slides from there
    // (`session::touch`); the cookie carries the absolute lifetime.
    let session_token = session::create(&state.db, user.id, state.session_idle).await?;
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
    let (cookies, csrf_token) = signed_out_cookies(state);
    (StatusCode::OK, cookies, Json(LogoutResponse { csrf_token })).into_response()
}

fn signed_out_cookies(
    state: &AppState,
) -> (
    AppendHeaders<[(axum::http::HeaderName, axum::http::HeaderValue); 2]>,
    String,
) {
    let csrf_token = token::generate();
    let cookies = AppendHeaders([
        SetCookie::clear(SESSION_COOKIE)
            .secure(state.cookie_secure)
            .header(),
        csrf_cookie(state, &csrf_token).header(),
    ]);
    (cookies, csrf_token)
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

/// Whether this instance sends account mail; the web client offers "forgot
/// password" only then.
async fn mail_status(State(state): State<AppState>) -> Json<MailStatus> {
    Json(MailStatus {
        enabled: state.mail.is_some(),
    })
}

/// Opens a confirmation link: with the account's password the address
/// counts as confirmed and this browser is signed in, whichever device
/// registered. The link alone is not enough: whoever registered someone
/// else's address would otherwise share the account its owner confirms by
/// clicking. A wrong password counts like a failed login and leaves the
/// link usable.
async fn verify_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<VerifyBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let user_id = tokens::peek(&state.db, &body.token, Purpose::VerifyEmail)
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    if body.password.is_empty() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "password", "required",
        )])));
    }
    let user = session::reload(&state.db, user_id).await?;
    limits::login_blocked(&state, &user.email)?;
    let stored = user::password_hash_by_id(&state.db, user_id).await?;
    let ok = body.password.chars().count() <= validate::PASSWORD_MAX
        && password::verify(body.password, stored).await?;
    if !ok {
        limits::login_failed(&state, &user.email);
        return Err(ApiError::Validation(FieldErrors::from([(
            "password", "wrong",
        )])));
    }

    // Re-registering a pending address in the meantime replaced the token,
    // so this one may be gone already.
    let user_id = tokens::verify_email(&state.db, &body.token)
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    info!(%user_id, "e-mail address confirmed");
    let user = session::reload(&state.db, user_id).await?;
    signed_in(&state, &headers, user, StatusCode::OK).await
}

/// Mails a reset link to the address if it has an account. The answer is
/// `202` either way; the link then works for an hour, once.
async fn request_password_reset(
    State(state): State<AppState>,
    Body(body): Body<ResetRequestBody>,
) -> Result<Response, ApiError> {
    let mailer = state.mail.as_ref().ok_or(ApiError::NotFound)?;
    let mut errors = FieldErrors::new();
    let email = validate::email(&body.email, &mut errors);
    validate::finish(errors)?;
    let email = email.expect("validated");

    if let Some((user_id, _)) = user::verified_by_email(&state.db, &email).await?
        && limits::mail_allowed(&state, &email)
    {
        let token = tokens::issue(&state.db, user_id, Purpose::ResetPassword).await?;
        mailer.spawn(user_id, &email, Mail::ResetPassword { token });
        info!(%user_id, "password reset requested");
    }
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({}))).into_response())
}

/// Sets a new password through a reset link. Like a password change every
/// session ends, and this browser gets a fresh one. The address counts as
/// confirmed: the link reached its inbox.
async fn confirm_password_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<ResetConfirmBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let mut errors = FieldErrors::new();
    if body.token.is_empty() {
        errors.insert("token", "required");
    }
    let new_password = validate::password(&body.password, &mut errors);
    validate::finish(errors)?;
    let new_password = new_password.expect("validated");

    // A dead link costs no Argon2 run.
    if tokens::peek(&state.db, &body.token, Purpose::ResetPassword)
        .await?
        .is_none()
    {
        return Err(ApiError::TokenInvalid);
    }
    let hash = password::hash(new_password).await?;
    let (user_id, revoked) = tokens::reset_password(&state.db, &body.token, &hash)
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    session::disconnect(&state, &revoked).await;
    info!(%user_id, sessions = revoked.len(), "password reset");

    let user = session::reload(&state.db, user_id).await?;
    signed_in(&state, &headers, user, StatusCode::OK).await
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
