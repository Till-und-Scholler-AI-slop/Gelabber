//! One WebSocket: session already checked, then subscribe / heartbeat /
//! catch-up / presence / typing.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use super::budget::{self, Admit, Class, FrameBudget};
use super::hub::ConnId;
use super::protocol::{
    CatchUp, ClientFrame, PresenceMode, PresenceStatus, ServerFrame, SigKind, Topic, VoiceEntry,
};
use crate::auth::session::CurrentSession;
use crate::auth::user::User;
use crate::error::ApiError;
use crate::servers::channel::{self, ChannelKind};
use crate::servers::membership;
use crate::state::AppState;

/// Largest client message and frame. The upgrade hands it to the WebSocket
/// layer, which refuses a bigger one before buffering it and ends the socket.
pub(super) const MAX_CLIENT_BYTES: usize = 16 * 1024;

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
    /// This client is going idle (explicit `st:i`).
    Idle,
    /// The page came to the front or left it.
    Focus(bool),
}

/// What a client frame answers. Its checks run in the frame's transaction,
/// under the server row lock; the answer is written only once that
/// transaction has ended, so a client that stops reading holds neither the
/// lock (a kick or ban waits for it) nor a pool connection (GEL-42). A
/// failed write ends the socket, so the client reconnects and resumes.
#[derive(Default)]
struct Reply {
    frames: Vec<ServerFrame>,
    subscribed: Option<Subscribed>,
}

/// A subscribe that passed its checks, with everything it sends read while
/// they held.
struct Subscribed {
    server_id: Uuid,
    topic: Topic,
    current: u64,
    epoch: Uuid,
    /// Replay or gap, then `ok`.
    texts: Vec<String>,
    presence: Option<ServerFrame>,
    roster: Option<Vec<VoiceEntry>>,
}

/// A message read off the socket. Text is parsed as it is read, so the
/// budget can tell typing and signal frames apart.
enum Inbound {
    /// A client frame, or `None` for text that is none.
    Text(Option<ClientFrame>),
    /// Ping, pong or binary.
    Other(Message),
}

/// A frame read but not yet let through by the budget, kept across the
/// select loop's other branches. `counted`: already in the metric.
struct Held {
    inbound: Inbound,
    counted: bool,
}

impl Held {
    fn class(&self) -> Class {
        match &self.inbound {
            Inbound::Text(Some(ClientFrame::Typing { .. })) => Class::Typing,
            Inbound::Text(Some(ClientFrame::Sig { .. })) => Class::Signal,
            Inbound::Text(Some(ClientFrame::Presence { .. })) => Class::Presence,
            _ => Class::Frame,
        }
    }
}

enum Read {
    Frame(Inbound),
    /// A typing frame over its budget: liveness only.
    Dropped,
    /// Close, a read error or the end of the stream.
    End,
}

