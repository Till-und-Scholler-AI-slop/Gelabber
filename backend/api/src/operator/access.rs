//! Who is an instance operator. Operators are named by `API_OPERATOR_EMAILS`;
//! no route grants or removes the right. While account mail is on, the
//! address must also be confirmed, so registering someone else's address
//! does not make anyone an operator.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::auth::session::CurrentSession;
use crate::auth::user::User;
use crate::auth::validate;
use crate::error::ApiError;
use crate::state::AppState;

/// The signed-in operator. Rejects with `401 unauthenticated` without a
/// session and with `404 not_found` for everyone else, so the operator
/// routes do not confirm that they exist.
#[derive(Debug, Clone)]
pub struct Operator {
    pub user: User,
    /// The session's Redis key ([`CurrentSession::key`]).
    pub session_key: String,
}

impl FromRequestParts<AppState> for Operator {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let session = CurrentSession::from_request_parts(parts, state).await?;
        if !is_signed_in_operator(state, &session.user).await? {
            return Err(ApiError::NotFound);
        }
        Ok(Self {
            session_key: session.key(),
            user: session.user,
        })
    }
}

/// Whether `user` is an operator, given whether its address is confirmed.
pub fn is_operator(state: &AppState, user: &User, verified: bool) -> bool {
    is_operator_address(state, &user.email, verified)
}

/// [`is_operator`] for an account known only by its stored address, as in
/// the list of accounts.
pub fn is_operator_address(state: &AppState, email: &str, verified: bool) -> bool {
    state.operators.contains(&validate::canonical(email)) && (state.mail.is_none() || verified)
}

/// [`is_operator`] for a signed-in account. Reads the confirmation only for
/// a listed address while mail is on; everyone else costs no query.
pub async fn is_signed_in_operator(state: &AppState, user: &User) -> Result<bool, ApiError> {
    if !is_operator(state, user, true) {
        return Ok(false);
    }
    if state.mail.is_none() {
        return Ok(true);
    }
    let verified = email_verified(state, user.id).await?;
    Ok(is_operator(state, user, verified))
}

async fn email_verified(state: &AppState, user_id: Uuid) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT email_verified_at IS NOT NULL FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(false))
}
