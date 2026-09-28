//! Durable object cleanup. No pool connection is held during S3 requests.
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::{StreamExt, stream};
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;
use crate::storage::StoreError;

const BATCH: i64 = 32;

pub fn start(state: AppState) {
    if state.storage_worker_started.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        // Slow HEADs must not delay already committed object deletion jobs.
        tokio::join!(run(&state, true), run(&state, false));
    });
}

async fn run(state: &AppState, expiry: bool) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tick.tick().await;
        if state.db.is_closed() {
            break;
        }
        let result = if expiry {
            expire_pending(state, BATCH).await
        } else {
            delete_pending(state, BATCH).await
        };
        if let Err(err) = result {
            tracing::warn!(error=?err, expiry, "storage cleanup batch failed");
        }
    }
}

#[derive(FromRow)]
struct Upload {
    id: Uuid,
    object_key: String,
    size_bytes: i64,
    grace_over: bool,
}

/// HEAD without locks, then lock and recheck. A concurrent bind wins safely;
/// a failed HEAD leaves metadata/reservation intact for a later retry.
pub async fn expire_pending(state: &AppState, limit: i64) -> Result<(), ApiError> {
    let uploads = sqlx::query_as::<_, Upload>(
        "SELECT id, object_key, size_bytes, clock_timestamp() >= expires_at + interval '15 minutes' AS grace_over FROM attachments \
         WHERE message_id IS NULL AND expires_at <= now() AND expiry_retry_at <= now() \
         ORDER BY expiry_retry_at, expires_at, id LIMIT $1",
    )
    .bind(limit.clamp(0, BATCH))
    .fetch_all(&state.db)
    .await?;
    let work = stream::iter(uploads)
        .map(|upload| async move { retire(state, &upload, true).await })
        .buffer_unordered(4);
    finish(work).await
}

/// Failed binding must not delete a foreign, successfully bound, or valid file.
/// Recheck row identity under lock after HEAD and after the message rollback.
pub async fn discard_mismatches(
    state: &AppState,
    channel: Uuid,
    user: Uuid,
    ids: &[Uuid],
) -> Result<(), ApiError> {
    let uploads = sqlx::query_as::<_, Upload>(
        "SELECT id, object_key, size_bytes, clock_timestamp() >= expires_at + interval '15 minutes' AS grace_over FROM attachments \
         WHERE id=ANY($1) AND channel_id=$2 AND uploader_id=$3 AND message_id IS NULL",
    )
    .bind(ids)
    .bind(channel)
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    for upload in uploads {
        retire(state, &upload, false).await?;
    }
    Ok(())
}

