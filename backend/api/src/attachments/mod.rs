//! Presign upload, metadata, and authenticated download (issue #7).
//!
//! The client asks for a presigned PUT, writes the bytes to MinIO itself,
//! then posts a message with `attachment_ids`. Downloads go through this
//! module so there is no public object URL without a membership check.
//! With `ATTACHMENT_DELIVERY=proxy` (#348) the upload link points at this
//! module instead of MinIO, and downloads stream through it as well, so
//! browsers only ever talk to the app's own origin.

pub mod cleanup;
pub mod sniff;
pub mod validate;

use std::collections::HashMap;
use std::time::SystemTime;

use axum::Router;
use axum::body::Body as RawBody;
use axum::extract::{Query, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_SECURITY_POLICY, CONTENT_TYPE, RANGE, VARY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::csrf::CSRF_HEADER;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::limits;
use crate::messages::{self, Message};
use crate::path::Id;
use crate::state::AppState;
use crate::storage::{self, ObjectBody, ObjectStore, PUT_TTL, StoreError, StoredObject};

use self::validate::{is_inline, is_text};

/// How long an upload may wait for its message. Up to ten files go up one
/// after another, so the first one must outlive the whole set on a slow
/// line, not just its own ten-minute upload link (`PUT_TTL`). The link
/// expires first; expiry cleanup and its grace for unfinished uploads run
/// from this later deadline.
pub const PENDING_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels/{id}/attachments", post(presign))
        .route("/api/attachments/{id}", get(download))
        .route("/api/attachments/{id}/upload", put(upload))
}

/// How long a browser may keep an attachment it got through the API: the
/// same 60 minutes a signed MinIO link and its cached copy live
/// (`storage::GET_TTL`, accepted for access revocation).
const PROXIED_CACHE: HeaderValue = HeaderValue::from_static("private, max-age=3600");

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
    expired: bool,
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
    request_headers: HeaderMap,
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
    let mut signed = state
        .store
        .presign_put(&object_key, &content_type, size)
        .map_err(store_internal)?;
    if state.store.proxies() {
        // The upload goes to `upload` below. A mutation there needs the CSRF
        // header like any other; this request carried a valid one, and the
        // client sends every header listed here (also one that predates
        // proxy delivery).
        signed.url = format!("/api/attachments/{id}/upload");
        if let Some(token) = request_headers
            .get(&CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
        {
            signed
                .headers
                .push(("X-CSRF-Token".to_owned(), token.to_owned()));
        }
    }

    let mut tx = state.db.begin().await?;
    // Parent locks precede the ledger lock. A concurrent parent cascade must
    // not hold the channel while waiting for a quota row owned by this insert.
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id=$1 FOR KEY SHARE")
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::Unauthenticated)?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM channels WHERE id=$1 FOR KEY SHARE")
        .bind(channel_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?;
    let quota_day = limits::reserve_upload_quota(&state, &mut tx, user.id, size).await?;
    sqlx::query(
        "INSERT INTO attachments \
         (id, channel_id, uploader_id, object_key, filename, content_type, size_bytes, quota_day, \
          expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now() + make_interval(secs => $9))",
    )
    .bind(id)
    .bind(channel_id)
    .bind(user.id)
    .bind(&object_key)
    .bind(&filename)
    .bind(&content_type)
    .bind(size)
    .bind(quota_day)
    .bind(PENDING_TTL.as_secs_f64())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

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

#[derive(Debug, Default, Deserialize)]
struct DownloadParams {
    /// `?download=1`: the file to save, not to show (see `ObjectStore::download`).
    #[serde(default)]
    download: Option<String>,
}

async fn download(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(attachment_id): Id,
    Query(params): Query<DownloadParams>,
    request_headers: HeaderMap,
) -> Result<Response, ApiError> {
    let row = sqlx::query_as::<_, StoredRow>(
        "SELECT id, message_id, channel_id, uploader_id, object_key, \
                filename, content_type, size_bytes \
         FROM attachments WHERE id = $1 AND (message_id IS NOT NULL OR expires_at > now())",
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

    // Pictures and media play in the page; everything else is only ever a
    // download, whichever link asked for it.
    let save = params.download.as_deref() == Some("1") || !is_inline(&row.content_type);
    let disposition =
        content_disposition(if save { "attachment" } else { "inline" }, &row.filename);
    let served = served_type(&row.content_type);
    // One clock read: the link's signing window and the redirect's cache
    // time must come from the same window.
    let now = SystemTime::now();
    let range = request_headers
        .get(RANGE)
        .and_then(|value| value.to_str().ok());
    let body = state
        .store
        .serve(
            &row.object_key,
            served,
            save.then_some(disposition.as_str()),
            range,
            now,
        )
        .await;
    match body {
        Ok(ObjectBody::Redirect(url)) => {
            let mut response = Redirect::temporary(&url).into_response();
            let headers = response.headers_mut();
            // The answer belongs to this session: after a logout, or for
            // another account in the same browser, it is asked for anew.
            headers.insert(VARY, HeaderValue::from_static("Cookie"));
            if save {
                // A click on "Herunterladen" is rare; check membership anew.
                headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
            } else {
                // The link is the same for the rest of its signing window;
                // the browser may reuse this redirect until then.
                let (_, left) = storage::get_window(now);
                if let Ok(value) = HeaderValue::from_str(&format!("private, max-age={left}")) {
                    headers.insert(CACHE_CONTROL, value);
                }
            }
            Ok(response)
        }
        Ok(ObjectBody::Stream(object)) => Ok(streamed(&disposition, served, save, object)),
        Err(StoreError::NotFound) => Err(ApiError::NotFound),
        Err(StoreError::RangeNotSatisfiable) => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{}", row.size_bytes)) {
                response.headers_mut().insert(CONTENT_RANGE, value);
            }
            Ok(response)
        }
        Err(err) => Err(store_internal(err)),
    }
}

