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
use crate::auth::session::CurrentSession;
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
    /// An authorized live claim needs its best-effort text hint after DB unlock.
    LiveStarted { server_id: Uuid, channel_id: Uuid },
    /// This client is going idle (explicit `st:i`).
    Idle,
}

pub async fn run(socket: WebSocket, state: AppState, session: CurrentSession) {
    let user = &session.user;
    let (tx, mut rx) = mpsc::channel(super::hub::OUTBOUND_CAPACITY);
    let (conn, mut revoked) = state
        .gateway
        .attach_session(user.id, session.key(), tx)
        .await;
    let expiry = (session.expires_at - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
    let expired = tokio::time::sleep(expiry);
    tokio::pin!(expired);
    debug!(user_id = %user.id, "ws connected");

    let mut last_client = Instant::now();
    let mut last_activity = Instant::now();
    let mut idle = false;
    let mut beat = tokio::time::interval(state.ws_heartbeat);
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires immediately; skip it so we do not heartbeat
    // before the client has had a chance to subscribe.
    beat.tick().await;
    let mut security = tokio::time::interval(std::time::Duration::from_secs(1));
    security.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    security.tick().await;

    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            biased;
            _ = revoked.changed() => break,
            _ = &mut expired => break,
            _ = security.tick() => {
                let Ok(Ok(Some(mut session_guard))) = tokio::time::timeout(std::time::Duration::from_secs(1), session.lock_live(&state.db)).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
                if !matches!(tokio::time::timeout(std::time::Duration::from_secs(1), state.gateway.refresh_voice(conn)).await, Ok(Ok(()))) { break; }
            }
            incoming = stream.next() => {
                let mut live_hint = None;
                let Ok(Ok(Some(mut session_guard))) = tokio::time::timeout(std::time::Duration::from_secs(1), session.lock_live(&state.db)).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        last_client = Instant::now();
                        if text.len() > MAX_CLIENT_BYTES {
                            let _ = send(&mut sink, ServerFrame::error("bad_request", None, None)).await;
                            continue;
                        }
                        match handle_text(&state, &mut session_guard, user, conn, &text, &mut sink).await {
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
                            Ok(FrameEffect::LiveStarted { server_id, channel_id }) => {
                                live_hint = Some((server_id, channel_id));
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
                        if !matches!(tokio::time::timeout(std::time::Duration::from_secs(1), sink.send(Message::Pong(payload))).await, Ok(Ok(()))) {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Binary(_))) => {
                        last_client = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                }
                // Return the only pool connection before the hint helper acquires
                // one. The signaling action itself already passed authorization.
                if session_guard.commit().await.is_err() { break; }
                if let Some((server_id, channel_id)) = live_hint {
                    crate::messages::post_live_hint(&state, user, server_id, channel_id).await;
                }
            }
            _ = beat.tick() => {
                let Ok(Ok(Some(mut session_guard))) = tokio::time::timeout(std::time::Duration::from_secs(1), session.lock_live(&state.db)).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
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
                let Ok(Ok(Some(mut session_guard))) = tokio::time::timeout(std::time::Duration::from_secs(1), session.lock_live(&state.db)).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
                if !state.gateway.wants_frame(conn, &frame).await { continue; }
                if !frame_allowed(&mut session_guard, user.id, &frame).await.unwrap_or(false) { continue; }
                if send(&mut sink, frame).await.is_err() {
                    break;
                }
            }
        }
    }

    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sink.send(Message::Close(None)),
    )
    .await;
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
    db: &mut sqlx::PgConnection,
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

    match frame {
        ClientFrame::Heartbeat => {
            // Liveness only — `last_client` was already updated in `run`.
            // Echoing would ping-pong with the browser client, which replies
            // to every server `h`.
            Ok(FrameEffect::Liveness)
        }
        ClientFrame::Subscribe { s, c, n, ep } => {
            let result = subscribe(state, db, user, conn, (s, c), (n, ep), sink).await;
            if result.is_err() {
                state.gateway.unsubscribe(conn, Topic::of(s, c)).await;
            }
            result?;
            Ok(FrameEffect::Activity)
        }
        ClientFrame::Unsubscribe { s, c } => {
            state.gateway.unsubscribe(conn, Topic::of(s, c)).await;
            Ok(FrameEffect::Activity)
        }
        ClientFrame::Sig { .. } => Ok(
            match super::signal::handle(state, db, user, conn, frame, sink).await? {
                Some((server_id, channel_id)) => FrameEffect::LiveStarted {
                    server_id,
                    channel_id,
                },
                None => FrameEffect::Activity,
            },
        ),
        ClientFrame::Presence { st } => Ok(match st {
            Some(PresenceStatus::Idle) => FrameEffect::Idle,
            _ => FrameEffect::Activity,
        }),
        ClientFrame::Typing { s, c, on } => {
            typing(state, db, user, conn, (s, c), on, sink).await?;
            Ok(FrameEffect::Activity)
        }
    }
}

