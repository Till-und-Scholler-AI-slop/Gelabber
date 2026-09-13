//! `/api/auth/*`: register, login, logout, session bootstrap.
//!
//! Every successful register/login answers with two `Set-Cookie` headers
//! (`gelabber_session`, `gelabber_csrf`; both `HttpOnly; SameSite=Lax`) and a
//! JSON body `{ "user": …, "csrf_token": … }`. The web client keeps the CSRF
//! token in memory and sends it as `X-CSRF-Token` on every mutation.

pub mod session;
pub mod user;
pub mod validate;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{AppendHeaders, IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::cookies::{self, CSRF_COOKIE, SESSION_COOKIE, SetCookie};
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::password;
use crate::state::AppState;
use crate::token;

use self::session::MaybeUser;
use self::user::User;

/// CSRF cookies outlive sessions on purpose: an anonymous visitor needs one
/// to be allowed to log in at all.
const CSRF_COOKIE_MAX_AGE_SECS: u64 = 365 * 24 * 3600;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/session", get(current_session))
}

#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
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
    Body(body): Body<RegisterBody>,
) -> Result<Response, ApiError> {
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

    signed_in(&state, user, StatusCode::CREATED).await
}

async fn login(
    State(state): State<AppState>,
    Body(body): Body<LoginBody>,
) -> Result<Response, ApiError> {
    let mut errors = FieldErrors::new();
    let email = validate::email(&body.email, &mut errors);
    if body.password.is_empty() {
        errors.insert("password", "required");
    }
    validate::finish(errors)?;
    let email = email.expect("validated");

    // Unknown address: verify against a dummy hash anyway so the response
    // time does not reveal whether the account exists.
    let found = user::credentials_by_email(&state.db, &email).await?;
    let stored = found.as_ref().map_or_else(
        || password::dummy_hash().to_owned(),
        |c| c.password_hash.clone(),
    );
    let ok = password::verify(body.password, stored).await?;

    let Some(credentials) = found.filter(|_| ok) else {
        return Err(ApiError::InvalidCredentials);
    };
    let user = session::reload(&state.db, credentials.id).await?;
    info!(user_id = %user.id, "user logged in");

    signed_in(&state, user, StatusCode::OK).await
}

/// Issues a session + fresh CSRF token for `user` and builds the response.
async fn signed_in(state: &AppState, user: User, status: StatusCode) -> Result<Response, ApiError> {
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
    if let Some(raw) = cookies::get(&headers, SESSION_COOKIE) {
        session::delete(&state.db, &raw).await?;
    }
    let csrf_token = token::generate();
    let cookies = AppendHeaders([
        SetCookie::clear(SESSION_COOKIE)
            .secure(state.cookie_secure)
            .header(),
        csrf_cookie(&state, &csrf_token).header(),
    ]);
    Ok((StatusCode::OK, cookies, Json(LogoutResponse { csrf_token })).into_response())
}

/// Bootstrap for the web client: who am I (if anyone) and which CSRF token
/// do I send. Issues the CSRF cookie when the browser has none yet.
async fn current_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    MaybeUser(user): MaybeUser,
) -> Response {
    let existing = cookies::get(&headers, CSRF_COOKIE).filter(|value| is_token(value));
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

fn is_token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
