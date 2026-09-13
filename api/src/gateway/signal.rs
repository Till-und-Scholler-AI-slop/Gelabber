//! Voice signaling on `op: "sig"` (issue #10).
//!
//! Session is already on the socket. Join checks membership, `join_voice`,
//! and a voice channel. Frames stay off the chat stream (`op: "e"`). The
//! SFU (issue 11) is not here — we only fan out signaling.

use axum::extract::ws::{Message, WebSocket};
use uuid::Uuid;

use super::hub::ConnId;
use super::protocol::{ClientFrame, ServerFrame, SigEvent, SigKind};
use crate::auth::user::User;
use crate::error::ApiError;
use crate::servers::channel::{Channel, ChannelKind};
use crate::servers::membership;
use crate::servers::permissions::Permission;
use crate::state::AppState;

const MAX_SDP: usize = 12_288;
const MAX_ICE: usize = 800;
const MAX_MID: usize = 32;

type Sink = futures_util::stream::SplitSink<WebSocket, Message>;

struct Call<'a> {
    state: &'a AppState,
    user: &'a User,
    conn: ConnId,
    server_id: Uuid,
    channel_id: Uuid,
    frame: &'a ClientFrame,
    sink: &'a mut Sink,
}

pub async fn handle(
    state: &AppState,
    user: &User,
    conn: ConnId,
    frame: ClientFrame,
    sink: &mut Sink,
) -> Result<(), ApiError> {
    let (Some(server_id), Some(channel_id), Some(kind)) = (frame.s, frame.c, frame.t) else {
        send_err(sink, "bad_request", frame.s, frame.c).await?;
        return Ok(());
    };

    let mut call = Call {
        state,
        user,
        conn,
        server_id,
        channel_id,
        frame: &frame,
        sink,
    };
    match kind {
        SigKind::J => join(&mut call).await,
        SigKind::L => leave(&mut call).await,
        SigKind::O | SigKind::A => negotiate(&mut call, kind).await,
        SigKind::I => ice(&mut call).await,
        SigKind::P | SigKind::U => publish(&mut call, kind).await,
        SigKind::M | SigKind::D => mute_deafen(&mut call, kind).await,
        SigKind::R => bad(&mut call).await,
    }
}

async fn join(call: &mut Call<'_>) -> Result<(), ApiError> {
    match authorize_join(
        &call.state.db,
        call.user.id,
        call.server_id,
        call.channel_id,
    )
    .await
    {
        Ok(()) => {}
        Err(err) => return reject(call, err).await,
    }

    let snapshot = call
        .state
        .gateway
        .join_voice(call.conn, call.user.id, call.server_id, call.channel_id)
        .await?;
    for event in snapshot {
        send_sig(call.sink, event).await?;
    }
    Ok(())
}

async fn leave(call: &mut Call<'_>) -> Result<(), ApiError> {
    let _ = call
        .state
        .gateway
        .leave_voice(call.conn, call.user.id, call.server_id, call.channel_id)
        .await?;
    Ok(())
}

async fn require_room(call: &mut Call<'_>) -> Result<bool, ApiError> {
    if call
        .state
        .gateway
        .in_voice(call.conn, call.channel_id)
        .await
    {
        return Ok(true);
    }
    send_err(
        call.sink,
        "bad_request",
        Some(call.server_id),
        Some(call.channel_id),
    )
    .await?;
    Ok(false)
}

async fn negotiate(call: &mut Call<'_>, kind: SigKind) -> Result<(), ApiError> {
    if !require_room(call).await? {
        return Ok(());
    }
    let Some(sdp) = call
        .frame
        .sdp
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return bad(call).await;
    };
    if sdp.len() > MAX_SDP {
        return bad(call).await;
    }
    call.state
        .gateway
        .publish_sig(SigEvent {
            t: kind,
            s: call.server_id,
            c: call.channel_id,
            u: call.user.id,
            sdp: Some(sdp.to_owned()),
            ice: None,
            mid: None,
            k: None,
            on: None,
            m: None,
            d: None,
        })
        .await
}

