//! One WebSocket: session already checked, then subscribe / heartbeat /
//! catch-up / presence / typing.

use std::collections::HashMap;
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

/// How long a socket without a voice seat goes without a session and access
/// check while it sends and receives nothing. Revocations reach it at once
/// through [`super::hub::Recheck`]; this bounds a lost notice (Redis down
/// included). A socket with a seat still checks every second: each check
/// gates a 5 s lease renewal, and renewing on a lost notice would keep a
/// revoked user in the roster for up to this backstop instead of one tick.
/// Seat holders are the few sockets in a call, so that poll stays cheap.
const SECURITY_BACKSTOP: std::time::Duration = std::time::Duration::from_secs(10);

enum FrameEffect {
    /// Heartbeat: keep the socket and Redis TTL, do not reset idle.
    Liveness,
    /// User is active on this client.
    Activity,
    /// An authorized live claim needs its best-effort text hint after DB unlock.
    LiveStarted { server_id: Uuid, channel_id: Uuid },
    /// This client is going idle (explicit `st:i`).
    Idle,
    /// The page came to the front or left it.
    Focus(bool),
}

pub async fn run(socket: WebSocket, state: AppState, session: CurrentSession) {
    let user = &session.user;
    let (tx, mut rx) = mpsc::channel(super::hub::OUTBOUND_CAPACITY);
    let (conn, mut revoked) = state
        .gateway
        .attach_session(user.id, session.key(), tx)
        .await;
    let Some(recheck) = state.gateway.recheck_signal(conn).await else {
        return;
    };
    // The upgrade itself just checked the session.
    let mut checked = Instant::now();
    // Fast path for the session's end; `lock_live` enforces it as well. The
    // expiry slides (#195): activity on this socket renews it below, and
    // requests or other sockets may have renewed it when the timer fires.
    let mut expires_at = session.expires_at;
    let expired = tokio::time::sleep_until(until(expires_at));
    tokio::pin!(expired);
    debug!(user_id = %user.id, "ws connected");

    let mut last_client = Instant::now();
    let mut last_activity = Instant::now();
    let mut idle = false;
    let mut focused = false;
    // A focus claim outlives a few missed heartbeats, not a dead socket.
    let focus_ttl = state.ws_dead * 2;
    let mut beat = tokio::time::interval(state.ws_heartbeat);
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires immediately; skip it so we do not heartbeat
    // before the client has had a chance to subscribe.
    beat.tick().await;
    // A per-socket phase spreads the checks over the second, so sockets that
    // reconnect together do not ask the pool on the same millisecond. Ticks
    // without a seat only check once `SECURITY_BACKSTOP` has passed.
    let phase = std::time::Duration::from_millis((conn.redis_id().as_u128() % 1000) as u64);
    let mut security = tokio::time::interval_at(
        tokio::time::Instant::now() + phase,
        std::time::Duration::from_secs(1),
    );
    security.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            biased;
            _ = revoked.changed() => break,
            _ = &mut expired => {
                match current_expiry(&state, &session).await {
                    Some(at) if at > chrono::Utc::now() => {
                        expires_at = at;
                        expired.as_mut().reset(until(at));
                    }
                    _ => break,
                }
            }
            _ = recheck.notified() => {
                if !security_tick(&state, &session, conn, &mut checked, true).await { break; }
            }
            _ = security.tick() => {
                if !security_tick(&state, &session, conn, &mut checked, false).await { break; }
            }
            incoming = stream.next() => {
                let mut live_hint = None;
                let mut active = false;
                let Ok(Ok(Some(mut session_guard))) = tokio::time::timeout(std::time::Duration::from_secs(1), session.lock_live(&state.db)).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
                checked = Instant::now();
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
                                active = true;
                                last_activity = Instant::now();
                                if idle {
                                    idle = false;
                                    let _ = state.gateway.set_conn_status(conn, PresenceStatus::Online).await;
                                }
                            }
                            Ok(FrameEffect::LiveStarted { server_id, channel_id }) => {
                                live_hint = Some((server_id, channel_id));
                                active = true;
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
                            Ok(FrameEffect::Focus(on)) => {
                                focused = on;
                                if on {
                                    active = true;
                                    last_activity = Instant::now();
                                }
                                if let Err(err) = state.gateway.set_focus(user.id, conn, on, focus_ttl).await {
                                    debug!(error = err.code(), "focus not stored");
                                }
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
                if active && let Some(at) = renew(&state, &session, expires_at).await {
                    expires_at = at;
                    expired.as_mut().reset(until(at));
                }
                if let Some((server_id, channel_id)) = live_hint {
                    crate::messages::post_live_hint(&state, user, server_id, channel_id).await;
                }
            }
            _ = beat.tick() => {
                let Ok(Ok(Some(mut session_guard))) = tokio::time::timeout(std::time::Duration::from_secs(1), session.lock_live(&state.db)).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
                checked = Instant::now();
                // Nothing below needs the transaction: return the connection
                // before the Redis writes and the socket write.
                if session_guard.rollback().await.is_err() { break; }
                if last_client.elapsed() >= state.ws_dead {
                    debug!(user_id = %user.id, "ws silent death");
                    break;
                }
                if !idle && last_activity.elapsed() >= state.ws_idle {
                    idle = true;
                    let _ = state.gateway.set_conn_status(conn, PresenceStatus::Idle).await;
                }
                if focused {
                    let _ = state.gateway.set_focus(user.id, conn, true, focus_ttl).await;
                }
                if send(&mut sink, ServerFrame::Heartbeat).await.is_err() {
                    break;
                }
            }
            frame = rx.recv() => {
                let Some(frame) = frame else { break };
                let Ok(batch) = take_batch(frame, &mut rx) else { break };
                match deliver(&state, &session, user.id, conn, &mut sink, batch).await {
                    Ok(true) => checked = Instant::now(),
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        }
    }

    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sink.send(Message::Close(None)),
    )
    .await;
    if focused {
        let _ = state
            .gateway
            .set_focus(user.id, conn, false, focus_ttl)
            .await;
    }
    if let Some((user_id, servers, typing)) = state.gateway.detach(conn).await {
        state
            .gateway
            .clear_conn(conn, user_id, &servers, &typing)
            .await;
    }
    debug!(user_id = %user.id, "ws disconnected");
}

