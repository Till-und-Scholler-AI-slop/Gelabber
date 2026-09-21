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

/// Chrome video answers (VP8/VP9/H264 + ICE + BUNDLE) regularly exceed 12 KiB.
/// A 12 KiB cap rejected those frames with `bad_request` and kicked the peer
/// the moment someone published camera / screen / Go Live.
pub(crate) const MAX_FRAME: usize = 64 * 1024;
pub(crate) const MAX_SDP: usize = 48 * 1024;
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
                            let _ = send(&mut sink, ServerFrame::error("bad_request")).await;
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
                return Err("bad_request");
            }
            state
                .sfu
                .apply_remote(peer_id, channel_id, sdp.to_owned(), frame.op == "o")
                .await
                .map_err(|err| {
                    warn!(error = %err, "sdp apply failed");
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
                    warn!(error = %err, "ice apply failed");
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
    fn chrome_video_sdp_fits() {
        assert!(MAX_SDP >= 48 * 1024);
        assert!(MAX_FRAME >= 64 * 1024);
        assert!(MAX_FRAME > MAX_SDP);
        assert!(MAX_SDP > 12_288);
        assert!(MAX_FRAME > 16 * 1024);
    }
}
