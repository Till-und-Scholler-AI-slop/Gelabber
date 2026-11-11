//! The account's own lifecycle (#519): `DELETE /api/me` deletes it.
//!
//! Deletion has two phases. The route ([`delete_me`]) checks that it is
//! really the owner, refuses while they own servers, and in one transaction
//! marks the row (`users.deleted_at`), ends every session and removes
//! sign-in tokens, memberships, invites and the data export. From then on
//! nobody can sign in to or address the account. Messages, DMs, files and
//! finally the row itself go in the background ([`deletion`]).

pub mod deletion;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::delete;
use serde::Deserialize;
use tracing::{info, warn};

use crate::auth::session::{self, CurrentSession};
use crate::auth::{self, LogoutResponse, Redacted, user, validate};
use crate::error::{ApiError, FieldErrors};
use crate::gateway::Recheck;
use crate::json::Body;
use crate::limits;
use crate::password;
use crate::servers::moderation;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/me", delete(delete_me))
}

/// The account's password, or for an account without one (it signs in
/// through a provider) its e-mail address.
#[derive(Default, Deserialize)]
pub struct DeleteAccountBody {
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub email: String,
}

impl std::fmt::Debug for DeleteAccountBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteAccountBody")
            .field("password", &Redacted)
            .field("email", &self.email)
            .finish()
    }
}

/// Deletes the signed-in account, for good. The password is checked like a
/// password change: wrong guesses count against the account like failed
/// logins. An account without a password types its address instead, from a
/// session signed in less than [`auth::REAUTH_WINDOW`] ago
/// (`403 reauth_required` otherwise). While the account owns a server the
/// answer is `409 owns_servers` with those servers.
///
/// Answers `202` with the cookies of a logout: the account is gone for
/// everyone at once, its data follows in the background.
async fn delete_me(
    State(state): State<AppState>,
    current: CurrentSession,
    Body(body): Body<DeleteAccountBody>,
) -> Result<Response, ApiError> {
    let user_id = current.user.id;
    let stored = user::password_hash_by_id(&state.db, user_id).await?;
    if stored.is_some() {
        if body.password.is_empty() {
            return Err(field("password", "required"));
        }
        let key = validate::canonical(&current.user.email);
        limits::login_attempt(&state, &key)?;
        let ok = body.password.chars().count() <= validate::PASSWORD_MAX
            && password::verify(body.password, stored).await?;
        if !ok {
            return Err(field("password", "wrong"));
        }
        limits::login_succeeded(&state, &key);
    } else {
        auth::recently_signed_in(&current)?;
        let typed = body.email.trim();
        if typed.is_empty() {
            return Err(field("email", "required"));
        }
        if validate::canonical(typed) != validate::canonical(&current.user.email) {
            return Err(field("email", "wrong"));
        }
    }

    let removed = deletion::mark_deleted(&state.db, user_id).await?;
    session::disconnect(&state, &removed.sessions).await;
    // Other members see the person leave each server at once (`k: "x"`).
    if !removed.servers.is_empty() {
        let mut conn = state.db.acquire().await?;
        for &server_id in &removed.servers {
            let left = match moderation::channel_ids(&mut conn, server_id).await {
                Ok(channels) => {
                    moderation::after_removal_with(
                        &state,
                        server_id,
                        &[user_id],
                        &channels,
                        "deleted",
                    )
                    .await
                }
                Err(err) => Err(err),
            };
            if let Err(err) = left {
                warn!(error = err.code(), %server_id, %user_id, "removal after account deletion failed");
            }
        }
    }
    state.gateway.announce_recheck(Recheck::User(user_id)).await;
    deletion::wake(&state);
    info!(
        %user_id,
        sessions = removed.sessions.len(),
        servers = removed.servers.len(),
        "account deleted, purge queued"
    );

    let (cookies, csrf_token) = auth::signed_out_cookies(&state);
    Ok((
        StatusCode::ACCEPTED,
        cookies,
        Json(LogoutResponse { csrf_token }),
    )
        .into_response())
}

fn field(name: &'static str, code: &'static str) -> ApiError {
    ApiError::Validation(FieldErrors::from([(name, code)]))
}
