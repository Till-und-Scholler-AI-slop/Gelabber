//! Presence snapshots racing a mass reconnect, against real Redis.
//!
//! `presence_snapshot` drops server-set members that have no status. If a
//! user lands in `p:s:{server}` before its status is written, a concurrent
//! snapshot removes it and every later snapshot shows it offline.
//!
//! The second case is a user that is still in the set from before (status
//! expired after a deploy, set kept alive by other announces): the announce
//! SADD is a no-op, so only the snapshot's own cleanup can avoid removing a
//! user whose status was written after the snapshot read it.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gelabber_api::gateway::{
    Gateway,
    protocol::{REDIS_PREFIX, ServerFrame},
};
use tokio::sync::mpsc;
use uuid::Uuid;

const USERS: usize = 40;
const SNAPSHOTTERS: usize = 4;
const ROUNDS: usize = 20;

fn gateway() -> Gateway {
    Gateway::new(
        redis::Client::open(std::env::var("REDIS_URL").unwrap()).unwrap(),
        8,
        Duration::from_secs(30),
        Duration::from_secs(2),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_snapshots_never_drop_announcing_users() {
    assert_eq!(
        lost_users(false).await,
        0,
        "fresh snapshots must list every user that announced"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_snapshots_never_drop_users_left_in_the_set() {
    assert_eq!(
        lost_users(true).await,
        0,
        "users already in the set without status must survive their announce"
    );
}

/// Runs the race and counts users missing from a fresh snapshot afterwards.
/// `left_in_set` puts every user in the server set without a status first.
async fn lost_users(left_in_set: bool) -> usize {
    let g = gateway();
    let mut raw = redis::Client::open(std::env::var("REDIS_URL").unwrap())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let mut lost = 0;
    for _ in 0..ROUNDS {
        let server_id = Uuid::new_v4();
        let mut conns = Vec::with_capacity(USERS);
        let mut receivers = Vec::with_capacity(USERS);
        for _ in 0..USERS {
            let (tx, rx) = mpsc::channel::<ServerFrame>(1024);
            let user_id = Uuid::new_v4();
            let id = g.attach(user_id, tx).await;
            g.watch_server(id, server_id).await;
            conns.push((id, user_id));
            receivers.push(rx);
        }
        if left_in_set {
            let mut sadd = redis::cmd("SADD");
            sadd.arg(format!("{REDIS_PREFIX}p:s:{server_id}"));
            for (_, user_id) in &conns {
                sadd.arg(user_id.to_string());
            }
            let _: i64 = sadd.query_async(&mut raw).await.unwrap();
        }

        let done = Arc::new(AtomicBool::new(false));
        let snapshotters: Vec<_> = (0..SNAPSHOTTERS)
            .map(|_| {
                let g = g.clone();
                let done = done.clone();
                tokio::spawn(async move {
                    while !done.load(Ordering::SeqCst) {
                        g.presence_snapshot(server_id).await.unwrap();
                    }
                })
            })
            .collect();
        let announces: Vec<_> = conns
            .iter()
            .map(|&(id, _)| {
                let g = g.clone();
                tokio::spawn(async move { g.announce_server(id, server_id).await.unwrap() })
            })
            .collect();
        for task in announces {
            task.await.unwrap();
        }
        done.store(true, Ordering::SeqCst);
        for task in snapshotters {
            task.await.unwrap();
        }

        let snap = g.presence_snapshot(server_id).await.unwrap();
        lost += conns
            .iter()
            .filter(|(_, user_id)| !snap.iter().any(|entry| entry.u == *user_id))
            .count();

        for (id, user_id) in conns {
            g.detach(id).await;
            g.drop_conn_presence(id, user_id, &[server_id].into())
                .await
                .unwrap();
        }
    }
    lost
}
