//! The feedback routes of signed-in people: the status the clients poll,
//! staging an image, creating an entry, the list of their own entries with
//! their threads, replying, marking replies read and loading the image.
//! Pending entries (public form, not confirmed) never belong to anyone here.

use std::collections::HashMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info;
use uuid::Uuid;

use super::{
    ContextBody, Enabled, Entry, EntryRow, IMAGE_MAX_BYTES, IMAGE_MAX_EDGE, MESSAGES_MAX, Message,
    MessageRow, enabled, entry_columns, image_key, message_columns, notify,
};
use crate::auth::session::CurrentUser;
use crate::auth::validate;
use crate::avatars::{self, PENDING_SECS, sniff};
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::limits::{self, FEEDBACK_PER_HOUR, FEEDBACK_REPLIES_PER_HOUR, HOUR};
use crate::path::Id;
use crate::state::AppState;
use crate::storage::StoreError;

/// Entries per page of `GET /api/feedback`.
pub const PAGE: i64 = 20;

pub fn router() -> Router<AppState> {
    let images = Router::new()
        .route("/api/feedback/images", post(upload_image))
        .layer(DefaultBodyLimit::max(IMAGE_MAX_BYTES));
    Router::new()
        .route("/api/feedback/status", get(status))
        .route("/api/feedback", get(list).post(create))
        .route("/api/feedback/{id}/messages", post(reply))
        .route("/api/feedback/{id}/read", post(read))
        .route("/api/feedback/{id}/image", get(image))
        .merge(images)
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub enabled: bool,
    /// The caller's entries with an operator reply they have not seen.
    pub unread: i64,
}

/// `GET /api/feedback/status`: whether feedback is on, and the unread
/// count of the dot. Answers while the feature is off.
async fn status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Status>, ApiError> {
    if !enabled(&state) {
        return Ok(Json(Status {
            enabled: false,
            unread: 0,
        }));
    }
    let unread: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM feedback WHERE user_id = $1 AND person_unread_since IS NOT NULL",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(Status {
        enabled: true,
        unread,
    }))
}

/// `429 rate_limited` for `bucket`, counted in the metrics.
fn limited(state: &AppState, bucket: &'static str, retry_after: u64) -> ApiError {
    state.metrics.rate_limited(bucket);
    ApiError::RateLimited { retry_after }
}

fn invalid_image() -> ApiError {
    let mut errors = FieldErrors::new();
    errors.insert("image", "invalid");
    ApiError::Validation(errors)
}

/// `POST /api/feedback/images`: stages one image for the entry about to be
/// created. The avatar flow with another key and no profile: type from the
/// bytes, declared size checked, quota charged, a pending cleanup row
/// before the PUT. Unclaimed after [`PENDING_SECS`], the worker deletes it.
async fn upload_image(
    _: Enabled,
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // An entry that the `feedback` bucket would refuse needs no image.
    if let Some(retry_after) =
        state
            .limiter
            .blocked("feedback", &user.id.to_string(), FEEDBACK_PER_HOUR)
    {
        return Err(limited(&state, "feedback", retry_after));
    }
    let content_type = sniff(&body)
        .filter(|_| crate::image_size::fits(&body, IMAGE_MAX_EDGE, IMAGE_MAX_EDGE))
        .ok_or_else(invalid_image)?;
    limits::consume_upload_quota(
        &state,
        &mut *state.db.acquire().await?,
        user.id,
        body.len() as i64,
    )
    .await?;
    let file = Uuid::new_v4();
    let key = image_key(user.id, file);
    // Durable intent first, as for avatars: an object whose entry never
    // comes is deleted by the cleanup worker.
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
    Ok((StatusCode::CREATED, Json(json!({ "image": file }))))
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    context: Option<ContextBody>,
}

/// The staged image's type and size, read from the object store; `None`
/// when there is no such object or it is no image of ours.
async fn staged_image(
    state: &AppState,
    key: &str,
) -> Result<Option<(&'static str, i32)>, ApiError> {
    let prefix = match state.store.prefix(key, 16).await {
        Ok(prefix) => prefix,
        Err(StoreError::NotFound) => return Ok(None),
        Err(err) => return Err(avatars::store_internal(err)),
    };
    let Some(content_type) = sniff(&prefix) else {
        return Ok(None);
    };
    let size = match state.store.head(key).await {
        Ok(meta) => meta.size,
        Err(StoreError::NotFound) => return Ok(None),
        Err(err) => return Err(avatars::store_internal(err)),
    };
    Ok(i32::try_from(size)
        .ok()
        .filter(|size| *size >= 1 && *size as usize <= IMAGE_MAX_BYTES)
        .map(|size| (content_type, size)))
}

