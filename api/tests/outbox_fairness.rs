//! Regression for the independently confirmed fixed candidate-window starvation.
mod common;
use gelabber_api::gateway::{EventDraft, EventKind, delivery};
use sqlx::PgPool;
use uuid::Uuid;

async fn insert(pool: &PgPool, count: usize) -> Vec<Uuid> {
    let mut channels = Vec::new();
    let mut tx = pool.begin().await.unwrap();
    for _ in 0..count {
        let channel = Uuid::new_v4();
        channels.push(channel);
        delivery::lock_channel(&mut tx, channel).await.unwrap();
        let id = delivery::revision(&mut tx).await.unwrap();
        delivery::enqueue(
            &mut tx,
            id,
            EventDraft {
                kind: EventKind::C,
                server_id: Uuid::new_v4(),
                channel_id: Some(channel),
                entity_id: None,
                delta: None,
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    channels
}

#[sqlx::test]
async fn thirty_two_busy_heads_do_not_starve_the_free_thirty_third_channel(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    let channels = insert(&pool, 33).await;
    let mut busy = pool.begin().await.unwrap();
    for &channel in &channels[..32] {
        delivery::lock_channel(&mut busy, channel).await.unwrap();
    }
    assert_eq!(delivery::deliver_pending(&state, 32).await.unwrap(), 0);
    assert_eq!(delivery::deliver_pending(&state, 32).await.unwrap(), 1);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_outbox WHERE channel_id=$1")
            .bind(channels[32])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    busy.commit().await.unwrap();
    let mut delivered = 0;
    for _ in 0..3 {
        delivered += delivery::deliver_pending(&state, 32).await.unwrap();
    }
    assert_eq!(delivered, 32);
}

#[sqlx::test]
async fn finite_scan_pass_revisits_old_heads_despite_continuous_new_channels(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    let channels = insert(&pool, 65).await;
    let mut busy = pool.begin().await.unwrap();
    for &channel in &channels[..64] {
        delivery::lock_channel(&mut busy, channel).await.unwrap();
    }
    assert_eq!(delivery::deliver_pending(&state, 32).await.unwrap(), 0);
    insert(&pool, 32).await;
    assert_eq!(delivery::deliver_pending(&state, 32).await.unwrap(), 0);
    insert(&pool, 32).await;
    assert_eq!(delivery::deliver_pending(&state, 32).await.unwrap(), 1);
    // After reaching the captured pass ceiling, growing tails must not keep
    // the cursor away from the now-unlocked original heads indefinitely.
    busy.commit().await.unwrap();
    for _ in 0..8 {
        insert(&pool, 32).await;
        assert!(delivery::deliver_pending(&state, 32).await.unwrap() <= 32);
    }
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_outbox WHERE channel_id=ANY($1)")
            .bind(&channels[..64])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}

#[sqlx::test]
async fn one_channel_can_deliver_a_full_batch_in_revision_order(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    let channel = Uuid::new_v4();
    let server = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    delivery::lock_channel(&mut tx, channel).await.unwrap();
    for _ in 0..32 {
        let id = delivery::revision(&mut tx).await.unwrap();
        delivery::enqueue(
            &mut tx,
            id,
            EventDraft {
                kind: EventKind::C,
                server_id: server,
                channel_id: Some(channel),
                entity_id: None,
                delta: None,
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(delivery::deliver_pending(&state, 32).await.unwrap(), 32);
    let mut redis = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let key = format!("gb:l:c:{channel}");
    let events: Vec<String> = redis::cmd("LRANGE")
        .arg(key)
        .arg(0)
        .arg(-1)
        .query_async(&mut redis)
        .await
        .unwrap();
    let revisions: Vec<i64> = events
        .iter()
        .map(|raw| {
            serde_json::from_str::<serde_json::Value>(raw).unwrap()["r"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(revisions.len(), state.ws_replay.min(32));
    // Redis replay is newest-first, so revisions strictly descend.
    assert!(revisions.windows(2).all(|pair| pair[0] > pair[1]));
}

/// Write handlers no longer run a delivery pass before their response; they
/// wake the worker instead (#294). Without the wake every row would wait for
/// the worker's 200 ms poll, so ten sequential writes would take about 2 s.
#[sqlx::test]
async fn a_wake_delivers_without_waiting_for_the_poll(pool: PgPool) {
    let state = common::ws_state(pool.clone());
    state.gateway.ensure_delivery(state.clone());
    let started = std::time::Instant::now();
    for _ in 0..10 {
        let channel = insert(&pool, 1).await[0];
        delivery::wake(&state);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let left: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM gateway_outbox WHERE channel_id=$1")
                        .bind(channel)
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                if left == 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the woken worker delivers the row");
    }
    let elapsed = started.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
}
