//! The account's own lifecycle (#519): `DELETE /api/me` deletes it.
//!
//! Deletion has two phases. The route ([`delete_me`]) checks that it is
//! really the owner, refuses while they own servers, and in one transaction
//! marks the row (`users.deleted_at`), ends every session and removes
//! sign-in tokens, memberships, invites and the data export. From then on
//! nobody can sign in to or address the account. Messages, DMs, files and
//! finally the row itself go in the background ([`deletion`]).
//!
//! `/api/me/export` requests a data export and tells its state; the archive
//! is built in the background ([`export`]) and downloaded from
//! `/api/me/export/download` by the signed-in account only.

pub mod deletion;
pub mod export;

use std::time::SystemTime;

use axum::Router;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_RANGE, RANGE, VARY};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::{delete, get};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::auth::session::{self, CurrentSession, CurrentUser};
use crate::auth::{self, LogoutResponse, Redacted, user, validate};
use crate::directory;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::Recheck;
use crate::json::Body;
use crate::limits;
use crate::password;
use crate::servers::moderation;
use crate::state::AppState;
use crate::storage::{ObjectBody, StoreError};

use self::export::ExportView;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me", delete(delete_me))
        .route("/api/me/export", get(export_status).post(request_export))
        .route("/api/me/export/download", get(download_export))
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
    // Whoever answers those queues hears the smaller count (`jq`).
    for &server_id in &removed.requests {
        directory::requests::publish_queue(&state, server_id).await;
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

/// `GET` and `POST /api/me/export`: the account's export, if any, and
/// whether account mail is on (then a mail says when it is ready).
#[derive(Debug, Serialize)]
pub struct ExportResponse {
    pub export: Option<ExportView>,
    pub mail: bool,
}

/// How long a ready export keeps a new request away.
const EXPORT_INTERVAL_SECS: f64 = 24.0 * 3600.0;

/// The account's data export. A ready one past its retention is no longer
/// shown; the worker deletes it on its next pass.
async fn export_status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<ExportResponse>, ApiError> {
    let export = sqlx::query_as::<_, ExportView>(
        "SELECT state, requested_at, finished_at, expires_at, size_bytes FROM account_exports \
         WHERE user_id = $1 AND NOT (state = 'ready' AND expires_at <= now())",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    Ok(Json(ExportResponse {
        export,
        mail: state.mail.is_some(),
    }))
}

/// Requests a data export: `202` with it queued. One that is queued or
/// being built answers `409 export_pending`; a ready one younger than 24 h
/// answers `429` until then. A failed one, an older one or one past its
/// retention is replaced, and its archive deleted.
async fn request_export(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<(StatusCode, Json<ExportResponse>), ApiError> {
    let mut tx = state.db.begin().await?;
    // A row that is not replaced is still locked by the conflict, so the
    // read below sees what kept it.
    let queued = sqlx::query_as::<_, ExportView>(
        "INSERT INTO account_exports (user_id, state) VALUES ($1, 'pending') \
         ON CONFLICT (user_id) DO UPDATE SET state = 'pending', requested_at = now(), \
             attempts = 0, claim = NULL, claimed_until = NULL, finished_at = NULL, \
             expires_at = NULL, object_key = NULL, size_bytes = NULL \
         WHERE account_exports.state = 'failed' \
            OR (account_exports.state = 'ready' \
                AND (account_exports.requested_at <= now() - interval '24 hours' \
                     OR account_exports.expires_at <= now())) \
         RETURNING state, requested_at, finished_at, expires_at, size_bytes",
    )
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(queued) = queued else {
        let (current, wait): (String, f64) = sqlx::query_as(
            "SELECT state, EXTRACT(EPOCH FROM requested_at - now())::float8 + $2 \
             FROM account_exports WHERE user_id = $1",
        )
        .bind(user.id)
        .bind(EXPORT_INTERVAL_SECS)
        .fetch_one(&mut *tx)
        .await?;
        tx.rollback().await?;
        return Err(if current == "ready" {
            ApiError::RateLimited {
                retry_after: wait.ceil().max(1.0) as u64,
            }
        } else {
            ApiError::ExportPending
        });
    };
    tx.commit().await?;
    export::wake(&state);
    info!(user_id = %user.id, "data export requested");
    Ok((
        StatusCode::ACCEPTED,
        Json(ExportResponse {
            export: Some(queued),
            mail: state.mail.is_some(),
        }),
    ))
}

/// The ready archive, to save: a redirect to MinIO, or the bytes through
/// the API (proxy delivery, memory store), like an attachment download.
/// `404` while there is none.
async fn download_export(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    request_headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (key, size, finished_at): (String, i64, DateTime<Utc>) = sqlx::query_as(
        "SELECT object_key, size_bytes, finished_at FROM account_exports \
         WHERE user_id = $1 AND state = 'ready' AND expires_at > now()",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let disposition = format!(
        "attachment; filename=\"gelabber-export-{}.zip\"",
        finished_at.format("%Y-%m-%d")
    );
    let range = request_headers
        .get(RANGE)
        .and_then(|value| value.to_str().ok());
    let body = state
        .store
        .serve(
            &key,
            "application/zip",
            Some(disposition.as_str()),
            range,
            SystemTime::now(),
        )
        .await;
    match body {
        Ok(ObjectBody::Redirect(url)) => {
            let mut response = Redirect::temporary(&url).into_response();
            let headers = response.headers_mut();
            headers.insert(VARY, HeaderValue::from_static("Cookie"));
            headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
            Ok(response)
        }
        Ok(ObjectBody::Stream(object)) => Ok(crate::attachments::streamed(
            &disposition,
            "application/zip",
            true,
            object,
        )),
        Err(StoreError::NotFound) => Err(ApiError::NotFound),
        Err(StoreError::RangeNotSatisfiable) => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(CONTENT_RANGE, value);
            }
            Ok(response)
        }
        Err(err) => Err(ApiError::Internal(format!("export download: {err}"))),
    }
}