fn until(at: chrono::DateTime<chrono::Utc>) -> tokio::time::Instant {
    tokio::time::Instant::now() + (at - chrono::Utc::now()).to_std().unwrap_or_default()
}

/// The session row's expiry now, or `None` once it is gone or unreadable.
async fn current_expiry(
    state: &AppState,
    session: &CurrentSession,
) -> Option<chrono::DateTime<chrono::Utc>> {
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sqlx::query_scalar(
            "SELECT expires_at FROM sessions WHERE token_hash = $1 AND user_id = $2",
        )
        .bind(&session.hash)
        .bind(session.user.id)
        .fetch_optional(&state.db),
    )
    .await
    .ok()?
    .ok()?
}

/// User activity on the socket counts as a use of the session, heartbeats
/// do not: a forgotten tab still ends after the idle timeout. Runs after the
/// frame's transaction committed, because the `UPDATE` would wait for the
/// `FOR SHARE` lock other sockets of this session hold. Throttled like HTTP
/// renewals, so most frames never reach the database. A failure only skips
/// the renewal; `None` keeps the current expiry.
async fn renew(
    state: &AppState,
    session: &CurrentSession,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if !crate::auth::session::needs_renewal(
        session.created_at,
        expires_at,
        state.session_idle,
        state.session_ttl,
    ) {
        return None;
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::auth::session::touch(state, &session.hash, session.created_at, expires_at),
    )
    .await
    .ok()?
    .ok()?
}

