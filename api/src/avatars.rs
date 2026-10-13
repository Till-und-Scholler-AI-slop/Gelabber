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

/// The web client sends 256×256 WebP, JPEG or PNG, far below this.
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
    // Durable intent first: if the request dies after the PUT (client
    // abort, crash, failed update), the cleanup worker removes the object.
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, next_attempt, retain_until) \
         VALUES ($1, now() + $2 * interval '1 second', now() + $2 * interval '1 second')",
    )
    .bind(&key)
    .bind(PENDING_SECS)
    .execute(&state.db)
    .await?;
    // A failed PUT may still land (a client-side timeout does not stop
    // MinIO), so its pending cleanup row stays: the worker deletes the key
    // once it is due. No immediate delete, which would withdraw the row.
    state
        .store
        .put(&key, content_type, body.to_vec())
        .await
        .map_err(store_internal)?;
    let patch = ProfilePatch {
        name: None,
        avatar_url: Some(Some(url_for(current.id, file))),
    };
    match save_profile(&state, current.id, &patch, Some(&key)).await? {
        Some(updated) => Ok(Json(updated)),
        None => Err(ApiError::Unauthenticated),
    }
}

/// How long an upload may take from its PUT to the profile update before
/// the cleanup worker treats it as abandoned.
const PENDING_SECS: f64 = 600.0;

/// Updates the profile and deletes the upload that this update really
/// replaced (as returned by the locked `UPDATE`, not as read at request
/// start), so overlapping writes never delete the object that stays.
/// `uploaded` is the new object's key: its pending cleanup is withdrawn in
/// the same transaction that points the profile at it. Deletion of the
/// replaced object is recorded in `storage_cleanup` before commit, so a
/// failed or interrupted delete is retried by the cleanup worker.
/// Another user's path is never touched.
pub async fn save_profile(
    state: &AppState,
    user_id: Uuid,
    patch: &ProfilePatch,
    uploaded: Option<&str>,
) -> Result<Option<User>, ApiError> {
    let staged = async {
        let mut tx = state.db.begin().await?;
        if let Some(key) = uploaded {
            // `attempts = 0`: the worker has not started deleting it. The
            // claim statement in `attachments::cleanup::delete_pending`
            // increments `attempts` and nothing ever resets it, so a row
            // the worker has claimed once can never be withdrawn here.
            let kept =
                sqlx::query("DELETE FROM storage_cleanup WHERE object_key=$1 AND attempts=0")
                    .bind(key)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            if kept == 0 {
                return Err(ApiError::Internal("avatar upload expired".into()));
            }
        }
        let Some(row) = user::update_profile(&mut *tx, user_id, patch).await? else {
            return Ok(None);
        };
        let replaced = row
            .previous_avatar_url
            .filter(|previous| row.user.avatar_url.as_deref() != Some(previous.as_str()))
            .and_then(|previous| parse_url(&previous))
            .filter(|(owner, _)| *owner == user_id)
            .map(|(owner, file)| object_key(owner, file));
        if let Some(key) = &replaced {
            sqlx::query(
                "INSERT INTO storage_cleanup (object_key, retain_until) VALUES ($1, now()) \
                 ON CONFLICT (object_key) DO UPDATE SET next_attempt=now()",
            )
            .bind(key)
            .execute(&mut *tx)
            .await?;
        }
        Ok(Some((tx, row.user, replaced)))
    }
    .await;
    let (tx, updated, replaced) = match staged {
        Ok(Some(staged)) => staged,
        // Rolled back: the profile does not point at the new object.
        Ok(None) | Err(_) => {
            if let Some(key) = uploaded {
                discard(state, key).await;
            }
            return staged.map(|_| None);
        }
    };
    // A commit error is ambiguous: keep the new object; if the commit did
    // not happen its pending cleanup is still there.
    tx.commit().await?;
    if let Some(key) = replaced {
        discard(state, &key).await;
    }
    Ok(Some(updated))
}

/// Best-effort immediate delete; the `storage_cleanup` row recorded
/// before stays until the object is gone, so the worker retries failures.
async fn discard(state: &AppState, key: &str) {
    match state.store.delete(key).await {
        Ok(()) => {
            if let Err(err) =
                sqlx::query("DELETE FROM storage_cleanup WHERE object_key=$1 AND claim IS NULL")
                    .bind(key)
                    .execute(&state.db)
                    .await
            {
                tracing::warn!(error = %err, key, "avatar cleanup row not removed");
            }
        }
        Err(err) => tracing::warn!(error = %err, key, "avatar object not deleted; worker retries"),
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
