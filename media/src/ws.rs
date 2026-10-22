//! Gelabber's authenticated v4 mediasoup control WebSocket.
//! A socket without a joined room lives at most `MEDIA_JOIN_DEADLINE_MS`,
//! and each client address may hold `MEDIA_PENDING_PER_IP` of them
//! (`crate::limits`). A joined socket is pinged every `MEDIA_HEARTBEAT_MS`
//! and closed once nothing arrived from it for `MEDIA_IDLE_TIMEOUT_MS`.
//! There is no Origin check on purpose: the socket is authenticated by the
//! single-use ticket in its first frame, not by a cookie.
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::limits::{PendingSlot, Refused};
use crate::protocol::{ClientFrame, MEDIA_PROTOCOL_VERSION, OUTBOUND_CAPACITY, ServerFrame};
use crate::sfu::PeerId;
use crate::state::AppState;
use crate::ticket;

/// Includes complete RTP capabilities/parameters, bounded before JSON decoding.
pub const MAX_FRAME: usize = 256 * 1024;
const WRITE_DEADLINE: Duration = Duration::from_secs(2);
/// How long a retired socket waits for the writer to send a terminal frame
/// before it falls back to a bare Close.
const RETIRE_GRACE: Duration = Duration::from_millis(250);
const SHARED_ADDRESS_WARN_EVERY: Duration = Duration::from_secs(600);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ws", get(upgrade))
        .route("/media/ws", get(upgrade))
}
async fn upgrade(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let peer = peer.map(|Extension(ConnectInfo(addr))| addr);
    let key = client_key(&headers, peer);
    // Refused before the upgrade: a plain HTTP answer holds no socket task.
    let slot = match state.pending.acquire(&key) {
        Ok(slot) => slot,
        Err(reason) => {
            debug!(?reason, "media socket refused: too many pending sockets");
            if reason == Refused::PerIp && proxy_without_forwarded_for(&headers, peer) {
                warn_shared_address(peer);
            }
            let status = match reason {
                Refused::PerIp => StatusCode::TOO_MANY_REQUESTS,
                Refused::Total => StatusCode::SERVICE_UNAVAILABLE,
            };
            return (status, [(RETRY_AFTER, "1")]).into_response();
        }
    };
    ws.max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| run(socket, state, key, slot))
}
/// The address the pending-socket cap counts: the rightmost
/// `X-Forwarded-For` entry, which the Caddy in front wrote, or the TCP peer.
fn client_key(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    gelabber_shared::client_addr::client_key(
        headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok()),
        peer.map(|addr| addr.ip()),
    )
}
/// A refusal for a private or loopback peer that sent no `X-Forwarded-For`
/// usually means a proxy in front that does not forward the client address:
/// then every client counts as that proxy and shares its pending slots.
fn proxy_without_forwarded_for(headers: &HeaderMap, peer: Option<SocketAddr>) -> bool {
    if headers.contains_key("x-forwarded-for") {
        return false;
    }
    match peer.map(|addr| addr.ip().to_canonical()) {
        Some(IpAddr::V4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Some(IpAddr::V6(ip)) => {
            ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
        }
        None => true,
    }
}
/// At most once per [`SHARED_ADDRESS_WARN_EVERY`], so a reconnect storm
/// does not flood the log.
fn warn_shared_address(peer: Option<SocketAddr>) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|err| err.into_inner());
    if last.is_some_and(|at| at.elapsed() < SHARED_ADDRESS_WARN_EVERY) {
        return;
    }
    *last = Some(Instant::now());
    warn!(
        peer = ?peer.map(|addr| addr.ip()),
        "pending-socket cap reached for a private address without X-Forwarded-For; \
         if a proxy is in front of media, it must send the client address \
         (Caddy: header_up X-Forwarded-For {{client_ip}}), or all its clients \
         share one address's MEDIA_PENDING_PER_IP"
    );
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
async fn run(socket: WebSocket, state: AppState, key: String, slot: PendingSlot) {
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
    let mut retired = false;
    // Held while no room is joined; see `crate::limits`.
    let mut pending_slot = Some(slot);
    // Anything from the client, Pongs included, shows the peer is there.
    let mut last_client = tokio::time::Instant::now();
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + state.heartbeat,
        state.heartbeat,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // A socket without a joined room closes at this deadline; Pings and bad
    // frames do not extend it. A join in progress is never cut off, because
    // it runs inside the stream branch below.
    let join_deadline = tokio::time::sleep(state.join_deadline);
    tokio::pin!(join_deadline);
    loop {
        tokio::select! {
            _ = &mut writer => { writer_finished = true; break; }
            _ = &mut join_deadline, if joined.is_none() => {
                debug!("media socket closed: no join before deadline");
                break;
            }
            _ = retirement.tick(), if joined.is_some() => {
                let (peer, channel) = joined.unwrap();
                if !state.sfu.peer_present(peer, channel).await { retired = true; break; }
            }
            // Evicts half-open peers: browsers answer a Ping on their own,
            // so a peer that stays silent past the timeout is gone.
            _ = heartbeat.tick(), if joined.is_some() => {
                if last_client.elapsed() >= state.idle_timeout {
                    debug!("media socket closed: nothing from the peer before the idle timeout");
                    break;
                }
                if !matches!(tokio::time::timeout(WRITE_DEADLINE, writes.send(Message::Ping(Default::default()))).await, Ok(Ok(()))) { break; }
            }
            incoming = stream.next() => {
                if matches!(incoming, Some(Ok(_))) { last_client = tokio::time::Instant::now(); }
                let message = match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let was_joined = joined.is_some();
                        let response = match parse_frame(&text) {
                            Ok(frame) => handle(&state, frame, &tx, &mut joined, &mut last_id).await,
                            Err(error) => error.into_frame(),
                        };
                        identity.send_replace(joined);
                        let mut refused = false;
                        if joined.is_some() {
                            pending_slot.take();
                        } else if was_joined {
                            // After a leave the socket gets a fresh deadline to
                            // rejoin, and counts as pending again until then.
                            join_deadline.as_mut().reset(tokio::time::Instant::now() + state.join_deadline);
                            match state.pending.acquire(&key) {
                                Ok(slot) => { pending_slot.replace(slot); }
                                Err(reason) => {
                                    debug!(?reason, "media socket closed after leave: too many pending sockets");
                                    refused = true;
                                }
                            }
                        }
                        let close = terminal(&response) || refused;
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
    // Sfu::shutdown and watch_authority emit gone/unauthorized only after
    // leave has removed the peer, so the retirement tick can see it gone
    // first. The writer then sends that frame and its own Close; the bare
    // Close below stays the fallback.
    if retired
        && !writer_finished
        && tokio::time::timeout(RETIRE_GRACE, &mut writer)
            .await
            .is_ok()
    {
        writer_finished = true;
    }
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
        ClientFrame::Join { tk, v, .. } => {
            if joined.is_some() {
                Err("bad_request")
            } else {
                join(state, &tk, v, out, joined).await
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
    version: u8,
    out: &mpsc::Sender<ServerFrame>,
    joined: &mut Option<(PeerId, Uuid)>,
) -> Result<Value, &'static str> {
    let claim = ticket::consume(state.sfu.redis(), code)
        .await
        .map_err(|_| "unauthorized")?
        .ok_or("unauthorized")?;
    let channel = claim.claim.c;
    let user = claim.claim.u;
    let peer = state
        .sfu
        .join_authorized_version(claim, version, out.clone())
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
    fn only_private_peers_without_forwarded_for_look_like_a_proxy() {
        let forwarded = {
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", "198.51.100.1".parse().unwrap());
            headers
        };
        let bare = HeaderMap::new();
        let peer = |raw: &str| Some(SocketAddr::new(raw.parse().unwrap(), 4000));
        for private in [
            "127.0.0.1",
            "10.0.0.5",
            "172.18.0.1",
            "192.168.1.2",
            "::1",
            "fd00::5",
            "::ffff:10.0.0.5",
        ] {
            assert!(
                proxy_without_forwarded_for(&bare, peer(private)),
                "{private}"
            );
            assert!(
                !proxy_without_forwarded_for(&forwarded, peer(private)),
                "{private}"
            );
        }
        for public in ["198.51.100.7", "2001:db8::1"] {
            assert!(
                !proxy_without_forwarded_for(&bare, peer(public)),
                "{public}"
            );
        }
    }
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
