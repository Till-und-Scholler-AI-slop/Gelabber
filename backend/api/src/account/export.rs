//! Data export (#519): the worker that builds an account's archive.
//!
//! `POST /api/me/export` queues a row in `account_exports` and wakes the
//! worker; it also runs every 5 s. Each pass first deletes archives past
//! their `expires_at` (the `account_export_cleanup` trigger queues the
//! object for `attachments::cleanup`), then builds queued exports one at a
//! time:
//!
//! 1. **Claim** the oldest `pending` row, or a `running` one whose claim ran
//!    out, of an account not under deletion, for 30 minutes. A row claimed
//!    more than [`ATTEMPTS`] times is `failed`.
//! 2. **Build** the ZIP into a temporary file in `std::env::temp_dir()`. The
//!    entries are written in a blocking task that reads the database in
//!    pages of 500 rows and object bytes through `ObjectStore::read`, never
//!    holding a connection across an object store request. The file is
//!    deleted whatever happens.
//! 3. **Upload** it to `exports/{user}/{uuid}.zip`, streamed from the file.
//! 4. **Finish:** mark it `ready` until now + `API_EXPORT_RETENTION_HOURS`,
//!    but only while the claim is still this one; otherwise the object is
//!    deleted at once. With account mail on, [`Mail::ExportReady`] goes to
//!    the account.
//!
//! A build that fails is tried again a minute later, and is `failed` after
//! its third attempt.
//!
//! The archive holds only what the person wrote or owns (layout in
//! `docs/native/protocol-rest-auth.md`): `README.txt`, `account.json`,
//! `profile/` images, their own messages per channel under `messages/`,
//! `reactions.json` and the files they sent under `files/`. Push endpoints,
//! the password hash, session tokens and other people's messages are never
//! part of it.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::body::{Body, Bytes};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use tokio::io::AsyncReadExt;
use tokio::runtime::Handle;
use tracing::{info, warn};
use uuid::Uuid;
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::avatars::ImageKind;
use crate::error::ApiError;
use crate::i18n::{self, Locale};
use crate::mail::Mail;
use crate::state::AppState;
use crate::storage::{ObjectStore, StoreError};

/// How often the worker looks for work without being woken.
const TICK: Duration = Duration::from_secs(5);
/// Rows per database page.
const PAGE: i64 = 500;
/// Claims an export gets before it is `failed`.
pub const ATTEMPTS: i32 = 3;
/// How long the upload of one archive may take.
const UPLOAD_DEADLINE: Duration = Duration::from_secs(20 * 60);
/// Bytes read from the archive file per chunk of the upload.
const CHUNK: usize = 64 * 1024;

/// One account's export as its owner sees it (`GET /api/me/export`).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ExportView {
    pub state: String,
    pub requested_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub size_bytes: Option<i64>,
}

/// Starts the export worker once per process (see the module comment).
pub fn start(state: AppState) {
    if state.export_worker_started.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        while !state.db.is_closed() {
            if let Err(err) = sweep(&state).await {
                warn!(error = ?err, "export retention sweep failed");
            }
            // One export at a time; after one, look for the next at once.
            let busy = match run_once(&state).await {
                Ok(worked) => worked,
                Err(err) => {
                    warn!(error = ?err, "export pass failed");
                    false
                }
            };
            if !busy {
                let wake = state.account_export.notified();
                let _ = tokio::time::timeout(TICK, wake).await;
            }
        }
    });
}

/// Tells this process's export worker that an export was requested.
pub fn wake(state: &AppState) {
    state.account_export.notify_one();
}

/// Deletes the exports past their retention; the trigger queues each
/// archive for deletion. Returns how many went.
pub async fn sweep(state: &AppState) -> Result<u64, ApiError> {
    let removed =
        sqlx::query("DELETE FROM account_exports WHERE state = 'ready' AND expires_at < now()")
            .execute(&state.db)
            .await?
            .rows_affected();
    if removed > 0 {
        info!(removed, "expired data exports removed");
    }
    Ok(removed)
}

