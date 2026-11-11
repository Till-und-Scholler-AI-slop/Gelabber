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

pub mod device;
pub mod devices;
pub mod oidc;
pub mod session;
pub mod tokens;
pub mod user;
pub mod validate;

use axum::Router;
use axum::extract::State;
use axum::http::header::{ACCEPT_LANGUAGE, HOST};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{AppendHeaders, IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::RegistrationMode;
use crate::cookies::{self, CSRF_COOKIE, SESSION_COOKIE, SetCookie};
use crate::error::{ApiError, FieldErrors};
use crate::i18n::{self, Locale};
use crate::json::Body;
use crate::limits::{self, MailBudget};
use crate::mail::{Mail, Mailer};
use crate::operator;
use crate::password;
use crate::state::AppState;
use crate::token;

use self::session::{CurrentSession, MaybeUser};
use self::tokens::Purpose;
use self::user::User;

/// CSRF cookies outlive sessions on purpose: an anonymous visitor needs one
/// to be allowed to log in at all.
const CSRF_COOKIE_MAX_AGE_SECS: u64 = 365 * 24 * 3600;

/// How recent a sign-in must be for a step that cannot ask for a password
/// because the account has none, such as setting the first one
/// (`403 reauth_required` otherwise).
pub const REAUTH_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Refuses with `403 reauth_required` unless `current` was signed in less
/// than [`REAUTH_WINDOW`] ago.
pub fn recently_signed_in(current: &CurrentSession) -> Result<(), ApiError> {
    let age = chrono::Utc::now() - current.created_at;
    match chrono::Duration::from_std(REAUTH_WINDOW) {
        Ok(window) if age < window => Ok(()),
        _ => Err(ApiError::ReauthRequired),
    }
}

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
        .route("/api/auth/registration", get(registration_status))
        .merge(oidc::routes::router())
        .route("/api/auth/sessions", get(devices::list))
        .route("/api/auth/sessions/{id}", delete(devices::revoke))
}

/// Request bodies with a password print it as `<redacted>`.
pub(crate) struct Redacted;

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
    /// `de` or `en`; without it the `Accept-Language` header decides, and
    /// without that German (clients older than v0.10).
    #[serde(default)]
    pub locale: Option<String>,
    /// A beta invitation's code (`operator::admission`); only an
    /// invitation-only instance needs one.
    #[serde(default)]
    pub invitation: Option<String>,
    /// The code of the server invite that led here, which admits a new
    /// account on an invitation-only instance as well.
    #[serde(default)]
    pub server_invite: Option<String>,
}

impl std::fmt::Debug for RegisterBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterBody")
            .field("email", &self.email)
            .field("password", &Redacted)
            .field("name", &self.name)
            .field("locale", &self.locale)
            .field("invitation", &Redacted)
            .field("server_invite", &Redacted)
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
    /// Whether `user` is an instance operator (`operator::access`); the
    /// clients offer the area "Betrieb" only then.
    pub operator: bool,
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

/// `GET /api/auth/registration`: whether registering needs an invitation.
#[derive(Debug, Serialize)]
pub struct RegistrationStatus {
    pub mode: RegistrationMode,
}

