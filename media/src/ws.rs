//! Gelabber's authenticated v4 mediasoup control WebSocket.
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::protocol::{ClientFrame, MEDIA_PROTOCOL_VERSION, OUTBOUND_CAPACITY, ServerFrame};
use crate::sfu::PeerId;
use crate::state::AppState;
use crate::ticket;

/// Includes complete RTP capabilities/parameters, bounded before JSON decoding.
pub const MAX_FRAME: usize = 256 * 1024;
const WRITE_DEADLINE: Duration = Duration::from_secs(2);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ws", get(upgrade))
        .route("/media/ws", get(upgrade))
}
async fn upgrade(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| run(socket, state))
}
fn terminal(frame: &ServerFrame) -> bool {
    matches!(frame,ServerFrame::Err { e,.. } if matches!(e.as_str(),"unauthorized"|"gone"|"update_required"))
}
#[derive(Debug)]
struct ParseError {
    id: Option<u32>,
}
impl ParseError {
    fn into_frame(self) -> ServerFrame {
        match self.id {
            Some(id) => ServerFrame::request_error(id, "bad_request"),
            None => ServerFrame::error("bad_request"),
        }
    }
}
fn parse_frame(text: &str) -> Result<ClientFrame, ParseError> {
    if text.len() > MAX_FRAME {
        return Err(ParseError { id: None });
    }
    let value: Value = serde_json::from_str(text).map_err(|_| ParseError { id: None })?;
    let id = value
        .get("id")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok());
    serde_json::from_value(value).map_err(|_| ParseError { id })
}
async fn run(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerFrame>(OUTBOUND_CAPACITY);
    let (writes, mut pending) = mpsc::channel::<Message>(32);
    let (identity, current) = tokio::sync::watch::channel::<Option<(PeerId, Uuid)>>(None);
    let writer_state = state.clone();
    // The socket writer must keep draining announcements while the reader is
    // awaiting Redis/native RPCs. Both paths retain bounded queues/deadlines.
    // Ordering contract: an RPC response is not ordered against announcements
    // raised while that RPC ran; clients must tolerate either order. `biased`
    // only keeps a ready response from waiting behind an announcement burst.
    let mut writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                message = pending.recv() => {
                    let Some(message) = message else { break; };
                    let close = matches!(message, Message::Close(_));
                    if !matches!(tokio::time::timeout(WRITE_DEADLINE, sink.send(message)).await, Ok(Ok(()))) || close { break; }
                }
                event = rx.recv() => {
                    let Some(event) = event else { break; };
                    let close = terminal(&event);
                    if close {
                        let joined = *current.borrow();
                        if let Some((peer, channel)) = joined { writer_state.sfu.leave(peer, channel).await; }
                    }
                    if send(&mut sink, event).await.is_err() { break; }
                    if close {
                        let _ = tokio::time::timeout(WRITE_DEADLINE, sink.send(Message::Close(None))).await;
                        break;
                    }
                }
            }
        }
    });
    let mut joined: Option<(PeerId, Uuid)> = None;
    let mut last_id = 0;
    let mut retirement = tokio::time::interval(Duration::from_millis(250));
    retirement.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut writer_finished = false;
    loop {
        tokio::select! {
            _ = &mut writer => { writer_finished = true; break; }
            _ = retirement.tick(), if joined.is_some() => {
                let (peer, channel) = joined.unwrap();
                if !state.sfu.peer_present(peer, channel).await { break; }
            }
            incoming = stream.next() => {
                let message = match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let response = match parse_frame(&text) {
                            Ok(frame) => handle(&state, frame, &tx, &mut joined, &mut last_id).await,
                            Err(error) => error.into_frame(),
                        };
                        identity.send_replace(joined);
                        let close = terminal(&response);
                        if close { leave_joined(&state, &mut joined).await; }
                        let Ok(text) = response.to_json() else { break; };
                        if !matches!(tokio::time::timeout(WRITE_DEADLINE, writes.send(Message::Text(text.into()))).await, Ok(Ok(()))) { break; }
                        if close { break; }
                        continue;
                    }
                    Some(Ok(Message::Ping(payload))) => Message::Pong(payload),
                    Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Binary(_))) => Message::Text(ServerFrame::error("bad_request").to_json().unwrap().into()),
                    _ => break,
                };
                if !matches!(tokio::time::timeout(WRITE_DEADLINE, writes.send(message)).await, Ok(Ok(()))) { break; }
            }
        }
    }
    leave_joined(&state, &mut joined).await;
    if !writer_finished {
        let _ = tokio::time::timeout(WRITE_DEADLINE, writes.send(Message::Close(None))).await;
        drop(writes);
        if tokio::time::timeout(WRITE_DEADLINE, &mut writer)
            .await
            .is_err()
        {
            writer.abort();
            let _ = writer.await;
        }
    }
}
/// Every terminal response/event crosses this native-stop barrier before send.
/// Taking the socket identity keeps cleanup idempotent on the loop's exit path.
async fn leave_joined(state: &AppState, joined: &mut Option<(PeerId, Uuid)>) {
    if let Some((peer, channel)) = joined.take() {
        state.sfu.leave(peer, channel).await;
    }
}
async fn handle(
    state: &AppState,
    frame: ClientFrame,
    out: &mpsc::Sender<ServerFrame>,
    joined: &mut Option<(PeerId, Uuid)>,
    last_id: &mut u32,
) -> ServerFrame {
    let id = frame.id();
    // The protocol gate precedes identity, Redis access and ticket consumption.
    if let ClientFrame::Join { v, .. } = &frame
        && *v != MEDIA_PROTOCOL_VERSION
    {
        return if id > 0 {
            ServerFrame::request_error(id, "update_required")
        } else {
            ServerFrame::error("update_required")
        };
    }
    if id == 0 || id <= *last_id {
        return ServerFrame::request_error(id, "bad_request");
    }
    *last_id = id;
    let result = match frame {
        ClientFrame::Join { tk, w, v, .. } => {
            if joined.is_some() {
                Err("bad_request")
            } else {
                join(state, &tk, w, v, out, joined).await
            }
        }
        ClientFrame::Leave { .. } => {
            leave_joined(state, joined).await;
            Ok(json!({}))
        }
        frame => {
            if let Some((peer, channel)) = *joined {
                state.sfu.rpc(peer, channel, frame).await.map_err(|err| {
                    warn!(peer=%peer.0,error=%err,code=err.code(),"mediasoup control failed");
                    err.code()
                })
            } else {
                Err("unauthorized")
            }
        }
    };
    match result {
        Ok(data) => ServerFrame::Result { id, data },
        Err(code) => ServerFrame::request_error(id, code),
    }
}
async fn join(
    state: &AppState,
    code: &str,
    watch: Option<Uuid>,
    version: u8,
    out: &mpsc::Sender<ServerFrame>,
    joined: &mut Option<(PeerId, Uuid)>,
) -> Result<Value, &'static str> {
    if watch.is_some_and(|user| user.is_nil()) {
        return Err("bad_request");
    }
    let claim = ticket::consume(&state.redis, code)
        .await
        .map_err(|_| "unauthorized")?
        .ok_or("unauthorized")?;
    let channel = claim.claim.c;
    let user = claim.claim.u;
    let peer = state
        .sfu
        .join_authorized_watch_version(claim, watch, version, out.clone())
        .await
        .map_err(|err| err.code())?;
    *joined = Some((peer, channel));
    let data = state
        .sfu
        .join_data(peer, channel)
        .await
        .map_err(|err| err.code())?;
    debug!(%user,%channel,"mediasoup ticket accepted");
    Ok(data)
}
async fn send(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: ServerFrame,
) -> Result<(), ()> {
    let text = frame.to_json().map_err(|_| ())?;
    tokio::time::timeout(
        WRITE_DEADLINE,
        sink.send(Message::Text(Utf8Bytes::from(text))),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_errors_retain_request_identity() {
        assert!(matches!(
            parse_frame(r#"{"op":"produce","id":7,"k":"sa","rtp":{}}"#),
            Err(ParseError { id: Some(7) })
        ));
        assert!(matches!(
            parse_frame("not-json"),
            Err(ParseError { id: None })
        ));
    }
    #[test]
    fn oversized_frames_are_bounded_before_decode() {
        assert!(parse_frame(&"€".repeat(MAX_FRAME)).is_err());
    }
    #[test]
    fn legacy_join_is_decodable_but_terminal() {
        assert!(matches!(
            parse_frame(r#"{"op":"j","tk":"abcdefghjkmn"}"#),
            Ok(ClientFrame::Join { v: 0, .. })
        ));
        assert!(terminal(&ServerFrame::error("update_required")));
        assert!(!terminal(&ServerFrame::request_error(1, "forbidden")));
    }
}