/// Claims and builds one queued export. `false` when there was none.
pub async fn run_once(state: &AppState) -> Result<bool, ApiError> {
    let claim = Uuid::new_v4();
    let claimed: Option<(Uuid, i32)> = sqlx::query_as(
        "UPDATE account_exports SET state = 'running', claim = $1, \
                claimed_until = now() + interval '30 minutes', attempts = attempts + 1 \
         WHERE user_id = (SELECT e.user_id FROM account_exports e JOIN users u ON u.id = e.user_id \
                          WHERE u.deleted_at IS NULL \
                            AND (e.state = 'pending' \
                                 OR (e.state = 'running' AND e.claimed_until < now())) \
                          ORDER BY e.requested_at FOR UPDATE OF e SKIP LOCKED LIMIT 1) \
         RETURNING user_id, attempts",
    )
    .bind(claim)
    .fetch_optional(&state.db)
    .await?;
    let Some((user_id, attempts)) = claimed else {
        return Ok(false);
    };
    if attempts > ATTEMPTS {
        give_up(&state.db, user_id, claim).await?;
        warn!(%user_id, attempts, "data export failed");
        return Ok(true);
    }

    let archive = TempFile::new();
    let built = build(state, user_id, archive.path().to_owned()).await;
    let result = match built {
        Ok(size) => publish(state, user_id, claim, &archive, size).await,
        Err(err) => Err(err),
    };
    drop(archive);
    if let Err(err) = result {
        warn!(error = ?err, %user_id, attempts, "data export attempt failed");
        if attempts >= ATTEMPTS {
            give_up(&state.db, user_id, claim).await?;
        } else {
            // Tried again once the short claim below runs out.
            sqlx::query(
                "UPDATE account_exports SET claim = NULL, \
                        claimed_until = now() + interval '1 minute' \
                 WHERE user_id = $1 AND claim = $2",
            )
            .bind(user_id)
            .bind(claim)
            .execute(&state.db)
            .await?;
        }
    }
    Ok(true)
}

async fn give_up(db: &PgPool, user_id: Uuid, claim: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE account_exports SET state = 'failed', claim = NULL, claimed_until = NULL, \
                finished_at = now() \
         WHERE user_id = $1 AND claim = $2",
    )
    .bind(user_id)
    .bind(claim)
    .execute(db)
    .await?;
    Ok(())
}

/// Uploads the archive and marks the export ready, or deletes the upload
/// again when the claim was lost meanwhile (a new request, the account's
/// deletion, an expired claim).
async fn publish(
    state: &AppState,
    user_id: Uuid,
    claim: Uuid,
    archive: &TempFile,
    size: u64,
) -> Result<(), ApiError> {
    let key = format!("exports/{user_id}/{}.zip", Uuid::new_v4());
    let file = tokio::fs::File::open(archive.path())
        .await
        .map_err(|err| internal("open archive", err))?;
    let body = Body::from_stream(futures_util::stream::unfold(file, |mut file| async move {
        let mut chunk = vec![0; CHUNK];
        match file.read(&mut chunk).await {
            Ok(0) => None,
            Ok(read) => {
                chunk.truncate(read);
                Some((Ok::<_, std::io::Error>(Bytes::from(chunk)), file))
            }
            Err(err) => Some((Err(err), file)),
        }
    }));
    state
        .store
        .upload(&key, "application/zip", size, body, UPLOAD_DEADLINE)
        .await
        .map_err(|err| internal("upload archive", err))?;

    let retention = state.export_retention.as_secs_f64();
    let finished: Option<(DateTime<Utc>, String, Locale)> = sqlx::query_as(
        "UPDATE account_exports e SET state = 'ready', object_key = $3, size_bytes = $4, \
                finished_at = now(), expires_at = now() + make_interval(secs => $5), \
                claim = NULL, claimed_until = NULL \
         FROM users u \
         WHERE e.user_id = $1 AND e.claim = $2 AND u.id = e.user_id AND u.deleted_at IS NULL \
         RETURNING e.expires_at, u.email, u.locale",
    )
    .bind(user_id)
    .bind(claim)
    .bind(&key)
    .bind(size as i64)
    .bind(retention)
    .fetch_optional(&state.db)
    .await?;
    let Some((expires_at, email, locale)) = finished else {
        if let Err(err) = state.store.delete(&key).await {
            warn!(error = %err, %user_id, "orphaned export archive not deleted");
        }
        info!(%user_id, "data export no longer wanted, archive deleted");
        return Ok(());
    };
    info!(%user_id, size, "data export ready");

    if let Some(mailer) = state.mail.clone() {
        tokio::spawn(async move {
            let mail = Mail::ExportReady { expires_at };
            match mailer.send(&email, &mail, locale).await {
                Ok(()) => info!(%user_id, kind = mail.kind(), "account mail sent"),
                Err(err) => {
                    warn!(%user_id, kind = mail.kind(), error = %err, "account mail failed")
                }
            }
        });
    }
    Ok(())
}

