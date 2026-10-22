#[path = "support/authority.rs"]
mod authority;
#[path = "support/control.rs"]
mod control;
// `/media/ws` caps sockets without a joined room per client address and in
// total, and evicts joined peers that answer no heartbeat (#286).

use std::net::SocketAddr;
use std::time::Duration;

use control::{Peer, Socket};
use futures_util::StreamExt;
use gelabber_media::{Config, app};
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error, Message};

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

/// Serves media with the peer address as the production binary does.
async fn serve(overrides: &'static [(&'static str, &'static str)]) -> (SocketAddr, redis::Client) {
    let redis_url = redis_url();
    let redis = redis::Client::open(redis_url.as_str()).unwrap_or_else(|err| {
        panic!("REDIS_URL must be a redis URL ({redis_url}): {err}");
    });
    redis
        .get_multiplexed_async_connection()
        .await
        .unwrap_or_else(|err| {
            panic!("REDIS_URL must be reachable ({redis_url}): {err}");
        });
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(redis_url.clone()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        _ => overrides
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| (*value).to_owned()),
    })
    .expect("config");
    let state = gelabber_media::AppState::from_config(&config)
        .await
        .expect("state");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind media test listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("serve");
    });
    (addr, redis)
}

async fn ticket(redis: &redis::Client) -> (String, authority::TestAuthority) {
    let code = gelabber_shared::ticket::generate();
    let lease = authority::mint(
        redis,
        &code,
        gelabber_shared::ticket::TicketClaim {
            u: uuid::Uuid::new_v4(),
            s: uuid::Uuid::new_v4(),
            c: uuid::Uuid::new_v4(),
            g: false,
        },
    )
    .await;
    (code, lease)
}

/// Opens `/media/ws` as the client `from` (an `X-Forwarded-For` entry), or as
/// the TCP peer without one. `Err` carries the HTTP status of a refusal.
async fn open(addr: SocketAddr, from: Option<&str>) -> Result<Socket, u16> {
    let mut request = format!("ws://{addr}/media/ws")
        .into_client_request()
        .expect("request");
    if let Some(ip) = from {
        request
            .headers_mut()
            .insert("x-forwarded-for", ip.parse().expect("header"));
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(Error::Http(response)) => Err(response.status().as_u16()),
        Err(err) => panic!("ws: {err}"),
    }
}

