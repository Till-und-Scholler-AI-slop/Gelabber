//! A server starts every background worker (GEL-434). The other tests
//! switch workers off or run their passes by hand, so a worker that a server
//! never starts would pass all of them.

mod common;

use std::time::Duration;

use gelabber_api::push::webpush::Vapid;
use gelabber_api::{AppState, Config, Worker, Workers, app};
use sqlx::PgPool;

/// Configured as a server with push and link previews on, the two
/// features whose workers need them.
fn server_state(pool: PgPool) -> AppState {
    let (vapid_private, _) = Vapid::generate();
    let config = Config::from_source(|key| match key {
        "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
        "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
        "REDIS_URL" => Some(common::redis_url()),
        "VAPID_PRIVATE_KEY" => Some(vapid_private.clone()),
        "VAPID_SUBJECT" => Some("mailto:admin@test.example".to_owned()),
        _ => None,
    })
    .expect("server config");
    AppState::with_pool(&config, pool).expect("state")
}

/// Every worker of [`Workers::default`], as a server runs them, finishes a
/// pass. The intervals are shorter, so the test does not wait minutes for
/// the first sweep. The struct is taken apart field by field: a new worker
/// does not compile here until it is listed with the passes that show it ran.
#[sqlx::test]
async fn a_server_runs_every_background_worker(pool: PgPool) {
    let Workers {
        delivery,
        storage_cleanup,
        pictures,
        account_purge,
        account_export,
        link_previews,
        presence_modes,
        last_call,
        feedback_sweep,
    } = Workers::default();
    let expected = [
        ("delivery", delivery, &[Worker::Delivery][..]),
        (
            "storage_cleanup",
            storage_cleanup,
            &[Worker::StorageDeletes, Worker::UploadExpiry],
        ),
        ("pictures", pictures, &[Worker::Pictures]),
        ("account_purge", account_purge, &[Worker::AccountPurge]),
        ("account_export", account_export, &[Worker::AccountExport]),
        ("link_previews", link_previews, &[Worker::PreviewSweep]),
        ("presence_modes", presence_modes, &[Worker::PresenceModes]),
        ("last_call", last_call, &[Worker::LastCall]),
        ("feedback_sweep", feedback_sweep, &[Worker::FeedbackSweep]),
    ];
    for (name, every, _) in &expected {
        assert!(every.is_some(), "Workers::default() leaves {name} off");
    }

    let mut state = server_state(pool);
    let short = |every: Option<Duration>| every.map(|_| Duration::from_millis(50));
    state.workers = Workers {
        delivery: short(delivery),
        storage_cleanup: short(storage_cleanup),
        pictures: short(pictures),
        account_purge: short(account_purge),
        account_export: short(account_export),
        link_previews: short(link_previews),
        presence_modes: short(presence_modes),
        last_call: short(last_call),
        feedback_sweep: short(feedback_sweep),
    };
    let _app = app(state.clone());

    for (name, _, passes) in expected {
        for &worker in passes {
            tokio::time::timeout(Duration::from_secs(30), state.worker_passes.first(worker))
                .await
                .unwrap_or_else(|_| panic!("{name}: no pass of {worker:?}"));
        }
    }
}