fn internal(what: &str, err: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(format!("data export: {what}: {err}"))
}

/// A file in the temporary directory that is deleted when this goes.
struct TempFile(PathBuf);

impl TempFile {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("gelabber-export-{}.zip", Uuid::new_v4())))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.0) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => warn!(error = %err, path = %self.0.display(), "export temp file left"),
        }
    }
}

/// Writes the archive of `user_id` to `path` and returns its size.
async fn build(state: &AppState, user_id: Uuid, path: PathBuf) -> Result<u64, ApiError> {
    let db = state.db.clone();
    let store = state.store.clone();
    let handle = Handle::current();
    tokio::task::spawn_blocking(move || {
        let file = File::create(&path).map_err(|err| internal("create archive", err))?;
        let mut archive = Archive {
            zip: zip::ZipWriter::new(file),
            db,
            store,
            handle,
            user_id,
        };
        archive.write_all()?;
        let file = archive
            .zip
            .finish()
            .map_err(|err| internal("finish archive", err))?;
        file.sync_all()
            .map_err(|err| internal("sync archive", err))?;
        let size = file
            .metadata()
            .map_err(|err| internal("archive size", err))?
            .len();
        Ok(size)
    })
    .await?
}

/// The archive being written, on a blocking thread: database pages and
/// object bytes are awaited through `handle`.
struct Archive {
    zip: zip::ZipWriter<File>,
    db: PgPool,
    store: ObjectStore,
    handle: Handle,
    user_id: Uuid,
}

#[derive(FromRow)]
struct AccountRow {
    id: Uuid,
    email: String,
    name: String,
    created_at: DateTime<Utc>,
    email_verified_at: Option<DateTime<Utc>>,
    bio: Option<String>,
    pronouns: Option<String>,
    status_text: Option<String>,
    accent_color: Option<String>,
    banner_color: Option<String>,
    avatar_decoration: Option<String>,
    notifications_hide_content: bool,
    locale: Locale,
    avatar_url: Option<String>,
    banner_url: Option<String>,
}

