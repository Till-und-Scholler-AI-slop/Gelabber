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

use std::collections::BTreeMap;

use axum::http::StatusCode;
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
    /// 409: e-mail already registered.
    EmailTaken,
    /// 500: anything unexpected. The string is logged, not returned.
    Internal(String),
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: FieldErrors,
}

impl ApiError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation_failed",
            Self::BadRequest(_) => "bad_request",
            Self::Unauthenticated => "unauthenticated",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Csrf => "csrf_invalid",
            Self::EmailTaken => "email_taken",
            Self::Internal(_) => "internal",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthenticated | Self::InvalidCredentials => StatusCode::UNAUTHORIZED,
            Self::Csrf => StatusCode::FORBIDDEN,
            Self::EmailTaken => StatusCode::CONFLICT,
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
            Self::EmailTaken => "This e-mail address is already registered.",
            Self::Internal(_) => "Something went wrong on our side.",
        }
    }

    pub fn body(&self) -> ErrorBody {
        let fields = match self {
            Self::Validation(fields) => fields.clone(),
            Self::EmailTaken => BTreeMap::from([("email", "taken")]),
            _ => BTreeMap::new(),
        };
        ErrorBody {
            error: self.code(),
            message: self.message(),
            fields,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match &self {
            Self::Internal(detail) => error!(error = %detail, "request failed"),
            Self::BadRequest(detail) => tracing::debug!(error = %detail, "bad request"),
            _ => {}
        }
        (self.status(), Json(self.body())).into_response()
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
}
