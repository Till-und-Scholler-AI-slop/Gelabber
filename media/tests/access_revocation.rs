//! Negative wire authorization regressions; unique local Redis test keys only.
use futures_util::{SinkExt, StreamExt};
use gelabber_media::{AppState, Config, app};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

async fn serve() -> (
    std::net::SocketAddr,
    redis::Client,
    std::sync::Arc<gelabber_media::sfu::Sfu>,
) {
    serve_at(std::env::var("REDIS_URL").expect("isolated REDIS_URL required")).await
}
async fn serve_at(
    url: String,
) -> (
    std::net::SocketAddr,
    redis::Client,
    std::sync::Arc<gelabber_media::sfu::Sfu>,
) {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(url.clone()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        "TURN_URLS" => Some("stun:127.0.0.1:3478".into()),
        _ => None,
    })
    .unwrap();
    let state = AppState::from_config(&config).unwrap();
    let redis = state.redis.clone();
    let sfu = state.sfu.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (addr, redis, sfu)
}

#[tokio::test]
async fn legacy_envelope_is_consumed_and_rejected_on_real_websocket() {
    let (addr, redis, sfu) = serve().await;
    let code = gelabber_shared::ticket::generate();
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let key = gelabber_shared::ticket::redis_key(&code);
    let _: () = redis::cmd("SET")
        .arg(&key)
        .arg(
            json!({"u": Uuid::new_v4(), "s": Uuid::new_v4(), "c": Uuid::new_v4(), "g": true})
                .to_string(),
        )
        .arg("EX")
        .arg(30)
        .query_async(&mut conn)
        .await
        .unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"op":"j", "tk":code}).to_string().into(),
    ))
    .await
    .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
    assert_eq!(
        value["e"], "unauthorized",
        "legacy envelope must not attach: {value}"
    );
    assert_eq!(sfu.room_count(), 0);
    let left: Option<String> = redis::cmd("GET")
        .arg(&key)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(left.is_none(), "invalid ticket is still one-use");
}

struct Authority {
    claim: gelabber_shared::ticket::AuthorizedTicketClaim,
    redis: redis::Client,
}
impl Authority {
    async fn new(redis: &redis::Client) -> Self {
        use gelabber_shared::ticket::{AuthorizedTicketClaim, TicketAuthorization, TicketClaim};
        let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
        let now: (u64, u64) = redis::cmd("TIME").query_async(&mut conn).await.unwrap();
        let claim = AuthorizedTicketClaim {
            claim: TicketClaim {
                u: Uuid::new_v4(),
                s: Uuid::new_v4(),
                c: Uuid::new_v4(),
                g: true,
            },
            auth: TicketAuthorization {
                session: Uuid::new_v4().simple().to_string().repeat(2),
                expires_at: now.0 + 60,
                member: Uuid::new_v4(),
                channel: Uuid::new_v4(),
            },
        };
        let authority = Self {
            claim,
            redis: redis.clone(),
        };
        authority.install().await;
        authority
    }
    fn keys(&self) -> Vec<String> {
        use gelabber_shared::ticket::*;
        vec![
            member_authority_key(self.claim.claim.s, self.claim.claim.u),
            channel_authority_key(self.claim.claim.c),
            session_authority_key(&self.claim.auth.session),
            media_demand_key(&self.claim.auth.session),
        ]
    }
    async fn set(&self, key: &str, value: &str) {
        let mut conn = self.redis.get_multiplexed_async_connection().await.unwrap();
        let _: () = redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async(&mut conn)
            .await
            .unwrap();
    }
    async fn install(&self) {
        let keys = self.keys();
        self.set(&keys[0], &self.claim.auth.member.to_string())
            .await;
        self.set(&keys[1], &self.claim.auth.channel.to_string())
            .await;
        let mut conn = self.redis.get_multiplexed_async_connection().await.unwrap();
        let _: () = redis::cmd("SET")
            .arg(&keys[2])
            .arg(self.claim.claim.u.to_string())
            .arg("PX")
            .arg(3000)
            .query_async(&mut conn)
            .await
            .unwrap();
    }
    async fn mint(&self) -> String {
        let code = gelabber_shared::ticket::generate();
        let mut conn = self.redis.get_multiplexed_async_connection().await.unwrap();
        let _: () = redis::cmd("SET")
            .arg(gelabber_shared::ticket::redis_key(&code))
            .arg(serde_json::to_string(&self.claim).unwrap())
            .arg("EX")
            .arg(30)
            .query_async(&mut conn)
            .await
            .unwrap();
        code
    }
    async fn cleanup(&self) {
        let mut conn = self.redis.get_multiplexed_async_connection().await.unwrap();
        let _: () = redis::cmd("DEL")
            .arg(self.keys())
            .query_async(&mut conn)
            .await
            .unwrap();
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn join(addr: std::net::SocketAddr, code: &str) -> (Socket, serde_json::Value) {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"op":"j","tk":code}).to_string().into(),
    ))
    .await
    .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let value = serde_json::from_str(response.to_text().unwrap()).unwrap();
    (ws, value)
}
async fn revoked(ws: &mut Socket, budget: Duration) {
    tokio::time::timeout(budget, async {
        loop {
            let msg = ws
                .next()
                .await
                .expect("server must send revocation")
                .unwrap();
            if msg.is_text() {
                let value: serde_json::Value =
                    serde_json::from_str(msg.to_text().unwrap()).unwrap();
                if value["e"] == "unauthorized" {
                    break;
                }
            }
        }
        assert!(
            matches!(
                ws.next().await,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None
            ),
            "server closes without voluntary client leave"
        );
    })
    .await
    .expect("bounded server-side revocation");
}

