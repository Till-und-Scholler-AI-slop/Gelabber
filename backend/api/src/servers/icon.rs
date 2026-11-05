//! Server icons: an uploaded image instead of the name's initials in the
//! server rail. Same storage rules as profile avatars (`crate::avatars`):
//! the browser sends a small, already scaled image, the API keeps it in
//! the object store under `server-icons/{server}/{file}` and serves the
//! bytes itself. Each upload gets a new file name, so a URL never changes
//! its content; a replaced or removed icon is deleted through
//! `storage_cleanup`, also when the server itself is deleted.
//!
//! Setting or removing the icon needs `manage_server`. Any signed-in
//! account may download one, because invite previews show it to people who
//! are not members yet; the URL holds two random UUIDs.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::HeaderValue;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, put};
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::avatars::{self, AVATAR_MAX_EDGE, MAX_BYTES, PENDING_SECS};
use crate::error::ApiError;
use crate::limits;
use crate::path::Id;
use crate::state::AppState;
use crate::storage::StoreError;

use super::membership::{self, Membership};
use super::permissions::Permission;
use super::{ServerRow, ServerView};

const URL_PREFIX: &str = "/api/server-icons/";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/servers/{id}/icon", put(upload).delete(remove))
        .route("/api/server-icons/{server}/{file}", get(download))
        .layer(DefaultBodyLimit::max(MAX_BYTES))
}

/// `/api/server-icons/{server}/{file}` with both parts as UUIDs, else `None`.
pub fn parse_url(url: &str) -> Option<(Uuid, Uuid)> {
    let (server, file) = url.strip_prefix(URL_PREFIX)?.split_once('/')?;
    Some((Uuid::parse_str(server).ok()?, Uuid::parse_str(file).ok()?))
}

fn url_for(server: Uuid, file: Uuid) -> String {
    format!("{URL_PREFIX}{server}/{file}")
}

fn object_key(server: Uuid, file: Uuid) -> String {
    format!("server-icons/{server}/{file}")
}

/// Records the deletion of `server_id`'s icon at `url` in the caller's
/// transaction and returns its key for an immediate delete after commit.
/// Another server's path is never touched.
pub async fn schedule_removal(
    tx: &mut sqlx::PgConnection,
    server_id: Uuid,
    url: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(key) = url
        .and_then(parse_url)
        .filter(|(owner, _)| *owner == server_id)
        .map(|(owner, file)| object_key(owner, file))
    else {
        return Ok(None);
    };
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, retain_until) VALUES ($1, now()) \
         ON CONFLICT (object_key) DO UPDATE SET next_attempt=now()",
    )
    .bind(&key)
    .execute(&mut *tx)
    .await?;
    Ok(Some(key))
}

async fn upload(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    body: Bytes,
) -> Result<Json<ServerView>, ApiError> {
    // Checked before the quota is charged; checked again under the lock.
    membership::load(&state.db, server_id, user.id)
        .await?
        .require(Permission::ManageServer)?;
    let content_type = avatars::sniff(&body)
        .ok_or_else(|| ApiError::BadRequest("image must be PNG, JPEG or WebP".into()))?;
    if !crate::image_size::fits(&body, AVATAR_MAX_EDGE, AVATAR_MAX_EDGE) {
        return Err(ApiError::BadRequest(format!(
            "image must be at most {AVATAR_MAX_EDGE}x{AVATAR_MAX_EDGE} pixels"
        )));
    }
    limits::consume_upload_quota(
        &state,
        &mut *state.db.acquire().await?,
        user.id,
        body.len() as i64,
    )
    .await?;
    let file = Uuid::new_v4();
    let key = object_key(server_id, file);
    // Durable intent first, as for avatars: an upload that never reaches
    // the server row is removed by the cleanup worker.
    sqlx::query(
        "INSERT INTO storage_cleanup (object_key, next_attempt, retain_until) \
         VALUES ($1, now() + $2 * interval '1 second', now() + $2 * interval '1 second')",
    )
    .bind(&key)
    .bind(PENDING_SECS)
    .execute(&state.db)
    .await?;
    state
        .store
        .put(&key, content_type, body.to_vec())
        .await
        .map_err(avatars::store_internal)?;
    Ok(Json(
        save(
            &state,
            server_id,
            user.id,
            Some((url_for(server_id, file), &key)),
        )
        .await?,
    ))
}

