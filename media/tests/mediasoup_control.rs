//! Native mediasoup resource/authority boundaries over the public v4 protocol.
//! Real decoded A/V and loss/keyframe recovery are covered by the browser suite.
use gelabber_shared::ticket::{self, TicketClaim};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;
#[path = "support/authority.rs"]
mod authority;
#[path = "support/control.rs"]
mod control;
use control::{Peer, produce, serve};
async fn member(
    state: &gelabber_media::AppState,
    user: Uuid,
    server: Uuid,
    channel: Uuid,
) -> (String, authority::TestAuthority) {
    let code = ticket::generate();
    let lease = authority::mint(
        &state.redis,
        &code,
        TicketClaim {
            u: user,
            s: server,
            c: channel,
            g: false,
        },
    )
    .await;
    (code, lease)
}
fn denied(value: &Value) {
    assert_eq!(
        value["op"], "err",
        "unauthorized operation accepted: {value}"
    );
}
#[tokio::test]
async fn sdk_empty_video_cname_does_not_poison_transport_for_later_audio() {
    let (addr, state) = serve().await;
    let server = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let (a, _la) = member(&state, owner, server, channel).await;
    let (b, _lb) = member(&state, Uuid::new_v4(), server, channel).await;
    let mut publisher = Peer::join(addr, &a, None).await;
    let mut viewer = Peer::join(addr, &b, None).await;
    publisher.transport("send").await;
    viewer.receive().await;
    viewer
        .ok(json!({"op":"w","u":owner,"k":"s","on":true}))
        .await;
    // Chrome's public SDK can supply an empty CNAME for RID-only simulcast.
    // Rust's public transport API generates a transport CNAME when it is absent.
    let mut screen = produce("s", Uuid::new_v4(), None, None);
    screen["rtp"]["rtcp"]["cname"] = json!("");
    publisher.ok(screen).await;
    let video = viewer.event("consumer").await;
    assert_eq!(video["k"], "s");
    let cname = video["rtpParameters"]["rtcp"]["cname"].as_str().unwrap();
    assert!(!cname.is_empty());
    publisher.ok(produce("a", Uuid::new_v4(), None, None)).await;
    let audio = viewer.event("consumer").await;
    assert_eq!(audio["k"], "a");
    assert!(
        !audio["rtpParameters"]["rtcp"]["cname"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    publisher.close().await;
    viewer.close().await;
}
#[tokio::test]
async fn native_transport_ids_and_directions_are_peer_scoped() {
    let (addr, state) = serve().await;
    let server = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let (a, _la) = member(&state, Uuid::new_v4(), server, channel).await;
    let (b, _lb) = member(&state, Uuid::new_v4(), server, channel).await;
    let mut a = Peer::join(addr, &a, None).await;
    let mut b = Peer::join(addr, &b, None).await;
    let transport = a.transport("send").await;
    denied(
        &b.rpc(json!({"op":"restartIce","transportId":transport}))
            .await,
    );
    denied(
        &b.rpc(json!({"op":"closeTransport","transportId":transport}))
            .await,
    );
    denied(&a.rpc(json!({"op":"transport","direction":"send"})).await);
    let restart = a
        .ok(json!({"op":"restartIce","transportId":transport}))
        .await;
    assert!(restart["iceParameters"]["usernameFragment"].is_string());
    a.ok(json!({"op":"closeTransport","transportId":transport}))
        .await;
    let next = a.transport("send").await;
    assert_ne!(next, transport);
    a.close().await;
    b.close().await;
}
#[tokio::test]
async fn publication_identity_survives_replacement_and_late_old_cleanup() {
    let (addr, state) = serve().await;
    let (code, _lease) = member(&state, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()).await;
    let mut peer = Peer::join(addr, &code, None).await;
    peer.transport("send").await;
    let epoch = Uuid::new_v4();
    let first = peer.ok(produce("a", epoch, None, None)).await;
    let old = first["producerId"].as_str().unwrap();
    let mut replacement = produce("a", epoch, None, None);
    replacement["expectedOldProducerId"] = json!(old);
    let next = peer.ok(replacement).await;
    assert_ne!(first["producerId"], next["producerId"]);
    peer.ok(json!({"op":"closeProducer","producerId":old}))
        .await;
    peer.ok(json!({"op":"pauseProducer","producerId":next["producerId"]}))
        .await;
    peer.ok(json!({"op":"resumeProducer","producerId":next["producerId"]}))
        .await;
    denied(&peer.rpc(produce("a", Uuid::new_v4(), None, None)).await);
    peer.close().await;
}
#[tokio::test]
async fn source_audio_requires_exact_live_parent_and_capture_epoch() {
    let (addr, state) = serve().await;
    let (code, _lease) = member(&state, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()).await;
    let mut peer = Peer::join(addr, &code, None).await;
    peer.transport("send").await;
    let epoch = Uuid::new_v4();
    denied(&peer.rpc(produce("sa", epoch, None, None)).await);
    let screen = peer.ok(produce("s", epoch, None, None)).await;
    let parent = screen["producerId"].as_str().unwrap();
    denied(
        &peer
            .rpc(produce("sa", Uuid::new_v4(), Some(parent), None))
            .await,
    );
    denied(
        &peer
            .rpc(produce("sa", epoch, Some("foreign-parent"), None))
            .await,
    );
    let audio = peer.ok(produce("sa", epoch, Some(parent), None)).await;
    peer.ok(json!({"op":"closeProducer","producerId":parent}))
        .await;
    denied(&peer.rpc(produce("sa", epoch, Some(parent), None)).await);
    peer.ok(json!({"op":"closeProducer","producerId":audio["producerId"]}))
        .await;
    peer.close().await;
}
#[tokio::test]
async fn consumer_is_paused_until_exact_generation_ready_and_watch_off_stops_native_consumer() {
    let (addr, state) = serve().await;
    let server = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let (a, _la) = member(&state, owner, server, channel).await;
    let (b, _lb) = member(&state, Uuid::new_v4(), server, channel).await;
    let mut publisher = Peer::join(addr, &a, None).await;
    let mut viewer = Peer::join(addr, &b, None).await;
    publisher.transport("send").await;
    viewer.receive().await;
    viewer
        .ok(json!({"op":"w","u":owner,"k":"s","on":true}))
        .await;
    let epoch = Uuid::new_v4();
    publisher.ok(produce("s", epoch, None, None)).await;
    let event = viewer.event("consumer").await;
    assert_eq!(event["owner"], owner.to_string());
    assert_eq!(event["k"], "s");
    assert_eq!(event["epoch"], epoch.to_string());
    denied(&viewer.rpc(json!({"op":"consumerReady","consumerId":event["consumerId"],"generation":Uuid::new_v4()})).await);
    viewer.ok(json!({"op":"consumerReady","consumerId":event["consumerId"],"generation":event["generation"]})).await;
    denied(&publisher.rpc(json!({"op":"consumerReady","consumerId":event["consumerId"],"generation":event["generation"]})).await);
    viewer.ok(json!({"op":"q","consumerId":event["consumerId"],"generation":event["generation"],"h":90,"congested":false})).await;
    viewer
        .ok(json!({"op":"w","u":owner,"k":"s","on":false}))
        .await;
    let closed = viewer.event("consumerClosed").await;
    assert_eq!(closed["consumerId"], event["consumerId"]);
    denied(&viewer.rpc(json!({"op":"consumerReady","consumerId":event["consumerId"],"generation":event["generation"]})).await);
    viewer
        .ok(json!({"op":"w","u":owner,"k":"s","on":true}))
        .await;
    let next = viewer.event("consumer").await;
    assert_ne!(next["consumerId"], event["consumerId"]);
    assert_ne!(next["generation"], event["generation"]);
    viewer.close().await;
    publisher.close().await;
}
#[tokio::test]
async fn audio_and_camera_automatic_consumers_remain_distinct_and_audio_parent_watch_is_shared() {
    let (addr, state) = serve().await;
    let server = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let (a, _la) = member(&state, owner, server, channel).await;
    let (b, _lb) = member(&state, Uuid::new_v4(), server, channel).await;
    let mut publisher = Peer::join(addr, &a, None).await;
    let mut viewer = Peer::join(addr, &b, None).await;
    publisher.transport("send").await;
    viewer.receive().await;
    let mic = publisher.ok(produce("a", Uuid::new_v4(), None, None)).await;
    let camera = publisher.ok(produce("v", Uuid::new_v4(), None, None)).await;
    let a = viewer.event("consumer").await;
    let v = viewer.event("consumer").await;
    assert_ne!(a["consumerId"], v["consumerId"]);
    assert!([a["producerId"].clone(), v["producerId"].clone()].contains(&mic["producerId"]));
    assert!([a["producerId"].clone(), v["producerId"].clone()].contains(&camera["producerId"]));
    let epoch = Uuid::new_v4();
    let screen = publisher.ok(produce("s", epoch, None, None)).await;
    let audio = publisher
        .ok(produce("sa", epoch, screen["producerId"].as_str(), None))
        .await;
    viewer
        .ok(json!({"op":"w","u":owner,"k":"s","on":true}))
        .await;
    let s = viewer.event("consumer").await;
    let sa = viewer.event("consumer").await;
    let pair = [s, sa];
    let audio_event = pair.iter().find(|e| e["k"] == "sa").unwrap();
    assert_eq!(audio_event["parent"], screen["producerId"]);
    assert_eq!(audio_event["producerId"], audio["producerId"]);
    viewer
        .ok(json!({"op":"w","u":owner,"k":"s","on":false}))
        .await;
    let first = viewer.event("consumerClosed").await;
    let second = viewer.event("consumerClosed").await;
    assert_ne!(first["consumerId"], second["consumerId"]);
    publisher
        .ok(json!({"op":"pauseProducer","producerId":mic["producerId"]}))
        .await;
    let state_event = viewer.event("consumerState").await;
    assert_eq!(state_event["paused"], true);
    viewer.close().await;
    publisher.close().await;
}
#[tokio::test]
async fn watch_only_cannot_create_send_transport_or_publish_even_microphone() {
    let (addr, state) = serve().await;
    let (code, _lease) = member(&state, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()).await;
    let mut viewer = Peer::join(addr, &code, Some(Uuid::new_v4())).await;
    denied(
        &viewer
            .rpc(json!({"op":"transport","direction":"send"}))
            .await,
    );
    denied(&viewer.rpc(produce("a", Uuid::new_v4(), None, None)).await);
    viewer.receive().await;
    viewer.close().await;
}
#[tokio::test]
async fn malformed_rtp_cannot_reserve_a_source_or_hide_later_success() {
    let (addr, state) = serve().await;
    let (code, _lease) = member(&state, Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()).await;
    let mut peer = Peer::join(addr, &code, None).await;
    peer.transport("send").await;
    let mut bad = produce("v", Uuid::new_v4(), None, None);
    bad["rtp"] = json!({});
    denied(&peer.rpc(bad).await);
    peer.ok(produce("v", Uuid::new_v4(), None, None)).await;
    peer.close().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while state.sfu.room_count() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("all room resources return");
    let metrics = state.sfu.metrics_text();
    assert!(metrics.contains("gelabber_media_peers 0"), "{metrics}");
}
