//! `Path<Uuid>` whose rejection is a JSON `404 not_found`: an id that does
//! not even parse cannot name an existing row, and the web client treats it
//! like any other missing resource.

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use uuid::Uuid;

use crate::error::ApiError;

pub struct Id(pub Uuid);

impl<S: Send + Sync> FromRequestParts<S> for Id {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        match Path::<Uuid>::from_request_parts(parts, state).await {
            Ok(Path(id)) => Ok(Self(id)),
            Err(_) => Err(ApiError::NotFound),
        }
    }
}
