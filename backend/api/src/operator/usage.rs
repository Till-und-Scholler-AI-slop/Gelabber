//! `GET /api/operator/usage`: the page "Auslastung". Only numbers the API
//! has today: the database, this process's sockets and the voice seats in
//! Redis. Media-server numbers (rooms, peers, bytes) stay in Grafana: the
//! API cannot read the media service's `/metrics`.

use axum::extract::State;
use axum::response::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::access::Operator;
use super::audit::{self, Action, Targets};
use crate::config::RegistrationMode;
use crate::error::ApiError;
use crate::state::AppState;

pub fn router() -> axum::Router<AppState> {
    use axum::routing::get;
    axum::Router::new().route("/api/operator/usage", get(usage))
}

#[derive(Debug, Serialize)]
pub struct Usage {
    pub generated_at: DateTime<Utc>,
    pub registration: RegistrationMode,
    pub accounts: Accounts,
    pub servers: Servers,
    pub messages: Messages,
    pub storage: Storage,
    pub now: Now,
}

/// From `users` and `sessions`. `sessions` has no last use, so there is no
/// "active today".
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Accounts {
    pub total: i64,
    /// Registered, address not confirmed yet, not disabled.
    pub pending: i64,
    pub disabled: i64,
    pub new_7d: i64,
    pub new_30d: i64,
    /// Accounts with at least one session that has not expired.
    pub with_live_session: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Servers {
    pub total: i64,
    pub blocked: i64,
    pub delisted: i64,
}

/// Messages by when they were sent, in server channels and in DMs.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Messages {
    pub server_24h: i64,
    pub server_7d: i64,
    pub dm_7d: i64,
}

/// Attachments of sent messages, and the bytes uploaded today (UTC).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Storage {
    pub attachment_bytes: i64,
    pub attachment_count: i64,
    pub uploaded_today_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct Now {
    /// Sockets of this API process only; Compose runs one.
    pub sockets_this_process: usize,
    /// People in a voice channel, on every API process.
    pub in_voice: usize,
    /// Of those, people sharing their screen.
    pub live: usize,
}

async fn usage(State(state): State<AppState>, op: Operator) -> Result<Json<Usage>, ApiError> {
    let accounts: Accounts = sqlx::query_as(
        "SELECT COUNT(*) AS total, \
             COUNT(*) FILTER (WHERE disabled_at IS NULL AND registration_pending \
                 AND email_verified_at IS NULL) AS pending, \
             COUNT(*) FILTER (WHERE disabled_at IS NOT NULL) AS disabled, \
             COUNT(*) FILTER (WHERE created_at > now() - interval '7 days') AS new_7d, \
             COUNT(*) FILTER (WHERE created_at > now() - interval '30 days') AS new_30d, \
             (SELECT COUNT(DISTINCT user_id) FROM sessions WHERE expires_at > now()) \
                 AS with_live_session \
         FROM users WHERE deleted_at IS NULL",
    )
    .fetch_one(&state.db)
    .await?;
    let servers: Servers = sqlx::query_as(
        "SELECT COUNT(*) AS total, \
             COUNT(*) FILTER (WHERE blocked_at IS NOT NULL) AS blocked, \
             COUNT(*) FILTER (WHERE delisted_at IS NOT NULL) AS delisted \
         FROM servers",
    )
    .fetch_one(&state.db)
    .await?;
    let messages: Messages = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE c.kind <> 'dm' \
                 AND m.created_at > now() - interval '24 hours') AS server_24h, \
             COUNT(*) FILTER (WHERE c.kind <> 'dm') AS server_7d, \
             COUNT(*) FILTER (WHERE c.kind = 'dm') AS dm_7d \
         FROM messages m JOIN channels c ON c.id = m.channel_id \
         WHERE m.created_at > now() - interval '7 days'",
    )
    .fetch_one(&state.db)
    .await?;
    let storage: Storage = sqlx::query_as(
        "SELECT COALESCE(SUM(size_bytes), 0)::bigint AS attachment_bytes, \
             COUNT(*) AS attachment_count, \
             (SELECT COALESCE(SUM(consumed), 0)::bigint FROM upload_daily_usage \
                 WHERE day = (now() AT TIME ZONE 'UTC')::date) AS uploaded_today_bytes \
         FROM attachments WHERE message_id IS NOT NULL",
    )
    .fetch_one(&state.db)
    .await?;
    let now = Now {
        sockets_this_process: state.gateway.connections.socket_count().await,
        ..voice(&state).await?
    };

    let mut conn = state.db.acquire().await?;
    audit::record(
        &mut conn,
        &op,
        Action::UsageRead,
        Targets::default(),
        json!({}),
    )
    .await?;
    Ok(Json(Usage {
        generated_at: Utc::now(),
        registration: state.registration,
        accounts,
        servers,
        messages,
        storage,
        now,
    }))
}

/// People in voice and people live, from each server's voice index. Only
/// servers of this instance's database count, so an index another instance
/// on the same Redis wrote is not mixed in.
async fn voice(state: &AppState) -> Result<Now, ApiError> {
    let indexed = state.gateway.voice_servers().await?;
    let servers: Vec<Uuid> = if indexed.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar("SELECT id FROM servers WHERE id = ANY($1)")
            .bind(&indexed)
            .fetch_all(&state.db)
            .await?
    };
    let (mut in_voice, mut live) = (0, 0);
    for server in servers {
        let entries = state.gateway.voice_snapshot(server).await?;
        in_voice += entries.len();
        live += entries.iter().filter(|entry| entry.s).count();
    }
    Ok(Now {
        sockets_this_process: 0,
        in_voice,
        live,
    })
}
