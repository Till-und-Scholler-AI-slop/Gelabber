//! One WebSocket: session already checked, then subscribe / heartbeat / catch-up.

use std::time::Instant;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use super::hub::ConnId;
use super::protocol::{CatchUp, ClientFrame, ServerFrame, Topic};
use crate::auth::user::User;
use crate::error::ApiError;
use crate::servers::channel::Channel;
use crate::servers::membership;
use crate::state::AppState;

const MAX_CLIENT_BYTES: usize = 8 * 1024;

pub async fn run(socket: WebSocket, state: AppState, user: User) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.gateway.attach(user.id, tx).await;
    debug!(user_id = %user.id, "ws connected");

    let mut last_client = Instant::now();
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
                        if let Err(err) = handle_text(&state, &user, conn, &text, &mut sink).await {
                            warn!(error = err.code(), user_id = %user.id, "ws frame failed");
                            let _ = send(&mut sink, ServerFrame::error("internal", None, None)).await;
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

    state.gateway.detach(conn).await;
    debug!(user_id = %user.id, "ws disconnected");
}

async fn handle_text(
    state: &AppState,
    user: &User,
    conn: ConnId,
    text: &str,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<(), ApiError> {
    let frame: ClientFrame = match serde_json::from_str(text) {
        Ok(frame) => frame,
        Err(_) => {
            send(sink, ServerFrame::error("bad_request", None, None)).await?;
            return Ok(());
        }
    };

    if frame.is_heartbeat() {
        send(sink, ServerFrame::Heartbeat).await?;
        return Ok(());
    }

    match frame.op.as_str() {
        "s" => subscribe(state, user, conn, frame, sink).await,
        "u" => unsubscribe(state, conn, frame).await,
        _ => {
            send(sink, ServerFrame::error("bad_request", frame.s, frame.c)).await?;
            Ok(())
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
            send(sink, ServerFrame::error("not_found", Some(server_id), frame.c)).await?;
            return Ok(());
        }
        Err(other) => return Err(other),
    }

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

    // Anything published between the log read and this flag clear is still
    // in Redis; a second pass fills that window, then live delivery starts.
    let (head, plan) = state.gateway.catch_up(topic, Some(current)).await?;
    if let CatchUp::Replay(events) = plan {
        for event in events {
            if event.n > current {
                send(sink, ServerFrame::event(event)).await?;
            }
        }
    }
    let _ = head;
    state.gateway.finish_catch_up(conn, topic).await;
    Ok(())
}

async fn unsubscribe(
    state: &AppState,
    conn: ConnId,
    frame: ClientFrame,
) -> Result<(), ApiError> {
    let Some(server_id) = frame.s else {
        return Ok(());
    };
    state
        .gateway
        .unsubscribe(conn, Topic::of(server_id, frame.c))
        .await;
    Ok(())
}

/// Membership + (optional) channel belongs to that server. Same 404
/// semantics as the REST API: unknown and foreign are indistinguishable.
async fn authorize(
    db: &sqlx::PgPool,
    user_id: Uuid,
    server_id: Uuid,
    channel_id: Option<Uuid>,
) -> Result<(), ApiError> {
    membership::load(db, server_id, user_id).await?;
    let Some(channel_id) = channel_id else {
        return Ok(());
    };
    let found = sqlx::query_as::<_, Channel>(
        "SELECT id, server_id, category_id, name, kind, created_at \
         FROM channels WHERE id = $1 AND server_id = $2",
    )
    .bind(channel_id)
    .bind(server_id)
    .fetch_optional(db)
    .await?;
    found.ok_or(ApiError::NotFound).map(|_| ())
}

async fn send(
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