async fn retire(state: &AppState, upload: &Upload, expired: bool) -> Result<(), ApiError> {
    let quota_state = match state.store.head(&upload.object_key).await {
        Ok(meta) if !expired && meta.size == upload.size_bytes => return Ok(()),
        Ok(_) => "consumed",
        Err(StoreError::NotFound) if expired && !upload.grace_over => {
            // HEAD cannot see an incomplete valid PUT. Keep its reservation
            // and expired metadata until grace has passed; no repeated HEADs
            // are needed during that wait. Bind/download already reject expiry.
            sqlx::query(
                "UPDATE attachments SET expiry_retry_at=GREATEST(expiry_retry_at, expires_at + interval '15 minutes') \
                 WHERE id=$1 AND object_key=$2 AND message_id IS NULL",
            ).bind(upload.id).bind(&upload.object_key).execute(&state.db).await?;
            return Ok(());
        }
        // grace_over was sampled BEFORE HEAD, not after it: a HEAD started
        // before the grace boundary cannot refund based on that older 404.
        Err(StoreError::NotFound) if expired => "unused",
        Err(StoreError::NotFound) => return Ok(()),
        Err(err) => {
            sqlx::query("UPDATE attachments SET expiry_retry_at=now()+interval '1 minute' WHERE id=$1 AND message_id IS NULL")
                .bind(upload.id).execute(&state.db).await?;
            tracing::warn!(error=%err, attachment=%upload.id, "upload HEAD failed; retaining reservation");
            return Ok(());
        }
    };
    let mut tx = state.db.begin().await?;
    let id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM attachments WHERE id=$1 AND object_key=$2 AND message_id IS NULL \
         AND (NOT $3 OR expires_at <= now()) FOR UPDATE",
    )
    .bind(upload.id)
    .bind(&upload.object_key)
    .bind(expired)
    .fetch_optional(&mut *tx)
    .await?;
    if id.is_some() {
        sqlx::query("UPDATE attachments SET quota_state=$2 WHERE id=$1")
            .bind(upload.id)
            .bind(quota_state)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM attachments WHERE id=$1")
            .bind(upload.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(FromRow)]
struct Job {
    object_key: String,
    claim: Uuid,
    reserved_bytes: i64,
}

/// Claim a bounded batch, release the DB connection, then perform I/O. Claim
/// expiry handles API crashes; acknowledgements compare the generation so an
/// older worker cannot delete a newer worker's retry job.
pub async fn delete_pending(state: &AppState, limit: i64) -> Result<(), ApiError> {
    let jobs = sqlx::query_as::<_, Job>(
        "WITH due AS (SELECT object_key FROM storage_cleanup \
           WHERE next_attempt <= now() AND (claimed_until IS NULL OR claimed_until <= now()) \
           AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.object_key=storage_cleanup.object_key) \
           ORDER BY next_attempt, object_key LIMIT $1 FOR UPDATE SKIP LOCKED) \
         UPDATE storage_cleanup j SET claim=gen_random_uuid(), claimed_until=now()+interval '6 minutes', \
             attempts=attempts+1 FROM due WHERE j.object_key=due.object_key \
         RETURNING j.object_key, j.claim, j.reserved_bytes",
    ).bind(limit.clamp(0, BATCH)).fetch_all(&state.db).await?;
    let work = stream::iter(jobs)
        .map(|job| async move { delete_one(state, job).await })
        .buffer_unordered(4);
    finish(work).await
}

async fn finish(
    work: impl futures_util::Stream<Item = Result<(), ApiError>>,
) -> Result<(), ApiError> {
    futures_util::pin_mut!(work);
    let mut failed = None;
    while let Some(result) = work.next().await {
        if let Err(err) = result {
            failed = Some(err);
        }
    }
    match failed {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

async fn delete_one(state: &AppState, job: Job) -> Result<(), ApiError> {
    if job.reserved_bytes > 0 {
        // Generation compare and quota transition are ONE DB statement. Each
        // job takes only one ledger; parent cascades never take these locks.
        let charged = sqlx::query(
            "WITH available AS (SELECT u.uploader_id,u.day FROM upload_daily_usage u \
               JOIN storage_cleanup j ON j.quota_uploader=u.uploader_id AND j.quota_day=u.day \
               WHERE j.object_key=$1 AND j.claim=$2 AND j.reserved_bytes=$3 \
               FOR UPDATE OF u SKIP LOCKED), \
             charge AS (UPDATE storage_cleanup j SET reserved_bytes=0 FROM available a \
               WHERE j.object_key=$1 AND j.claim=$2 AND j.reserved_bytes=$3 \
               AND j.quota_uploader=a.uploader_id AND j.quota_day=a.day \
               RETURNING j.quota_uploader,j.quota_day) \
             UPDATE upload_daily_usage u SET reserved=reserved-$3, consumed=consumed+$3 \
               FROM charge WHERE u.uploader_id=charge.quota_uploader AND u.day=charge.quota_day",
        )
        .bind(&job.object_key)
        .bind(job.claim)
        .bind(job.reserved_bytes)
        .execute(&state.db)
        .await?;
        if charged.rows_affected() == 0 {
            // Skip a busy ledger rather than occupying a small pool waiting
            // for an unrelated transaction; keep the reservation and retry.
            sqlx::query("UPDATE storage_cleanup SET claim=NULL,claimed_until=NULL,next_attempt=now()+interval '1 second' WHERE object_key=$1 AND claim=$2")
                .bind(&job.object_key).bind(job.claim).execute(&state.db).await?;
            return Ok(());
        }
    }
    let deleted = match state.store.delete(&job.object_key).await {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(error=%err, "object delete failed; durable retry retained");
            false
        }
    };
    // Keys are immutable random attachment IDs. Check there is still no
    // metadata before ack, in addition to comparing the worker generation.
    if deleted {
        sqlx::query(
            "DELETE FROM storage_cleanup WHERE object_key=$1 AND claim=$2 \
                 AND retain_until <= now() AND reserved_bytes=0 \
                 AND NOT EXISTS (SELECT 1 FROM attachments WHERE object_key=$1)",
        )
        .bind(&job.object_key)
        .bind(job.claim)
        .execute(&state.db)
        .await?;
    }
    sqlx::query(
        "UPDATE storage_cleanup SET claim=NULL, claimed_until=NULL, \
             next_attempt=now()+($3 * interval '1 second') WHERE object_key=$1 AND claim=$2",
    )
    .bind(&job.object_key)
    .bind(job.claim)
    .bind(if deleted { 10.0f64 } else { 1.0f64 })
    .execute(&state.db)
    .await?;
    Ok(())
}