/// The next frame the socket's budget lets through. While a frame waits for
/// it, nothing more is read, so the client's sends back up in TCP. Cancel
/// safe: a waiting frame stays in `held` for the next call.
async fn next_inbound(
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    held: &mut Option<Held>,
    budget: &mut FrameBudget,
    metrics: &crate::metrics::HttpMetrics,
) -> Read {
    loop {
        let mut frame = match held.take() {
            Some(frame) => frame,
            None => {
                let inbound = match stream.next().await {
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Read::End,
                    Some(Ok(Message::Text(text))) => {
                        Inbound::Text(serde_json::from_str(&text).ok())
                    }
                    Some(Ok(other)) => Inbound::Other(other),
                };
                Held {
                    inbound,
                    counted: false,
                }
            }
        };
        match budget.admit(frame.class(), Instant::now()) {
            Admit::Handle => return Read::Frame(frame.inbound),
            Admit::Drop => {
                metrics.rate_limited(budget::TYPING_BUCKET);
                return Read::Dropped;
            }
            Admit::Wait { wait, bucket } => {
                if !frame.counted {
                    metrics.rate_limited(bucket);
                    frame.counted = true;
                }
                *held = Some(frame);
                tokio::time::sleep(wait).await;
            }
        }
    }
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
    // Redis lost the manual mode (a restart): put the record's back.
    crate::presence::restore(&state, None, user.id).await;

    let mut last_client = Instant::now();
    let mut last_activity = Instant::now();
    let mut idle = false;
    let mut focused = false;
    let mut rosters = Rosters::new();
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
    let mut shutdown = state.gateway.shutdown_signal();
    let mut close = None;
    let mut budget = FrameBudget::new(&state.limits, Instant::now());
    let mut held = None;

    loop {
        tokio::select! {
            biased;
            _ = revoked.changed() => break,
            _ = async { drop(shutdown.wait_for(|on| *on).await) } => {
                // 1012 Service Restart: the client reconnects to the next process.
                close = Some(CloseFrame { code: 1012, reason: Utf8Bytes::from_static("restart") });
                break;
            }
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
            // The budget comes before the session lock: a frame over it
            // costs no database or Redis work.
            read = next_inbound(&mut stream, &mut held, &mut budget, &state.metrics) => {
                let incoming = match read {
                    Read::Frame(incoming) => incoming,
                    Read::Dropped => {
                        last_client = Instant::now();
                        continue;
                    }
                    Read::End => break,
                };
                let mut active = false;
                let mut reply = Reply::default();
                let mut pong = None;
                let Ok(Some(mut session_guard)) = session.lock_live_within(&state.gateway_db, SESSION_LOCK_WAIT).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
                checked = Instant::now();
                match incoming {
                    Inbound::Text(frame) => {
                        last_client = Instant::now();
                        match handle_text(&state, &mut session_guard, user, conn, frame, &mut reply).await {
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
                                reply.frames.push(ServerFrame::error("internal", None, None));
                            }
                        }
                    }
                    Inbound::Other(Message::Ping(payload)) => {
                        last_client = Instant::now();
                        pong = Some(payload);
                    }
                    // Pong or binary.
                    Inbound::Other(_) => {
                        last_client = Instant::now();
                    }
                }
                // Return the only pool connection before the renewal below
                // acquires one, and release the row locks before writing.
                if session_guard.commit().await.is_err() { break; }
                if let Some(payload) = pong
                    && !matches!(tokio::time::timeout(std::time::Duration::from_secs(1), sink.send(Message::Pong(payload))).await, Ok(Ok(())))
                {
                    break;
                }
                if write_reply(&state, conn, &mut sink, reply, &mut rosters).await.is_err() { break; }
                if active && let Some(at) = renew(&state, &session, expires_at).await {
                    expires_at = at;
                    expired.as_mut().reset(until(at));
                }
            }
            _ = beat.tick() => {
                let Ok(Some(mut session_guard)) = session.lock_live_within(&state.gateway_db, SESSION_LOCK_WAIT).await else { break; };
                if state.gateway.reconcile_access(conn, &mut session_guard).await.is_err() { break; }
                checked = Instant::now();
                // Nothing below needs the transaction: return the connection
                // before the Redis writes and the socket write.
                if session_guard.rollback().await.is_err() { break; }
                // A frame waiting for the budget shows the client is there.
                if last_client.elapsed() >= state.ws_dead && held.is_none() {
                    debug!(user_id = %user.id, "ws silent death");
                    break;
                }
                let listening = state.gateway.listening(conn).await;
                if idle_timer(&mut last_activity, idle, listening, state.ws_idle) {
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
                let Ok(batch) = take_batch(frame, &mut rx, user.id) else { break };
                match deliver(&state, &session, user.id, conn, &mut sink, batch, &mut rosters).await {
                    Ok(true) => checked = Instant::now(),
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        }
    }

    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sink.send(Message::Close(close)),
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

/// The socket's own idle timer, each heartbeat: `true` when it goes idle
/// now. A seat the user can hear in counts as activity (#566), so the timer
/// starts when they leave or deafen. An explicit `st:i` still counts.
fn idle_timer(
    last_activity: &mut Instant,
    idle: bool,
    listening: bool,
    limit: std::time::Duration,
) -> bool {
    if listening {
        *last_activity = Instant::now();
        return false;
    }
    !idle && last_activity.elapsed() >= limit
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
        crate::auth::session::touch(state, &session.hash, session.created_at, expires_at, None),
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
    let Ok(Some(mut session_guard)) = session
        .lock_live_within(&state.gateway_db, SESSION_LOCK_WAIT)
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
    frame: Option<ClientFrame>,
    reply: &mut Reply,
) -> Result<FrameEffect, ApiError> {
    let frame = match frame {
        Some(frame) => frame,
        None => {
            reply
                .frames
                .push(ServerFrame::error("bad_request", None, None));
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
            // The scope tells the client which subscribe to retry. A database
            // error also fails the frame's commit, which ends the socket.
            if let Err(err) = subscribe(state, db, user, conn, (s, c), (n, ep), reply).await {
                state.gateway.unsubscribe(conn, Topic::of(s, c)).await;
                warn!(error = err.code(), user_id = %user.id, "ws subscribe failed");
                reply
                    .frames
                    .push(ServerFrame::error("internal", Some(s), c));
            }
            Ok(FrameEffect::Activity)
        }
        ClientFrame::Unsubscribe { s, c } => {
            state.gateway.unsubscribe(conn, Topic::of(s, c)).await;
            Ok(FrameEffect::Activity)
        }
        ClientFrame::Sig { .. } => {
            super::signal::handle(state, db, user, conn, frame, &mut reply.frames).await?;
            Ok(FrameEffect::Activity)
        }
        ClientFrame::Focus { on } => Ok(FrameEffect::Focus(on)),
        ClientFrame::Presence { st } => Ok(match st {
            Some(PresenceStatus::Idle) => FrameEffect::Idle,
            _ => FrameEffect::Activity,
        }),
        ClientFrame::Typing { s, c, on } => {
            typing(state, db, user, conn, (s, c), on, &mut reply.frames).await?;
            Ok(FrameEffect::Activity)
        }
    }
}

/// Check and register a subscribe and read everything it sends, under the
/// server row lock. Nothing is written here: [`write_reply`] does that once
/// the frame's transaction has ended.
async fn subscribe(
    state: &AppState,
    db: &mut sqlx::PgConnection,
    user: &User,
    conn: ConnId,
    scope: (Uuid, Option<Uuid>),
    resume: (Option<u64>, Option<Uuid>),
    reply: &mut Reply,
) -> Result<(), ApiError> {
    let (server_id, channel_id) = scope;
    let _guard = if channel_id.is_some_and(|cid| cid == server_id) {
        None // DM authorization below still verifies participants.
    } else {
        match membership::lock_server_conn(db, server_id, false).await {
            Ok(()) => Some(()),
            Err(ApiError::NotFound) => {
                reply
                    .frames
                    .push(ServerFrame::error("not_found", Some(server_id), channel_id));
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    };
    match authorize(&mut *db, user.id, server_id, channel_id).await {
        Ok(()) => {}
        Err(err @ (ApiError::NotFound | ApiError::ServerBlocked)) => {
            reply
                .frames
                .push(ServerFrame::error(err.code(), Some(server_id), channel_id));
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
    let mut texts = Vec::new();
    match plan {
        CatchUp::None => {}
        CatchUp::Replay(events) => {
            for event in events {
                if let Some(frame) = ServerFrame::event(event).for_user(user.id) {
                    texts.push(to_json(&frame)?);
                }
            }
        }
        CatchUp::Gap => {
            texts.push(to_json(
                &ServerFrame::gap(server_id, channel_id).with_epoch(epoch),
            )?);
        }
    }
    texts.push(to_json(
        &ServerFrame::subscribed(server_id, channel_id, current).with_epoch(epoch),
    )?);

    // Presence is ephemeral: its snapshot goes out after the sequenced `ok`
    // so it never looks like catch-up and never bumps `n`. A failure here
    // leaves the chat subscription standing; the roster pass and later
    // presence frames fill in. The announce stays under the server row lock:
    // once a kick has cleared this user's presence on the server, it must not
    // put it back.
    // After a Redis restart the sockets only subscribe again (`resync`):
    // the manual mode has to be back before this one announces (#590).
    crate::presence::restore(state, Some(&mut *db), user.id).await;
    if let Err(err) = state.gateway.announce_server(conn, server_id).await {
        warn!(error = err.code(), "presence announce on subscribe failed");
    }
    let presence = match state.gateway.presence_snapshot(server_id).await {
        Ok(snap) => Some(ServerFrame::presence_snap(server_id, snap)),
        Err(err) => {
            warn!(error = err.code(), "presence snapshot on subscribe failed");
            None
        }
    };
    let roster = match state.gateway.voice_snapshot(server_id).await {
        Ok(voice) => Some(if _guard.is_some() {
            visible_seats(voice, &hidden_rooms(&mut *db, server_id, user.id).await?)
        } else {
            voice
        }),
        Err(err) => {
            warn!(error = err.code(), "voice snapshot on subscribe failed");
            None
        }
    };
    reply.subscribed = Some(Subscribed {
        server_id,
        topic,
        current,
        epoch,
        texts,
        presence,
        roster,
    });
    Ok(())
}

/// Write a client frame's [`Reply`] once its transaction has ended. `Err`
/// ends the socket.
async fn write_reply(
    state: &AppState,
    conn: ConnId,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    reply: Reply,
    rosters: &mut Rosters,
) -> Result<(), ApiError> {
    for frame in reply.frames {
        send(sink, frame).await?;
    }
    let Some(sub) = reply.subscribed else {
        return Ok(());
    };
    let written = async {
        // The whole replay shares one deadline rather than one per frame.
        send_texts_within(sink, sub.texts, state.ws_replay_deadline).await?;
        // Live Pub/Sub frames that arrived since the log was read sit in the
        // per-socket queue. Flush them now (drop dups by `n`). A removal
        // since the check has dropped the topic and its queue already.
        state
            .gateway
            .connections
            .finish_catch_up_epoch(conn, sub.topic, sub.current, Some(sub.epoch))
            .await;
        if let Some(frame) = sub.presence {
            send(sink, frame).await?;
        }
        if let Some(seats) = sub.roster {
            send(sink, ServerFrame::voice_snap(sub.server_id, seats.clone())).await?;
            rosters.insert(sub.server_id, seats);
        }
        Ok::<_, ApiError>(())
    }
    .await;
    if let Err(err) = written {
        state.gateway.unsubscribe(conn, sub.topic).await;
        debug!(error = err.code(), "ws subscribe write failed");
        return Err(err);
    }
    Ok(())
}

/// The limited channels of `server_id` that `user_id` cannot see. Their
/// seats leave the voice roster, so nobody learns who sits or is live there.
async fn hidden_rooms(
    db: &mut sqlx::PgConnection,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<HashSet<Uuid>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM channels WHERE server_id = $1 AND limited AND NOT channel_visible_to(id, $2)",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .collect())
}

fn visible_seats(seats: Vec<VoiceEntry>, hidden: &HashSet<Uuid>) -> Vec<VoiceEntry> {
    seats
        .into_iter()
        .filter(|seat| !hidden.contains(&seat.c))
        .collect()
}

async fn typing(
    state: &AppState,
    db: &mut sqlx::PgConnection,
    user: &User,
    conn: ConnId,
    scope: (Uuid, Uuid),
    on: bool,
    out: &mut Vec<ServerFrame>,
) -> Result<(), ApiError> {
    let (server_id, channel_id) = scope;
    let _guard = if channel_id == server_id {
        None
    } else {
        match membership::lock_server_conn(db, server_id, false).await {
            Ok(()) => Some(()),
            Err(ApiError::NotFound) => {
                out.push(ServerFrame::error(
                    "not_found",
                    Some(server_id),
                    Some(channel_id),
                ));
                return Ok(());
            }
            Err(err) => return Err(err),
        }
    };
    match authorize(&mut *db, user.id, server_id, Some(channel_id)).await {
        Ok(()) => {}
        Err(err @ (ApiError::NotFound | ApiError::ServerBlocked)) => {
            out.push(ServerFrame::error(
                err.code(),
                Some(server_id),
                Some(channel_id),
            ));
            return Ok(());
        }
        Err(other) => return Err(other),
    }
    // Invisible (#590): nobody sees this account type. A stop still goes
    // out, for a start sent before the switch.
    if on
        && state
            .gateway
            .presence_mode(user.id)
            .await
            .unwrap_or_default()
            == PresenceMode::Invisible
    {
        return Ok(());
    }
    // A DM closed by a block carries no typing either way (#565); stopping
    // still goes through so nothing is left on.
    if on
        && channel_id == server_id
        && crate::blocks::in_dm(&mut *db, channel_id, user.id).await? != crate::blocks::Block::None
    {
        return Ok(());
    }
    state
        .gateway
        .set_typing(conn, server_id, channel_id, user.id, on)
        .await
}

/// Membership + (optional) channel belongs to that server and is visible to
/// the caller — or the caller is a participant of a 1:1 DM. For DMs the
/// protocol `s` is the channel id. Same 404 semantics as the REST API:
/// unknown, foreign and hidden are indistinguishable.
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
    let member = membership::load(&mut *db, server_id, user_id).await?;
    if !member.sees(&channel) {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Frames already queued behind `first` share its dequeue check: one session
/// lock, one authorization per scope, one write. Bounded so a batch takes no
/// longer to write than one large frame did before. The byte bound is checked
/// before taking the next frame, so a batch holds at most `BATCH_BYTES` plus
/// one frame of any size.
const BATCH_FRAMES: usize = 64;

/// How long a socket's check waits for its session row lock, after it has a
/// pool connection ([`CurrentSession::lock_live_within`]).
const SESSION_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
const BATCH_BYTES: usize = 16 * 1024;

type Scope = (Uuid, Option<Uuid>);

/// The voice roster this socket last received per server, as filtered for
/// its user. A roster pass that changes only a hidden room filters to the
/// same list and is dropped, so the frame itself does not tell that
/// something happened there.
type Rosters = HashMap<Uuid, Vec<VoiceEntry>>;

/// Frames queued for the socket of `user`, each as they may read it,
/// without those not for them ([`ServerFrame::for_user`]): possibly none.
fn take_batch(
    first: ServerFrame,
    rx: &mut mpsc::Receiver<ServerFrame>,
    user: Uuid,
) -> Result<Vec<(ServerFrame, String)>, ApiError> {
    let mut batch = Vec::new();
    let mut bytes = 0;
    let mut next = Some(first);
    while let Some(frame) = next {
        if let Some(frame) = frame.for_user(user) {
            let json = to_json(&frame)?;
            bytes += json.len();
            batch.push((frame, json));
        }
        if batch.len() >= BATCH_FRAMES || bytes >= BATCH_BYTES {
            break;
        }
        next = rx.try_recv().ok();
    }
    Ok(batch)
}

/// Write a batch as consecutive runs of frames for one server, each checked
/// in its own live-session transaction: one `reconcile_access`, one
/// authorization per scope, then end it and write. No transaction holds more
/// than one server row, and none is open while the socket is written to: a
/// client that stops reading must not hold off a kick or ban (GEL-42). Every
/// frame of a run was queued before its check, so a removal that commits
/// after the check only sees frames from before it go out; later frames are
/// checked again. Unscoped frames join the current run; queue order is kept.
/// `true` once a run passed its check.
async fn deliver(
    state: &AppState,
    session: &CurrentSession,
    user_id: Uuid,
    conn: ConnId,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    batch: Vec<(ServerFrame, String)>,
    rosters: &mut Rosters,
) -> Result<bool, ApiError> {
    let mut checked = false;
    let dms = state.gateway.socket_dms(conn).await;
    for run in server_runs(batch) {
        let mut db = session
            .lock_live_within(&state.gateway_db, SESSION_LOCK_WAIT)
            .await?
            .ok_or(ApiError::Unauthenticated)?;
        state.gateway.reconcile_access(conn, &mut db).await?;
        checked = true;
        let mut allowed = HashMap::new();
        let mut hidden: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
        let mut out = Vec::with_capacity(run.len());
        for (frame, json) in run {
            if !state.gateway.wants_frame(conn, &frame).await {
                continue;
            }
            let ok = match dequeue_scope(&frame, &dms) {
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
            if !ok {
                continue;
            }
            // A roster is server-wide: re-serialize it without hidden rooms,
            // and drop it when this socket already has that list.
            match frame {
                ServerFrame::Sig {
                    t: SigKind::R,
                    s,
                    snap: Some(seats),
                    ..
                } => {
                    let hidden = match hidden.entry(s) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(hidden_rooms(&mut db, s, user_id).await?)
                        }
                    };
                    let (seats, json) = if seats.iter().any(|seat| hidden.contains(&seat.c)) {
                        let seats = visible_seats(seats, hidden);
                        let json = to_json(&ServerFrame::voice_snap(s, seats.clone()))?;
                        (seats, json)
                    } else {
                        (seats, json)
                    };
                    if rosters.get(&s) != Some(&seats) {
                        rosters.insert(s, seats);
                        out.push(json);
                    }
                }
                _ => out.push(json),
            }
        }
        // Read-only: end it before the write, which may wait on the client.
        db.rollback().await?;
        send_texts(sink, out).await?;
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

/// [`frame_scope`] for the dequeue check. Presence on a DM is published on
/// the DM id, which is no server row: it is checked as the DM it is. A DM
/// discovery notice carries only the channel id and is only ever queued for
/// that DM's own participants; it passes unchecked, so the notice that a
/// deleted account's DM is gone (`account::deletion`) reaches the peer.
fn dequeue_scope(frame: &ServerFrame, dms: &HashSet<Uuid>) -> Option<Scope> {
    if matches!(frame, ServerFrame::Dm { .. }) {
        return None;
    }
    match frame_scope(frame) {
        Some((sid, None))
            if matches!(frame, ServerFrame::Presence { .. }) && dms.contains(&sid) =>
        {
            Some((sid, Some(sid)))
        }
        scope => scope,
    }
}

/// Check at dequeue time too: already queued events cannot leak after removal.
/// The server row lock stays held in `db`'s transaction until the run's
/// check is done, so no removal commits in the middle of it.
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
        Err(ApiError::NotFound | ApiError::ServerBlocked) => Ok(false),
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
    send_texts_within(sink, texts, std::time::Duration::from_secs(1)).await
}

async fn send_texts_within(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    texts: Vec<String>,
    deadline: std::time::Duration,
) -> Result<(), ApiError> {
    if texts.is_empty() {
        return Ok(());
    }
    tokio::time::timeout(deadline, async {
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

    /// The server's own idle timer leaves a socket in a call alone and
    /// starts counting when the call ends (#566).
    #[test]
    fn idle_timer_waits_while_listening_in_voice() {
        use std::time::Duration;
        let limit = Duration::from_secs(300);
        let long_ago = Instant::now() - Duration::from_secs(3600);

        let mut last = long_ago;
        assert!(idle_timer(&mut last, false, false, limit), "no call: idle");
        let mut last = long_ago;
        assert!(!idle_timer(&mut last, true, false, limit), "already idle");

        let mut last = long_ago;
        assert!(!idle_timer(&mut last, false, true, limit), "in a call");
        assert!(
            last.elapsed() < Duration::from_secs(1),
            "the call counts as activity"
        );
        assert!(
            !idle_timer(&mut last, false, false, limit),
            "left: the five minutes start now"
        );
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
        let user = crate::auth::user::insert(
            &pool,
            "renew@example.test",
            "renew",
            "unused",
            Default::default(),
        )
        .await
        .unwrap();
        let raw = crate::auth::session::create(
            &pool,
            user.id,
            Duration::from_secs(120),
            crate::auth::device::Device::UNKNOWN,
        )
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
            id: uuid::Uuid::nil(),
            device_client: crate::auth::device::DeviceClient::Unknown,
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
        let user = crate::auth::user::insert(
            &pool,
            "tick@example.test",
            "tick",
            "unused",
            Default::default(),
        )
        .await
        .unwrap();
        let raw = crate::auth::session::create(
            &pool,
            user.id,
            Duration::from_secs(600),
            crate::auth::device::Device::UNKNOWN,
        )
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
            id: uuid::Uuid::nil(),
            device_client: crate::auth::device::DeviceClient::Unknown,
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

    /// #612: a socket's check waits for a busy pool (bounded by the pool's
    /// acquire timeout) instead of ending the socket after a second, but a
    /// session row held by a logout still ends it after `SESSION_LOCK_WAIT`.
    #[sqlx::test]
    async fn socket_check_waits_for_the_pool_but_not_for_a_held_session(pool: sqlx::PgPool) {
        use std::time::Duration;
        let small = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        let user = crate::auth::user::insert(
            &pool,
            "wait@example.test",
            "wait",
            "unused",
            Default::default(),
        )
        .await
        .unwrap();
        let raw = crate::auth::session::create(
            &pool,
            user.id,
            Duration::from_secs(600),
            crate::auth::device::Device::UNKNOWN,
        )
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
            hash: hash.clone(),
            created_at,
            expires_at,
            id: uuid::Uuid::nil(),
            device_client: crate::auth::device::DeviceClient::Unknown,
        };

        // The only connection is busy for longer than the row lock may wait.
        let busy = small.acquire().await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(SESSION_LOCK_WAIT + Duration::from_millis(500)).await;
            drop(busy);
        });
        let started = std::time::Instant::now();
        let tx = session
            .lock_live_within(&small, SESSION_LOCK_WAIT)
            .await
            .expect("waits for the pool")
            .expect("session is live");
        assert!(started.elapsed() > SESSION_LOCK_WAIT);
        tx.rollback().await.unwrap();
        release.await.unwrap();

        // A logout holding the row ends the check after the lock wait.
        let mut logout = pool.begin().await.unwrap();
        sqlx::query("SELECT 1 FROM sessions WHERE token_hash = $1 FOR UPDATE")
            .bind(&hash)
            .execute(&mut *logout)
            .await
            .unwrap();
        let started = std::time::Instant::now();
        assert!(
            session
                .lock_live_within(&small, SESSION_LOCK_WAIT)
                .await
                .is_err()
        );
        assert!(started.elapsed() < SESSION_LOCK_WAIT + Duration::from_secs(1));
        logout.rollback().await.unwrap();
    }
}