async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<RegisterBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let mut errors = FieldErrors::new();
    let email = validate::new_email(&body.email, &mut errors);
    let name = validate::name(&body.name, &mut errors);
    let pw = validate::password(&body.password, &mut errors);
    let locale = match body.locale.as_deref() {
        Some(raw) => Locale::parse(raw).or_else(|| {
            errors.insert("locale", "invalid");
            None
        }),
        None => Some(
            headers
                .get(ACCEPT_LANGUAGE)
                .and_then(Locale::from_accept_language)
                .unwrap_or(Locale::De),
        ),
    };
    validate::finish(errors)?;
    let (email, name, pw, locale) = (
        email.expect("validated"),
        name.expect("validated"),
        pw.expect("validated"),
        locale.expect("validated"),
    );
    // Before any account lookup, so a refusal tells nothing about one.
    let admitted = {
        let mut conn = state.db.acquire().await?;
        operator::admission::admit(
            &state,
            &mut conn,
            &email,
            body.invitation.as_deref(),
            body.server_invite.as_deref(),
        )
        .await?
    };

    let hash = password::hash(pw).await?;
    let Some(mailer) = state.mail.as_ref() else {
        let mut tx = state.db.begin().await?;
        let user = user::insert_in(&mut tx, &email, &name, &hash, locale).await?;
        operator::admission::redeem(&mut tx, &admitted, user.id).await?;
        tx.commit().await?;
        info!(user_id = %user.id, "user registered");
        return signed_in(&state, &headers, user, StatusCode::CREATED).await;
    };

    // Every branch answers alike; only the mail differs. A confirmed
    // address keeps its account untouched, whoever typed it; a pending one
    // now carries this registrant's credentials. Once the address had its
    // hourly mails nothing is written either: a takeover without a new
    // link would leave the owner without a working one. Only a new or
    // taken-over pending row uses up an invitation.
    if limits::mail_allowed(&state, MailBudget::Registration, &email) {
        let outgoing = match register_unverified(&state, &email, &name, &hash, locale, &admitted)
            .await
        {
            Ok(Some(user_id)) => {
                info!(%user_id, "user registered, awaiting confirmation");
                Ok(Some((user_id, Outgoing::Confirm)))
            }
            Ok(None) => user::verified_by_email(&state.db, &email)
                .await
                .map(|found| match found {
                    Some((user_id, true)) => Some((user_id, Outgoing::AlreadyRegistered)),
                    Some((user_id, false)) => Some((user_id, Outgoing::Confirm)),
                    None => None,
                }),
            Err(err) => Err(err),
        };
        match outgoing {
            Ok(Some((user_id, outgoing))) => {
                send_mail(
                    &state,
                    mailer,
                    MailBudget::Registration,
                    user_id,
                    &email,
                    outgoing,
                );
            }
            Ok(None) => limits::mail_unsent(&state, MailBudget::Registration, &email),
            Err(err) => {
                limits::mail_unsent(&state, MailBudget::Registration, &email);
                return Err(err);
            }
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

/// [`user::insert_unverified_in`] and, for a new or taken-over pending row,
/// [`operator::admission::redeem`] in one transaction.
async fn register_unverified(
    state: &AppState,
    email: &str,
    name: &str,
    hash: &str,
    locale: Locale,
    admitted: &operator::admission::Admitted,
) -> Result<Option<uuid::Uuid>, ApiError> {
    let mut tx = state.db.begin().await?;
    let id = user::insert_unverified_in(&mut tx, email, name, hash, locale).await?;
    if let Some(id) = id {
        operator::admission::redeem(&mut tx, admitted, id).await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// An account mail before its link's token exists.
#[derive(Debug, Clone, Copy)]
enum Outgoing {
    Confirm,
    Reset,
    AlreadyRegistered,
    /// A provider (its display name) was connected to the account (#518).
    ProviderConnected(&'static str),
}

/// Sends a mail the caller reserved from `budget` ([`limits::mail_allowed`])
/// after the response. The link's token is issued there as well: written
/// before answering, it would make a known address answer slower than an
/// unknown one. A mail that cannot be sent gives its reservation back.
fn send_mail(
    state: &AppState,
    mailer: &Mailer,
    budget: MailBudget,
    user_id: uuid::Uuid,
    email: &str,
    outgoing: Outgoing,
) {
    let (state, mailer, email) = (state.clone(), mailer.clone(), email.to_owned());
    tokio::spawn(async move {
        let mail = match outgoing {
            Outgoing::Confirm => tokens::issue(&state.db, user_id, Purpose::VerifyEmail)
                .await
                .map(|token| Mail::ConfirmEmail { token }),
            Outgoing::Reset => tokens::issue(&state.db, user_id, Purpose::ResetPassword)
                .await
                .map(|token| Mail::ResetPassword { token }),
            Outgoing::AlreadyRegistered => Ok(Mail::AlreadyRegistered),
            Outgoing::ProviderConnected(name) => Ok(Mail::ProviderConnected {
                provider_name: name.to_owned(),
            }),
        }
        .map_err(|err| ("token", format!("{err:?}")));
        // The account's language, read here too so the answer's timing
        // stays the same for every address.
        let mail = match mail {
            Ok(mail) => i18n::of_recipient(&state.db, user_id)
                .await
                .map(|locale| (mail, locale))
                .map_err(|err| ("locale", format!("{err:?}"))),
            Err(err) => Err(err),
        };
        let sent = match mail {
            Ok((mail, locale)) => mailer
                .send(&email, &mail, locale)
                .await
                .map(|()| mail.kind())
                .map_err(|err| (mail.kind(), err)),
            Err(err) => Err(err),
        };
        match sent {
            Ok(kind) => info!(%user_id, kind, "account mail sent"),
            Err((kind, err)) => {
                limits::mail_unsent(&state, budget, &email);
                warn!(%user_id, kind, error = %err, "account mail failed");
            }
        }
    });
}

/// Sends `mail` to an address without an account (a beta invitation) in
/// `locale`, after the response. The caller reserved it from `budget`
/// ([`limits::mail_allowed`]); a mail that cannot be sent gives the
/// reservation back. Does nothing without account mail.
pub fn send_mail_to(
    state: &AppState,
    address: &str,
    locale: Locale,
    mail: Mail,
    budget: MailBudget,
) {
    let Some(mailer) = state.mail.clone() else {
        limits::mail_unsent(state, budget, address);
        return;
    };
    let (state, address) = (state.clone(), address.to_owned());
    tokio::spawn(async move {
        let kind = mail.kind();
        match mailer.send(&address, &mail, locale).await {
            Ok(()) => info!(kind, "mail sent"),
            Err(err) => {
                limits::mail_unsent(&state, budget, &address);
                warn!(kind, error = %err, "mail failed");
            }
        }
    });
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
    let typed = email.expect("validated");
    // Registration stores the ASCII form of the domain; an older account
    // may still carry the spelling that was typed.
    let ascii = validate::ascii_domain(&typed);
    let email = ascii.clone().unwrap_or_else(|| typed.clone());
    limits::login_attempt(&state, &email)?;

    // Register never stores anything longer, so this cannot match; fail
    // before the lookup and the hash, with the same answer as a wrong
    // password.
    if body.password.chars().count() > validate::PASSWORD_MAX {
        return Err(ApiError::InvalidCredentials);
    }

    // Unknown address: `verify` runs Argon2 against a dummy hash anyway so
    // the response time does not reveal whether the account exists.
    let mut found = user::credentials_by_email(&state.db, &typed).await?;
    if found.is_none()
        && let Some(ascii) = &ascii
    {
        found = user::credentials_by_email(&state.db, ascii).await?;
    }
    // An account without a password (migration 0033) runs against the
    // dummy hash too and answers like a wrong password.
    let stored = found.as_ref().and_then(|c| c.password_hash.clone());
    let ok = password::verify(body.password, stored).await?;

    let Some(credentials) = found.filter(|_| ok) else {
        return Err(ApiError::InvalidCredentials);
    };
    limits::login_succeeded(&state, &email);
    // Only after the password matched, so the answer tells nothing to
    // someone who does not know it. A disabled account gets no new
    // confirmation link either.
    if credentials.disabled {
        return Err(ApiError::AccountDisabled);
    }
    if !credentials.email_verified
        && let Some(mailer) = state.mail.as_ref()
    {
        if limits::mail_allowed(&state, MailBudget::Registration, &email) {
            send_mail(
                &state,
                mailer,
                MailBudget::Registration,
                credentials.id,
                &email,
                Outgoing::Confirm,
            );
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
/// A disabled account is refused here (`403 account_disabled`), whichever
/// path signed it in.
async fn signed_in(
    state: &AppState,
    headers: &HeaderMap,
    user: User,
    status: StatusCode,
) -> Result<Response, ApiError> {
    let (cookies, body) = signed_in_parts(state, headers, user).await?;
    Ok((status, AppendHeaders(cookies), Json(body)).into_response())
}

/// The `Set-Cookie` headers (session, CSRF) and body of a sign-in.
type SessionCookies = [(axum::http::HeaderName, axum::http::HeaderValue); 2];

/// [`signed_in`] for a caller that answers in its own shape: a provider's
/// callback redirects, and its JSON routes add fields to the body.
async fn signed_in_parts(
    state: &AppState,
    headers: &HeaderMap,
    user: User,
) -> Result<(SessionCookies, SessionResponse), ApiError> {
    // One read for both: a disabled account gets no session (#517), and
    // the operator flag needs the confirmation while mail is on.
    let (disabled, verified): (bool, bool) = sqlx::query_as(
        "SELECT disabled_at IS NOT NULL, email_verified_at IS NOT NULL FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::Internal(format!("user {} vanished mid-request", user.id)))?;
    if disabled {
        info!(user_id = %user.id, "sign-in refused, account disabled");
        return Err(ApiError::AccountDisabled);
    }
    let operator = operator::access::is_operator(state, &user, verified);
    let previous = cookies::get(headers, SESSION_COOKIE);
    session::revoke_current(state, previous.as_deref()).await?;
    // Expired-row housekeeping is separate from the targeted logout.
    session::revoke(&state.db, None).await?;
    // The row starts with the idle window and slides from there
    // (`session::touch`); the cookie carries the absolute lifetime.
    // The device is the client this answer reaches: the browser of a
    // provider's callback, the app of a desktop exchange.
    let device = device::Device::from_headers(headers);
    let session_token = session::create(&state.db, user.id, state.session_idle, device).await?;
    let csrf_token = token::generate();

    let cookies = [
        SetCookie::new(SESSION_COOKIE, session_token, state.session_ttl.as_secs())
            .secure(state.cookie_secure)
            .header(),
        csrf_cookie(state, &csrf_token).header(),
    ];
    let body = SessionResponse {
        user: Some(user),
        csrf_token,
        operator,
    };
    Ok((cookies, body))
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

pub(crate) fn signed_out_cookies(
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
///
/// An account without a password (it signs in through a provider) sets its
/// first one without `current_password`, but only from a session signed in
/// less than [`REAUTH_WINDOW`] ago (`403 reauth_required` otherwise).
async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    current: CurrentSession,
    Body(body): Body<PasswordBody>,
) -> Result<Response, ApiError> {
    transport_allowed(&state, &headers)?;
    let stored = user::password_hash_by_id(&state.db, current.user.id).await?;
    let mut errors = FieldErrors::new();
    if stored.is_some() && body.current_password.is_empty() {
        errors.insert("current_password", "required");
    }
    let new_password = validate::password(&body.password, &mut errors);
    validate::finish(errors)?;
    let new_password = new_password.expect("validated");

    if stored.is_some() {
        let key = validate::canonical(&current.user.email);
        limits::login_attempt(&state, &key)?;
        let ok = body.current_password.chars().count() <= validate::PASSWORD_MAX
            && password::verify(body.current_password, stored).await?;
        if !ok {
            return Err(ApiError::Validation(FieldErrors::from([(
                "current_password",
                "wrong",
            )])));
        }
        limits::login_succeeded(&state, &key);
    } else {
        recently_signed_in(&current)?;
    }
    let user = current.user;

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

/// Whether registering needs an invitation (`API_REGISTRATION`); the
/// clients show the invitation field then.
async fn registration_status(State(state): State<AppState>) -> Json<RegistrationStatus> {
    Json(RegistrationStatus {
        mode: state.registration,
    })
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
    let key = validate::canonical(&user.email);
    limits::login_attempt(&state, &key)?;
    // An account without a password has no pending registration to confirm
    // (a provider's takeover clears it); `verify` then answers `wrong`.
    let stored = user::password_hash_by_id(&state.db, user_id).await?;
    let ok = body.password.chars().count() <= validate::PASSWORD_MAX
        && password::verify(body.password, stored).await?;
    if !ok {
        return Err(ApiError::Validation(FieldErrors::from([(
            "password", "wrong",
        )])));
    }
    limits::login_succeeded(&state, &key);
    // A disabled account stays unconfirmed and keeps its link (#517).
    if user::is_disabled(&state.db, user_id).await? {
        return Err(ApiError::AccountDisabled);
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
    let typed = email.expect("validated");
    // Either spelling of the domain, as at sign-in.
    let ascii = validate::ascii_domain(&typed);
    let mut found = user::verified_by_email(&state.db, &typed).await?;
    if found.is_none()
        && let Some(ascii) = &ascii
    {
        found = user::verified_by_email(&state.db, ascii).await?;
    }
    let email = ascii.unwrap_or(typed);

    if let Some((user_id, _)) = found
        && limits::mail_allowed(&state, MailBudget::Reset, &email)
    {
        send_mail(
            &state,
            mailer,
            MailBudget::Reset,
            user_id,
            &email,
            Outgoing::Reset,
        );
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
    let user_id = tokens::peek(&state.db, &body.token, Purpose::ResetPassword)
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    // A disabled account keeps its password (#517).
    if user::is_disabled(&state.db, user_id).await? {
        return Err(ApiError::AccountDisabled);
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
) -> Result<Response, ApiError> {
    let existing = cookies::get(&headers, CSRF_COOKIE).filter(|value| token::is_valid(value));
    let (csrf_token, set_cookie) = match existing {
        Some(value) => (value, None),
        None => {
            let fresh = token::generate();
            let cookie = csrf_cookie(&state, &fresh);
            (fresh, Some(cookie))
        }
    };

    let operator = match &user {
        Some(user) => operator::access::is_signed_in_operator(&state, user).await?,
        None => false,
    };
    let body = Json(SessionResponse {
        user,
        csrf_token,
        operator,
    });
    Ok(match set_cookie {
        Some(cookie) => (AppendHeaders([cookie.header()]), body).into_response(),
        None => body.into_response(),
    })
}

fn csrf_cookie(state: &AppState, value: &str) -> SetCookie {
    SetCookie::new(CSRF_COOKIE, value, CSRF_COOKIE_MAX_AGE_SECS).secure(state.cookie_secure)
}