/// Recheck the session and every subscription, then renew voice leases.
/// `false` ends the socket. A socket with a seat checks on every tick, since
/// a renewal follows only a passed check; one without a seat only when
/// `forced` (a [`super::hub::Recheck`] notice) or after `SECURITY_BACKSTOP`
/// without any check. Every frame in and every delivered batch runs the same
/// check anyway. The transaction is read-only and ends before the renewal,
/// which does Redis I/O and waits on the process-wide `voice_ops` mutex.
async fn security_tick(
    state: &AppState,
    session: &CurrentSession,
    conn: ConnId,
    checked: &mut Instant,
    forced: bool,
) -> bool {
    if !forced && checked.elapsed() < SECURITY_BACKSTOP && !state.gateway.holds_seat(conn).await {
        return true;
    }
    let Ok(Ok(Some(mut session_guard))) = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        session.lock_live(&state.db),
    )
    .await
    else {
        return false;
    };
    if state
        .gateway
        .reconcile_access(conn, &mut session_guard)
        .await
        .is_err()
    {
        return false;
    }
    if session_guard.rollback().await.is_err() {
        return false;
    }
    *checked = Instant::now();
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            state.gateway.refresh_voice(conn),
        )
        .await,
        Ok(Ok(()))
    )
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
        ClientFrame::Focus { on } => Ok(FrameEffect::Focus(on)),
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
    } else {
        state.gateway.mark_dm(conn, server_id).await;
    }
    let topic = Topic::of(server_id, channel_id);
    state.gateway.watch_topic(conn, server_id, topic).await;
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

/// Frames already queued behind `first` share its dequeue check: one session
/// lock, one authorization per scope, one write. Bounded so a batch takes no
/// longer to write than one large frame did before. The byte bound is checked
/// before taking the next frame, so a batch holds at most `BATCH_BYTES` plus
/// one frame of any size.
const BATCH_FRAMES: usize = 64;
const BATCH_BYTES: usize = 16 * 1024;

type Scope = (Uuid, Option<Uuid>);

fn take_batch(
    first: ServerFrame,
    rx: &mut mpsc::Receiver<ServerFrame>,
) -> Result<Vec<(ServerFrame, String)>, ApiError> {
    let json = to_json(&first)?;
    let mut bytes = json.len();
    let mut batch = vec![(first, json)];
    while batch.len() < BATCH_FRAMES && bytes < BATCH_BYTES {
        let Ok(frame) = rx.try_recv() else { break };
        let json = to_json(&frame)?;
        bytes += json.len();
        batch.push((frame, json));
    }
    Ok(batch)
}

/// Write a batch as consecutive runs of frames for one server, each in its
/// own live-session transaction: one `reconcile_access`, one authorization
/// per scope, then write and end it. The server row lock from a run's scope
/// check is held until that run is flushed (so a kick cannot commit between
/// check and write), but never across another server's frames, and no
/// transaction holds more than one server row. Unscoped frames join the
/// current run; queue order is kept. `true` once a run passed its check.
async fn deliver(
    state: &AppState,
    session: &CurrentSession,
    user_id: Uuid,
    conn: ConnId,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    batch: Vec<(ServerFrame, String)>,
) -> Result<bool, ApiError> {
    let mut checked = false;
    for run in server_runs(batch) {
        let mut db = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            session.lock_live(&state.db),
        )
        .await
        .map_err(|_| ApiError::Internal("session lock timed out".into()))??
        .ok_or(ApiError::Unauthenticated)?;
        state.gateway.reconcile_access(conn, &mut db).await?;
        checked = true;
        let mut allowed = HashMap::new();
        let mut out = Vec::with_capacity(run.len());
        for (frame, json) in run {
            if !state.gateway.wants_frame(conn, &frame).await {
                continue;
            }
            let ok = match frame_scope(&frame) {
                None => true,
                Some(scope) => match allowed.get(&scope) {
                    Some(&ok) => ok,
                    // An error aborts the transaction: the caller ends the
                    // socket and the client resyncs.
                    None => *allowed
                        .entry(scope)
                        .or_insert(scope_allowed(&mut db, user_id, scope).await?),
                },
            };
            if ok {
                out.push(json);
            }
        }
        send_texts(sink, out).await?;
        // Read-only: end it now so this server's row lock is released before
        // the next run takes its own.
        db.rollback().await?;
    }
    Ok(checked)
}

