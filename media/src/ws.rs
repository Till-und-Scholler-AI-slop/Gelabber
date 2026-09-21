//! Media WebSocket. Ticket in the first `j` frame; SDP/ICE stay off the
//! chat gateway.

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::protocol::{ClientFrame, ServerFrame};
use crate::sfu::PeerId;
use crate::state::AppState;
use crate::ticket::{self, TicketClaim};

/// Chrome video answers (many codecs, a second m-line, ICE candidates in the
/// SDP) blow past 12 KiB and can pass 48 KiB. Rejecting that frame as
/// `bad_request` made the viewer leave the voice channel the moment a stream
/// started. 192 KiB still bounds a single signaling frame.
pub const MAX_FRAME: usize = 256 * 1024;
pub const MAX_SDP: usize = 192 * 1024;
const MAX_ICE: usize = 800;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ws", get(upgrade))
        .route("/media/ws", get(upgrade))
}

async fn upgrade(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| run(socket, state))
}

async fn run(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerFrame>();

    let mut joined: Option<(PeerId, Uuid)> = None;

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_FRAME {
                            warn!(
                                bytes = text.len(),
                                max = MAX_FRAME,
                                "media frame too large"
                            );
                            // An answer this large never reaches apply_remote.
                            // Abort only when the frame is that answer and an
                            // offer is still outstanding.
                            if answer_frame(&text)
                                && let Some((peer_id, channel_id)) = joined
                            {
                                let _ = state
                                    .sfu
                                    .abort_outstanding_offer(peer_id, channel_id)
                                    .await;
                            }
                            let _ = send(&mut sink, ServerFrame::error("negotiation_failed")).await;
                            continue;
                        }
                        match handle(&state, &text, &tx, &mut joined).await {
                            Ok(Some(frame)) => {
                                if send(&mut sink, frame).await.is_err() {
                                    break;
                                }
                            }
                            Ok(None) => {}
                            Err(code) => {
                                let _ = send(&mut sink, ServerFrame::error(code)).await;
                                if code == "unauthorized" || code == "gone" {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_) | Message::Binary(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                }
            }
            frame = rx.recv() => {
                let Some(frame) = frame else { break };
                if send(&mut sink, frame).await.is_err() {
                    break;
                }
            }
        }
    }

    if let Some((peer_id, channel_id)) = joined {
        state.sfu.leave(peer_id, channel_id).await;
    }
}

