//! Multi-seat deltas observed through actual Redis Pub/Sub.
use futures_util::StreamExt;
use gelabber_api::gateway::{
    Gateway,
    protocol::{ServerFrame, SigEvent, SigKind, TrackKind},
};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;
fn gateway() -> Gateway {
    Gateway::new(
        redis::Client::open(std::env::var("REDIS_URL").unwrap()).unwrap(),
        8,
        Duration::from_secs(10),
        Duration::from_secs(2),
    )
}
async fn subscriber(c: Uuid) -> redis::aio::PubSub {
    let mut sub = redis::Client::open(std::env::var("REDIS_URL").unwrap())
        .unwrap()
        .get_async_pubsub()
        .await
        .unwrap();
    sub.subscribe(format!("gb:v:{c}")).await.unwrap();
    sub
}
async fn next(sub: &mut redis::aio::PubSub) -> SigEvent {
    let msg = tokio::time::timeout(Duration::from_secs(1), sub.on_message().next())
        .await
        .unwrap()
        .unwrap();
    serde_json::from_str(&msg.get_payload::<String>().unwrap()).unwrap()
}
async fn flags(sub: &mut redis::aio::PubSub, m: bool, d: bool) {
    let one = next(sub).await;
    assert_eq!((one.t, one.on), (SigKind::M, Some(m)));
    let two = next(sub).await;
    assert_eq!((two.t, two.on), (SigKind::D, Some(d)));
}
async fn quiet(sub: &mut redis::aio::PubSub) {
    assert!(
        tokio::time::timeout(Duration::from_millis(150), sub.on_message().next())
            .await
            .is_err(),
        "no false global unpublish/leave"
    );
}

#[tokio::test]
async fn mute_deafen_and_leave_deltas_match_all_active_seats() {
    let g = gateway();
    let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let (tx1, _rx1) = mpsc::channel::<ServerFrame>(128);
    let (a, _) = g.attach_session(u, "a".repeat(64), tx1).await;
    let (tx2, _rx2) = mpsc::channel::<ServerFrame>(128);
    let (b, _) = g.attach_session(u, "b".repeat(64), tx2).await;
    g.join_voice(a, u, s, c).await.unwrap();
    g.join_voice(b, u, s, c).await.unwrap();
    let mut sub = subscriber(c).await;
    g.set_voice_mute(a, u, s, c, true).await.unwrap();
    let delta = next(&mut sub).await;
    assert_eq!((delta.t, delta.on), (SigKind::M, Some(false)));
    g.set_voice_deafen(a, u, s, c, true).await.unwrap();
    let delta = next(&mut sub).await;
    assert_eq!((delta.t, delta.on), (SigKind::D, Some(false)));
    let delta = next(&mut sub).await;
    assert_eq!((delta.t, delta.on), (SigKind::M, Some(false)));
    g.set_voice_mute(b, u, s, c, true).await.unwrap();
    let delta = next(&mut sub).await;
    assert_eq!((delta.t, delta.on), (SigKind::M, Some(true)));
    g.set_voice_deafen(b, u, s, c, true).await.unwrap();
    let delta = next(&mut sub).await;
    assert_eq!((delta.t, delta.on), (SigKind::D, Some(true)));
    let delta = next(&mut sub).await;
    assert_eq!((delta.t, delta.on), (SigKind::M, Some(true)));
    g.leave_voice(b, u, s, c).await.unwrap();
    flags(&mut sub, true, true).await;
    let view = g.voice_snapshot(s).await.unwrap();
    assert!(view[0].m && view[0].d);
    // Idempotent join must preserve the existing muted/deafened aggregate.
    g.join_voice(a, u, s, c).await.unwrap();
    let joined = next(&mut sub).await;
    assert_eq!(joined.t, SigKind::J);
    assert_eq!((joined.m, joined.d), (Some(true), Some(true)));
    g.detach(a).await;
    g.detach(b).await;
}