/// `POST /api/feedback`: a new entry, with the image staged before.
async fn create(
    _: Enabled,
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<CreateBody>,
) -> Result<(StatusCode, Json<Entry>), ApiError> {
    let mut errors = FieldErrors::new();
    let kind = super::kind(body.kind.as_deref(), &mut errors);
    let text = super::text(body.text.as_deref(), &mut errors);
    let context = super::context(body.context, &mut errors);
    let file = match body.image.as_deref().map(Uuid::parse_str) {
        None => None,
        Some(Ok(file)) => Some(file),
        Some(Err(_)) => {
            errors.insert("image", "invalid");
            None
        }
    };
    validate::finish(errors)?;
    let (Some(kind), Some(text), Some(context)) = (kind, text, context) else {
        return Err(ApiError::Internal(
            "feedback: validated fields missing".into(),
        ));
    };
    // Someone else's file is no object under the caller's prefix.
    let image = match file {
        None => None,
        Some(file) => {
            let key = image_key(user.id, file);
            let (content_type, size) = staged_image(&state, &key)
                .await?
                .ok_or_else(invalid_image)?;
            Some((key, content_type, size))
        }
    };
    let bucket_id = user.id.to_string();
    state
        .limiter
        .hit("feedback", &bucket_id, FEEDBACK_PER_HOUR, HOUR)
        .map_err(|retry_after| limited(&state, "feedback", retry_after))?;
    let created = async {
        let mut tx = state.db.begin().await?;
        if let Some((key, _, _)) = &image {
            // Only a staging row the cleanup worker has not started on,
            // and not yet due: used, or expired, the image is gone.
            let claimed = sqlx::query(
                "DELETE FROM storage_cleanup \
                 WHERE object_key = $1 AND attempts = 0 AND next_attempt > now()",
            )
            .bind(key)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if claimed == 0 {
                return Err(invalid_image());
            }
        }
        let row: EntryRow = sqlx::query_as(concat!(
            "INSERT INTO feedback (kind, user_id, client, os, app_version, locale, page, body, \
             image_key, image_type, image_bytes) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING ",
            entry_columns!()
        ))
        .bind(kind.as_str())
        .bind(user.id)
        .bind(&context.client)
        .bind(&context.os)
        .bind(&context.app_version)
        .bind(&context.locale)
        .bind(&context.page)
        .bind(&text)
        .bind(image.as_ref().map(|(key, _, _)| key))
        .bind(image.as_ref().map(|(_, content_type, _)| *content_type))
        .bind(image.as_ref().map(|(_, _, size)| *size))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }
    .await;
    let row = match created {
        Ok(row) => row,
        Err(err) => {
            // Nothing was stored: the attempt costs no entry.
            state.limiter.refund("feedback", &bucket_id);
            return Err(err);
        }
    };
    notify::new_entry(&state, row.id);
    info!(feedback_id = %row.id, kind = kind.as_str(), "feedback received");
    Ok((StatusCode::CREATED, Json(row.for_person(Vec::new()))))
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    // A string, so that a bad value is a field error, not axum's plain 400.
    offset: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EntryPage {
    pub entries: Vec<Entry>,
    pub total: i64,
}

/// `GET /api/feedback?offset=`: the caller's entries with their threads,
/// latest activity first, [`PAGE`] at a time.
async fn list(
    _: Enabled,
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<ListParams>,
) -> Result<Json<EntryPage>, ApiError> {
    let mut errors = FieldErrors::new();
    let offset = crate::operator::accounts::offset(params.offset.as_deref(), &mut errors);
    validate::finish(errors)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM feedback WHERE user_id = $1 AND status <> 'pending'",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    let rows: Vec<EntryRow> = sqlx::query_as(concat!(
        "SELECT ",
        entry_columns!(),
        " FROM feedback WHERE user_id = $1 AND status <> 'pending' \
         ORDER BY last_activity_at DESC, id DESC LIMIT $2 OFFSET $3"
    ))
    .bind(user.id)
    .bind(PAGE)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let mut threads: HashMap<Uuid, Vec<MessageRow>> = HashMap::new();
    if !ids.is_empty() {
        let messages: Vec<MessageRow> = sqlx::query_as(concat!(
            "SELECT ",
            message_columns!(),
            " FROM feedback_messages WHERE feedback_id = ANY($1) ORDER BY created_at, id"
        ))
        .bind(&ids)
        .fetch_all(&state.db)
        .await?;
        for message in messages {
            threads
                .entry(message.feedback_id)
                .or_default()
                .push(message);
        }
    }
    let entries = rows
        .into_iter()
        .map(|row| {
            let messages = threads.remove(&row.id).unwrap_or_default();
            row.for_person(messages)
        })
        .collect();
    Ok(Json(EntryPage { entries, total }))
}

