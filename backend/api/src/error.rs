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
    /// 403: an operator disabled the account (`users.disabled_at`). Only
    /// answered to someone who proved the password or held a mailed link.
    AccountDisabled,
    /// 403: registration on an invitation-only instance without a beta
    /// invitation or a server invite (`operator::admission`).
    InvitationRequired,
    /// 410: the registration's invitation code is unknown, used, revoked,
    /// expired, for another address, or a blocked server's invite.
    InvitationInvalid,
    /// 403: an operator blocked the server (`servers.blocked_at`). Members
    /// still see it in their list; only leaving and deleting it work.
    ServerBlocked,
    /// 409: the listed server takes requests to join, not direct joins.
    JoinRequestRequired,
    /// 409: the server declined the caller's last request to join, and the
    /// cooldown has `retry_after` seconds left.
    JoinRequestDeclined { retry_after: u64 },
    /// 409: the caller or the server has as many pending requests to join as
    /// the directory allows.
    JoinRequestsFull,
    /// 409: a request to join a server the caller is a member of.
    AlreadyMember,
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
            Self::AccountDisabled => "account_disabled",
            Self::InvitationRequired => "invitation_required",
            Self::InvitationInvalid => "invitation_invalid",
            Self::ServerBlocked => "server_blocked",
            Self::JoinRequestRequired => "join_request_required",
            Self::JoinRequestDeclined { .. } => "join_request_declined",
            Self::JoinRequestsFull => "join_requests_full",
            Self::AlreadyMember => "already_member",
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
            Self::AccountDisabled => StatusCode::FORBIDDEN,
            Self::InvitationRequired => StatusCode::FORBIDDEN,
            Self::InvitationInvalid => StatusCode::GONE,
            Self::ServerBlocked => StatusCode::FORBIDDEN,
            Self::JoinRequestRequired
            | Self::JoinRequestDeclined { .. }
            | Self::JoinRequestsFull
            | Self::AlreadyMember => StatusCode::CONFLICT,
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
            Self::AccountDisabled => "This account has been disabled.",
            Self::InvitationRequired => "Registration needs an invitation.",
            Self::InvitationInvalid => "This invitation is unknown, expired or already used.",
            Self::ServerBlocked => "This server has been blocked by the operator.",
            Self::JoinRequestRequired => "This server takes requests to join.",
            Self::JoinRequestDeclined { .. } => {
                "Your request to join was not accepted. You can ask again later."
            }
            Self::JoinRequestsFull => "Too many open requests to join. Try again later.",
            Self::AlreadyMember => "You are already a member of this server.",
        }
    }

    pub fn body(&self) -> ErrorBody {
        let fields = match self {
            Self::Validation(fields) => fields.clone(),
            Self::EmailTaken => BTreeMap::from([("email", "taken")]),
            Self::InvitationRequired => BTreeMap::from([("invitation", "required")]),
            Self::InvitationInvalid => BTreeMap::from([("invitation", "invalid")]),
            _ => BTreeMap::new(),
        };
        let retry_after = match self {
            Self::RateLimited { retry_after }
            | Self::UpstreamQuota { retry_after }
            | Self::JoinRequestDeclined { retry_after } => Some(*retry_after),
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

    #[test]
    fn account_disabled_is_403_without_fields() {
        let err = ApiError::AccountDisabled;
        let json = serde_json::to_value(err.body()).unwrap();
        assert_eq!(json["error"], "account_disabled");
        assert!(json.get("fields").is_none());
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn invitation_errors_name_the_invitation_field() {
        let err = ApiError::InvitationRequired;
        let json = serde_json::to_value(err.body()).unwrap();
        assert_eq!(json["error"], "invitation_required");
        assert_eq!(json["fields"]["invitation"], "required");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);

        let err = ApiError::InvitationInvalid;
        let json = serde_json::to_value(err.body()).unwrap();
        assert_eq!(json["error"], "invitation_invalid");
        assert_eq!(json["fields"]["invitation"], "invalid");
        assert_eq!(err.status(), StatusCode::GONE);
    }

    #[test]
    fn server_blocked_is_403_without_fields() {
        let err = ApiError::ServerBlocked;
        let json = serde_json::to_value(err.body()).unwrap();
        assert_eq!(json["error"], "server_blocked");
        assert!(json.get("fields").is_none());
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn join_errors_are_409_and_the_decline_carries_retry_after() {
        for (err, code) in [
            (ApiError::JoinRequestRequired, "join_request_required"),
            (
                ApiError::JoinRequestDeclined { retry_after: 3600 },
                "join_request_declined",
            ),
            (ApiError::JoinRequestsFull, "join_requests_full"),
            (ApiError::AlreadyMember, "already_member"),
        ] {
            let json = serde_json::to_value(err.body()).unwrap();
            assert_eq!(json["error"], code);
            assert!(json.get("fields").is_none());
            assert_eq!(err.status(), StatusCode::CONFLICT, "{code}");
        }
        let json = serde_json::to_value(ApiError::JoinRequestDeclined { retry_after: 3600 }.body())
            .unwrap();
        assert_eq!(json["retry_after"], 3600);
        let json = serde_json::to_value(ApiError::AlreadyMember.body()).unwrap();
        assert!(json.get("retry_after").is_none());
        // Not a rate limit: no Retry-After header.
        let response = ApiError::JoinRequestDeclined { retry_after: 60 }.into_response();
        assert!(response.headers().get(RETRY_AFTER).is_none());
    }
}