/// Split a batch where the server of a scoped frame changes.
fn server_runs(batch: Vec<(ServerFrame, String)>) -> Vec<Vec<(ServerFrame, String)>> {
    let mut runs: Vec<Vec<(ServerFrame, String)>> = Vec::new();
    let mut current = None;
    for item in batch {
        let server = frame_scope(&item.0).map(|(sid, _)| sid);
        let split = match (current, server) {
            (Some(run), Some(sid)) => run != sid,
            _ => runs.is_empty(),
        };
        if server.is_some() {
            current = server;
        }
        match runs.last_mut() {
            Some(run) if !split => run.push(item),
            _ => runs.push(vec![item]),
        }
    }
    runs
}

fn frame_scope(frame: &ServerFrame) -> Option<Scope> {
    match frame {
        ServerFrame::Dm { c } => Some((*c, Some(*c))),
        ServerFrame::Event { s, c, .. }
        | ServerFrame::Gap { s, c, .. }
        | ServerFrame::Sig { s, c, .. } => Some((*s, *c)),
        ServerFrame::Presence { s, .. } => Some((*s, None)),
        ServerFrame::Typing { s, c, .. } => Some((*s, Some(*c))),
        _ => None,
    }
}

/// Check at dequeue time too: already queued events cannot leak after removal.
/// The server row lock stays held in `db`'s transaction until the frames of
/// this scope are written.
async fn scope_allowed(
    db: &mut sqlx::PgConnection,
    user_id: Uuid,
    (sid, cid): Scope,
) -> Result<bool, ApiError> {
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
}

fn to_json(frame: &ServerFrame) -> Result<String, ApiError> {
    frame
        .to_json()
        .map_err(|err| ApiError::Internal(format!("serialize ws frame: {err}")))
}