async fn handle(
    state: &AppState,
    text: &str,
    out: &mpsc::UnboundedSender<ServerFrame>,
    joined: &mut Option<(PeerId, Uuid)>,
) -> Result<Option<ServerFrame>, &'static str> {
    let frame: ClientFrame = serde_json::from_str(text).map_err(|_| "bad_request")?;
    match frame.op.as_str() {
        "j" => {
            if joined.is_some() {
                return Err("bad_request");
            }
            let tk = frame.tk.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let Some(tk) = tk else {
                return Err("unauthorized");
            };
            let claim = ticket::consume(&state.redis, tk)
                .await
                .map_err(|_| "internal")?
                .ok_or("unauthorized")?;
            join(state, claim, out, joined).await
        }
        "o" | "a" => {
            let (peer_id, channel_id) = joined.ok_or("unauthorized")?;
            let sdp = frame
                .sdp
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("bad_request")?;
            if sdp.len() > MAX_SDP {
                warn!(
                    peer = %peer_id.0,
                    bytes = sdp.len(),
                    max = MAX_SDP,
                    op = %frame.op,
                    "media sdp too large"
                );
                if frame.op == "a" {
                    let _ = state.sfu.abort_outstanding_offer(peer_id, channel_id).await;
                }
                return Err("negotiation_failed");
            }
            state
                .sfu
                .apply_remote(peer_id, channel_id, sdp.to_owned(), frame.op == "o")
                .await
                .map_err(|err| {
                    warn!(
                        peer = %peer_id.0,
                        bytes = sdp.len(),
                        op = %frame.op,
                        error = %err,
                        "sdp apply failed"
                    );
                    "negotiation_failed"
                })?;
            Ok(None)
        }
        "i" => {
            let (peer_id, channel_id) = joined.ok_or("unauthorized")?;
            let ice = frame
                .ice
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("bad_request")?;
            if ice.len() > MAX_ICE {
                return Err("bad_request");
            }
            state
                .sfu
                .add_ice(peer_id, channel_id, ice.to_owned(), frame.mid)
                .await
                .map_err(|err| {
                    warn!(
                        peer = %peer_id.0,
                        error = %err,
                        "ice apply failed"
                    );
                    "ice_failed"
                })?;
            Ok(None)
        }
        "p" => {
            let (peer_id, channel_id) = joined.ok_or("unauthorized")?;
            let k = frame
                .k
                .as_deref()
                .map(str::trim)
                .filter(|s| *s == "v" || *s == "s" || *s == "l")
                .ok_or("bad_request")?;
            state
                .sfu
                .announce(peer_id, channel_id, k)
                .await
                .map_err(|_| "bad_request")?;
            Ok(None)
        }
        // Subscriber could not answer our offer. Roll that offer back and
        // release publications that queued behind it.
        "x" => {
            let (peer_id, channel_id) = joined.ok_or("unauthorized")?;
            state
                .sfu
                .abort_offer(peer_id, channel_id)
                .await
                .map_err(|_| "bad_request")?;
            Ok(None)
        }
        // Publisher offer failed before the announced track arrived.
        "u" => {
            let (peer_id, channel_id) = joined.ok_or("unauthorized")?;
            let k = frame
                .k
                .as_deref()
                .map(str::trim)
                .filter(|s| *s == "v" || *s == "s" || *s == "l")
                .ok_or("bad_request")?;
            state
                .sfu
                .retract(peer_id, channel_id, k)
                .await
                .map_err(|_| "bad_request")?;
            Ok(None)
        }
        "l" => {
            if let Some((peer_id, channel_id)) = joined.take() {
                state.sfu.leave(peer_id, channel_id).await;
            }
            Ok(None)
        }
        _ => Err("bad_request"),
    }
}

async fn join(
    state: &AppState,
    claim: TicketClaim,
    out: &mpsc::UnboundedSender<ServerFrame>,
    joined: &mut Option<(PeerId, Uuid)>,
) -> Result<Option<ServerFrame>, &'static str> {
    let peer_id = state
        .sfu
        .join(claim.clone(), out.clone())
        .await
        .map_err(|err| {
            warn!(error = %err, "sfu join failed");
            "internal"
        })?;
    *joined = Some((peer_id, claim.c));
    debug!(user = %claim.u, channel = %claim.c, "media ticket accepted");
    Ok(Some(ServerFrame::Ok {
        c: claim.c.to_string(),
        u: claim.u.to_string(),
    }))
}

/// `{"op":"a"...}` at the start of a frame. Used when the body is too
/// large to treat as a normal signaling message. Anything else, including
/// a publisher offer, must not abort the subscriber's current offer.
fn answer_frame(text: &str) -> bool {
    let n = text.len().min(64);
    let head = &text[..n];
    head.contains("\"op\":\"a\"") || head.contains("\"op\": \"a\"")
}

async fn send(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: ServerFrame,
) -> Result<(), ()> {
    let json = frame.to_json().map_err(|_| ())?;
    sink.send(Message::Text(Utf8Bytes::from(json)))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_frame_is_only_an_answer() {
        assert!(answer_frame(r#"{"op":"a","sdp":"v=0"}"#));
        assert!(answer_frame("{\"op\": \"a\", \"sdp\": \"v=0\"}"));
        assert!(!answer_frame(r#"{"op":"o","sdp":"v=0"}"#));
        assert!(!answer_frame(r#"{"op":"p","k":"s"}"#));
        assert!(!answer_frame("not-json"));
    }

    #[test]
    fn chrome_video_sdp_fits() {
        assert!(MAX_SDP >= 192 * 1024);
        assert!(MAX_FRAME >= 256 * 1024);
        assert!(MAX_FRAME > MAX_SDP);
        // A Chrome video answer around 60 KiB used to miss the 48 KiB cap.
        assert!(60 * 1024 < MAX_SDP);
        assert!(MAX_SDP > 12_288);
        assert!(MAX_FRAME > 16 * 1024);
    }
}
