//! JSON error envelope shared by every `/api` route.
//!
//! ```json
//! {"error":"validation_failed","message":"…","fields":{"email":"invalid"}}
//! ```
//!
//! `error` is a stable machine code the web client switches on, `fields`
//! carries per-field codes (`required`, `invalid`, `too_short`, `too_long`,
//! `taken`) so the UI can render them inline in its own language. `message`
//! is a short English fallback. Internal failures never leak their cause into
//! the body; the detail goes to the `error!` log line.
//!
//! The `Forbidden` message and `NotFound` are the only per-resource signals:
//! a server the caller is not a member of answers `404`, never `403`, so the
//! API does not confirm that a foreign server id exists.

use std::collections::BTreeMap;

use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde::Serialize;
use tracing::error;

pub type FieldErrors = BTreeMap<&'static str, &'static str>;

#[derive(Debug)]
pub enum ApiError {
    /// 422 with per-field codes.
    Validation(FieldErrors),
    /// 400: body is not the JSON shape the route expects.
    BadRequest(String),
    /// 401: no or expired session cookie.
    Unauthenticated,
    /// 401: e-mail/password pair does not match.
    InvalidCredentials,
    /// 403: mutation without a matching `X-CSRF-Token`.
    Csrf,
    /// 403: sign-in over plain HTTP from another machine; the session cookie
    /// would cross the network unencrypted.
    InsecureTransport,
    /// 403: signed in, but the membership/permission does not allow it. The
    /// string is the public message (static copy, no user data).
    Forbidden(&'static str),
    /// 404: the resource does not exist — or exists but the caller is not a
    /// member, which is deliberately indistinguishable.
    NotFound,
    /// 409: e-mail already registered.
    EmailTaken,
    /// 403: right password, but the address is not confirmed yet while
    /// account mail is on. A new confirmation link is on its way.
    EmailUnverified,
    /// 410: a mailed confirmation or reset link is unknown, expired or used.
    TokenInvalid,
    /// 409: another device saved a newer theme revision.
    ThemeConflict,
    /// 410: the invite link exists but is expired or used up.
    InviteInvalid,
    /// 403: the caller is banned from this server (invite rejoin included).
    Banned,
    /// 429: a request window (auth / API / messages / uploads) is exhausted.
    RateLimited { retry_after: u64 },
    /// 429: the caller's daily upload byte quota is full.
    QuotaExceeded,
    /// 502: an outside service the API relays (the GIF provider) failed.
    /// The string is logged, not returned.
    Upstream(String),
    /// 503: the GIF provider refused the instance's API key for now (its
    /// quota is shared by every user). Not the caller's own limit.
    UpstreamQuota { retry_after: u64 },
    /// 500: anything unexpected. The string is logged, not returned.
    Internal(String),
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: FieldErrors,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

impl ApiError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation_failed",
            Self::BadRequest(_) => "bad_request",
            Self::Unauthenticated => "unauthenticated",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Csrf => "csrf_invalid",
            Self::InsecureTransport => "insecure_transport",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound => "not_found",
            Self::EmailTaken => "email_taken",
            Self::EmailUnverified => "email_unverified",
            Self::TokenInvalid => "token_invalid",
            Self::ThemeConflict => "theme_conflict",
            Self::InviteInvalid => "invite_invalid",
            Self::Banned => "banned",
            Self::RateLimited { .. } => "rate_limited",
            Self::QuotaExceeded => "quota_exceeded",
            Self::Upstream(_) => "upstream_unavailable",
            Self::UpstreamQuota { .. } => "upstream_quota",
            Self::Internal(_) => "internal",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthenticated | Self::InvalidCredentials => StatusCode::UNAUTHORIZED,
            Self::Csrf
            | Self::InsecureTransport
            | Self::Forbidden(_)
            | Self::Banned
            | Self::EmailUnverified => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::EmailTaken | Self::ThemeConflict => StatusCode::CONFLICT,
            Self::InviteInvalid | Self::TokenInvalid => StatusCode::GONE,
            Self::RateLimited { .. } | Self::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::UpstreamQuota { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn message(&self) -> &'static str {
        match self {
            Self::Validation(_) => "Some fields are invalid.",
            Self::BadRequest(_) => "Request body could not be read.",
            Self::Unauthenticated => "Sign in required.",
            Self::InvalidCredentials => "E-mail or password is wrong.",
            Self::Csrf => "Missing or invalid CSRF token.",
            Self::InsecureTransport => {
                "Sign-in needs HTTPS. Open the app over https:// (or http://localhost on this machine)."
            }
            Self::Forbidden(message) => message,
            Self::NotFound => "Not found.",
            Self::EmailTaken => "This e-mail address is already registered.",
            Self::EmailUnverified => "Confirm your e-mail address first. We sent you a new link.",
            Self::TokenInvalid => "This link has expired or has already been used.",
            Self::ThemeConflict => "Themes changed on another device. Reload and try again.",
            Self::InviteInvalid => "This invite link has expired or been used up.",
            Self::Banned => "You are banned from this server.",
            Self::RateLimited { .. } => "Too many requests. Try again in a moment.",
            Self::QuotaExceeded => "Daily upload quota reached.",
            Self::Upstream(_) => "The GIF service did not answer. Try again in a moment.",
            Self::UpstreamQuota { .. } => {
                "This server has used up its GIF requests for now. Try again later."
            }
            Self::Internal(_) => "Something went wrong on our side.",
        }
    }

    pub fn body(&self) -> ErrorBody {
        let fields = match self {
            Self::Validation(fields) => fields.clone(),
            Self::EmailTaken => BTreeMap::from([("email", "taken")]),
            _ => BTreeMap::new(),
        };
        let retry_after = match self {
            Self::RateLimited { retry_after } | Self::UpstreamQuota { retry_after } => {
                Some(*retry_after)
            }
            _ => None,
        };
        ErrorBody {
            error: self.code(),
            message: self.message(),
            fields,
            retry_after,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match &self {
            Self::Internal(detail) => error!(error = %detail, "request failed"),
            Self::Upstream(detail) => tracing::warn!(error = %detail, "upstream failed"),
            Self::BadRequest(detail) => tracing::debug!(error = %detail, "bad request"),
            _ => {}
        }
        let retry_after = match &self {
            Self::RateLimited { retry_after } | Self::UpstreamQuota { retry_after } => {
                Some(*retry_after)
            }
            _ => None,
        };
        let mut response = (self.status(), Json(self.body())).into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(RETRY_AFTER, value);
        }
        response
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(format!("database error: {err}"))
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(err: tokio::task::JoinError) -> Self {
        Self::Internal(format!("blocking task failed: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_body_carries_field_codes() {
        let err = ApiError::Validation(BTreeMap::from([("email", "invalid")]));
        let json = serde_json::to_value(err.body()).unwrap();
        assert_eq!(json["error"], "validation_failed");
        assert_eq!(json["fields"]["email"], "invalid");
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn internal_body_hides_detail() {
        let err = ApiError::Internal("connection reset by peer".into());
        let json = serde_json::to_value(err.body()).unwrap();
        assert_eq!(json["error"], "internal");
        assert!(json.get("fields").is_none());
        assert!(!json.to_string().contains("connection reset"));
    }

    #[test]
    fn rate_limited_body_carries_retry_after() {
        let err = ApiError::RateLimited { retry_after: 12 };
        let json = serde_json::to_value(err.body()).unwrap();
        assert_eq!(json["error"], "rate_limited");
        assert_eq!(json["retry_after"], 12);
        assert_eq!(err.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