#[derive(Serialize, FromRow)]
struct NotificationSetting {
    scope_kind: String,
    scope_id: Uuid,
    level: Option<String>,
    muted: bool,
    muted_until: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct SessionRow {
    client: String,
    os: String,
    created_at: DateTime<Utc>,
    last_used_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct ServerRow {
    id: Uuid,
    name: String,
    owner: bool,
    joined_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct InviteRow {
    code: String,
    server_id: Uuid,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    max_uses: Option<i32>,
    uses: i32,
}

#[derive(FromRow)]
struct ChannelRow {
    id: Uuid,
    name: String,
    kind: String,
    server_id: Option<Uuid>,
    server_name: Option<String>,
    peer_id: Option<Uuid>,
    peer_name: Option<String>,
}

#[derive(FromRow)]
struct MessageRow {
    id: Uuid,
    created_at: DateTime<Utc>,
    edited_at: Option<DateTime<Utc>>,
    content: String,
    reply_to: Option<Uuid>,
    gif: Option<sqlx::types::Json<Value>>,
    system_kind: Option<String>,
    system_params: Option<sqlx::types::Json<Value>>,
}

#[derive(FromRow)]
struct AttachmentRow {
    id: Uuid,
    message_id: Uuid,
    object_key: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
}

#[derive(Serialize, FromRow)]
struct ReactionRow {
    message_id: Uuid,
    channel_id: Uuid,
    emoji: String,
}

impl Archive {
    fn wait<T>(&self, future: impl Future<Output = Result<T, sqlx::Error>>) -> Result<T, ApiError> {
        self.handle.block_on(future).map_err(ApiError::from)
    }

    fn start(&mut self, name: &str, size: u64) -> Result<(), ApiError> {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(size >= u64::from(u32::MAX));
        self.zip
            .start_file(name, options)
            .map_err(|err| internal("start entry", err))
    }

    fn bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), ApiError> {
        self.start(name, bytes.len() as u64)?;
        self.zip
            .write_all(bytes)
            .map_err(|err| internal("write entry", err))
    }

    fn json(&mut self, name: &str, value: &impl Serialize) -> Result<(), ApiError> {
        let bytes =
            serde_json::to_vec_pretty(value).map_err(|err| internal("encode entry", err))?;
        self.bytes(name, &bytes)
    }

    /// Copies an object into the entry `name`. `false` when the object is
    /// gone (deleted with its message meanwhile); the entry is then not
    /// written.
    fn object(&mut self, name: &str, key: &str, size: u64) -> Result<bool, ApiError> {
        let mut chunks = match self.handle.block_on(self.store.read(key)) {
            Ok(chunks) => chunks,
            Err(StoreError::NotFound) => return Ok(false),
            Err(err) => return Err(internal("read object", err)),
        };
        self.start(name, size)?;
        while let Some(chunk) = self.handle.block_on(chunks.next()) {
            let chunk = chunk.map_err(|err| internal("read object", err))?;
            self.zip
                .write_all(&chunk)
                .map_err(|err| internal("write entry", err))?;
        }
        Ok(true)
    }

    fn write_all(&mut self) -> Result<(), ApiError> {
        let account = self.account()?;
        self.bytes(
            "README.txt",
            i18n::account::export_readme(account).as_bytes(),
        )?;
        self.messages()?;
        self.reactions()?;
        Ok(())
    }

    /// `account.json` and `profile/`; returns the account's language.
    fn account(&mut self) -> Result<Locale, ApiError> {
        let user_id = self.user_id;
        let db = self.db.clone();
        let row: AccountRow = self.wait(
            sqlx::query_as(
                "SELECT id, email, name, created_at, email_verified_at, bio, pronouns, \
                        status_text, accent_color, banner_color, avatar_decoration, \
                        notifications_hide_content, locale, avatar_url, banner_url \
                 FROM users WHERE id = $1",
            )
            .bind(user_id)
            .fetch_one(&db),
        )?;
        let themes: Option<sqlx::types::Json<Value>> = self.wait(
            sqlx::query_scalar("SELECT document FROM account_themes WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&db),
        )?;
        let notification_settings: Vec<NotificationSetting> = self.wait(
            sqlx::query_as(
                "SELECT scope_kind, scope_id, level, muted, muted_until, updated_at \
                 FROM notification_settings WHERE user_id = $1 ORDER BY scope_kind, scope_id",
            )
            .bind(user_id)
            .fetch_all(&db),
        )?;
        let sessions: Vec<SessionRow> = self.wait(
            sqlx::query_as(
                "SELECT device_client AS client, device_os AS os, created_at, last_used_at \
                 FROM sessions WHERE user_id = $1 AND expires_at > now() \
                 ORDER BY last_used_at DESC, id",
            )
            .bind(user_id)
            .fetch_all(&db),
        )?;
        let servers: Vec<ServerRow> = self.wait(
            sqlx::query_as(
                "SELECT s.id, s.name, s.owner_id = $1 AS owner, m.joined_at \
                 FROM server_members m JOIN servers s ON s.id = m.server_id \
                 WHERE m.user_id = $1 ORDER BY m.joined_at, s.id",
            )
            .bind(user_id)
            .fetch_all(&db),
        )?;
        let invites: Vec<InviteRow> = self.wait(
            sqlx::query_as(
                "SELECT code, server_id, created_at, expires_at, max_uses, uses \
                 FROM invites WHERE created_by = $1 ORDER BY created_at, code",
            )
            .bind(user_id)
            .fetch_all(&db),
        )?;
        self.json(
            "account.json",
            &json!({
                "id": row.id,
                "email": row.email,
                "name": row.name,
                "created_at": row.created_at,
                "email_verified_at": row.email_verified_at,
                "bio": row.bio,
                "pronouns": row.pronouns,
                "status_text": row.status_text,
                "accent_color": row.accent_color,
                "banner_color": row.banner_color,
                "avatar_decoration": row.avatar_decoration,
                "notifications_hide_content": row.notifications_hide_content,
                "locale": row.locale,
                "themes": themes.map(|document| document.0),
                "notification_settings": notification_settings,
                "sessions": sessions,
                "servers": servers,
                "invites": invites,
            }),
        )?;

        // Uploaded profile images only; a link is not the person's file.
        let images = [
            (ImageKind::Avatar, "avatar", row.avatar_url),
            (ImageKind::Banner, "banner", row.banner_url),
        ];
        for (kind, name, url) in images {
            let Some(key) = url.and_then(|url| kind.owned_object_key(user_id, &url)) else {
                continue;
            };
            let (content_type, bytes) = match self.handle.block_on(self.store.fetch(&key)) {
                Ok(object) => object,
                Err(StoreError::NotFound) => continue,
                Err(err) => return Err(internal("read profile image", err)),
            };
            let extension = image_extension(&content_type);
            self.bytes(&format!("profile/{name}.{extension}"), &bytes)?;
        }
        Ok(row.locale)
    }

    /// One file per channel the person wrote in, with their messages and
    /// the files they sent with them.
    fn messages(&mut self) -> Result<(), ApiError> {
        let user_id = self.user_id;
        let db = self.db.clone();
        let channels: Vec<Uuid> = self.wait(
            sqlx::query_scalar(
                "SELECT DISTINCT channel_id FROM messages WHERE author_id = $1 ORDER BY channel_id",
            )
            .bind(user_id)
            .fetch_all(&db),
        )?;
        for channel_id in channels {
            let channel: Option<ChannelRow> = self.wait(
                sqlx::query_as(
                    "SELECT c.id, c.name, c.kind, s.id AS server_id, s.name AS server_name, \
                            peer.id AS peer_id, peer.name AS peer_name \
                     FROM channels c \
                     LEFT JOIN servers s ON s.id = c.server_id \
                     LEFT JOIN LATERAL ( \
                         SELECT u.id, u.name FROM channel_members p JOIN users u ON u.id = p.user_id \
                         WHERE c.kind = 'dm' AND p.channel_id = c.id AND p.user_id <> $2 \
                         LIMIT 1) peer ON true \
                     WHERE c.id = $1",
                )
                .bind(channel_id)
                .bind(user_id)
                .fetch_optional(&db),
            )?;
            // Deleted since the list was read.
            let Some(channel) = channel else {
                continue;
            };
            let messages = self.channel_messages(channel_id)?;
            let server = channel
                .server_id
                .zip(channel.server_name)
                .map(|(id, name)| json!({ "id": id, "name": name }));
            let peer = channel
                .peer_id
                .zip(channel.peer_name)
                .map(|(id, name)| json!({ "id": id, "name": name }));
            let name = match channel.server_id {
                Some(server_id) => format!("messages/servers/{server_id}/{}.json", channel.id),
                None => format!("messages/dms/{}.json", channel.id),
            };
            self.json(
                &name,
                &json!({
                    "channel": {
                        "id": channel.id,
                        "name": channel.name,
                        "kind": channel.kind,
                        "server": server,
                        "peer": peer,
                    },
                    "messages": messages,
                }),
            )?;
        }
        Ok(())
    }

    /// The person's messages in one channel, in `(created_at, id)` order,
    /// page by page; their files are written as each page is read.
    fn channel_messages(&mut self, channel_id: Uuid) -> Result<Vec<Value>, ApiError> {
        let user_id = self.user_id;
        let db = self.db.clone();
        let mut out = Vec::new();
        let mut after: Option<(DateTime<Utc>, Uuid)> = None;
        loop {
            let page: Vec<MessageRow> = self.wait(
                sqlx::query_as(
                    "SELECT id, created_at, edited_at, content, reply_to, gif, system_kind, \
                            system_params \
                     FROM messages \
                     WHERE author_id = $1 AND channel_id = $2 \
                       AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4)) \
                     ORDER BY created_at, id LIMIT $5",
                )
                .bind(user_id)
                .bind(channel_id)
                .bind(after.map(|(at, _)| at))
                .bind(after.map(|(_, id)| id))
                .bind(PAGE)
                .fetch_all(&db),
            )?;
            let Some(last) = page.last() else {
                break;
            };
            after = Some((last.created_at, last.id));
            let full = page.len() as i64 == PAGE;
            let ids: Vec<Uuid> = page.iter().map(|message| message.id).collect();
            let files: Vec<AttachmentRow> = self.wait(
                sqlx::query_as(
                    "SELECT id, message_id, object_key, filename, content_type, size_bytes \
                     FROM attachments WHERE message_id = ANY($1) AND uploader_id = $2 \
                     ORDER BY created_at, id",
                )
                .bind(&ids)
                .bind(user_id)
                .fetch_all(&db),
            )?;
            let mut listed: Vec<(Uuid, Value)> = Vec::with_capacity(files.len());
            for file in files {
                let path = format!("files/{}/{}", file.id, safe_file_name(&file.filename));
                let size = u64::try_from(file.size_bytes).unwrap_or(0);
                let stored = self.object(&path, &file.object_key, size)?;
                listed.push((
                    file.message_id,
                    json!({
                        "id": file.id,
                        "filename": file.filename,
                        "content_type": file.content_type,
                        "size_bytes": file.size_bytes,
                        "path": stored.then_some(path),
                    }),
                ));
            }
            for message in page {
                let attachments: Vec<&Value> = listed
                    .iter()
                    .filter(|(owner, _)| *owner == message.id)
                    .map(|(_, file)| file)
                    .collect();
                out.push(json!({
                    "id": message.id,
                    "created_at": message.created_at,
                    "edited_at": message.edited_at,
                    "content": message.content,
                    "reply_to": message.reply_to,
                    "gif": message.gif.map(|gif| gif.0),
                    "system_kind": message.system_kind,
                    "system_params": message.system_params.map(|params| params.0),
                    "attachments": attachments,
                }));
            }
            if !full {
                break;
            }
        }
        Ok(out)
    }

    /// `reactions.json`: every reaction the person gave.
    fn reactions(&mut self) -> Result<(), ApiError> {
        let user_id = self.user_id;
        let db = self.db.clone();
        let mut out: Vec<ReactionRow> = Vec::new();
        let mut after = (Uuid::nil(), String::new());
        loop {
            let page: Vec<ReactionRow> = self.wait(
                sqlx::query_as(
                    "SELECT r.message_id, m.channel_id, r.emoji \
                     FROM message_reactions r JOIN messages m ON m.id = r.message_id \
                     WHERE r.user_id = $1 AND (r.message_id, r.emoji) > ($2, $3) \
                     ORDER BY r.message_id, r.emoji LIMIT $4",
                )
                .bind(user_id)
                .bind(after.0)
                .bind(&after.1)
                .bind(PAGE)
                .fetch_all(&db),
            )?;
            let full = page.len() as i64 == PAGE;
            if let Some(last) = page.last() {
                after = (last.message_id, last.emoji.clone());
            }
            out.extend(page);
            if !full {
                break;
            }
        }
        self.json("reactions.json", &out)
    }
}

/// The file extension of an uploaded profile image's stored type.
fn image_extension(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "bin",
    }
}

/// A file name that is safe as one path part on every system: path
/// separators, characters Windows forbids and control characters become
/// `_`, it is cut to 200 bytes, leading and trailing dots and spaces go,
/// and a Windows device name gets a leading `_`. The same rules as the
/// native app's downloads (`safe_file_name` there).
pub fn safe_file_name(name: &str) -> String {
    let mut bytes = 0;
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= 200
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim_start_matches('.');
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if cleaned.is_empty() {
        "file".to_owned()
    } else if reserved {
        format!("_{cleaned}")
    } else {
        cleaned.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_cannot_leave_their_folder() {
        assert_eq!(safe_file_name("cat.png"), "cat.png");
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("a\\b/c"), "a_b_c");
        assert_eq!(safe_file_name("line\nbreak\u{0}.txt"), "line_break_.txt");
        assert_eq!(safe_file_name(".."), "file");
        assert_eq!(safe_file_name("   "), "file");
        assert_eq!(safe_file_name("con.txt"), "_con.txt");
        assert_eq!(safe_file_name("Grüße.pdf"), "Grüße.pdf");
        assert!(safe_file_name(&"ä".repeat(300)).len() <= 200);
    }

    #[test]
    fn profile_images_keep_their_type_as_extension() {
        assert_eq!(image_extension("image/png"), "png");
        assert_eq!(image_extension("image/jpeg"), "jpg");
        assert_eq!(image_extension("image/webp"), "webp");
        assert_eq!(image_extension("application/octet-stream"), "bin");
    }
}