async fn ice(call: &mut Call<'_>) -> Result<(), ApiError> {
    if !require_room(call).await? {
        return Ok(());
    }
    let Some(ice) = call
        .frame
        .ice
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return bad(call).await;
    };
    if ice.len() > MAX_ICE {
        return bad(call).await;
    }
    let mid = match call.frame.mid.as_deref() {
        Some(mid) if mid.len() > MAX_MID => return bad(call).await,
        Some("") => None,
        other => other.map(str::to_owned),
    };
    call.state
        .gateway
        .publish_sig(SigEvent {
            t: SigKind::I,
            s: call.server_id,
            c: call.channel_id,
            u: call.user.id,
            sdp: None,
            ice: Some(ice.to_owned()),
            mid,
            k: None,
            on: None,
            m: None,
            d: None,
        })
        .await
}

async fn publish(call: &mut Call<'_>, kind: SigKind) -> Result<(), ApiError> {
    if !require_room(call).await? {
        return Ok(());
    }
    let Some(track) = call.frame.k else {
        return bad(call).await;
    };
    // Camera (`v`) and screen (`s`) are for anyone already in the voice
    // room. Go Live (issue 14) is a separate product surface.
    if !call
        .state
        .gateway
        .set_voice_pub(
            call.conn,
            call.user.id,
            call.server_id,
            call.channel_id,
            track,
            kind == SigKind::P,
        )
        .await?
    {
        return bad(call).await;
    }
    Ok(())
}

async fn mute_deafen(call: &mut Call<'_>, kind: SigKind) -> Result<(), ApiError> {
    if !require_room(call).await? {
        return Ok(());
    }
    let Some(on) = call.frame.on else {
        return bad(call).await;
    };
    let ok = if kind == SigKind::M {
        call.state
            .gateway
            .set_voice_mute(call.conn, call.user.id, call.server_id, call.channel_id, on)
            .await?
    } else {
        call.state
            .gateway
            .set_voice_deafen(call.conn, call.user.id, call.server_id, call.channel_id, on)
            .await?
    };
    if !ok {
        return bad(call).await;
    }
    Ok(())
}

async fn authorize_join(
    db: &sqlx::PgPool,
    user_id: Uuid,
    server_id: Uuid,
    channel_id: Uuid,
) -> Result<(), ApiError> {
    let member = membership::load(db, server_id, user_id).await?;
    member.require(Permission::JoinVoice)?;
    let channel = load_channel(db, server_id, channel_id).await?;
    if channel.kind != ChannelKind::Voice {
        return Err(ApiError::BadRequest(
            "Voice signaling is for voice channels.".into(),
        ));
    }
    Ok(())
}

async fn load_channel(
    db: &sqlx::PgPool,
    server_id: Uuid,
    channel_id: Uuid,
) -> Result<Channel, ApiError> {
    sqlx::query_as::<_, Channel>(
        "SELECT id, server_id, category_id, name, kind, created_at \
         FROM channels WHERE id = $1 AND server_id = $2",
    )
    .bind(channel_id)
    .bind(server_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

async fn reject(call: &mut Call<'_>, err: ApiError) -> Result<(), ApiError> {
    let code = match err {
        ApiError::NotFound => "not_found",
        ApiError::Forbidden(_) => "forbidden",
        ApiError::BadRequest(_) => "bad_request",
        other => return Err(other),
    };
    send_err(call.sink, code, Some(call.server_id), Some(call.channel_id)).await
}

async fn bad(call: &mut Call<'_>) -> Result<(), ApiError> {
    send_err(
        call.sink,
        "bad_request",
        Some(call.server_id),
        Some(call.channel_id),
    )
    .await
}

async fn send_err(
    sink: &mut Sink,
    code: &'static str,
    server_id: Option<Uuid>,
    channel_id: Option<Uuid>,
) -> Result<(), ApiError> {
    super::conn::send(sink, ServerFrame::error(code, server_id, channel_id)).await
}

async fn send_sig(sink: &mut Sink, event: SigEvent) -> Result<(), ApiError> {
    super::conn::send(sink, ServerFrame::sig(event)).await
}
