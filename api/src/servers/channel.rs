//! Categories and channels of a server. Both require `manage_channels` to
//! change; reading is part of the server detail (`GET /api/servers/{id}`).

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{patch, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgPool};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::Id;
use crate::state::AppState;

use super::membership::{self, Membership};
use super::permissions::Permission;
use super::validate;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/servers/{id}/categories", post(create_category))
        .route(
            "/api/categories/{id}",
            patch(update_category).delete(delete_category),
        )
        .route("/api/servers/{id}/channels", post(create_channel))
        .route(
            "/api/channels/{id}",
            patch(update_channel).delete(delete_channel),
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
    /// 1:1 DM. Not creatable via `/api/servers/{id}/channels`.
    Dm,
}

impl ChannelKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Dm => "dm",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "voice" => Some(Self::Voice),
            "dm" => Some(Self::Dm),
            _ => None,
        }
    }

    /// Text and DM share the message REST + WS paths. Voice does not.
    pub fn is_messaging(self) -> bool {
        matches!(self, Self::Text | Self::Dm)
    }
}

impl From<String> for ChannelKind {
    /// Column → enum. The CHECK constraint guarantees one of the three names.
    fn from(value: String) -> Self {
        Self::from_name(&value).unwrap_or(Self::Text)
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Category {
    pub id: Uuid,
    pub server_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Channel {
    pub id: Uuid,
    pub server_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub name: String,
    #[sqlx(try_from = "String")]
    pub kind: ChannelKind,
    pub created_at: DateTime<Utc>,
}

pub async fn list_categories(db: &PgPool, server_id: Uuid) -> Result<Vec<Category>, ApiError> {
    Ok(sqlx::query_as::<_, Category>("SELECT id, server_id, name, created_at FROM categories WHERE server_id = $1 ORDER BY created_at, id")
    .bind(server_id)
    .fetch_all(db)
    .await?)
}

pub async fn list_channels(db: &PgPool, server_id: Uuid) -> Result<Vec<Channel>, ApiError> {
    Ok(sqlx::query_as::<_, Channel>("SELECT id, server_id, category_id, name, kind, created_at FROM channels WHERE server_id = $1 ORDER BY created_at, id")
    .bind(server_id)
    .fetch_all(db)
    .await?)
}

pub async fn insert_category<'e>(
    db: impl PgExecutor<'e>,
    server_id: Uuid,
    name: &str,
) -> Result<Category, ApiError> {
    Ok(sqlx::query_as::<_, Category>("INSERT INTO categories (server_id, name) VALUES ($1, $2) RETURNING id, server_id, name, created_at")
    .bind(server_id)
    .bind(name)
    .fetch_one(db)
    .await?)
}

pub async fn insert_channel<'e>(
    db: impl PgExecutor<'e>,
    server_id: Uuid,
    category_id: Option<Uuid>,
    name: &str,
    kind: ChannelKind,
) -> Result<Channel, ApiError> {
    Ok(sqlx::query_as::<_, Channel>(
        "INSERT INTO channels (server_id, category_id, name, kind) VALUES ($1, $2, $3, $4) \
         RETURNING id, server_id, category_id, name, kind, created_at",
    )
    .bind(server_id)
    .bind(category_id)
    .bind(name)
    .bind(kind.name())
    .fetch_one(db)
    .await?)
}

pub async fn get(db: &PgPool, channel_id: Uuid) -> Result<Option<Channel>, ApiError> {
    Ok(sqlx::query_as::<_, Channel>(
        "SELECT id, server_id, category_id, name, kind, created_at FROM channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_optional(db)
    .await?)
}

/// The caller is one of the (exactly two) DM participants.
pub async fn require_participant(
    db: &PgPool,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let is_member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM channel_members WHERE channel_id = $1 AND user_id = $2)",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    if is_member {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

/// A category id from the client is only usable if it belongs to the same
/// server; anything else is reported as an invalid field, not as a 404 that
/// would confirm the foreign id exists.
async fn check_category(
    db: &PgPool,
    server_id: Uuid,
    category_id: Option<Uuid>,
) -> Result<(), ApiError> {
    let Some(category_id) = category_id else {
        return Ok(());
    };
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1 AND server_id = $2)",
    )
    .bind(category_id)
    .bind(server_id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(ApiError::Validation(FieldErrors::from([(
            "category_id",
            "invalid",
        )])))
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateCategoryBody {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateCategoryBody {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelBody {
    #[serde(default)]
    pub name: String,
    pub kind: Option<String>,
    pub category_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateChannelBody {
    pub name: Option<String>,
    /// `""` moves the channel out of its category.
    pub category_id: Option<String>,
}

async fn create_category(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<CreateCategoryBody>,
) -> Result<Response, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ManageChannels)?;

    let mut errors = FieldErrors::new();
    let name = validate::name(&body.name, &mut errors);
    validate::finish(errors)?;

    let category = insert_category(&state.db, server_id, &name.expect("validated")).await?;
    info!(server_id = %server_id, category_id = %category.id, "category created");
    Ok((StatusCode::CREATED, Json(category)).into_response())
}

async fn update_category(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(category_id): Id,
    Body(body): Body<UpdateCategoryBody>,
) -> Result<Json<Category>, ApiError> {
    let (member, current) = category_for(&state.db, category_id, user.id).await?;
    member.require(Permission::ManageChannels)?;

    let mut errors = FieldErrors::new();
    let name = body.name.and_then(|raw| validate::name(&raw, &mut errors));
    validate::finish(errors)?;
    let Some(name) = name else {
        return Ok(Json(current));
    };

    let updated = sqlx::query_as::<_, Category>(
        "UPDATE categories SET name = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, server_id, name, created_at",
    )
    .bind(category_id)
    .bind(&name)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(updated))
}

async fn delete_category(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(category_id): Id,
) -> Result<StatusCode, ApiError> {
    let (member, _) = category_for(&state.db, category_id, user.id).await?;
    member.require(Permission::ManageChannels)?;

    // Channels stay (category_id → NULL via the foreign key).
    sqlx::query("DELETE FROM categories WHERE id = $1")
        .bind(category_id)
        .execute(&state.db)
        .await?;
    info!(server_id = %member.server.id, category_id = %category_id, "category deleted");
    Ok(StatusCode::NO_CONTENT)
}

async fn create_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<CreateChannelBody>,
) -> Result<Response, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ManageChannels)?;

    let mut errors = FieldErrors::new();
    let kind = validate::kind(body.kind.as_deref(), &mut errors);
    let name = kind.and_then(|kind| validate::channel_name(&body.name, kind, &mut errors));
    let category_id = validate::category_id(body.category_id.as_deref(), &mut errors).flatten();
    validate::finish(errors)?;
    let (kind, name) = (kind.expect("validated"), name.expect("validated"));
    check_category(&state.db, server_id, category_id).await?;

    let channel = insert_channel(&state.db, server_id, category_id, &name, kind).await?;
    info!(server_id = %server_id, channel_id = %channel.id, kind = kind.name(), "channel created");
    Ok((StatusCode::CREATED, Json(channel)).into_response())
}

