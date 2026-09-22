//! Short SFU join tickets (issue 11). Session + `join_voice` + voice
//! channel, then a 12-char code in Redis (`gb:mt:{code}`, 30 s). The
//! media binary consumes it. Not a LiveKit/JWT token.
//!
//! The payload is [`gelabber_shared::ticket::TicketClaim`], including
//! whether this caller may Go Live. The SFU refuses an `l` announce
//! without that bit.

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::post;
use gelabber_shared::ice::IceServer;
use gelabber_shared::ticket::{self, TicketClaim};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::ApiError;
use crate::path::Id;
use crate::servers::channel::{Channel, ChannelKind};
use crate::servers::membership;
use crate::servers::permissions::Permission;
use crate::state::AppState;

pub use gelabber_shared::ice::parse_ice_servers;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/channels/{id}/media-ticket", post(issue_ticket))
}

#[derive(Debug, Serialize)]
pub struct MediaTicket {
    pub ticket: String,
    pub expires_in: u64,
    pub media_path: &'static str,
    pub ice_servers: Vec<IceServer>,
}

async fn issue_ticket(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
) -> Result<Json<MediaTicket>, ApiError> {
    let channel = load_channel(&state.db, channel_id).await?;
    // DMs have no server — same 404 as `channel_for` / `messaging_channel`.
    let server_id = channel.server_id.ok_or(ApiError::NotFound)?;
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::JoinVoice)?;
    if channel.kind != ChannelKind::Voice {
        return Err(ApiError::BadRequest(
            "Media tickets are for voice channels.".into(),
        ));
    }
    let go_live = member.can(Permission::GoLive);

    let ticket = mint(
        &state.redis,
        TicketClaim {
            u: user.id,
            s: server_id,
            c: channel.id,
            g: go_live,
        },
        state.media_ticket_ttl,
    )
    .await?;

    Ok(Json(MediaTicket {
        ticket,
        expires_in: state.media_ticket_ttl.as_secs().max(1),
        media_path: "/media/ws",
        ice_servers: state.ticket_ice_servers(user.id),
    }))
}

async fn load_channel(db: &sqlx::PgPool, channel_id: Uuid) -> Result<Channel, ApiError> {
    sqlx::query_as::<_, Channel>(
        "SELECT id, server_id, category_id, name, kind, created_at \
         FROM channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

async fn mint(
    redis: &redis::Client,
    claim: TicketClaim,
    ttl: Duration,
) -> Result<String, ApiError> {
    let payload = ticket::encode(&claim)
        .map_err(|err| ApiError::Internal(format!("serialize media ticket: {err}")))?;
    let secs = ttl.as_secs().max(1);
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;

    for _ in 0..8 {
        let code = ticket::generate();
        let key = ticket::redis_key(&code);
        let set: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(&payload)
            .arg("EX")
            .arg(secs)
            .arg("NX")
            .query_async(&mut conn)
            .await
            .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;
        if set.as_deref() == Some("OK") {
            return Ok(code);
        }
    }
    Err(ApiError::Internal(
        "could not allocate a media ticket".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_media_contract() {
        let code = ticket::generate();
        assert_eq!(code.len(), ticket::CODE_LEN);
        assert!(code.bytes().all(|b| ticket::ALPHABET.contains(&b)));
    }
}
