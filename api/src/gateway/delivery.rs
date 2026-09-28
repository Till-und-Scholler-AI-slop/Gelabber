//! Transactional message/discovery outbox, serialized per channel.
use super::{EventDraft, EventKind, Gateway};
use crate::{AppState, error::ApiError};
use serde_json::Value;
use sqlx::{FromRow, PgConnection};
use std::sync::atomic::Ordering;
use std::time::Duration;
use uuid::Uuid;

/// Writers take this before modifying messages; delivery takes the same lock.
/// IDs allocated while held therefore follow committed channel write order.
pub async fn lock_channel(db: &mut PgConnection, channel: Uuid) -> Result<(), ApiError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 507))")
        .bind(channel.to_string())
        .execute(db)
        .await?;
    Ok(())
}
pub async fn revision(db: &mut PgConnection) -> Result<i64, ApiError> {
    Ok(sqlx::query_scalar("SELECT nextval('gateway_revision_seq')")
        .fetch_one(db)
        .await?)
}
pub async fn enqueue(db: &mut PgConnection, id: i64, draft: EventDraft) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO gateway_outbox (id, channel_id, server_id, kind, entity_id, delta) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(id).bind(draft.channel_id.ok_or_else(||ApiError::Internal("message outbox requires channel".into()))?).bind(draft.server_id).bind(draft.kind.as_str()).bind(draft.entity_id).bind(draft.delta).execute(db).await?;
    Ok(())
}
pub async fn enqueue_dm(db: &mut PgConnection, user: Uuid, channel: Uuid) -> Result<(), ApiError> {
    let id = revision(db).await?;
    sqlx::query("INSERT INTO gateway_outbox (id, channel_id, server_id, target_user, kind) VALUES ($1,$2,$2,$3,'dm')")
        .bind(id).bind(channel).bind(user).execute(db).await?;
    Ok(())
}
#[derive(FromRow)]
struct Pending {
    id: i64,
    channel_id: Uuid,
    server_id: Uuid,
    target_user: Option<Uuid>,
    kind: String,
    entity_id: Option<Uuid>,
    delta: Option<Value>,
}

/// One finite scan pass is bounded by its captured maximum DB revision. New
/// writes cannot extend that pass indefinitely and starve its older heads.
#[derive(Default)]
pub(super) struct ScanCursor {
    after: i64,
    through: i64,
}
#[derive(FromRow)]
struct Candidate {
    id: i64,
    channel_id: Uuid,
}

async fn candidates(state: &AppState, limit: usize) -> Result<Vec<Candidate>, ApiError> {
    // Hold only the scan cursor across DB selection, never across channel locks
    // or Redis I/O. HTTP retries and the periodic worker share the same cursor.
    let mut scan = state.connections.inner.delivery_scan.lock().await;
    if scan.after >= scan.through {
        scan.after = 0;
        scan.through = sqlx::query_scalar("SELECT COALESCE(MAX(id),0) FROM gateway_outbox")
            .fetch_one(&state.db)
            .await?;
    }
    let rows:Vec<Candidate>=sqlx::query_as(
        "SELECT o.id,o.channel_id FROM gateway_outbox o WHERE o.id>$1 AND o.id<=$2 \
         AND o.next_attempt<=now() AND NOT EXISTS \
           (SELECT 1 FROM gateway_outbox earlier WHERE earlier.channel_id=o.channel_id AND earlier.id<o.id) \
         ORDER BY o.id LIMIT $3",
    ).bind(scan.after).bind(scan.through).bind(limit as i64).fetch_all(&state.db).await?;
    scan.after = rows.last().map(|r| r.id).unwrap_or(scan.through);
    Ok(rows)
}

