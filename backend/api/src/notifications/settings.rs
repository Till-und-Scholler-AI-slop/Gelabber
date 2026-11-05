//! `GET /api/notification-settings`, `PUT /api/servers/{id}/notification-settings`
//! and `PUT /api/channels/{id}/notification-settings`.
//!
//! A setting is stored as given: `level` NULL inherits (a channel its
//! server's, a server or DM `all`). A mute is `muted` with an optional end
//! the server computes from `mute_minutes`, so a client clock cannot shorten
//! or stretch it. A row that inherits and is not muted is deleted. Every
//! change goes to the account's other sockets (`ns`), which fetch again.

use axum::{
    Router,
    extract::State,
    response::Json,
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    auth::session::CurrentUser,
    error::{ApiError, FieldErrors},
    gateway::protocol::UserFrame,
    json::Body,
    messages::messaging_channel,
    path::Id,
    servers::membership,
    state::AppState,
};

/// A mute lasts at most a year when it has an end at all.
pub const MAX_MUTE_MINUTES: i64 = 366 * 24 * 60;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/notification-settings", get(list))
        .route("/api/servers/{id}/notification-settings", put(put_server))
        .route("/api/channels/{id}/notification-settings", put(put_channel))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum Level {
    All,
    Mentions,
    None,
}

#[derive(Debug, Serialize, FromRow, PartialEq)]
pub struct Setting {
    /// `server` or `channel`.
    pub scope: String,
    pub id: Uuid,
    pub level: Option<Level>,
    pub muted: bool,
    pub muted_until: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct SettingBody {
    #[serde(default)]
    level: Option<Level>,
    #[serde(default)]
    muted: bool,
    /// With `muted`: minutes from now; absent mutes until turned back on.
    #[serde(default)]
    mute_minutes: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<Setting>>, ApiError> {
    Ok(Json(
        sqlx::query_as(
            "SELECT scope_kind AS scope, scope_id AS id, level, muted, muted_until \
             FROM notification_settings WHERE user_id=$1 ORDER BY scope_kind, scope_id",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await?,
    ))
}

async fn put_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<SettingBody>,
) -> Result<Json<Setting>, ApiError> {
    membership::load(&state.db, server_id, user.id).await?;
    store(&state, user.id, "server", server_id, body).await
}

async fn put_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    Body(body): Body<SettingBody>,
) -> Result<Json<Setting>, ApiError> {
    // Text channels of the user's servers and their DMs; voice is no message
    // resource and answers 404 like everywhere else.
    messaging_channel(&state.db, channel_id, user.id).await?;
    store(&state, user.id, "channel", channel_id, body).await
}

async fn store(
    state: &AppState,
    user: Uuid,
    scope: &'static str,
    id: Uuid,
    body: SettingBody,
) -> Result<Json<Setting>, ApiError> {
    if let Some(minutes) = body.mute_minutes
        && (!body.muted || !(1..=MAX_MUTE_MINUTES).contains(&minutes))
    {
        return Err(ApiError::Validation(FieldErrors::from([(
            "mute_minutes",
            "invalid",
        )])));
    }
    let setting = if body.level.is_none() && !body.muted {
        sqlx::query(
            "DELETE FROM notification_settings WHERE user_id=$1 AND scope_kind=$2 AND scope_id=$3",
        )
        .bind(user)
        .bind(scope)
        .bind(id)
        .execute(&state.db)
        .await?;
        Setting {
            scope: scope.to_owned(),
            id,
            level: None,
            muted: false,
            muted_until: None,
        }
    } else {
        sqlx::query_as(
            "INSERT INTO notification_settings (user_id,scope_kind,scope_id,level,muted,muted_until)
             VALUES ($1,$2,$3,$4,$5, CASE WHEN $5 AND $6::bigint IS NOT NULL
                 THEN now() + make_interval(mins => $6::int) END)
             ON CONFLICT (user_id,scope_kind,scope_id) DO UPDATE
             SET level=EXCLUDED.level, muted=EXCLUDED.muted, muted_until=EXCLUDED.muted_until,
                 updated_at=now()
             RETURNING scope_kind AS scope, scope_id AS id, level, muted, muted_until",
        )
        .bind(user)
        .bind(scope)
        .bind(id)
        .bind(body.level)
        .bind(body.muted)
        .bind(body.mute_minutes)
        .fetch_one(&state.db)
        .await?
    };
    if let Err(error) = state
        .gateway
        .publish_user(user, &UserFrame::NotificationSettings)
        .await
    {
        tracing::debug!(error = error.code(), "notification settings not announced");
    }
    Ok(Json(setting))
}
