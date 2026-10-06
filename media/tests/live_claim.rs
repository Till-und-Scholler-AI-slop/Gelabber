//! Actual v4 WebSocket/Redis Live claims with real mediasoup Producers.
use gelabber_media::AppState;
use gelabber_media::live::{LiveAcquireOutcome, try_acquire};
use gelabber_shared::ticket::{self, AuthorizedTicketClaim, TicketClaim};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;
#[path = "support/authority.rs"]
mod authority;
#[path = "support/control.rs"]
mod control;
use control::{Peer, produce, serve};
async fn fixture(state: &AppState) -> (String, AuthorizedTicketClaim, authority::TestAuthority) {
    let code = ticket::generate();
    let lease = authority::mint(
        &state.redis,
        &code,
        TicketClaim {
            u: Uuid::new_v4(),
            s: Uuid::new_v4(),
            c: Uuid::new_v4(),
            g: true,
        },
    )
    .await;
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let raw: String = redis::cmd("GET")
        .arg(ticket::redis_key(&code))
        .query_async(&mut conn)
        .await
        .unwrap();
    (code, serde_json::from_str(&raw).unwrap(), lease)
}
fn live_record(claim: &AuthorizedTicketClaim, nonce: Uuid) -> Value {
    json!({"u":claim.claim.u,"s":claim.claim.s,"c":claim.claim.c,"session":claim.auth.session,"seat":Uuid::new_v4(),"nonce":nonce})
}
async fn set_live(
    state: &AppState,
    claim: &AuthorizedTicketClaim,
    value: &Value,
    ttl: Option<u64>,
) {
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let mut command = redis::cmd("SET");
    command
        .arg(format!("gb:live:{}", claim.claim.c))
        .arg(value.to_string());
    if let Some(ttl) = ttl {
        command.arg("PX").arg(ttl);
    }
    let _: () = command.query_async(&mut conn).await.unwrap();
}
async fn key(state: &AppState, nonce: Uuid) -> Option<String> {
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    redis::cmd("GET")
        .arg(gelabber_media::live::peer_key(nonce))
        .query_async(&mut conn)
        .await
        .unwrap()
}
async fn set_peer_lease(state: &AppState, nonce: Uuid, owner: Uuid, ttl: Option<u64>) {
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let mut command = redis::cmd("SET");
    command
        .arg(gelabber_media::live::peer_key(nonce))
        .arg(owner.to_string());
    if let Some(ttl) = ttl {
        command.arg("PX").arg(ttl);
    }
    let _: () = command.query_async(&mut conn).await.unwrap();
}
async fn peer_ttl(state: &AppState, nonce: Uuid) -> i64 {
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    redis::cmd("PTTL")
        .arg(gelabber_media::live::peer_key(nonce))
        .query_async(&mut conn)
        .await
        .unwrap()
}
async fn cleanup(state: &AppState, claim: &AuthorizedTicketClaim, nonce: Uuid) {
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: () = redis::cmd("DEL")
        .arg(format!("gb:live:{}", claim.claim.c))
        .arg(gelabber_media::live::peer_key(nonce))
        .query_async(&mut conn)
        .await
        .unwrap();
}

async fn publisher(addr: std::net::SocketAddr, code: &str) -> Peer {
    let mut peer = Peer::join(addr, code, None).await;
    peer.transport("send").await;
    peer
}
fn accepted(frame: &Value) -> bool {
    if frame["op"] == "err" {
        assert_eq!(frame["e"], "forbidden", "claim rejection: {frame}");
        false
    } else {
        assert_eq!(frame["op"], "result");
        true
    }
}