/// The bytes through the API (proxy delivery, memory store), with the
/// headers MinIO would have answered a signed link with, and `sandbox` on
/// top: the file now comes from the app's own origin, so even a document
/// opened on its own must never run as part of the app.
fn streamed(disposition: &str, content_type: &str, save: bool, object: StoredObject) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(content_type) {
        headers.insert(CONTENT_TYPE, value);
    }
    headers.insert(
        CACHE_CONTROL,
        if save {
            HeaderValue::from_static("private, no-store")
        } else {
            PROXIED_CACHE
        },
    );
    headers.insert(VARY, HeaderValue::from_static("Cookie"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    if let Ok(value) = HeaderValue::from_str(disposition) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    if object.ranges {
        headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Some(length) = object.content_length {
        headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    }
    let status = match object
        .content_range
        .as_deref()
        .and_then(|range| HeaderValue::from_str(range).ok())
    {
        Some(range) => {
            headers.insert(CONTENT_RANGE, range);
            StatusCode::PARTIAL_CONTENT
        }
        None => StatusCode::OK,
    };
    (status, headers, object.body).into_response()
}

/// Proxy delivery: the browser's PUT of a presigned upload (#348). The
/// same rules as MinIO's signed link: only the uploader, only before the
/// link expires and the file is bound, the signed type and exactly the
/// signed size. The upload may replace bytes put earlier, as a second PUT
/// to the link could; `check_uploads` reads them at bind time.
async fn upload(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(attachment_id): Id,
    request_headers: HeaderMap,
    body: RawBody,
) -> Result<StatusCode, ApiError> {
    if !state.store.proxies() {
        return Err(ApiError::NotFound);
    }
    let row = sqlx::query_as::<_, StoredRow>(
        "SELECT id, message_id, channel_id, uploader_id, object_key, \
                filename, content_type, size_bytes \
         FROM attachments \
         WHERE id = $1 AND uploader_id = $2 AND message_id IS NULL \
           AND expires_at > now() AND created_at > now() - make_interval(secs => $3)",
    )
    .bind(attachment_id)
    .bind(user.id)
    .bind(PUT_TTL.as_secs_f64())
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    messages::messaging_channel(&state.db, row.channel_id, user.id)
        .await?
        .require_send_files()?;

    let declared_type = request_headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !declared_type.trim().eq_ignore_ascii_case(&row.content_type) {
        return Err(ApiError::Validation(FieldErrors::from([(
            "content_type",
            "invalid",
        )])));
    }
    let declared_size = request_headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i64>().ok());
    if declared_size != Some(row.size_bytes) {
        return Err(ApiError::Validation(FieldErrors::from([(
            "size", "invalid",
        )])));
    }

    match state
        .store
        .upload(
            &row.object_key,
            &row.content_type,
            row.size_bytes as u64,
            body,
        )
        .await
    {
        Ok(()) => {}
        Err(StoreError::SizeMismatch) => {
            return Err(ApiError::Validation(FieldErrors::from([(
                "size", "invalid",
            )])));
        }
        Err(err) => return Err(store_internal(err)),
    }
    info!(
        channel_id = %row.channel_id,
        attachment_id = %row.id,
        size = row.size_bytes,
        "attachment uploaded through the api"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// The type a file is served with: the checked type from the database, and
/// any text as plain text, so a renamed `.html` can never render as a page.
/// No charset: text may be in any encoding (see `sniff`).
fn served_type(content_type: &str) -> &str {
    if is_text(content_type) {
        "text/plain"
    } else {
        content_type
    }
}

/// `Content-Disposition` with an ASCII `filename` every browser reads and,
/// for any other name, the exact one as RFC 6266 `filename*`. Quotes,
/// backslashes and control characters never reach the header.
fn content_disposition(kind: &str, filename: &str) -> String {
    let ascii: String = filename
        .chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() || !c.is_ascii() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut value = format!("{kind}; filename=\"{ascii}\"");
    if !filename.is_ascii() {
        value.push_str("; filename*=UTF-8''");
        for byte in filename.bytes() {
            if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
                value.push(byte as char);
            } else {
                value.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    value
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

pub async fn for_message(
    db: impl sqlx::PgExecutor<'_>,
    message_id: Uuid,
) -> Result<Vec<Attachment>, ApiError> {
    let rows = sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, message_id, filename, content_type, size_bytes \
         FROM attachments WHERE message_id = $1 ORDER BY created_at, id",
    )
    .bind(message_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Attachment::from).collect())
}

/// One field per position in `attachment_ids`, so the client can name the
/// file a check rejected (issue #255).
const AT: [&str; validate::ATTACHMENTS_MAX] = [
    "attachment_ids.0",
    "attachment_ids.1",
    "attachment_ids.2",
    "attachment_ids.3",
    "attachment_ids.4",
    "attachment_ids.5",
    "attachment_ids.6",
    "attachment_ids.7",
    "attachment_ids.8",
    "attachment_ids.9",
];

fn invalid() -> ApiError {
    ApiError::Validation(FieldErrors::from([("attachment_ids", "invalid")]))
}

/// The store side of binding, asked before the message transaction takes
/// the channel lock: each object exists with its declared size and its
/// first bytes are the declared type, and all of them fit one message.
/// All objects are asked at once; a slow store holds this request only,
/// never the channel. The database conditions are checked again under the
/// lock in `bind_to_message`.
///
/// The upload link stays valid until its expiry, so the uploader could
/// still replace the bytes with others of the same signed length after
/// this check. The served type (always the checked type, text as plain
/// text) and `nosniff` keep that harmless; this check catches mistakes.
pub async fn check_uploads(
    db: &PgPool,
    store: &ObjectStore,
    channel_id: Uuid,
    uploader_id: Uuid,
    ids: &[Uuid],
) -> Result<(), ApiError> {
    if ids.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, PendingRow>(
        "SELECT id, channel_id, uploader_id, object_key, filename, \
                content_type, size_bytes, message_id, expires_at <= now() AS expired \
         FROM attachments WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(db)
    .await?;
    let mut ordered = Vec::with_capacity(ids.len());
    let mut total: i64 = 0;
    for id in ids {
        let row = rows.iter().find(|row| row.id == *id).ok_or_else(invalid)?;
        if row.channel_id != channel_id
            || row.uploader_id != uploader_id
            || row.message_id.is_some()
            || row.expired
        {
            return Err(invalid());
        }
        total += row.size_bytes;
        ordered.push(row);
    }
    if total > validate::MESSAGE_SIZE_MAX {
        return Err(ApiError::Validation(FieldErrors::from([(
            "attachment_ids",
            "too_large",
        )])));
    }
    let checks = ordered.iter().enumerate().map(|(index, row)| async move {
        let meta = match store.head(&row.object_key).await {
            Ok(meta) => meta,
            Err(StoreError::NotFound) => return Err(invalid()),
            Err(err) => return Err(store_internal(err)),
        };
        if meta.size != row.size_bytes {
            // The caller quarantines objects of the wrong size.
            return Err(ApiError::Validation(FieldErrors::from([(
                "size", "invalid",
            )])));
        }
        // The declared type must be what the bytes are. A mismatch stays
        // unbound and goes with the expiry cleanup.
        let head = match store.prefix(&row.object_key, sniff::PREFIX_LEN).await {
            Ok(head) => head,
            Err(StoreError::NotFound) => return Err(invalid()),
            Err(err) => return Err(store_internal(err)),
        };
        if !sniff::matches(&row.content_type, &head) {
            return Err(ApiError::Validation(FieldErrors::from([(
                AT[index], "mismatch",
            )])));
        }
        Ok(())
    });
    // The first failing file in the order the client sent them.
    for result in futures_util::future::join_all(checks).await {
        result?;
    }
    Ok(())
}

/// Bind pending uploads to a new message, under the channel lock. Each id
/// must still belong to this channel and uploader, be unbound and not
/// expired. `check_uploads` has already asked the store.
pub async fn bind_to_message(
    tx: &mut Transaction<'_, Postgres>,
    message_id: Uuid,
    channel_id: Uuid,
    uploader_id: Uuid,
    ids: &[Uuid],
) -> Result<Vec<Attachment>, ApiError> {
    let mut bound = Vec::with_capacity(ids.len());
    for id in ids {
        let row = sqlx::query_as::<_, PendingRow>(
            "SELECT id, channel_id, uploader_id, object_key, filename, \
                    content_type, size_bytes, message_id, expires_at <= now() AS expired \
             FROM attachments WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(invalid)?;
        if row.channel_id != channel_id
            || row.uploader_id != uploader_id
            || row.message_id.is_some()
            || row.expired
        {
            return Err(invalid());
        }
        let updated = sqlx::query_as::<_, AttachmentRow>(
            "UPDATE attachments SET message_id = $2, quota_state='consumed' \
             WHERE id = $1 AND message_id IS NULL AND expires_at > clock_timestamp() \
             RETURNING id, message_id, filename, content_type, size_bytes",
        )
        .bind(id)
        .bind(message_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(invalid)?;
        bound.push(updated.into());
    }
    Ok(bound)
}

fn store_internal(err: StoreError) -> ApiError {
    match err {
        StoreError::Unconfigured => ApiError::Internal("object store is not configured".into()),
        other => ApiError::Internal(format!("object store: {other}")),
    }
}

#[cfg(test)]
mod disposition_tests {
    use super::content_disposition;

    #[test]
    fn ascii_names_stay_and_quotes_or_controls_never_reach_the_header() {
        assert_eq!(
            content_disposition("attachment", "pic.png"),
            "attachment; filename=\"pic.png\""
        );
        assert_eq!(
            content_disposition("inline", "a\"b\\c\r\nd.png"),
            "inline; filename=\"a_b_c__d.png\""
        );
    }

    #[test]
    fn other_names_carry_the_exact_name_encoded() {
        assert_eq!(
            content_disposition("attachment", "Bär 1.png"),
            "attachment; filename=\"B_r 1.png\"; filename*=UTF-8''B%C3%A4r%201.png"
        );
    }
}
