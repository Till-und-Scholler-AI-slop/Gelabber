//! The media socket `/media/ws` (docs/native/protocol-gateway.md §8):
//! numbered requests answered by `result`/`err`, server events in between,
//! in no particular order relative to the answers.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::logic::MEDIA_REQUEST_DEADLINE;

/// A failed request: a server code (`forbidden`, `unavailable`, …) or a
/// local one (`request_timeout`, `connection_closed`, `request_overflow`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MediaError(pub String);

impl MediaError {
    pub(crate) fn new(code: &str) -> Self {
        Self(code.to_owned())
    }

    pub(crate) fn code(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the socket reports besides answers.
#[derive(Debug)]
pub(crate) enum SocketEvent {
    /// `consumer`, `consumerClosed`, `consumerState`, `producerClosed`,
    /// `codecs`, `layers`.
    Event(Value),
    /// An `err` without a request id.
    Error(String),
    Closed,
}

/// The web drops inbound frames above 1 MiB.
const MAX_FRAME: usize = 1 << 20;
/// At most this many requests wait for answers at once.
const MAX_PENDING: usize = 64;

type Answer = Result<Value, MediaError>;

pub(crate) struct MediaSocket {
    out: mpsc::UnboundedSender<Message>,
    pending: Mutex<HashMap<u32, oneshot::Sender<Answer>>>,
    next: AtomicU32,
    closed: AtomicBool,
}

impl MediaSocket {
    /// Opens the socket; frames that are not answers go to `events`.
    pub(crate) async fn connect(
        url: &str,
        events: mpsc::UnboundedSender<SocketEvent>,
    ) -> Result<Arc<Self>, MediaError> {
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let connect = tokio_tungstenite::connect_async_with_config(url, Some(config), false);
        let (stream, _) = tokio::time::timeout(MEDIA_REQUEST_DEADLINE, connect)
            .await
            .map_err(|_| MediaError::new("connect_timeout"))?
            .map_err(|e| {
                tracing::debug!("media socket: {e}");
                MediaError::new("network")
            })?;
        let (mut sink, mut source) = stream.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let socket = Arc::new(Self {
            out,
            pending: Mutex::new(HashMap::new()),
            next: AtomicU32::new(0),
            closed: AtomicBool::new(false),
        });
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                let close = matches!(message, Message::Close(_));
                if sink.send(message).await.is_err() || close {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        let reader = Arc::downgrade(&socket);
        tokio::spawn(async move {
            while let Some(message) = source.next().await {
                let Some(socket) = reader.upgrade() else {
                    return;
                };
                match message {
                    Ok(Message::Text(text)) => socket.on_text(&text, &events),
                    // Pings are answered by tungstenite while reading.
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            if let Some(socket) = reader.upgrade() {
                socket.fail_all();
            }
            let _ = events.send(SocketEvent::Closed);
        });
        Ok(socket)
    }

    fn on_text(&self, text: &str, events: &mpsc::UnboundedSender<SocketEvent>) {
        let Ok(frame) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let id = frame["id"].as_u64().and_then(|id| u32::try_from(id).ok());
        match (frame["op"].as_str(), id) {
            (Some("result"), Some(id)) => self.answer(id, Ok(frame["data"].clone())),
            (Some("err"), Some(id)) => {
                let code = frame["e"].as_str().unwrap_or("bad_request");
                self.answer(id, Err(MediaError::new(code)));
            }
            (Some("err"), None) => {
                let code = frame["e"].as_str().unwrap_or("bad_request").to_owned();
                let _ = events.send(SocketEvent::Error(code));
            }
            (Some(_), _) => {
                let _ = events.send(SocketEvent::Event(frame));
            }
            _ => {}
        }
    }

    fn answer(&self, id: u32, answer: Answer) {
        if let Some(waiting) = self.pending.lock().remove(&id) {
            let _ = waiting.send(answer);
        }
    }

    fn fail_all(&self) {
        self.closed.store(true, Ordering::Release);
        for (_, waiting) in self.pending.lock().drain() {
            let _ = waiting.send(Err(MediaError::new("connection_closed")));
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }

    fn frame(&self, op: &str, data: Value) -> (u32, Message) {
        let id = self.next.fetch_add(1, Ordering::AcqRel) + 1;
        let mut frame = match data {
            Value::Object(map) => Value::Object(map),
            _ => json!({}),
        };
        frame["op"] = op.into();
        frame["id"] = id.into();
        (id, Message::Text(frame.to_string().into()))
    }

    /// Sends a request and waits up to ten seconds for its answer.
    pub(crate) async fn request(&self, op: &str, data: Value) -> Answer {
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut pending = self.pending.lock();
            // Checked under the lock `fail_all` drains, so a request racing
            // a close fails now instead of waiting out the deadline.
            if !self.is_open() {
                return Err(MediaError::new("connection_closed"));
            }
            if pending.len() >= MAX_PENDING {
                return Err(MediaError::new("request_overflow"));
            }
            // Ids rise in the order the frames go out: numbered and queued
            // under one lock.
            let (id, message) = self.frame(op, data);
            pending.insert(id, tx);
            if self.out.send(message).is_err() {
                pending.remove(&id);
                return Err(MediaError::new("connection_closed"));
            }
            id
        };
        match tokio::time::timeout(MEDIA_REQUEST_DEADLINE, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(MediaError::new("connection_closed")),
            Err(_) => {
                self.pending.lock().remove(&id);
                Err(MediaError::new("request_timeout"))
            }
        }
    }

    /// A request whose answer nobody waits for (the final `l`).
    pub(crate) fn notify(&self, op: &str, data: Value) {
        let _pending = self.pending.lock();
        let (_, message) = self.frame(op, data);
        let _ = self.out.send(message);
    }

    pub(crate) fn close(&self) {
        let _ = self.out.send(Message::Close(None));
        self.fail_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A server that answers `j` and `capabilities`, sends an event and an
    /// unsolicited error, and refuses everything else.
    async fn server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let frame: Value = serde_json::from_str(&text).unwrap();
                let id = frame["id"].clone();
                let reply = match frame["op"].as_str().unwrap() {
                    "j" => {
                        assert_eq!(frame["v"], 4);
                        // An event may overtake the answer.
                        ws.send(Message::Text(
                            json!({"op": "codecs", "video": ["video/vp8"], "revision": 1})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                        json!({"op": "result", "id": id, "data": {"v": 4}})
                    }
                    "slow" => continue,
                    "boom" => json!({"op": "err", "e": "gone"}),
                    _ => json!({"op": "err", "id": id, "e": "forbidden"}),
                };
                ws.send(Message::Text(reply.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        format!("ws://{addr}")
    }

    #[tokio::test]
    async fn requests_get_their_answers_and_events_go_aside() {
        let url = server().await;
        let (tx, mut events) = mpsc::unbounded_channel();
        let socket = MediaSocket::connect(&url, tx).await.unwrap();
        let joined = socket.request("j", json!({"tk": "x", "v": 4})).await;
        assert_eq!(joined.unwrap()["v"], 4);
        match events.recv().await.unwrap() {
            SocketEvent::Event(frame) => assert_eq!(frame["op"], "codecs"),
            other => panic!("{other:?}"),
        }
        let refused = socket.request("w", json!({})).await;
        assert_eq!(refused, Err(MediaError::new("forbidden")));
        socket.notify("boom", json!({}));
        match events.recv().await.unwrap() {
            SocketEvent::Error(code) => assert_eq!(code, "gone"),
            other => panic!("{other:?}"),
        }
        socket.close();
        assert!(!socket.is_open());
        assert_eq!(
            socket.request("j", json!({})).await,
            Err(MediaError::new("connection_closed"))
        );
    }
}