async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<ServerView>, ApiError> {
    Ok(Json(save(&state, server_id, user.id, None).await?))
}

/// Points the server at the new icon (`Some((url, key))`) or at none, and
/// deletes the icon this update really replaced, as read under the
/// server's exclusive lock. Rolled back, the new object is discarded; a
/// commit error is ambiguous and keeps it (its pending cleanup row is
/// still there if the commit did not happen).
async fn save(
    state: &AppState,
    server_id: Uuid,
    user_id: Uuid,
    icon: Option<(String, &str)>,
) -> Result<ServerView, ApiError> {
    let staged = stage(state, server_id, user_id, icon.as_ref()).await;
    let (tx, server, replaced) = match staged {
        Ok(staged) => staged,
        Err(err) => {
            if let Some((_, key)) = icon {
                avatars::discard(state, key).await;
            }
            return Err(err);
        }
    };
    tx.commit().await?;
    if let Some(key) = replaced {
        avatars::discard(state, &key).await;
    }
    Ok(Membership { server, user_id }.into())
}

type Staged<'a> = (
    sqlx::Transaction<'a, sqlx::Postgres>,
    ServerRow,
    Option<String>,
);

async fn stage<'a>(
    state: &'a AppState,
    server_id: Uuid,
    user_id: Uuid,
    icon: Option<&(String, &str)>,
) -> Result<Staged<'a>, ApiError> {
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user_id).await?;
    member.require(Permission::ManageServer)?;
    if let Some((_, key)) = icon {
        // `attempts = 0`: see `avatars::save_profile`.
        let kept = sqlx::query("DELETE FROM storage_cleanup WHERE object_key=$1 AND attempts=0")
            .bind(key)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if kept == 0 {
            return Err(ApiError::Internal("server icon upload expired".into()));
        }
    }
    let url = icon.as_ref().map(|(url, _)| url.as_str());
    let server = sqlx::query_as::<_, ServerRow>(
        "UPDATE servers SET icon_url = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, name, owner_id, member_permissions, icon_url, created_at",
    )
    .bind(server_id)
    .bind(url)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    let previous = member
        .server
        .icon_url
        .as_deref()
        .filter(|old| Some(*old) != url);
    let replaced = schedule_removal(&mut tx, server_id, previous).await?;
    Ok((tx, server, replaced))
}

async fn download(
    State(state): State<AppState>,
    _: CurrentUser,
    Path((server, file)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (Ok(server), Ok(file)) = (Uuid::parse_str(&server), Uuid::parse_str(&file)) else {
        return Err(ApiError::NotFound);
    };
    let (_, bytes) = match state.store.fetch(&object_key(server, file)).await {
        Ok(object) => object,
        Err(StoreError::NotFound) => return Err(ApiError::NotFound),
        Err(err) => return Err(avatars::store_internal(err)),
    };
    // Only what the bytes prove, never the stored header.
    let content_type = avatars::sniff(&bytes).ok_or(ApiError::NotFound)?;
    if !crate::image_size::fits(&bytes, AVATAR_MAX_EDGE, AVATAR_MAX_EDGE) {
        return Err(ApiError::NotFound);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_our_own_icon_paths() {
        let server = Uuid::new_v4();
        let file = Uuid::new_v4();
        assert_eq!(parse_url(&url_for(server, file)), Some((server, file)));
        assert_eq!(
            object_key(server, file),
            format!("server-icons/{server}/{file}")
        );
        assert_eq!(parse_url(&format!("/api/avatars/{server}/{file}")), None);
        assert_eq!(parse_url(&format!("{URL_PREFIX}{server}")), None);
        assert_eq!(parse_url(&format!("{URL_PREFIX}{server}/{file}/x")), None);
        assert_eq!(parse_url("https://example.com/api/server-icons/a/b"), None);
    }
}