#[tokio::test]
async fn ticket_bit_alone_and_mismatched_live_claims_are_rejected_on_wire() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    let mut peer = publisher(addr, &code).await;
    assert!(!accepted(
        &peer
            .rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
            .await
    ));
    let good = live_record(&claim, nonce);
    for field in ["u", "s", "c", "session", "nonce", "seat"] {
        let mut bad = good.clone();
        bad[field] = if field == "seat" {
            Value::Null
        } else {
            json!(Uuid::new_v4())
        };
        set_live(&state, &claim, &bad, Some(5000)).await;
        assert!(
            !accepted(
                &peer
                    .rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
                    .await
            ),
            "mismatched {field}"
        );
        assert!(key(&state, nonce).await.is_none());
    }
    set_live(&state, &claim, &good, None).await;
    assert!(
        !accepted(
            &peer
                .rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
                .await
        ),
        "unleased record"
    );
    for seat in ["", "invalid-seat", "00000000-0000-0000-0000-000000000000"] {
        let mut bad = good.clone();
        bad["seat"] = json!(seat);
        set_live(&state, &claim, &bad, Some(5000)).await;
        assert!(!accepted(
            &peer
                .rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
                .await
        ));
    }
    set_live(&state, &claim, &good, Some(5000)).await;
    assert!(!accepted(
        &peer.rpc(produce("l", Uuid::new_v4(), None, None)).await
    ));
    peer.ok(produce("l", Uuid::new_v4(), None, Some(nonce)))
        .await;
    assert!(!accepted(
        &peer
            .rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
            .await
    ));
    peer.ok(produce("v", Uuid::new_v4(), None, None)).await;
    peer.close().await;
    cleanup(&state, &claim, nonce).await;
}
#[tokio::test]
async fn exact_session_can_bind_only_one_media_peer_and_stale_release_is_safe() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let another = ticket::generate();
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: () = redis::cmd("SET")
        .arg(ticket::redis_key(&another))
        .arg(serde_json::to_string(&claim).unwrap())
        .arg("EX")
        .arg(30)
        .query_async(&mut conn)
        .await
        .unwrap();
    let nonce = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, nonce), Some(5000)).await;
    let mut a = publisher(addr, &code).await;
    let mut b = publisher(addr, &another).await;
    let (a_result, b_result) = tokio::join!(
        a.rpc(produce("l", Uuid::new_v4(), None, Some(nonce))),
        b.rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
    );
    for result in [&a_result, &b_result] {
        if result["op"] == "err" {
            assert_eq!(
                result["e"], "live_busy",
                "exclusive peer rejection: {result}"
            );
        } else {
            assert!(accepted(result));
        }
    }
    let won_a = a_result["op"] == "result";
    let won_b = b_result["op"] == "result";
    assert_ne!(won_a, won_b);
    let owner = key(&state, nonce).await.unwrap();
    gelabber_media::live::release(&state.redis, nonce, Uuid::new_v4()).await;
    assert_eq!(key(&state, nonce).await.as_deref(), Some(owner.as_str()));
    let (winner, loser) = if won_a {
        (&mut a, &mut b)
    } else {
        (&mut b, &mut a)
    };
    winner.close().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while key(&state, nonce).await.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    loser
        .ok(produce("l", Uuid::new_v4(), None, Some(nonce)))
        .await;
    gelabber_media::live::release(&state.redis, nonce, owner.parse().unwrap()).await;
    assert!(key(&state, nonce).await.is_some());
    loser.close().await;
    cleanup(&state, &claim, nonce).await;
}

#[tokio::test]
async fn orphaned_peer_lease_is_busy_until_expiry_without_takeover_or_extension() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    let old_owner = Uuid::new_v4();
    let epoch = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, nonce), Some(5000)).await;
    let mut peer = publisher(addr, &code).await;
    // Reproduce the Redis lease left behind by a dead media process without
    // claiming that this native control test executed a process crash or RTP.
    set_peer_lease(&state, nonce, old_owner, Some(700)).await;
    let before = peer_ttl(&state, nonce).await;
    let busy = peer.rpc(produce("l", epoch, None, Some(nonce))).await;
    assert_eq!(busy["op"], "err");
    assert_eq!(busy["e"], "live_busy");
    assert_eq!(key(&state, nonce).await, Some(old_owner.to_string()));
    let after = peer_ttl(&state, nonce).await;
    assert!(
        after > 0 && after <= before,
        "busy must not extend peer TTL"
    );
    assert!(
        state
            .sfu
            .metrics_text()
            .lines()
            .any(|line| line == "gelabber_mediasoup_producers 0")
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while key(&state, nonce).await.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("unchanged old peer lease expires");
    let produced = peer.ok(produce("l", epoch, None, Some(nonce))).await;
    assert!(produced["producerId"].is_string());
    assert_eq!(produced["epoch"], epoch.to_string());
    let new_owner = key(&state, nonce).await.unwrap();
    assert_ne!(new_owner, old_owner.to_string());
    gelabber_media::live::release(&state.redis, nonce, old_owner).await;
    assert_eq!(key(&state, nonce).await, Some(new_owner));
    peer.close().await;
    cleanup(&state, &claim, nonce).await;
}

