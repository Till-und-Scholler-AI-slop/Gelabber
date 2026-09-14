//! Join the media WS only with a short internal ticket.

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
    let state = gelabber_media::AppState::from_config(&config).expect("state");
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
    ws.send(Message::Text(r#"{"op":"j"}"#.into()))
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
    let user = uuid::Uuid::from_u128(1);
    let server = uuid::Uuid::from_u128(2);
    let channel = uuid::Uuid::from_u128(3);
    let code = "abcdefghjkmn";
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("SET")
        .arg(format!("gb:mt:{code}"))
        .arg(format!(
            r#"{{"u":"{user}","s":"{server}","c":"{channel}"}}"#
        ))
        .arg("EX")
        .arg(30)
        .query_async(&mut conn)
        .await
        .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
        .await
        .expect("ws");
    ws.send(Message::Text(
        format!(r#"{{"op":"j","tk":"{code}"}}"#).into(),
    ))
    .await
    .unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("frame")
        .expect("ok")
        .expect("text");
    let text = msg.to_text().unwrap();
    assert!(text.contains(r#""op":"ok""#), "{text}");
    assert!(text.contains(&channel.to_string()), "{text}");
    assert!(!text.contains("livekit"));

    let leftover: Option<String> = redis::cmd("GET")
        .arg(format!("gb:mt:{code}"))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(leftover.is_none(), "ticket is single-use");
}
