//! Real mediasoup Worker + authenticated WebSocket control fixtures.
//! These fixtures exercise native resources, not decoded packet forwarding.
#![allow(dead_code)]
use futures_util::{SinkExt, StreamExt};
use gelabber_media::{AppState, Config, app};
use serde_json::{Value, json};
use std::{collections::VecDeque, time::Duration};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
pub async fn serve() -> (std::net::SocketAddr, AppState) {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(std::env::var(key).expect("isolated REDIS_URL required")),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        _ => None,
    })
    .unwrap();
    let state = AppState::from_config(&config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let runtime = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(runtime)).await.unwrap();
    });
    (addr, state)
}
pub struct Peer {
    pub socket: Socket,
    pub next_id: u32,
    pub capabilities: Value,
    pub events: VecDeque<Value>,
}
impl Peer {
    pub async fn join(addr: std::net::SocketAddr, code: &str, watch: Option<Uuid>) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/media/ws"))
            .await
            .unwrap();
        let mut peer = Self {
            socket,
            next_id: 0,
            capabilities: Value::Null,
            events: VecDeque::new(),
        };
        let response = peer.rpc(json!({"op":"j","v":4,"tk":code,"w":watch})).await;
        assert_eq!(response["op"], "result", "join: {response}");
        assert_eq!(response["data"]["v"], 4);
        peer.capabilities = response["data"]["routerRtpCapabilities"].clone();
        assert!(
            peer.capabilities["codecs"]
                .as_array()
                .is_some_and(|c| !c.is_empty())
        );
        peer
    }
    pub async fn rpc(&mut self, mut request: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        request["id"] = json!(id);
        self.socket
            .send(Message::Text(request.to_string().into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let msg = self
                    .socket
                    .next()
                    .await
                    .expect("socket remains open")
                    .unwrap();
                if let Message::Text(raw) = msg {
                    let frame: Value = serde_json::from_str(&raw).unwrap();
                    if frame["id"] == id {
                        return frame;
                    }
                    self.events.push_back(frame);
                }
            }
        })
        .await
        .expect("bounded native control request")
    }
    pub async fn ok(&mut self, request: Value) -> Value {
        let response = self.rpc(request).await;
        assert_eq!(response["op"], "result", "native control: {response}");
        response["data"].clone()
    }
    pub async fn transport(&mut self, direction: &str) -> String {
        let value = self
            .ok(json!({"op":"transport","direction":direction}))
            .await;
        assert!(
            value["iceCandidates"]
                .as_array()
                .is_some_and(|c| !c.is_empty())
        );
        value["id"]
            .as_str()
            .expect("native transport id")
            .to_owned()
    }
    pub async fn receive(&mut self) {
        self.ok(json!({"op":"capabilities","rtp":self.capabilities}))
            .await;
        self.transport("recv").await;
    }
    pub async fn event(&mut self, op: &str) -> Value {
        if let Some(index) = self.events.iter().position(|event| event["op"] == op) {
            return self.events.remove(index).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let msg = self
                    .socket
                    .next()
                    .await
                    .expect("event socket open")
                    .unwrap();
                if let Message::Text(raw) = msg {
                    let value: Value = serde_json::from_str(&raw).unwrap();
                    if value["op"] == op {
                        return value;
                    }
                    self.events.push_back(value);
                }
            }
        })
        .await
        .expect("bounded native consumer event")
    }
    pub async fn close(&mut self) {
        self.socket.close(None).await.unwrap();
    }
}
pub fn rtp(kind: &str) -> Value {
    let audio = matches!(kind, "a" | "sa" | "la");
    let ssrc = Uuid::new_v4().as_u128() as u32 | 1;
    json!({"mid":Uuid::new_v4().simple().to_string(),
        "codecs":[if audio {
            json!({"mimeType":"audio/opus","payloadType":111,"clockRate":48000,"channels":2,"parameters":{"useinbandfec":1,"stereo":1},"rtcpFeedback":[]})
        }else {json!({"mimeType":"video/VP8","payloadType":96,"clockRate":90000,"parameters":{},"rtcpFeedback":[{"type":"nack"},{"type":"nack","parameter":"pli"}]})}],
        "headerExtensions":[],"encodings":[{"ssrc":ssrc}],"rtcp":{"cname":Uuid::new_v4().to_string(),"reducedSize":true}})
}
pub fn produce(kind: &str, epoch: Uuid, parent: Option<&str>, nonce: Option<Uuid>) -> Value {
    json!({"op":"produce","k":kind,"epoch":epoch,"rtp":rtp(kind),"parent":parent,"lc":nonce,"height":360})
}