#[tokio::test]
async fn rejects_consumed_ticket_after_authority_rotation_and_allows_immediate_rejoin() {
    let (addr, redis, sfu) = serve().await;
    let mut authority = Authority::new(&redis).await;
    let old = authority.mint().await;
    let (mut ws, value) = join(addr, &old).await;
    assert_eq!(value["op"], "ok");
    let (_, replay) = join(addr, &old).await;
    assert_eq!(replay["e"], "unauthorized");
    let outstanding = authority.mint().await;
    authority.claim.auth.member = Uuid::new_v4();
    authority.install().await;
    revoked(&mut ws, Duration::from_millis(1700)).await;
    let (_, stale) = join(addr, &outstanding).await;
    assert_eq!(stale["e"], "unauthorized");
    // Retained API compatibility deny must not block the new incarnation.
    authority
        .set(
            &gelabber_shared::ticket::deny_key(authority.claim.claim.s, authority.claim.claim.u),
            "1",
        )
        .await;
    let fresh = authority.mint().await;
    let (mut next, accepted) = join(addr, &fresh).await;
    assert_eq!(accepted["op"], "ok");
    next.close(None).await.unwrap();
    authority.cleanup().await;
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("DEL")
        .arg(gelabber_shared::ticket::deny_key(
            authority.claim.claim.s,
            authority.claim.claim.u,
        ))
        .query_async(&mut conn)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(sfu.room_count(), 0);
}

#[tokio::test]
async fn channel_rotation_revokes_open_peer_and_old_unconsumed_ticket() {
    let (addr, redis, _) = serve().await;
    let authority = Authority::new(&redis).await;
    let (mut ws, value) = join(addr, &authority.mint().await).await;
    assert_eq!(value["op"], "ok");
    let outstanding = authority.mint().await;
    authority
        .set(&authority.keys()[1], &Uuid::new_v4().to_string())
        .await;
    revoked(&mut ws, Duration::from_millis(1700)).await;
    let (_, stale) = join(addr, &outstanding).await;
    assert_eq!(stale["e"], "unauthorized");
    authority.cleanup().await;
}