/// At most 32 head candidates and publications per invocation. Busy heads
/// advance the shared round-robin cursor and are revisited next finite pass.
pub async fn deliver_pending(state: &AppState, limit: usize) -> Result<usize, ApiError> {
    let limit = limit.min(32);
    if limit == 0 {
        return Ok(0);
    }
    let mut delivered = 0;
    let mut examined = 0;
    while examined < limit {
        let batch = candidates(state, limit - examined).await?;
        if batch.is_empty() {
            break;
        }
        for candidate in batch {
            examined += 1;
            let channel = candidate.channel_id;
            let mut tx = state.db.begin().await?;
            let locked: bool =
                sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,507))")
                    .bind(channel.to_string())
                    .fetch_one(&mut *tx)
                    .await?;
            if !locked {
                continue;
            }
            let pending:Option<Pending>=sqlx::query_as("SELECT id,channel_id,server_id,target_user,kind,entity_id,delta FROM gateway_outbox o WHERE channel_id=$1 AND next_attempt <= now() AND NOT EXISTS (SELECT 1 FROM gateway_outbox earlier WHERE earlier.channel_id=o.channel_id AND earlier.id<o.id) ORDER BY id LIMIT 1 FOR UPDATE").bind(channel).fetch_optional(&mut *tx).await?;
            let Some(pending) = pending else {
                tx.commit().await?;
                continue;
            };
            let result = if let Some(user) = pending.target_user {
                state
                    .gateway
                    .publish_discovery(user, pending.channel_id, pending.id)
                    .await
            } else {
                let kind = match pending.kind.as_str() {
                    "c" => EventKind::C,
                    "e" => EventKind::E,
                    "d" => EventKind::D,
                    _ => return Err(ApiError::Internal("invalid outbox kind".into())),
                };
                state
                    .events
                    .publish_revision(
                        EventDraft {
                            kind,
                            server_id: pending.server_id,
                            channel_id: Some(pending.channel_id),
                            entity_id: pending.entity_id,
                            delta: pending.delta,
                        },
                        Some(pending.id),
                    )
                    .await
                    .map(|_| ())
            };
            if let Err(err) = result {
                sqlx::query(
                    "UPDATE gateway_outbox SET next_attempt=now()+interval '1 second' WHERE id=$1",
                )
                .bind(pending.id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Err(err);
            }
            sqlx::query("DELETE FROM gateway_outbox WHERE id=$1")
                .bind(pending.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            delivered += 1;
        }
    }
    Ok(delivered)
}
impl Gateway {
    pub fn ensure_delivery(&self, state: AppState) {
        if self
            .connections
            .inner
            .delivery_started
            .swap(true, Ordering::SeqCst)
        {
            return;
        }
        tokio::spawn(async move {
            while !state.db.is_closed() {
                if let Err(err) = deliver_pending(&state, 32).await {
                    tracing::debug!(error = err.code(), "outbox delivery will retry");
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    }
    async fn publish_discovery(
        &self,
        user: Uuid,
        channel: Uuid,
        revision: i64,
    ) -> Result<(), ApiError> {
        self.with_conn(|mut conn| async move {
            redis::Script::new("local last=tonumber(redis.call('GET',KEYS[2]) or '0'); if last >= tonumber(ARGV[1]) then return 0 end; redis.call('PUBLISH',KEYS[1],ARGV[2]); redis.call('SET',KEYS[2],ARGV[1]); return 1")
                .key(format!("gb:dm:{user}")).key(format!("gb:dm:delivered:{user}:{channel}")).arg(revision).arg(serde_json::json!({"op":"dm","c":channel}).to_string()).invoke_async::<i32>(&mut conn).await
        }).await.map_err(super::hub::redis_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[sqlx::test]
    async fn busy_channel_does_not_starve_another_channels_outbox(pool: sqlx::PgPool) {
        let config = crate::Config::from_source(|key| match key {
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".into()),
            "REDIS_URL" => Some(std::env::var("REDIS_URL").unwrap()),
            _ => None,
        })
        .unwrap();
        let state = AppState::with_pool(&config, pool.clone()).unwrap();
        let (hot, other, s) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut tx = pool.begin().await.unwrap();
        for c in [hot, other] {
            lock_channel(&mut tx, c).await.unwrap();
            let id = revision(&mut tx).await.unwrap();
            enqueue(
                &mut tx,
                id,
                EventDraft {
                    kind: EventKind::C,
                    server_id: s,
                    channel_id: Some(c),
                    entity_id: None,
                    delta: None,
                },
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
        let mut busy = pool.begin().await.unwrap();
        lock_channel(&mut busy, hot).await.unwrap();
        assert_eq!(deliver_pending(&state, 32).await.unwrap(), 1);
        let remaining: Vec<Uuid> = sqlx::query_scalar("SELECT channel_id FROM gateway_outbox")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, vec![hot]);
        busy.commit().await.unwrap();
        assert_eq!(deliver_pending(&state, 32).await.unwrap(), 1);
    }
}
