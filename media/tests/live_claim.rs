//! Real WebSocket/Redis Live claim boundaries; only UUID-scoped test keys.
use futures_util::{SinkExt, StreamExt};
use gelabber_media::{AppState, Config, app};
use gelabber_shared::ticket::{self, AuthorizedTicketClaim, TicketClaim};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;
#[path = "support/authority.rs"]
mod authority;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn serve() -> (std::net::SocketAddr, AppState) {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(std::env::var(key).unwrap()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        _ => None,
    })
    .unwrap();
    let state = AppState::from_config(&config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let runtime = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(runtime)).await.unwrap();
    });
    (addr, state)
}
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
async fn join(addr: std::net::SocketAddr, code: &str, watch: Option<Uuid>) -> Socket {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .unwrap();
    ws.send(Message::Text(
        json!({"op":"j","tk":code,"w":watch}).to_string().into(),
    ))
    .await
    .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ack: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(ack["op"], "ok");
    ws
}
async fn announce(ws: &mut Socket, kind: &str, track: &str, nonce: Option<Uuid>) -> bool {
    ws.send(Message::Text(
        json!({"op":"p","k":kind,"t":track,"lc":nonce})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let marker = Uuid::new_v4().as_bytes().to_vec();
    ws.send(Message::Ping(marker.clone().into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(raw) => {
                    let frame: Value = serde_json::from_str(&raw).unwrap();
                    if frame["op"] == "err" {
                        assert_eq!(frame["e"], "forbidden");
                        return false;
                    }
                }
                Message::Pong(payload) if payload.as_ref() == marker => return true,
                _ => {}
            }
        }
    })
    .await
    .unwrap()
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

#[tokio::test]
async fn ticket_bit_alone_and_mismatched_live_claims_are_rejected_on_wire() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    let mut ws = join(addr, &code, None).await;
    assert!(!announce(&mut ws, "l", "video", Some(nonce)).await);
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
            !announce(&mut ws, "l", "video", Some(nonce)).await,
            "mismatched {field}"
        );
        assert!(key(&state, nonce).await.is_none());
    }
    set_live(&state, &claim, &good, None).await;
    assert!(
        !announce(&mut ws, "l", "video", Some(nonce)).await,
        "unleased record"
    );
    for seat in ["", "invalid-seat", "00000000-0000-0000-0000-000000000000"] {
        let mut bad = good.clone();
        bad["seat"] = json!(seat);
        set_live(&state, &claim, &bad, Some(5000)).await;
        assert!(!announce(&mut ws, "l", "video", Some(nonce)).await);
    }
    set_live(&state, &claim, &good, Some(5000)).await;
    assert!(!announce(&mut ws, "l", "video", None).await);
    assert!(announce(&mut ws, "l", "video", Some(nonce)).await);
    assert!(!announce(&mut ws, "l", "second-video", Some(nonce)).await);
    assert!(announce(&mut ws, "v", "camera", None).await);
    ws.close(None).await.unwrap();
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
    let mut a = join(addr, &code, None).await;
    let mut b = join(addr, &another, None).await;
    let (accepted_a, accepted_b) = tokio::join!(
        announce(&mut a, "l", "a", Some(nonce)),
        announce(&mut b, "l", "b", Some(nonce))
    );
    assert_ne!(accepted_a, accepted_b);
    let owner = key(&state, nonce).await.unwrap();
    gelabber_media::live::release(&state.redis, nonce, Uuid::new_v4()).await;
    assert_eq!(key(&state, nonce).await.as_deref(), Some(owner.as_str()));
    let (winner, loser) = if accepted_a {
        (&mut a, &mut b)
    } else {
        (&mut b, &mut a)
    };
    winner.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while key(&state, nonce).await.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(announce(loser, "l", "recovery", Some(nonce)).await);
    gelabber_media::live::release(&state.redis, nonce, owner.parse().unwrap()).await;
    assert!(key(&state, nonce).await.is_some());
    loser.close(None).await.unwrap();
    cleanup(&state, &claim, nonce).await;
}

#[tokio::test]
async fn expired_live_claim_stops_live_only_and_fresh_claim_can_rejoin() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, nonce), Some(1200)).await;
    let mut ws = join(addr, &code, None).await;
    assert!(announce(&mut ws, "l", "video", Some(nonce)).await);
    tokio::time::timeout(Duration::from_millis(3000), async {
        loop {
            if let Message::Text(raw) = ws.next().await.unwrap().unwrap() {
                let frame: Value = serde_json::from_str(&raw).unwrap();
                if frame["e"] == "forbidden" {
                    assert_eq!(frame["lc"], nonce.to_string());
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(key(&state, nonce).await.is_none());
    assert_eq!(state.sfu.room_count(), 1);
    assert!(announce(&mut ws, "s", "screen", None).await);
    // Restoring the old JSON cannot resurrect a withdrawn publication handshake.
    set_live(&state, &claim, &live_record(&claim, nonce), Some(5000)).await;
    assert!(!announce(&mut ws, "l", "video", Some(nonce)).await);
    let next = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, next), Some(5000)).await;
    assert!(announce(&mut ws, "l", "video", Some(next)).await);
    ws.close(None).await.unwrap();
    cleanup(&state, &claim, nonce).await;
    cleanup(&state, &claim, next).await;
}

#[tokio::test]
async fn watch_peer_cannot_publish_video_even_with_matching_authority() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    set_live(&state, &claim, &live_record(&claim, nonce), Some(5000)).await;
    let mut ws = join(addr, &code, Some(claim.claim.u)).await;
    assert!(!announce(&mut ws, "l", "video", Some(nonce)).await);
    assert!(!announce(&mut ws, "v", "camera", None).await);
    assert!(key(&state, nonce).await.is_none());
    ws.close(None).await.unwrap();
    cleanup(&state, &claim, nonce).await;
}

#[tokio::test]
async fn media_renews_its_peer_lease_but_never_recreates_the_gateway_claim() {
    let (addr, state) = serve().await;
    let (code, claim, _lease) = fixture(&state).await;
    let nonce = Uuid::new_v4();
    let record = live_record(&claim, nonce);
    set_live(&state, &claim, &record, Some(5000)).await;
    let mut ws = join(addr, &code, None).await;
    assert!(announce(&mut ws, "l", "live", Some(nonce)).await);
    // API owner refresh, beyond the initial 5s TTL; media can refresh only CAS.
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut conn = state
            .redis
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let refreshed: i64 = redis::cmd("EVAL")
            .arg("if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('PEXPIRE', KEYS[1], 5000) end return 0")
            .arg(1).arg(format!("gb:live:{}", claim.claim.c)).arg(record.to_string()).query_async(&mut conn).await.unwrap();
        assert_eq!(refreshed, 1);
    }
    assert!(key(&state, nonce).await.is_some());
    assert!(announce(&mut ws, "l", "live", Some(nonce)).await);
    cleanup(&state, &claim, nonce).await;
    tokio::time::timeout(Duration::from_millis(1600), async {
        loop {
            if let Message::Text(raw) = ws.next().await.unwrap().unwrap() {
                let frame: Value = serde_json::from_str(&raw).unwrap();
                if frame["e"] == "forbidden" {
                    break;
                }
            }
        }
    })
    .await
    .expect("claim loss stops Live on the next authority check");
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
    assert!(!exists, "media must not recreate a Gateway claim");
    assert_eq!(state.sfu.room_count(), 1);
    ws.close(None).await.unwrap();
}
