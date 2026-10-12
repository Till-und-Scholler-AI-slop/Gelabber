//! `Json<T>` whose rejection is the API's JSON error envelope instead of
//! axum's plain-text default.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

pub struct Body<T>(pub T);

impl<S, T> FromRequest<S> for Body<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(map_rejection(rejection)),
        }
    }
}

fn map_rejection(rejection: JsonRejection) -> ApiError {
    match rejection {
        // serde names the offending value ("invalid type: integer `1234`"),
        // which can be a password. The detail is only logged, never sent;
        // keep it free of request data.
        JsonRejection::JsonDataError(_) => {
            ApiError::BadRequest("JSON body does not match the expected shape".to_owned())
        }
        JsonRejection::JsonSyntaxError(err) => ApiError::BadRequest(err.body_text()),
        JsonRejection::MissingJsonContentType(err) => ApiError::BadRequest(err.body_text()),
        JsonRejection::BytesRejection(err) => ApiError::BadRequest(err.body_text()),
        other => ApiError::BadRequest(other.body_text()),
    }
}

/// Convenience so handlers can return `Body(value)` as a JSON response.
impl<T: serde::Serialize> IntoResponse for Body<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}
