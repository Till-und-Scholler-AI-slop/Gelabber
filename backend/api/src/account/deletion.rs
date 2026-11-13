//! Account deletion (#519): phase one ([`mark_deleted`]) and the purge
//! worker that removes the rest.
//!
//! The worker runs every 2 s and when a deletion wakes it. It takes the
//! accounts marked `deleted_at`, oldest first, and removes for each, in
//! this order and in short transactions:
//!
//! 1. its messages, 500 per batch (their attachments go to
//!    `storage_cleanup` through the `attachment_cleanup` trigger, reactions
//!    on them and their previews cascade);
//! 2. its DM channels with both sides' messages; the peer is told the way a
//!    new DM is announced (`op: "dm"`), so its list reloads without it;
//! 3. uploads not bound to a message;
//! 4. last, the avatar and banner objects (queued in `storage_cleanup`), the
//!    images of the person's feedback (unset, so the `feedback_image_cleanup`
//!    trigger queues them), the upload ledger, and the row: reactions, read
//!    state, settings, themes, identities and bans of the person cascade,
//!    `banned_by` of the bans they issued becomes `NULL`, and their feedback
//!    stays for the operators with `user_id`, `author_id` and `done_by`
//!    `NULL`. With account mail on, the address it had then gets
//!    [`Mail::AccountDeleted`] in the language it had.
//!
//! Every batch takes `pg_try_advisory_xact_lock` on the account, so two API
//! processes never work on one account at the same time. The worker holds
//! no connection across object store requests: it only queues objects for
//! `attachments::cleanup`.

use std::sync::atomic::Ordering;
use std::time::Duration;

use sqlx::{PgPool, Postgres, Transaction};
use tracing::{info, warn};
use uuid::Uuid;

use crate::avatars::ImageKind;
use crate::error::{ApiError, OwnedServer};
use crate::gateway::{Recheck, delivery};
use crate::i18n::Locale;
use crate::mail::Mail;
use crate::state::AppState;

/// How often the worker looks for deleted accounts without being woken.
const TICK: Duration = Duration::from_secs(2);
/// Messages removed per transaction.
const MESSAGE_BATCH: i64 = 500;
/// Accounts one pass looks at; one that keeps failing does not hold up the
/// others.
const PASS: i64 = 8;

/// What phase one removed, for the work after its commit.
#[derive(Debug)]
pub struct Removed {
    /// Hashes of the sessions that ended, whose sockets and media tickets
    /// close next (`session::disconnect`).
    pub sessions: Vec<Vec<u8>>,
    /// Servers the person left.
    pub servers: Vec<Uuid>,
    /// Servers whose queue lost the person's pending join request.
    pub requests: Vec<Uuid>,
}

/// Phase one, in one transaction: marks the account deleted and removes its
/// sessions, sign-in tokens, memberships (with their roles), the invites it
/// created, its pending join requests (declined ones stay until the purge)
/// and its data export (the trigger queues the archive). Refuses
/// with `409 owns_servers` while the account owns a server, and with `401`
/// when it is gone or under deletion already.
///
/// The row lock (`FOR UPDATE`) also orders this against a server being
/// created by or handed to the person: those key-share lock the row and
/// check `deleted_at` (`servers::account_alive`), so either they wait and
/// then refuse, or this waits and then sees the server they made.
pub async fn mark_deleted(db: &PgPool, user_id: Uuid) -> Result<Removed, ApiError> {
    let mut tx = db.begin().await?;
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if found.is_none() {
        return Err(ApiError::Unauthenticated);
    }
    let owned = sqlx::query_as::<_, OwnedServer>(
        "SELECT s.id, s.name, count(m.user_id) AS member_count \
         FROM servers s LEFT JOIN server_members m ON m.server_id = s.id \
         WHERE s.owner_id = $1 GROUP BY s.id, s.name ORDER BY s.created_at, s.id",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    if !owned.is_empty() {
        return Err(ApiError::OwnsServers(owned));
    }
    sqlx::query("UPDATE users SET deleted_at = now(), updated_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let sessions: Vec<Vec<u8>> =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING token_hash")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
    sqlx::query("DELETE FROM account_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let servers: Vec<Uuid> =
        sqlx::query_scalar("DELETE FROM server_members WHERE user_id = $1 RETURNING server_id")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
    let requests: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM server_join_requests WHERE user_id = $1 AND status = 'pending' \
         RETURNING server_id",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM invites WHERE created_by = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM account_exports WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Removed {
        sessions,
        servers,
        requests,
    })
}

/// Starts the purge worker once per process (see the module comment).
pub fn start(state: AppState) {
    if state.purge_worker_started.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        while !state.db.is_closed() {
            let busy = match purge_pending(&state).await {
                Ok(purged) => purged > 0,
                Err(err) => {
                    warn!(error = ?err, "account purge pass failed");
                    false
                }
            };
            // An account finished: look for the next one at once.
            if !busy {
                let wake = state.account_purge.notified();
                let _ = tokio::time::timeout(TICK, wake).await;
            }
        }
    });
}

/// Tells this process's purge worker that an account was marked deleted.
pub fn wake(state: &AppState) {
    state.account_purge.notify_one();
}