#[tokio::test]
async fn exact_session_logout_does_not_revoke_second_session_of_same_user() {
    let (addr, redis, _) = serve().await;
    let a = Authority::new(&redis).await;
    let mut b = Authority::new(&redis).await;
    b.cleanup().await;
    b.claim.claim = a.claim.claim.clone();
    b.claim.auth.member = a.claim.auth.member;
    b.claim.auth.channel = a.claim.auth.channel;
    b.install().await;
    let (mut first, value) = join(addr, &a.mint().await).await;
    assert_eq!(value["op"], "ok");
    let (mut second, value) = join(addr, &b.mint().await).await;
    assert_eq!(value["op"], "ok");
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("DEL")
        .arg(&a.keys()[2])
        .query_async(&mut conn)
        .await
        .unwrap();
    revoked(&mut first, Duration::from_millis(1700)).await;
    second
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"op":"o","sdp":"invalid"}).to_string().into(),
        ))
        .await
        .unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(1), second.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(msg.to_text().unwrap()).unwrap()["e"],
        "negotiation_failed",
        "second exact session remains authorized"
    );
    second.close(None).await.unwrap();
    a.cleanup().await;
    b.cleanup().await;
}

#[tokio::test]
async fn authority_lease_expiry_closes_open_peer_within_three_seconds_plus_check_interval() {
    let (addr, redis, sfu) = serve().await;
    let authority = Authority::new(&redis).await;
    let started = tokio::time::Instant::now();
    let (mut ws, value) = join(addr, &authority.mint().await).await;
    assert_eq!(value["op"], "ok");
    revoked(&mut ws, Duration::from_millis(4500)).await;
    assert!(started.elapsed() < Duration::from_millis(4500));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(sfu.room_count(), 0);
    authority.cleanup().await;
}

#[tokio::test]
async fn watch_only_peer_refreshes_demand_without_renewing_session_and_keeps_other_tab_demand() {
    let (addr, redis, _) = serve().await;
    let authority = Authority::new(&redis).await;
    let (mut a, value) = join(addr, &authority.mint().await).await;
    assert_eq!(value["op"], "ok");
    let (mut b, value) = join(addr, &authority.mint().await).await;
    assert_eq!(value["op"], "ok");
    let keys = authority.keys();
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("EXPIRE")
        .arg(&keys[3])
        .arg(1)
        .query_async(&mut conn)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1250)).await;
    let demand: i64 = redis::cmd("TTL")
        .arg(&keys[3])
        .query_async(&mut conn)
        .await
        .unwrap();
    let lease: i64 = redis::cmd("PTTL")
        .arg(&keys[2])
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(demand >= 33);
    assert!(
        (1..1900).contains(&lease),
        "media must not renew API authority: {lease}"
    );
    a.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let demand: Option<String> = redis::cmd("GET")
        .arg(&keys[3])
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(demand.as_deref(), Some("1"));
    b.close(None).await.unwrap();
    authority.cleanup().await;
}

#[tokio::test]
async fn absolute_expiry_and_malformed_authorization_never_attach() {
    let (addr, redis, sfu) = serve().await;
    let authority = Authority::new(&redis).await;
    let now: (u64, u64) = redis::cmd("TIME")
        .query_async(&mut redis.get_multiplexed_async_connection().await.unwrap())
        .await
        .unwrap();
    let raw = serde_json::to_value(&authority.claim).unwrap();
    let mut expired = raw.clone();
    expired["auth"]["expires_at"] = json!(now.0 - 1);
    let mut wrong_user = raw.clone();
    wrong_user["u"] = json!(Uuid::new_v4());
    let mut invalid_hash = raw.clone();
    invalid_hash["auth"]["session"] = json!("F".repeat(64));
    let mut nil_nonce = raw.clone();
    nil_nonce["auth"]["member"] = json!(Uuid::nil());
    let mut missing = raw.clone();
    missing.as_object_mut().unwrap().remove("auth");
    let mut imprecise = raw.clone();
    imprecise["auth"]["expires_at"] = json!(9_007_199_254_740_992u64);
    for raw in [
        expired,
        wrong_user,
        invalid_hash,
        nil_nonce,
        missing,
        imprecise,
    ] {
        let code = gelabber_shared::ticket::generate();
        let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
        let _: () = redis::cmd("SET")
            .arg(gelabber_shared::ticket::redis_key(&code))
            .arg(raw.to_string())
            .arg("EX")
            .arg(30)
            .query_async(&mut conn)
            .await
            .unwrap();
        let (_, value) = join(addr, &code).await;
        assert_eq!(value["e"], "unauthorized");
    }
    assert_eq!(sfu.room_count(), 0);
    authority.cleanup().await;
}