/// Retries until the server has given a closed socket's slot back.
async fn opens_within(addr: SocketAddr, from: Option<&str>, limit: Duration) -> Socket {
    tokio::time::timeout(limit, async {
        loop {
            if let Ok(socket) = open(addr, from).await {
                return socket;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no slot free after {limit:?}"))
}

/// Reads until the server ends the socket: a Close frame, an error or EOF.
async fn closes_within(socket: &mut Socket, limit: Duration) {
    let ended = tokio::time::timeout(limit, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "socket still open after {limit:?}");
}

#[tokio::test]
async fn each_address_holds_its_share_of_pending_sockets() {
    let (addr, _) = serve(&[("MEDIA_PENDING_PER_IP", "2")]).await;
    let mut first = open(addr, Some("198.51.100.1")).await.expect("first");
    let _second = open(addr, Some("198.51.100.1")).await.expect("second");
    assert_eq!(open(addr, Some("198.51.100.1")).await.err(), Some(429));
    // Only the rightmost entry counts: the client wrote the ones to its left.
    assert_eq!(
        open(addr, Some("203.0.113.9, 198.51.100.1")).await.err(),
        Some(429)
    );
    let _other = open(addr, Some("198.51.100.2"))
        .await
        .expect("another address has its own share");
    first.close(None).await.expect("close");
    let _again = opens_within(addr, Some("198.51.100.1"), Duration::from_secs(2)).await;
}

#[tokio::test]
async fn without_forwarded_for_the_peer_address_counts() {
    let (addr, _) = serve(&[("MEDIA_PENDING_PER_IP", "1")]).await;
    let _first = open(addr, None).await.expect("first");
    assert_eq!(open(addr, None).await.err(), Some(429));
    let _forwarded = open(addr, Some("198.51.100.1"))
        .await
        .expect("a forwarded client is not the proxy's peer");
}

#[tokio::test]
async fn the_valve_refuses_every_address_when_full() {
    let (addr, _) = serve(&[("MEDIA_PENDING_MAX", "2")]).await;
    let _one = open(addr, Some("198.51.100.1")).await.expect("one");
    let _two = open(addr, Some("198.51.100.2")).await.expect("two");
    assert_eq!(open(addr, Some("198.51.100.3")).await.err(), Some(503));
}

#[tokio::test]
async fn the_join_deadline_gives_the_slot_back() {
    let (addr, _) = serve(&[
        ("MEDIA_PENDING_PER_IP", "1"),
        ("MEDIA_JOIN_DEADLINE_MS", "200"),
    ])
    .await;
    let mut silent = open(addr, None).await.expect("first");
    closes_within(&mut silent, Duration::from_secs(2)).await;
    let _next = opens_within(addr, None, Duration::from_secs(2)).await;
}

#[tokio::test]
async fn a_joined_socket_gives_its_slot_back() {
    let (addr, redis) = serve(&[("MEDIA_PENDING_PER_IP", "1")]).await;
    let (code, _lease) = ticket(&redis).await;
    let mut peer = Peer::join(addr, &code, None).await;
    let _pending = open(addr, None)
        .await
        .expect("the joined socket no longer counts");
    peer.close().await;
}

#[tokio::test]
async fn a_socket_that_leaves_needs_a_slot_again() {
    let (addr, redis) = serve(&[("MEDIA_PENDING_PER_IP", "1")]).await;
    let (code, _lease) = ticket(&redis).await;
    let mut peer = Peer::join(addr, &code, None).await;
    // Takes the address's only slot while the peer is joined.
    let _pending = open(addr, None).await.expect("pending");
    let left = peer.rpc(json!({"op":"l"})).await;
    assert_eq!(left["op"], "result", "leave: {left}");
    closes_within(&mut peer.socket, Duration::from_secs(2)).await;
}

#[tokio::test]
async fn a_peer_that_answers_pings_stays() {
    let (addr, redis) = serve(&[
        ("MEDIA_HEARTBEAT_MS", "100"),
        ("MEDIA_IDLE_TIMEOUT_MS", "300"),
    ])
    .await;
    let (code, _lease) = ticket(&redis).await;
    let mut peer = Peer::join(addr, &code, None).await;
    let mut pings = 0;
    // Reading answers each Ping (tungstenite queues the Pong on its own).
    let ended = tokio::time::timeout(Duration::from_millis(1000), async {
        loop {
            match peer.socket.next().await {
                Some(Ok(Message::Ping(_))) => pings += 1,
                Some(Ok(Message::Text(raw))) => {
                    peer.events.push_back(serde_json::from_str(&raw).unwrap());
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(ended.is_err(), "answering peer was closed within 1s");
    assert!(pings >= 3, "only {pings} pings in 1s at a 100ms heartbeat");
    let capabilities = peer.capabilities.clone();
    peer.ok(json!({"op":"capabilities","rtp":capabilities}))
        .await;
    peer.close().await;
}

#[tokio::test]
async fn a_silent_peer_is_closed_after_the_idle_timeout() {
    let (addr, redis) = serve(&[
        ("MEDIA_HEARTBEAT_MS", "100"),
        ("MEDIA_IDLE_TIMEOUT_MS", "300"),
    ])
    .await;
    let (code, _lease) = ticket(&redis).await;
    let mut peer = Peer::join(addr, &code, None).await;
    // Not reading means no Pong goes out: a half-open peer as the SFU sees it.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    closes_within(&mut peer.socket, Duration::from_secs(1)).await;
}