/// One pass: purges up to [`PASS`] accounts under deletion, oldest first.
/// Returns how many were removed completely. An account another process is
/// working on is skipped; one that fails is logged and tried next pass.
pub async fn purge_pending(state: &AppState) -> Result<usize, ApiError> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE deleted_at IS NOT NULL ORDER BY deleted_at, id LIMIT $1",
    )
    .bind(PASS)
    .fetch_all(&state.db)
    .await?;
    let mut purged = 0;
    for user_id in ids {
        match purge(state, user_id).await {
            Ok(true) => purged += 1,
            Ok(false) => {}
            Err(err) => warn!(error = ?err, %user_id, "account purge failed, will retry"),
        }
    }
    Ok(purged)
}

/// Removes everything of one account under deletion. `false` when another
/// process holds it or it is gone already.
pub async fn purge(state: &AppState, user_id: Uuid) -> Result<bool, ApiError> {
    // 1. Messages.
    loop {
        let Some(mut tx) = begin(state, user_id).await? else {
            return Ok(false);
        };
        let removed = sqlx::query(
            "DELETE FROM messages WHERE id IN \
                 (SELECT id FROM messages WHERE author_id = $1 LIMIT $2)",
        )
        .bind(user_id)
        .bind(MESSAGE_BATCH)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if removed < MESSAGE_BATCH as u64 {
            break;
        }
    }

    // 2. DM channels, one per transaction.
    loop {
        let Some(mut tx) = begin(state, user_id).await? else {
            return Ok(false);
        };
        let dm: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
            "SELECT c.id, (SELECT p.user_id FROM channel_members p \
                           WHERE p.channel_id = c.id AND p.user_id <> $1 LIMIT 1) \
             FROM channel_members me JOIN channels c ON c.id = me.channel_id AND c.kind = 'dm' \
             WHERE me.user_id = $1 LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((channel_id, peer)) = dm else {
            tx.commit().await?;
            break;
        };
        // The writers' channel lock: a message being written now commits
        // first, and its queued events go with the channel below.
        delivery::lock_channel(&mut tx, channel_id).await?;
        sqlx::query("DELETE FROM channels WHERE id = $1")
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM gateway_outbox WHERE channel_id = $1")
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        // As for a new DM: the peer's clients reload their DM list, which
        // no longer has this one.
        if let Some(peer) = peer {
            delivery::enqueue_dm(&mut tx, peer, channel_id).await?;
        }
        tx.commit().await?;
        state.gateway.drop_channel_log(channel_id).await;
        state
            .gateway
            .announce_recheck(Recheck::Channel(channel_id))
            .await;
        delivery::wake(state);
        info!(%user_id, %channel_id, "dm of a deleted account removed");
    }

    // 3. Uploads not bound to a message.
    let Some(mut tx) = begin(state, user_id).await? else {
        return Ok(false);
    };
    sqlx::query("DELETE FROM attachments WHERE uploader_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // 4. Profile and feedback images, the upload ledger and the row.
    let Some(mut tx) = begin(state, user_id).await? else {
        return Ok(false);
    };
    let (email, locale, avatar_url, banner_url): (String, Locale, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT email, locale, avatar_url, banner_url FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    let images = [
        (ImageKind::Avatar, avatar_url),
        (ImageKind::Banner, banner_url),
    ];
    for (kind, url) in images {
        let Some(key) = url.and_then(|url| kind.owned_object_key(user_id, &url)) else {
            continue;
        };
        sqlx::query(
            "INSERT INTO storage_cleanup (object_key, retain_until) VALUES ($1, now()) \
             ON CONFLICT (object_key) DO UPDATE SET next_attempt = now()",
        )
        .bind(key)
        .execute(&mut *tx)
        .await?;
    }
    // Feedback images go with the account; the entries stay. Staged images
    // under `feedback/{user}/` already have their pending cleanup rows.
    sqlx::query(
        "UPDATE feedback SET image_key = NULL, image_type = NULL, image_bytes = NULL \
         WHERE user_id = $1 AND image_key IS NOT NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    // Queued deletions of the person's uploads still carry the bytes they
    // reserved; with the ledger gone there is nothing to settle them
    // against, so they are released and the objects simply deleted.
    sqlx::query(
        "UPDATE storage_cleanup SET reserved_bytes = 0 \
         WHERE quota_uploader = $1 AND reserved_bytes > 0",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM upload_daily_usage WHERE uploader_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    info!(%user_id, "account purged");

    if let Some(mailer) = state.mail.clone() {
        tokio::spawn(async move {
            let mail = Mail::AccountDeleted;
            match mailer.send(&email, &mail, locale).await {
                Ok(()) => info!(%user_id, kind = mail.kind(), "account mail sent"),
                Err(err) => {
                    warn!(%user_id, kind = mail.kind(), error = %err, "account mail failed")
                }
            }
        });
    }
    Ok(true)
}

/// A transaction holding the account's purge lock, or `None` when another
/// process holds it or the account is no longer under deletion.
async fn begin(
    state: &AppState,
    user_id: Uuid,
) -> Result<Option<Transaction<'static, Postgres>>, ApiError> {
    let mut tx = state.db.begin().await?;
    let locked: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(hashtext('account-purge'), hashtext($1::text))",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !locked {
        return Ok(None);
    }
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(pending.then_some(tx))
}