#[tokio::test]
async fn consumed_grant_is_rechecked_before_sfu_attach() {
    let (_, redis, sfu) = serve().await;
    let authority = Authority::new(&redis).await;
    let consumed = gelabber_media::ticket::consume(&redis, &authority.mint().await)
        .await
        .unwrap()
        .unwrap();
    authority
        .set(&authority.keys()[0], &Uuid::new_v4().to_string())
        .await;
    let (out, _) = tokio::sync::mpsc::unbounded_channel();
    assert!(matches!(
        sfu.join_authorized(consumed, out).await,
        Err(gelabber_media::error::SfuError::Revoked)
    ));
    assert_eq!(sfu.room_count(), 0);
    authority.cleanup().await;
}

#[tokio::test]
async fn absolute_session_expiry_revokes_even_while_lease_still_exists() {
    let (addr, redis, _) = serve().await;
    let mut authority = Authority::new(&redis).await;
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let now: (u64, u64) = redis::cmd("TIME").query_async(&mut conn).await.unwrap();
    authority.claim.auth.expires_at = now.0 + 2;
    let (mut ws, value) = join(addr, &authority.mint().await).await;
    assert_eq!(value["op"], "ok");
    revoked(&mut ws, Duration::from_millis(2700)).await;
    let lease: i64 = redis::cmd("PTTL")
        .arg(&authority.keys()[2])
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(
        lease > 0,
        "absolute expiration is enforced before authority TTL"
    );
    authority.cleanup().await;
}

#[tokio::test]
async fn redis_failure_revokes_open_peer_and_rejects_new_ticket_without_affecting_control() {
    let url = std::env::var("REDIS_URL").unwrap();
    let upstream = redis::Client::open(url.clone()).unwrap();
    let connection_info = upstream.get_connection_info();
    let redis::ConnectionAddr::Tcp(host, port) = connection_info.addr() else {
        panic!("local TCP Redis required");
    };
    let target = (host.clone(), *port);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = listener.local_addr().unwrap();
    let (cut, mut stop) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                incoming = listener.accept() => {
                    let (mut client, _) = incoming.unwrap();
                    let target = target.clone(); let mut stop = stop.clone();
                    tokio::spawn(async move {
                        let mut upstream = tokio::net::TcpStream::connect(target).await.unwrap();
                        tokio::select! {
                            _ = stop.changed() => {},
                            _ = tokio::io::copy_bidirectional(&mut client, &mut upstream) => {},
                        }
                    });
                }
            }
        }
    });
    let (addr, proxy_redis, sfu) = serve_at(format!("redis://{proxy}")).await;
    let mut authority = Authority::new(&proxy_redis).await;
    let (mut ws, value) = join(addr, &authority.mint().await).await;
    assert_eq!(value["op"], "ok");
    let outstanding = authority.mint().await;
    let (control_addr, direct, _) = serve().await;
    let control = Authority::new(&direct).await;
    let (mut control_ws, value) = join(control_addr, &control.mint().await).await;
    assert_eq!(value["op"], "ok");
    cut.send_replace(true);
    revoked(&mut ws, Duration::from_millis(1700)).await;
    let (_, value) = join(addr, &outstanding).await;
    assert_eq!(value["e"], "unauthorized");
    control_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"op":"o","sdp":"invalid"}).to_string().into(),
        ))
        .await
        .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(1), control_ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(message.to_text().unwrap()).unwrap()["e"],
        "negotiation_failed"
    );
    control_ws.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(sfu.room_count(), 0);
    authority.redis = upstream;
    authority.cleanup().await;
    control.cleanup().await;
}