#[derive(Debug, Deserialize)]
pub struct ReplyBody {
    #[serde(default)]
    text: Option<String>,
}

/// `POST /api/feedback/{id}/messages`: the sender answers in their thread.
/// A reply on a done entry opens it again.
async fn reply(
    _: Enabled,
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(id): Id,
    Body(body): Body<ReplyBody>,
) -> Result<(StatusCode, Json<Message>), ApiError> {
    let mut errors = FieldErrors::new();
    let text = super::text(body.text.as_deref(), &mut errors);
    validate::finish(errors)?;
    let Some(text) = text else {
        return Err(ApiError::Internal(
            "feedback: validated text missing".into(),
        ));
    };
    let mut tx = state.db.begin().await?;
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM feedback WHERE id = $1 AND user_id = $2 AND status <> 'pending' \
         FOR UPDATE",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    if found.is_none() {
        return Err(ApiError::NotFound);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM feedback_messages WHERE feedback_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if count >= MESSAGES_MAX {
        let mut errors = FieldErrors::new();
        errors.insert("text", "too_many");
        return Err(ApiError::Validation(errors));
    }
    state
        .limiter
        .hit(
            "feedback_reply",
            &user.id.to_string(),
            FEEDBACK_REPLIES_PER_HOUR,
            HOUR,
        )
        .map_err(|retry_after| limited(&state, "feedback_reply", retry_after))?;
    let message: MessageRow = sqlx::query_as(concat!(
        "INSERT INTO feedback_messages (feedback_id, author_kind, author_id, body) \
         VALUES ($1, 'person', $2, $3) RETURNING ",
        message_columns!()
    ))
    .bind(id)
    .bind(user.id)
    .bind(&text)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE feedback SET last_activity_at = now(), status = 'open', done_at = NULL, \
         done_by = NULL WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    notify::person_reply(&state, id, message.body.clone());
    Ok((StatusCode::CREATED, Json(message.for_person(None))))
}

#[derive(Debug, Default, Deserialize)]
pub struct ReadBody {
    /// The newest message of the entry the client showed. Operator replies
    /// after it stay unread. A string, so that a bad value is a field error.
    #[serde(default)]
    through: Option<String>,
}

/// `POST /api/feedback/{id}/read`: the person has seen the replies up to
/// `through`, or all of them without a body (older clients). A reply that
/// arrived after the client loaded the list keeps the entry unread, so its
/// "Neu" mark and the dot survive the read call that raced it.
async fn read(
    _: Enabled,
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(id): Id,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let body: ReadBody = if body.is_empty() {
        ReadBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| {
            ApiError::BadRequest("JSON body does not match the expected shape".to_owned())
        })?
    };
    let through = match body.through.as_deref().map(Uuid::parse_str) {
        None => None,
        Some(Ok(through)) => Some(through),
        Some(Err(_)) => {
            let mut errors = FieldErrors::new();
            errors.insert("through", "invalid");
            return Err(ApiError::Validation(errors));
        }
    };
    let mut tx = state.db.begin().await?;
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM feedback WHERE id = $1 AND user_id = $2 AND status <> 'pending' \
         FOR UPDATE",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    if found.is_none() {
        return Err(ApiError::NotFound);
    }
    let seen: Option<DateTime<Utc>> = match through {
        None => None,
        Some(through) => Some(
            sqlx::query_scalar(
                "SELECT created_at FROM feedback_messages WHERE id = $1 AND feedback_id = $2",
            )
            .bind(through)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                let mut errors = FieldErrors::new();
                errors.insert("through", "invalid");
                ApiError::Validation(errors)
            })?,
        ),
    };
    // The mark moves to the first operator reply after what was seen, or
    // goes. It never comes back once cleared, so a stale tab cannot mark
    // replies unread again that another tab already read.
    sqlx::query(
        "UPDATE feedback SET person_unread_since = CASE \
             WHEN $2::timestamptz IS NULL OR person_unread_since IS NULL THEN NULL \
             ELSE (SELECT min(m.created_at) FROM feedback_messages m \
                   WHERE m.feedback_id = $1 AND m.author_kind = 'operator' \
                     AND m.created_at > $2 AND m.created_at >= person_unread_since) \
         END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(seen)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/feedback/{id}/image`: the image of the caller's own entry.
async fn image(
    _: Enabled,
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(id): Id,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (key, content_type, size): (String, String, i32) = sqlx::query_as(
        "SELECT image_key, image_type, image_bytes FROM feedback \
         WHERE id = $1 AND user_id = $2 AND status <> 'pending' AND image_key IS NOT NULL",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    super::serve_image(&state, &key, &content_type, size, &headers).await
}
