//! Uploaded avatars (#208). The browser sends a small square image; the API
//! keeps it in the object store and serves the bytes itself, so viewers
//! never contact a foreign host and MinIO stays off the avatar path. The
//! profile stores `/api/avatars/{user}/{file}`: no schema change, and each
//! upload gets a new file name, so a URL never changes its content.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::HeaderValue;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, put};
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::auth::user::{self, ProfilePatch, User};
use crate::error::ApiError;
use crate::state::AppState;
use crate::storage::StoreError;

/// The web client sends 256×256 WebP or PNG, far below this.
pub const MAX_BYTES: usize = 512 * 1024;
const URL_PREFIX: &str = "/api/avatars/";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me/avatar", put(upload))
        .route("/api/avatars/{user}/{file}", get(download))
        .layer(DefaultBodyLimit::max(MAX_BYTES))
}

/// `/api/avatars/{user}/{file}` with both parts as UUIDs, else `None`.
pub fn parse_url(url: &str) -> Option<(Uuid, Uuid)> {
    let (user, file) = url.strip_prefix(URL_PREFIX)?.split_once('/')?;
    Some((Uuid::parse_str(user).ok()?, Uuid::parse_str(file).ok()?))
}

fn url_for(user: Uuid, file: Uuid) -> String {
    format!("{URL_PREFIX}{user}/{file}")
}

fn object_key(user: Uuid, file: Uuid) -> String {
    format!("avatars/{user}/{file}")
}

/// The type comes from the bytes, never from the request header. No SVG:
/// it can carry script.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

async fn upload(
    State(state): State<AppState>,
    CurrentUser(current): CurrentUser,
    body: Bytes,
) -> Result<Json<User>, ApiError> {
    let content_type = sniff(&body)
        .ok_or_else(|| ApiError::BadRequest("avatar must be a PNG, JPEG or WebP image".into()))?;
    let file = Uuid::new_v4();
    let key = object_key(current.id, file);
    state
        .store
        .put(&key, content_type, body.to_vec())
        .await
        .map_err(store_internal)?;
    let patch = ProfilePatch {
        name: None,
        avatar_url: Some(Some(url_for(current.id, file))),
    };
    let Some(updated) = user::update_profile(&state.db, current.id, &patch).await? else {
        discard(&state, &key).await;
        return Err(ApiError::Unauthenticated);
    };
    discard_replaced(&state, &current, &updated).await;
    Ok(Json(updated))
}

/// Deletes the previous upload once the profile no longer points at it.
/// Another user's path set through `PATCH /api/me` is never touched.
pub async fn discard_replaced(state: &AppState, before: &User, after: &User) {
    let Some((owner, file)) = before.avatar_url.as_deref().and_then(parse_url) else {
        return;
    };
    if owner == before.id && before.avatar_url != after.avatar_url {
        discard(state, &object_key(owner, file)).await;
    }
}

async fn discard(state: &AppState, key: &str) {
    if let Err(err) = state.store.delete(key).await {
        tracing::warn!(error = %err, key, "avatar object not deleted");
    }
}

async fn download(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
    Path((user, file)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (Ok(user), Ok(file)) = (Uuid::parse_str(&user), Uuid::parse_str(&file)) else {
        return Err(ApiError::NotFound);
    };
    let (_, bytes) = match state.store.fetch(&object_key(user, file)).await {
        Ok(object) => object,
        Err(StoreError::NotFound) => return Err(ApiError::NotFound),
        Err(err) => return Err(store_internal(err)),
    };
    // Served under our own origin: only what the bytes prove, never the
    // stored header, and no sniffing by the browser.
    let content_type = sniff(&bytes).ok_or(ApiError::NotFound)?;
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            ),
            (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        ],
        bytes,
    )
        .into_response())
}

fn store_internal(err: StoreError) -> ApiError {
    match err {
        StoreError::Unconfigured => ApiError::Internal("object store is not configured".into()),
        other => ApiError::Internal(format!("object store: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_our_own_avatar_paths() {
        let user = Uuid::new_v4();
        let file = Uuid::new_v4();
        assert_eq!(parse_url(&url_for(user, file)), Some((user, file)));
        assert_eq!(parse_url("https://example.com/api/avatars/a/b"), None);
        assert_eq!(parse_url(&format!("/api/avatars/{user}")), None);
        assert_eq!(parse_url(&format!("/api/avatars/{user}/{file}/x")), None);
        assert_eq!(parse_url(&format!("/api/avatars/{user}/../{file}")), None);
    }

    #[test]
    fn sniffs_raster_images_only() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(sniff(b""), None);
    }
}
