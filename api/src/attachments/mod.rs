//! Presign upload, metadata, and authenticated download (issue #7).
//!
//! The client asks for a presigned PUT, writes the bytes to MinIO itself,
//! then posts a message with `attachment_ids`. The API never accepts the
//! file body. Downloads go through this module so there is no public
//! object URL without a membership check.

pub mod validate;

use std::collections::HashMap;

use axum::Router;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::messages::{self, Message};
use crate::path::Id;
use crate::state::AppState;
use crate::storage::{ObjectBody, ObjectStore, PUT_TTL, StoreError};

use self::validate::is_image;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels/{id}/attachments", post(presign))
        .route("/api/attachments/{id}", get(download))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Attachment {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
}

#[derive(Debug, Clone, FromRow)]
struct AttachmentRow {
    id: Uuid,
    message_id: Option<Uuid>,
    filename: String,
    content_type: String,
    size_bytes: i64,
}

impl From<AttachmentRow> for Attachment {
    fn from(row: AttachmentRow) -> Self {
        Self {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size: row.size_bytes,
        }
    }
}

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct PendingRow {
    id: Uuid,
    channel_id: Uuid,
    uploader_id: Uuid,
    object_key: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
    message_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct StoredRow {
    id: Uuid,
    message_id: Option<Uuid>,
    channel_id: Uuid,
    uploader_id: Uuid,
    object_key: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
}

#[derive(Debug, Deserialize)]
pub struct PresignBody {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub size: i64,
}

#[derive(Debug, Serialize)]
pub struct PresignResponse {
    pub id: Uuid,
    pub upload_url: String,
    pub headers: HashMap<String, String>,
    pub expires_in: u64,
    pub attachment: Attachment,
}

async fn presign(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    Body(body): Body<PresignBody>,
) -> Result<(StatusCode, Json<PresignResponse>), ApiError> {
    let access = messages::messaging_channel(&state.db, channel_id, user.id).await?;
    access.require_send_files()?;

    let mut errors = FieldErrors::new();
    let filename = validate::filename(&body.filename, &mut errors);
    let content_type = validate::content_type(&body.content_type, &mut errors);
    let size = validate::size(body.size, &mut errors);
    validate::finish(errors)?;
    let filename = filename.expect("validated");
    let content_type = content_type.expect("validated");
    let size = size.expect("validated");

    if let Err(err) = state.store.ensure_ready().await {
        return Err(store_internal(err));
    }

    let id = Uuid::new_v4();
    let object_key = format!("att/{id}");
    let signed = state
        .store
        .presign_put(&object_key, &content_type, size)
        .map_err(store_internal)?;

    sqlx::query(
        "INSERT INTO attachments \
         (id, channel_id, uploader_id, object_key, filename, content_type, size_bytes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(channel_id)
    .bind(user.id)
    .bind(&object_key)
    .bind(&filename)
    .bind(&content_type)
    .bind(size)
    .execute(&state.db)
    .await?;

    info!(
        channel_id = %channel_id,
        attachment_id = %id,
        size,
        content_type = %content_type,
        "attachment presigned"
    );

    let headers = signed.headers.into_iter().collect();
    Ok((
        StatusCode::CREATED,
        Json(PresignResponse {
            id,
            upload_url: signed.url,
            headers,
            expires_in: PUT_TTL.as_secs(),
            attachment: Attachment {
                id,
                filename,
                content_type,
                size,
            },
        }),
    ))
}

async fn download(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(attachment_id): Id,
) -> Result<Response, ApiError> {
    let row = sqlx::query_as::<_, StoredRow>(
        "SELECT id, message_id, channel_id, uploader_id, object_key, \
                filename, content_type, size_bytes \
         FROM attachments WHERE id = $1",
    )
    .bind(attachment_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Pending (not yet on a message): only the uploader. Bound: any member
    // of the text channel. Foreign / unknown stays `404`.
    if row.message_id.is_none() && row.uploader_id != user.id {
        return Err(ApiError::NotFound);
    }
    messages::messaging_channel(&state.db, row.channel_id, user.id).await?;

    match state.store.get(&row.object_key).await {
        Ok(ObjectBody::Redirect(url)) => Ok(Redirect::temporary(&url).into_response()),
        Ok(ObjectBody::Bytes {
            content_type,
            bytes,
        }) => Ok(inline_bytes(&row.filename, &content_type, bytes)),
        Err(StoreError::NotFound) => Err(ApiError::NotFound),
        Err(err) => Err(store_internal(err)),
    }
}

fn inline_bytes(filename: &str, content_type: &str, bytes: Vec<u8>) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(content_type) {
        headers.insert(CONTENT_TYPE, value);
    }
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=60"),
    );
    let disposition = if is_image(content_type) {
        format!("inline; filename=\"{}\"", sanitize_disposition(filename))
    } else {
        format!(
            "attachment; filename=\"{}\"",
            sanitize_disposition(filename)
        )
    };
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    (headers, bytes).into_response()
}

fn sanitize_disposition(name: &str) -> String {
    name.chars()
        .map(|c| if c == '"' || c.is_control() { '_' } else { c })
        .collect()
}

/// Load attachments for a page of messages (one query).
pub async fn for_messages(db: &PgPool, messages: &mut [Message]) -> Result<(), ApiError> {
    let ids: Vec<Uuid> = messages.iter().map(|m| m.id).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, message_id, filename, content_type, size_bytes \
         FROM attachments WHERE message_id = ANY($1) \
         ORDER BY created_at, id",
    )
    .bind(&ids)
    .fetch_all(db)
    .await?;
    let mut by_message: HashMap<Uuid, Vec<Attachment>> = HashMap::new();
    for row in rows {
        if let Some(message_id) = row.message_id {
            by_message.entry(message_id).or_default().push(row.into());
        }
    }
    for message in messages {
        message.attachments = by_message.remove(&message.id).unwrap_or_default();
    }
    Ok(())
}

