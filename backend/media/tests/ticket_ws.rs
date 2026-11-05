#[path = "support/authority.rs"]
mod authority;
// Join the media WS only with an authorized short internal ticket.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use gelabber_media::{Config, app};
use tokio_tungstenite::tungstenite::Message;

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

async fn serve() -> (std::net::SocketAddr, redis::Client) {
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
        "TURN_URLS" => Some("stun:127.0.0.1:3478,turn:127.0.0.1:3478".to_owned()),
        "TURN_USERNAME" => Some("gelabber".to_owned()),
        "TURN_PASSWORD" => Some("gelabberturn".to_owned()),
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

#[tokio::test]
async fn rejects_join_without_ticket() {
    let (addr, _) = serve().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .expect("ws");
    ws.send(Message::Text(r#"{"op":"j","id":1,"v":4}"#.into()))
        .await
        .unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("frame")
        .expect("ok")
        .expect("text");
    let text = msg.to_text().unwrap();
    assert!(text.contains(r#""e":"unauthorized""#), "{text}");
}

#[tokio::test]
async fn accepts_short_ticket_and_binds_room() {
    let (addr, redis) = serve().await;
    let user = uuid::Uuid::new_v4();
    let server = uuid::Uuid::new_v4();
    let channel = uuid::Uuid::new_v4();
    let code = gelabber_shared::ticket::generate();
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let _authority = authority::mint(
        &redis,
        &code,
        gelabber_shared::ticket::TicketClaim {
            u: user,
            s: server,
            c: channel,
            g: false,
        },
    )
    .await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .expect("ws");
    ws.send(Message::Text(
        format!(r#"{{"op":"j","id":1,"v":4,"tk":"{code}"}}"#).into(),
    ))
    .await
    .unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("frame")
        .expect("ok")
        .expect("text");
    let text = msg.to_text().unwrap();
    assert!(text.contains(r#""op":"result""#), "{text}");
    assert!(text.contains(&channel.to_string()), "{text}");
    assert!(!text.contains("livekit"));

    let leftover: Option<String> = redis::cmd("GET")
        .arg(format!("gb:mt:{code}"))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(leftover.is_none(), "ticket is single-use");
}

#[tokio::test]
async fn old_media_protocol_is_rejected_without_consuming_ticket() {
    let (addr, redis) = serve().await;
    let code = gelabber_shared::ticket::generate();
    let _lease = authority::mint(
        &redis,
        &code,
        gelabber_shared::ticket::TicketClaim {
            u: uuid::Uuid::new_v4(),
            s: uuid::Uuid::new_v4(),
            c: uuid::Uuid::new_v4(),
            g: false,
        },
    )
    .await;
    let (mut old, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .unwrap();
    old.send(Message::Text(
        format!(r#"{{"op":"j","tk":"{code}","v":2}}"#).into(),
    ))
    .await
    .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), old.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let error: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(error["e"], "update_required");
    let left: Option<String> = redis::cmd("GET")
        .arg(gelabber_shared::ticket::redis_key(&code))
        .query_async(&mut redis.get_multiplexed_async_connection().await.unwrap())
        .await
        .unwrap();
    assert!(
        left.is_some(),
        "version rejection must not burn a valid one-use ticket"
    );
    let (mut current, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .unwrap();
    current
        .send(Message::Text(
            format!(r#"{{"op":"j","id":1,"tk":"{code}","v":4}}"#).into(),
        ))
        .await
        .unwrap();
    let accepted = tokio::time::timeout(Duration::from_secs(3), current.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(accepted.to_text().unwrap()).unwrap();
    assert_eq!(value["op"], "result");
    assert_eq!(value["data"]["v"], 4);
    current.close(None).await.unwrap();
}