async fn send_texts(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    texts: Vec<String>,
) -> Result<(), ApiError> {
    if texts.is_empty() {
        return Ok(());
    }
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        for text in texts {
            sink.feed(Message::Text(Utf8Bytes::from(text))).await?;
        }
        sink.flush().await
    })
    .await
    .map_err(|_| ApiError::Internal("ws send timed out".into()))?
    .map_err(|err| ApiError::Internal(format!("ws send: {err}")))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(frame: ServerFrame) -> (ServerFrame, String) {
        let json = to_json(&frame).unwrap();
        (frame, json)
    }

    #[test]
    fn runs_split_only_where_the_server_changes() {
        let (a, b, c, u) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let batch = vec![
            item(ServerFrame::Heartbeat),
            item(ServerFrame::presence(a, u, PresenceStatus::Online)),
            item(ServerFrame::typing(a, c, u, true)),
            item(ServerFrame::Heartbeat),
            item(ServerFrame::presence(b, u, PresenceStatus::Online)),
            item(ServerFrame::presence(a, u, PresenceStatus::Idle)),
        ];
        let expected: Vec<Vec<String>> = vec![
            batch[..4].iter().map(|(_, json)| json.clone()).collect(),
            vec![batch[4].1.clone()],
            vec![batch[5].1.clone()],
        ];
        let runs: Vec<Vec<String>> = server_runs(batch)
            .into_iter()
            .map(|run| run.into_iter().map(|(_, json)| json).collect())
            .collect();
        assert_eq!(runs, expected);
    }

    /// Socket activity renews a session that is due, through the frame's
    /// stale copy of the expiry, and leaves a fresh one alone.
    #[sqlx::test]
    async fn activity_renews_a_due_session_only(pool: sqlx::PgPool) {
        use std::time::Duration;
        let config = crate::Config::from_source(|key| match key {
            "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".into()),
            "REDIS_URL" => Some(std::env::var("REDIS_URL").unwrap()),
            "API_SESSION_TTL_HOURS" => Some("4".to_owned()),
            "API_SESSION_IDLE_HOURS" => Some("1".to_owned()),
            _ => None,
        })
        .unwrap();
        let state = AppState::with_pool(&config, pool.clone()).unwrap();
        let user = crate::auth::user::insert(&pool, "renew@example.test", "renew", "unused")
            .await
            .unwrap();
        let raw = crate::auth::session::create(&pool, user.id, Duration::from_secs(120))
            .await
            .unwrap();
        let hash = crate::token::hash(&raw);
        let (created_at, expires_at): (
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ) = sqlx::query_as("SELECT created_at, expires_at FROM sessions WHERE token_hash = $1")
            .bind(&hash)
            .fetch_one(&pool)
            .await
            .unwrap();
        let session = CurrentSession {
            user,
            hash: hash.clone(),
            created_at,
            expires_at,
        };

        let renewed = renew(&state, &session, expires_at).await.expect("renewed");
        assert!(
            renewed - expires_at > chrono::Duration::minutes(55),
            "{renewed}"
        );
        let stored: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash = $1")
                .bind(&hash)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, renewed);
        assert_eq!(
            renew(&state, &session, renewed).await,
            None,
            "fresh: no write"
        );

        sqlx::query("DELETE FROM sessions")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            renew(&state, &session, expires_at).await,
            None,
            "gone: no revival"
        );
    }

    /// The tick waits on `voice_ops` while another voice operation holds it.
    /// On a one-connection pool, that wait must not keep the only connection.
    #[sqlx::test]
    async fn security_tick_returns_the_pool_connection_before_voice_renewal(pool: sqlx::PgPool) {
        use std::time::Duration;
        let config = crate::Config::from_source(|key| match key {
            "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".into()),
            "REDIS_URL" => Some(std::env::var("REDIS_URL").unwrap()),
            _ => None,
        })
        .unwrap();
        let small = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(2))
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        let state = AppState::with_pool(&config, small.clone()).unwrap();
        let user = crate::auth::user::insert(&pool, "tick@example.test", "tick", "unused")
            .await
            .unwrap();
        let raw = crate::auth::session::create(&pool, user.id, Duration::from_secs(600))
            .await
            .unwrap();
        let hash = crate::token::hash(&raw);
        let (created_at, expires_at) =
            sqlx::query_as("SELECT created_at, expires_at FROM sessions WHERE token_hash = $1")
                .bind(&hash)
                .fetch_one(&pool)
                .await
                .unwrap();
        let session = CurrentSession {
            user,
            hash,
            created_at,
            expires_at,
        };
        let server: Uuid = sqlx::query_scalar(
            "INSERT INTO servers (name, owner_id) VALUES ('Tick', $1) RETURNING id",
        )
        .bind(session.user.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO server_members (server_id, user_id) VALUES ($1, $2)")
            .bind(server)
            .bind(session.user.id)
            .execute(&pool)
            .await
            .unwrap();
        let voice: Uuid = sqlx::query_scalar(
            "INSERT INTO channels (server_id, name, kind) VALUES ($1, 'Voice', 'voice') RETURNING id",
        )
        .bind(server)
        .fetch_one(&pool)
        .await
        .unwrap();
        state
            .gateway
            .wait_ready(Duration::from_secs(2))
            .await
            .unwrap();
        let (tx, _rx) = mpsc::channel(super::super::hub::OUTBOUND_CAPACITY);
        let (conn, _revoked) = state
            .gateway
            .attach_session(session.user.id, session.key(), tx)
            .await;
        // A seat the owner may keep, so reconcile_access leaves it and the
        // renewal has to take voice_ops.
        state
            .gateway
            .join_voice(conn, session.user.id, server, voice)
            .await
            .unwrap();

        let busy = state.gateway.connections.inner.voice_ops.lock().await;
        let tick = tokio::spawn({
            let (state, session) = (state.clone(), session.clone());
            async move {
                let mut checked = Instant::now();
                security_tick(&state, &session, conn, &mut checked, false).await
            }
        });
        // Long enough for the session check and reconcile on a local database.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!tick.is_finished(), "the tick waits on voice_ops");
        let free = tokio::time::timeout(Duration::from_millis(200), small.acquire()).await;
        assert!(
            matches!(free, Ok(Ok(_))),
            "the tick kept the only pool connection while waiting on voice_ops"
        );
        drop(free);
        drop(busy);
        assert!(
            tick.await.unwrap(),
            "the tick renews once voice_ops is free"
        );
        assert!(state.gateway.in_voice(conn, server, voice).await);
        state.gateway.detach(conn).await;
    }
}
