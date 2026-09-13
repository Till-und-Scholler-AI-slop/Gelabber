//! Short SFU join tickets (issue 11). Session + `join_voice` + voice
//! channel, then a 12-char code in Redis (`gb:mt:{code}`, 30 s). The
//! media binary consumes it. Not a LiveKit/JWT token.

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::post;
use serde::Serialize;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::ApiError;
use crate::path::Id;
use crate::servers::channel::{Channel, ChannelKind};
use crate::servers::membership;
use crate::servers::permissions::Permission;
use crate::state::AppState;

/// Same alphabet as invite codes (readable, 30^12).
const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
const CODE_LEN: usize = 12;
const REDIS_PREFIX: &str = "gb:mt:";

pub fn router() -> Router<AppState> {
    Router::new().route("/api/channels/{id}/media-ticket", post(issue_ticket))
}

#[derive(Debug, Clone, Serialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// Comma-separated STUN/TURN URLs. Credentials attach only to `turn:` /
/// `turns:` entries.
pub fn parse_ice_servers(
    urls: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) -> Vec<IceServer> {
    let Some(urls) = urls.filter(|raw| !raw.trim().is_empty()) else {
        return Vec::new();
    };
    let mut stun = Vec::new();
    let mut turn = Vec::new();
    for raw in urls.split(',') {
        let url = raw.trim();
        if url.is_empty() {
            continue;
        }
        if url.starts_with("turn:") || url.starts_with("turns:") {
            turn.push(url.to_owned());
        } else {
            stun.push(url.to_owned());
        }
    }
    let mut out = Vec::new();
    if !stun.is_empty() {
        out.push(IceServer {
            urls: stun,
            username: None,
            credential: None,
        });
    }
    if !turn.is_empty() {
        out.push(IceServer {
            urls: turn,
            username: username.map(str::to_owned).filter(|s| !s.is_empty()),
            credential: password.map(str::to_owned).filter(|s| !s.is_empty()),
        });
    }
    out
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

    let ticket = mint(
        &state.redis,
        user.id,
        server_id,
        channel.id,
        state.media_ticket_ttl,
    )
    .await?;

    Ok(Json(MediaTicket {
        ticket,
        expires_in: state.media_ticket_ttl.as_secs().max(1),
        media_path: "/media/ws",
        ice_servers: state.ice_servers.clone(),
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

fn generate_code() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    (0..CODE_LEN)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

async fn mint(
    redis: &redis::Client,
    user_id: Uuid,
    server_id: Uuid,
    channel_id: Uuid,
    ttl: Duration,
) -> Result<String, ApiError> {
    let payload = serde_json::json!({
        "u": user_id,
        "s": server_id,
        "c": channel_id,
    })
    .to_string();
    let secs = ttl.as_secs().max(1);
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| ApiError::Internal(format!("redis: {err}")))?;

    for _ in 0..8 {
        let code = generate_code();
        let key = format!("{REDIS_PREFIX}{code}");
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
        let code = generate_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)));
    }
}