#[tokio::test]
async fn stale_refresh_and_leave_do_not_unpublish_same_user_replacement_claim() {
    let old_api = gateway();
    let new_api = gateway();
    let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let (tx, _rx) = mpsc::channel::<ServerFrame>(128);
    let (old, _) = old_api.attach_session(u, "a".repeat(64), tx).await;
    old_api.join_voice(old, u, s, c).await.unwrap();
    old_api
        .set_voice_pub(old, u, s, c, TrackKind::L, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5200)).await;
    let (tx, _rx2) = mpsc::channel::<ServerFrame>(128);
    let (new, _) = new_api.attach_session(u, "b".repeat(64), tx).await;
    new_api.join_voice(new, u, s, c).await.unwrap();
    let mut sub = subscriber(c).await;
    new_api
        .set_voice_pub(new, u, s, c, TrackKind::L, true)
        .await
        .unwrap();
    let started = next(&mut sub).await;
    assert_eq!(started.t, SigKind::P);
    assert!(started.lc.is_some());
    old_api.refresh_voice(old).await.unwrap();
    quiet(&mut sub).await;
    old_api.leave_voice(old, u, s, c).await.unwrap();
    flags(&mut sub, false, false).await;
    quiet(&mut sub).await;
    assert!(new_api.voice_snapshot(s).await.unwrap()[0].l);
    let mut redis = redis::Client::open(std::env::var("REDIS_URL").unwrap())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let raw: String = redis::cmd("GET")
        .arg(format!("gb:live:{c}"))
        .query_async(&mut redis)
        .await
        .unwrap();
    let owner: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(owner["nonce"], started.lc.unwrap().to_string());
    old_api.detach(old).await;
    new_api.detach(new).await;
}

#[tokio::test]
async fn unpublish_and_leave_keep_another_seats_camera_publication() {
    let g = gateway();
    let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let (tx, _rx) = mpsc::channel::<ServerFrame>(128);
    let (a, _) = g.attach_session(u, "a".repeat(64), tx).await;
    let (tx, _rx2) = mpsc::channel::<ServerFrame>(128);
    let (b, _) = g.attach_session(u, "b".repeat(64), tx).await;
    g.join_voice(a, u, s, c).await.unwrap();
    g.join_voice(b, u, s, c).await.unwrap();
    g.set_voice_pub(a, u, s, c, TrackKind::V, true)
        .await
        .unwrap();
    g.set_voice_pub(b, u, s, c, TrackKind::V, true)
        .await
        .unwrap();
    let mut sub = subscriber(c).await;
    g.set_voice_pub(a, u, s, c, TrackKind::V, false)
        .await
        .unwrap();
    quiet(&mut sub).await;
    g.leave_voice(a, u, s, c).await.unwrap();
    flags(&mut sub, false, false).await;
    quiet(&mut sub).await;
    g.set_voice_pub(b, u, s, c, TrackKind::V, false)
        .await
        .unwrap();
    let ended = next(&mut sub).await;
    assert_eq!((ended.t, ended.k), (SigKind::U, Some(TrackKind::V)));
    g.detach(a).await;
    g.detach(b).await;
}

#[tokio::test]
async fn stale_audio_refresh_preserves_same_user_replacement_live_audio() {
    let old_api = gateway();
    let new_api = gateway();
    let (u, s, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let (old_tx, _old_rx) = mpsc::channel::<ServerFrame>(128);
    let (old, _) = old_api.attach_session(u, "a".repeat(64), old_tx).await;
    old_api.join_voice(old, u, s, c).await.unwrap();
    for kind in [TrackKind::L, TrackKind::La] {
        old_api
            .set_voice_pub(old, u, s, c, kind, true)
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(5200)).await;
    let (new_tx, _new_rx) = mpsc::channel::<ServerFrame>(128);
    let (new, _) = new_api.attach_session(u, "b".repeat(64), new_tx).await;
    new_api.join_voice(new, u, s, c).await.unwrap();
    let mut sub = subscriber(c).await;
    for kind in [TrackKind::L, TrackKind::La] {
        new_api
            .set_voice_pub(new, u, s, c, kind, true)
            .await
            .unwrap();
        let event = next(&mut sub).await;
        assert_eq!((event.t, event.k), (SigKind::P, Some(kind)));
        assert!(event.lc.is_some());
    }
    old_api.refresh_voice(old).await.unwrap();
    quiet(&mut sub).await;
    old_api.leave_voice(old, u, s, c).await.unwrap();
    flags(&mut sub, false, false).await;
    quiet(&mut sub).await;
    let (observer_tx, _observer_rx) = mpsc::channel::<ServerFrame>(128);
    let observer_user = Uuid::new_v4();
    let (observer, _) = new_api
        .attach_session(observer_user, "c".repeat(64), observer_tx)
        .await;
    let snapshot = new_api
        .join_voice(observer, observer_user, s, c)
        .await
        .unwrap();
    assert!(snapshot.iter().any(|event| event.k == Some(TrackKind::L)));
    assert!(snapshot.iter().any(|event| event.k == Some(TrackKind::La)));
    old_api.detach(old).await;
    new_api.detach(new).await;
    new_api.detach(observer).await;
}