async fn update_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
    Body(body): Body<UpdateChannelBody>,
) -> Result<Json<Channel>, ApiError> {
    let (member, current) = channel_for(&state.db, channel_id, user.id).await?;
    member.require(Permission::ManageChannels)?;

    let mut errors = FieldErrors::new();
    let name = body
        .name
        .and_then(|raw| validate::channel_name(&raw, current.kind, &mut errors));
    let category = validate::category_id(body.category_id.as_deref(), &mut errors);
    validate::finish(errors)?;
    if name.is_none() && category.is_none() {
        return Ok(Json(current));
    }
    if let Some(category_id) = category {
        let server_id = current.server_id.ok_or(ApiError::NotFound)?;
        check_category(&state.db, server_id, category_id).await?;
    }

    let updated = sqlx::query_as::<_, Channel>(
        "UPDATE channels \
         SET name = COALESCE($2, name), \
             category_id = CASE WHEN $3 THEN $4 ELSE category_id END, \
             updated_at = now() \
         WHERE id = $1 \
         RETURNING id, server_id, category_id, name, kind, created_at",
    )
    .bind(channel_id)
    .bind(name.as_deref())
    .bind(category.is_some())
    .bind(category.flatten())
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(updated))
}

async fn delete_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
) -> Result<StatusCode, ApiError> {
    let (member, _) = channel_for(&state.db, channel_id, user.id).await?;
    member.require(Permission::ManageChannels)?;

    sqlx::query("DELETE FROM channels WHERE id = $1")
        .bind(channel_id)
        .execute(&state.db)
        .await?;
    info!(server_id = %member.server.id, channel_id = %channel_id, "channel deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// Resolves a category to the caller's membership in its server. A category
/// in a server the caller is not part of is a plain 404.
async fn category_for(
    db: &PgPool,
    category_id: Uuid,
    user_id: Uuid,
) -> Result<(Membership, Category), ApiError> {
    let category = sqlx::query_as::<_, Category>(
        "SELECT id, server_id, name, created_at FROM categories WHERE id = $1",
    )
    .bind(category_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let member = membership::load(db, category.server_id, user_id).await?;
    Ok((member, category))
}

/// Resolves a channel to the caller's membership. A channel in a server the
/// caller is not part of is a plain 404 — same as a missing id.
pub async fn channel_for(
    db: &PgPool,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<(Membership, Channel), ApiError> {
    let channel = get(db, channel_id).await?.ok_or(ApiError::NotFound)?;
    let server_id = channel.server_id.ok_or(ApiError::NotFound)?;
    let member = membership::load(db, server_id, user_id).await?;
    Ok((member, channel))
}