pub async fn for_message(db: &PgPool, message_id: Uuid) -> Result<Vec<Attachment>, ApiError> {
    let rows = sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, message_id, filename, content_type, size_bytes \
         FROM attachments WHERE message_id = $1 ORDER BY created_at, id",
    )
    .bind(message_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Attachment::from).collect())
}

/// Bind pending uploads to a new message. Each id must belong to this
/// channel and uploader, still be unbound, and the object must exist
/// in the store with the claimed size.
pub async fn bind_to_message(
    tx: &mut Transaction<'_, Postgres>,
    store: &ObjectStore,
    message_id: Uuid,
    channel_id: Uuid,
    uploader_id: Uuid,
    ids: &[Uuid],
) -> Result<Vec<Attachment>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut bound = Vec::with_capacity(ids.len());
    for id in ids {
        let row = sqlx::query_as::<_, PendingRow>(
            "SELECT id, channel_id, uploader_id, object_key, filename, \
                    content_type, size_bytes, message_id \
             FROM attachments WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| ApiError::Validation(FieldErrors::from([("attachment_ids", "invalid")])))?;
        if row.channel_id != channel_id
            || row.uploader_id != uploader_id
            || row.message_id.is_some()
        {
            return Err(ApiError::Validation(FieldErrors::from([(
                "attachment_ids",
                "invalid",
            )])));
        }
        let meta = match store.head(&row.object_key).await {
            Ok(meta) => meta,
            Err(StoreError::NotFound) => {
                return Err(ApiError::Validation(FieldErrors::from([(
                    "attachment_ids",
                    "invalid",
                )])));
            }
            Err(err) => return Err(store_internal(err)),
        };
        if meta.size != row.size_bytes {
            return Err(ApiError::Validation(FieldErrors::from([(
                "size", "invalid",
            )])));
        }
        let updated = sqlx::query_as::<_, AttachmentRow>(
            "UPDATE attachments SET message_id = $2 \
             WHERE id = $1 AND message_id IS NULL \
             RETURNING id, message_id, filename, content_type, size_bytes",
        )
        .bind(id)
        .bind(message_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| ApiError::Validation(FieldErrors::from([("attachment_ids", "invalid")])))?;
        bound.push(updated.into());
    }
    Ok(bound)
}

pub async fn drop_objects(store: &ObjectStore, message_id: Uuid, db: &PgPool) {
    let keys: Vec<String> =
        match sqlx::query_scalar("SELECT object_key FROM attachments WHERE message_id = $1")
            .bind(message_id)
            .fetch_all(db)
            .await
        {
            Ok(keys) => keys,
            Err(err) => {
                warn!(error = %err, %message_id, "could not list attachment keys");
                return;
            }
        };
    for key in keys {
        if let Err(err) = store.delete(&key).await {
            warn!(error = %err, %key, "attachment object delete failed");
        }
    }
}

fn store_internal(err: StoreError) -> ApiError {
    match err {
        StoreError::Unconfigured => ApiError::Internal("object store is not configured".into()),
        other => ApiError::Internal(format!("object store: {other}")),
    }
}
