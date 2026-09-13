//! One WebSocket: session already checked, then subscribe / heartbeat /
//! catch-up / presence / typing.

use std::time::Instant;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use super::hub::ConnId;
use super::protocol::{CatchUp, ClientFrame, PresenceStatus, ServerFrame, Topic};
use crate::auth::user::User;
use crate::error::ApiError;
use crate::servers::channel::{self, ChannelKind};
use crate::servers::membership;
use crate::state::AppState;

const MAX_CLIENT_BYTES: usize = 16 * 1024;

enum FrameEffect {
    /// Heartbeat: keep the socket and Redis TTL, do not reset idle.
    Liveness,
    /// User is active on this client.
    Activity,
    /// This client is going idle (explicit `st:i`).
    Idle,
}

pub async fn run(socket: WebSocket, state: AppState, user: User) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.gateway.attach(user.id, tx).await;
    debug!(user_id = %user.id, "ws connected");

    let mut last_client = Instant::now();
    let mut last_activity = Instant::now();
    let mut idle = false;
    let mut beat = tokio::time::interval(state.ws_heartbeat);
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires immediately; skip it so we do not heartbeat
    // before the client has had a chance to subscribe.
    beat.tick().await;

    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        last_client = Instant::now();
                        if text.len() > MAX_CLIENT_BYTES {
                            let _ = send(&mut sink, ServerFrame::error("bad_request", None, None)).await;
                            continue;
                        }
                        match handle_text(&state, &user, conn, &text, &mut sink).await {
                            Ok(FrameEffect::Liveness) => {
                                let _ = state.gateway.touch_presence(conn).await;
                            }
                            Ok(FrameEffect::Activity) => {
                                last_activity = Instant::now();
                                if idle {
                                    idle = false;
                                    let _ = state.gateway.set_conn_status(conn, PresenceStatus::Online).await;
                                }
                            }
                            Ok(FrameEffect::Idle) => {
                                idle = true;
                                let _ = state.gateway.set_conn_status(conn, PresenceStatus::Idle).await;
                            }
                            Err(err) => {
                                warn!(error = err.code(), user_id = %user.id, "ws frame failed");
                                let _ = send(&mut sink, ServerFrame::error("internal", None, None)).await;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        last_client = Instant::now();
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Binary(_))) => {
                        last_client = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                }
            }
            _ = beat.tick() => {
                if last_client.elapsed() >= state.ws_dead {
                    debug!(user_id = %user.id, "ws silent death");
                    break;
                }
                if !idle && last_activity.elapsed() >= state.ws_idle {
                    idle = true;
                    let _ = state.gateway.set_conn_status(conn, PresenceStatus::Idle).await;
                }
                if send(&mut sink, ServerFrame::Heartbeat).await.is_err() {
                    break;
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

    if let Some((user_id, servers, typing)) = state.gateway.detach(conn).await {
        state
            .gateway
            .clear_conn(conn, user_id, &servers, &typing)
            .await;
    }
    debug!(user_id = %user.id, "ws disconnected");
}

async fn handle_text(
    state: &AppState,
    user: &User,
    conn: ConnId,
    text: &str,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<FrameEffect, ApiError> {
    let frame: ClientFrame = match serde_json::from_str(text) {
        Ok(frame) => frame,
        Err(_) => {
            send(sink, ServerFrame::error("bad_request", None, None)).await?;
            return Ok(FrameEffect::Liveness);
        }
    };

    if frame.is_heartbeat() {
        // Liveness only — `last_client` was already updated in `run`.
        // Echoing would ping-pong with the browser client, which replies
        // to every server `h`.
        return Ok(FrameEffect::Liveness);
    }

    match frame.op.as_str() {
        "s" => {
            subscribe(state, user, conn, frame, sink).await?;
            Ok(FrameEffect::Activity)
        }
        "u" => {
            unsubscribe(state, conn, frame).await?;
            Ok(FrameEffect::Activity)
        }
        "sig" => {
            super::signal::handle(state, user, conn, frame, sink).await?;
            Ok(FrameEffect::Activity)
        }
        "p" => Ok(match frame.st {
            Some(PresenceStatus::Idle) => FrameEffect::Idle,
            _ => FrameEffect::Activity,
        }),
        "y" => {
            typing(state, user, conn, frame, sink).await?;
            Ok(FrameEffect::Activity)
        }
        _ => {
            send(sink, ServerFrame::error("bad_request", frame.s, frame.c)).await?;
            Ok(FrameEffect::Liveness)
        }
    }
}

async fn subscribe(
    state: &AppState,
    user: &User,
    conn: ConnId,
    frame: ClientFrame,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<(), ApiError> {
    let Some(server_id) = frame.s else {
        send(sink, ServerFrame::error("bad_request", None, frame.c)).await?;
        return Ok(());
    };
    match authorize(&state.db, user.id, server_id, frame.c).await {
        Ok(()) => {}
        Err(ApiError::NotFound) => {
            send(
                sink,
                ServerFrame::error("not_found", Some(server_id), frame.c),
            )
            .await?;
            return Ok(());
        }
        Err(other) => return Err(other),
    }

    state.gateway.watch_server(conn, server_id).await;
    let topic = Topic::of(server_id, frame.c);
    state.gateway.begin_catch_up(conn, topic).await;

    let (current, plan) = state.gateway.catch_up(topic, frame.n).await?;
    match plan {
        CatchUp::None => {}
        CatchUp::Replay(events) => {
            for event in events {
                send(sink, ServerFrame::event(event)).await?;
            }
        }
        CatchUp::Gap => {
            send(sink, ServerFrame::gap(server_id, frame.c)).await?;
        }
    }
    send(sink, ServerFrame::subscribed(server_id, frame.c, current)).await?;

    // Live Pub/Sub frames that arrived while we were reading the log sit
    // in the per-socket queue. Flush them now (drop dups by `n`).
    state.gateway.finish_catch_up(conn, topic, current).await;

    // Presence is ephemeral: announce after the sequenced `ok` so a snapshot
    // never looks like catch-up and never bumps `n`.
    state
        .gateway
        .set_conn_status(conn, PresenceStatus::Online)
        .await?;
    state.gateway.announce_server(conn, server_id).await?;
    let snap = state.gateway.presence_snapshot(server_id).await?;
    send(sink, ServerFrame::presence_snap(server_id, snap)).await?;
    Ok(())
}

async fn unsubscribe(state: &AppState, conn: ConnId, frame: ClientFrame) -> Result<(), ApiError> {
    let Some(server_id) = frame.s else {
        return Ok(());
    };
    state
        .gateway
        .unsubscribe(conn, Topic::of(server_id, frame.c))
        .await;
    Ok(())
}

async fn typing(
    state: &AppState,
    user: &User,
    conn: ConnId,
    frame: ClientFrame,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<(), ApiError> {
    let (Some(server_id), Some(channel_id), Some(on)) = (frame.s, frame.c, frame.on) else {
        send(sink, ServerFrame::error("bad_request", frame.s, frame.c)).await?;
        return Ok(());
    };
    match authorize(&state.db, user.id, server_id, Some(channel_id)).await {
        Ok(()) => {}
        Err(ApiError::NotFound) => {
            send(
                sink,
                ServerFrame::error("not_found", Some(server_id), Some(channel_id)),
            )
            .await?;
            return Ok(());
        }
        Err(other) => return Err(other),
    }
    state
        .gateway
        .set_typing(conn, server_id, channel_id, user.id, on)
        .await
}

/// Membership + (optional) channel belongs to that server — or the caller
/// is a participant of a 1:1 DM. For DMs the protocol `s` is the channel
/// id. Same 404 semantics as the REST API: unknown and foreign are
/// indistinguishable.
async fn authorize(
    db: &sqlx::PgPool,
    user_id: Uuid,
    server_id: Uuid,
    channel_id: Option<Uuid>,
) -> Result<(), ApiError> {
    let Some(channel_id) = channel_id else {
        membership::load(db, server_id, user_id).await?;
        return Ok(());
    };
    let channel = channel::get(db, channel_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if channel.kind == ChannelKind::Dm {
        if server_id != channel.id {
            return Err(ApiError::NotFound);
        }
        return channel::require_participant(db, channel.id, user_id).await;
    }
    if channel.server_id != Some(server_id) {
        return Err(ApiError::NotFound);
    }
    membership::load(db, server_id, user_id).await?;
    Ok(())
}

pub(super) async fn send(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: ServerFrame,
) -> Result<(), ApiError> {
    let json = frame
        .to_json()
        .map_err(|err| ApiError::Internal(format!("serialize ws frame: {err}")))?;
    sink.send(Message::Text(Utf8Bytes::from(json)))
        .await
        .map_err(|err| ApiError::Internal(format!("ws send: {err}")))
}
