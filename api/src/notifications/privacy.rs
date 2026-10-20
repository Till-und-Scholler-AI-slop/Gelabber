//! `GET` and `PUT /api/notification-privacy` (issue #268): whether the
//! account's notifications carry the message. With `hide_content` a push says
//! only who wrote (`push::hidden_title`), and the pages show the same text.
//! A change goes to the account's other sockets (`ns`), which fetch again.

use axum::{Router, extract::State, response::Json, routing::get};
use serde::{Deserialize, Serialize};

use crate::{
    auth::session::CurrentUser, error::ApiError, gateway::protocol::UserFrame, json::Body,
    state::AppState,
};

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/api/notification-privacy", get(read).put(write))
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Privacy {
    pub hide_content: bool,
}

async fn read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Privacy>, ApiError> {
    sqlx::query_as("SELECT notifications_hide_content AS hide_content FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
        .map(Json)
        .ok_or(ApiError::Unauthenticated)
}

async fn write(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<Privacy>,
) -> Result<Json<Privacy>, ApiError> {
    let saved: Privacy = sqlx::query_as(
        "UPDATE users SET notifications_hide_content=$2 WHERE id=$1
         RETURNING notifications_hide_content AS hide_content",
    )
    .bind(user.id)
    .bind(body.hide_content)
    .fetch_optional(&state.db)
    .await?
    // The account went between the extractor and the update.
    .ok_or(ApiError::Unauthenticated)?;
    if let Err(error) = state
        .gateway
        .publish_user(user.id, &UserFrame::NotificationSettings)
        .await
    {
        tracing::debug!(error = error.code(), "notification privacy not announced");
    }
    Ok(Json(saved))
}