#[tokio::test]
async fn busy_requires_valid_authority_claim_and_a_bounded_foreign_peer_lease() {
    let (_, state) = serve().await;
    let (_, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    let old_owner = Uuid::new_v4();
    let new_owner = Uuid::new_v4();
    let record = live_record(&claim, nonce);
    set_live(&state, &claim, &record, Some(5000)).await;
    set_peer_lease(&state, nonce, old_owner, Some(3000)).await;
    assert_eq!(
        try_acquire(&state.redis, &claim, nonce, new_owner)
            .await
            .unwrap(),
        LiveAcquireOutcome::Busy
    );
    assert!(
        gelabber_media::live::validate_and_acquire(&state.redis, &claim, nonce, new_owner)
            .await
            .unwrap()
            .is_none()
    );
    let mut wrong_authority = claim.clone();
    wrong_authority.auth.member = Uuid::new_v4();
    assert_eq!(
        try_acquire(&state.redis, &wrong_authority, nonce, new_owner)
            .await
            .unwrap(),
        LiveAcquireOutcome::Denied
    );
    let mut wrong_claim = record.clone();
    wrong_claim["session"] = json!(Uuid::new_v4());
    set_live(&state, &claim, &wrong_claim, Some(5000)).await;
    assert_eq!(
        try_acquire(&state.redis, &claim, nonce, new_owner)
            .await
            .unwrap(),
        LiveAcquireOutcome::Denied
    );
    for ttl in [None, Some(6000)] {
        set_live(&state, &claim, &record, ttl).await;
        assert_eq!(
            try_acquire(&state.redis, &claim, nonce, new_owner)
                .await
                .unwrap(),
            LiveAcquireOutcome::Denied
        );
    }
    set_live(&state, &claim, &record, Some(5000)).await;
    for ttl in [None, Some(6000)] {
        set_peer_lease(&state, nonce, old_owner, ttl).await;
        assert_eq!(
            try_acquire(&state.redis, &claim, nonce, new_owner)
                .await
                .unwrap(),
            LiveAcquireOutcome::Denied
        );
        assert_eq!(key(&state, nonce).await, Some(old_owner.to_string()));
    }
    cleanup(&state, &claim, nonce).await;
}

#[tokio::test]
async fn short_valid_live_claim_is_busy_without_publication_and_can_retry_same_capture() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    let epoch = Uuid::new_v4();
    let record = live_record(&claim, nonce);
    let mut peer = publisher(addr, &code).await;
    set_live(&state, &claim, &record, Some(800)).await;
    let busy = peer.rpc(produce("l", epoch, None, Some(nonce))).await;
    assert_eq!(busy["op"], "err");
    assert_eq!(busy["e"], "live_busy");
    assert!(key(&state, nonce).await.is_none());
    assert!(
        state
            .sfu
            .metrics_text()
            .lines()
            .any(|line| line == "gelabber_mediasoup_producers 0")
    );
    set_live(&state, &claim, &record, Some(5000)).await;
    let produced = peer.ok(produce("l", epoch, None, Some(nonce))).await;
    assert_eq!(produced["epoch"], epoch.to_string());
    peer.close().await;
    cleanup(&state, &claim, nonce).await;
}
#[tokio::test]
async fn expired_live_claim_stops_live_only_and_fresh_claim_can_rejoin() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, nonce), Some(1800)).await;
    let mut peer = publisher(addr, &code).await;
    peer.ok(produce("l", Uuid::new_v4(), None, Some(nonce)))
        .await;
    let notice = peer.event("err").await;
    assert_eq!(notice["e"], "forbidden");
    assert_eq!(notice["lc"], nonce.to_string());
    assert!(key(&state, nonce).await.is_none());
    assert_eq!(state.sfu.room_count(), 1);
    peer.ok(produce("s", Uuid::new_v4(), None, None)).await;
    set_live(&state, &claim, &live_record(&claim, nonce), Some(5000)).await;
    assert!(!accepted(
        &peer
            .rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
            .await
    ));
    let next = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, next), Some(5000)).await;
    peer.ok(produce("l", Uuid::new_v4(), None, Some(next)))
        .await;
    peer.close().await;
    cleanup(&state, &claim, nonce).await;
    cleanup(&state, &claim, next).await;
}
#[tokio::test]
async fn watch_peer_cannot_publish_video_even_with_matching_authority() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, nonce), Some(5000)).await;
    let mut peer = Peer::join(addr, &code, Some(claim.claim.u)).await;
    assert!(!accepted(
        &peer
            .rpc(produce("l", Uuid::new_v4(), None, Some(nonce)))
            .await
    ));
    assert!(!accepted(
        &peer.rpc(produce("v", Uuid::new_v4(), None, None)).await
    ));
    assert!(key(&state, nonce).await.is_none());
    peer.close().await;
    cleanup(&state, &claim, nonce).await;
}
#[tokio::test]
async fn media_renews_its_peer_lease_but_never_recreates_the_gateway_claim() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    let record = live_record(&claim, nonce);
    set_live(&state, &claim, &record, Some(5000)).await;
    let mut peer = publisher(addr, &code).await;
    let live = peer
        .ok(produce("l", Uuid::new_v4(), None, Some(nonce)))
        .await;
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut conn = state
            .redis
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let refreshed:i64=redis::cmd("EVAL").arg("if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('PEXPIRE', KEYS[1], 5000) end return 0").arg(1).arg(format!("gb:live:{}",claim.claim.c)).arg(record.to_string()).query_async(&mut conn).await.unwrap();
        assert_eq!(refreshed, 1);
    }
    assert!(key(&state, nonce).await.is_some());
    peer.ok(json!({"op":"resumeProducer","producerId":live["producerId"]}))
        .await;
    cleanup(&state, &claim, nonce).await;
    tokio::time::timeout(Duration::from_millis(1600), peer.event("err"))
        .await
        .expect("claim loss next authority check");
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let exists: bool = redis::cmd("EXISTS")
        .arg(format!("gb:live:{}", claim.claim.c))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(!exists, "media must not recreate Gateway claim");
    assert_eq!(state.sfu.room_count(), 1);
    peer.close().await;
}
#[tokio::test]
async fn source_audio_shares_the_exact_parent_live_claim_and_cannot_outlive_it() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, nonce), Some(5000)).await;
    let mut peer = publisher(addr, &code).await;
    let epoch = Uuid::new_v4();
    assert!(!accepted(
        &peer.rpc(produce("la", epoch, None, Some(nonce))).await
    ));
    let parent = peer.ok(produce("l", epoch, None, Some(nonce))).await;
    let parent_id = parent["producerId"].as_str().unwrap();
    let owner = key(&state, nonce).await;
    assert!(!accepted(
        &peer
            .rpc(produce("la", epoch, Some(parent_id), Some(Uuid::new_v4())))
            .await
    ));
    peer.ok(produce("la", epoch, Some(parent_id), Some(nonce)))
        .await;
    assert_eq!(
        key(&state, nonce).await,
        owner,
        "audio retains exclusiveowner"
    );
    peer.ok(json!({"op":"closeProducer","producerId":parent_id}))
        .await;
    assert!(!accepted(
        &peer
            .rpc(produce("la", epoch, Some(parent_id), Some(nonce)))
            .await
    ));
    assert!(key(&state, nonce).await.is_none());
    peer.close().await;
    cleanup(&state, &claim, nonce).await;
}
