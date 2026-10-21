#[path = "support/authority.rs"]
mod authority;
#[path = "support/control.rs"]
mod control;
// `/media/ws` closes sockets that hold no joined room past MEDIA_JOIN_DEADLINE_MS.

use std::time::Duration;

use control::{Peer, Socket};
use futures_util::{SinkExt, StreamExt};
use gelabber_media::{Config, app};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

async fn serve(join_deadline_ms: &'static str) -> (std::net::SocketAddr, redis::Client) {
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
        "MEDIA_JOIN_DEADLINE_MS" => Some(join_deadline_ms.to_owned()),
        _ => None,
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
        axum::serve(listener, app(state)).await.expect("serve");
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

async fn connect(addr: std::net::SocketAddr) -> Socket {
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .expect("ws");
    socket
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
async fn silent_socket_is_closed_at_the_deadline() {
    let (addr, _) = serve("200").await;
    let mut socket = connect(addr).await;
    closes_within(&mut socket, Duration::from_secs(2)).await;
}

#[tokio::test]
async fn pings_do_not_extend_the_deadline() {
    let (addr, _) = serve("200").await;
    let mut socket = connect(addr).await;
    let mut ping = tokio::time::interval(Duration::from_millis(50));
    let ended = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                _ = ping.tick() => {
                    if socket.send(Message::Ping(Default::default())).await.is_err() { break; }
                }
                message = socket.next() => match message {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                },
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "pinging socket still open after 2s");
}

#[tokio::test]
async fn joined_socket_outlives_the_deadline() {
    let (addr, redis) = serve("200").await;
    let (code, _lease) = ticket(&redis).await;
    let mut peer = Peer::join(addr, &code, None).await;
    // Four deadlines pass; whatever the room sends meanwhile is no Close.
    let ended = tokio::time::timeout(Duration::from_millis(800), async {
        loop {
            match peer.socket.next().await {
                Some(Ok(Message::Text(raw))) => {
                    peer.events.push_back(serde_json::from_str(&raw).unwrap());
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(ended.is_err(), "joined socket was closed within 800ms");
    let capabilities = peer.capabilities.clone();
    peer.ok(json!({"op":"capabilities","rtp":capabilities}))
        .await;
    peer.close().await;
}

#[tokio::test]
async fn left_socket_is_closed_at_the_deadline() {
    let (addr, redis) = serve("200").await;
    let (code, _lease) = ticket(&redis).await;
    let mut peer = Peer::join(addr, &code, None).await;
    peer.ok(json!({"op":"l"})).await;
    closes_within(&mut peer.socket, Duration::from_secs(2)).await;
}

#[tokio::test]
async fn leave_starts_a_fresh_deadline_for_a_rejoin() {
    let (addr, redis) = serve("1000").await;
    let (first, _first) = ticket(&redis).await;
    let (second, _second) = ticket(&redis).await;
    let mut peer = Peer::join(addr, &first, None).await;
    // The deadline from the upgrade passes while the socket is joined.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    peer.ok(json!({"op":"l"})).await;
    let rejoined = peer.rpc(json!({"op":"j","v":4,"tk":second})).await;
    assert_eq!(rejoined["op"], "result", "rejoin: {rejoined}");
    peer.close().await;
}