async fn subscribe(
    state: &AppState,
    db: &mut sqlx::PgConnection,
    user: &User,
    conn: ConnId,
    scope: (Uuid, Option<Uuid>),
    resume: (Option<u64>, Option<Uuid>),
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<(), ApiError> {
    let (server_id, channel_id) = scope;
    let _guard = if channel_id.is_some_and(|cid| cid == server_id) {
        None // DM authorization below still verifies participants.
    } else {
        match membership::lock_server_conn(db, server_id, false).await {
            Ok(()) => Some(()),
            Err(ApiError::NotFound) => {
                send(
                    sink,
                    ServerFrame::error("not_found", Some(server_id), channel_id),
                )
                .await?;
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    };
    match authorize(&mut *db, user.id, server_id, channel_id).await {
        Ok(()) => {}
        Err(ApiError::NotFound) => {
            send(
                sink,
                ServerFrame::error("not_found", Some(server_id), channel_id),
            )
            .await?;
            return Ok(());
        }
        Err(other) => return Err(other),
    }

    if _guard.is_some() {
        state
            .gateway
            .bind_server(conn, &mut *db, server_id, user.id)
            .await?;
    }
    state.gateway.watch_server(conn, server_id).await;
    let topic = Topic::of(server_id, channel_id);
    state.gateway.begin_catch_up(conn, topic).await;

    let (current, epoch, plan) = state
        .gateway
        .events
        .catch_up_epoch(topic, resume.0, resume.1)
        .await?;
    match plan {
        CatchUp::None => {}
        CatchUp::Replay(events) => {
            for event in events {
                send(sink, ServerFrame::event(event)).await?;
            }
        }
        CatchUp::Gap => {
            send(
                sink,
                ServerFrame::gap(server_id, channel_id).with_epoch(epoch),
            )
            .await?;
        }
    }
    send(
        sink,
        ServerFrame::subscribed(server_id, channel_id, current).with_epoch(epoch),
    )
    .await?;

    // Live Pub/Sub frames that arrived while we were reading the log sit
    // in the per-socket queue. Flush them now (drop dups by `n`).
    state
        .gateway
        .connections
        .finish_catch_up_epoch(conn, topic, current, Some(epoch))
        .await;

    // Presence is ephemeral: announce after the sequenced `ok` so a snapshot
    // never looks like catch-up and never bumps `n`.
    state
        .gateway
        .set_conn_status(conn, PresenceStatus::Online)
        .await?;
    state.gateway.announce_server(conn, server_id).await?;
    let snap = state.gateway.presence_snapshot(server_id).await?;
    send(sink, ServerFrame::presence_snap(server_id, snap)).await?;
    let voice = state.gateway.voice_snapshot(server_id).await?;
    send(sink, ServerFrame::voice_snap(server_id, voice)).await?;
    Ok(())
}

async fn typing(
    state: &AppState,
    db: &mut sqlx::PgConnection,
    user: &User,
    conn: ConnId,
    scope: (Uuid, Uuid),
    on: bool,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Result<(), ApiError> {
    let (server_id, channel_id) = scope;
    let _guard = if channel_id == server_id {
        None
    } else {
        match membership::lock_server_conn(db, server_id, false).await {
            Ok(()) => Some(()),
            Err(ApiError::NotFound) => {
                send(
                    sink,
                    ServerFrame::error("not_found", Some(server_id), Some(channel_id)),
                )
                .await?;
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    };
    match authorize(&mut *db, user.id, server_id, Some(channel_id)).await {
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
    db: &mut sqlx::PgConnection,
    user_id: Uuid,
    server_id: Uuid,
    channel_id: Option<Uuid>,
) -> Result<(), ApiError> {
    let Some(channel_id) = channel_id else {
        membership::load(&mut *db, server_id, user_id).await?;
        return Ok(());
    };
    let channel = channel::get(&mut *db, channel_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if channel.kind == ChannelKind::Dm {
        if server_id != channel.id {
            return Err(ApiError::NotFound);
        }
        let participant: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM channel_members WHERE channel_id = $1 AND user_id = $2)",
        )
        .bind(channel.id)
        .bind(user_id)
        .fetch_one(&mut *db)
        .await?;
        return if participant {
            Ok(())
        } else {
            Err(ApiError::NotFound)
        };
    }
    if channel.server_id != Some(server_id) {
        return Err(ApiError::NotFound);
    }
    membership::load(&mut *db, server_id, user_id).await?;
    Ok(())
}

/// Check at dequeue time too: already queued events cannot leak after removal.
async fn frame_allowed(
    db: &mut sqlx::PgConnection,
    user_id: Uuid,
    frame: &ServerFrame,
) -> Result<bool, ApiError> {
    let scope = match frame {
        ServerFrame::Dm { c } => Some((*c, Some(*c))),
        ServerFrame::Event { s, c, .. }
        | ServerFrame::Gap { s, c, .. }
        | ServerFrame::Sig { s, c, .. } => Some((*s, *c)),
        ServerFrame::Presence { s, .. } => Some((*s, None)),
        ServerFrame::Typing { s, c, .. } => Some((*s, Some(*c))),
        _ => None,
    };
    if let Some((sid, cid)) = scope {
        if cid != Some(sid) {
            match membership::lock_server_conn(db, sid, false).await {
                Ok(()) => {}
                Err(ApiError::NotFound) => return Ok(false),
                Err(err) => return Err(err),
            }
        }
        match authorize(db, user_id, sid, cid).await {
            Ok(()) => Ok(true),
            Err(ApiError::NotFound) => Ok(false),
            Err(err) => Err(err),
        }
    } else {
        Ok(true)
    }
}

pub(super) async fn send(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: ServerFrame,
) -> Result<(), ApiError> {
    let json = frame
        .to_json()
        .map_err(|err| ApiError::Internal(format!("serialize ws frame: {err}")))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sink.send(Message::Text(Utf8Bytes::from(json))),
    )
    .await
    .map_err(|_| ApiError::Internal("ws send timed out".into()))?
    .map_err(|err| ApiError::Internal(format!("ws send: {err}")))
}
