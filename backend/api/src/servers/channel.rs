//! Categories and channels of a server. Both require `manage_channels` to
//! change; reading is part of the server detail (`GET /api/servers/{id}`).
//!
//! A channel can be limited to roles (`role_ids`, v0.10). A channel the caller
//! cannot see is a `404` here as everywhere else; limiting one runs the
//! access-change flow of [`access`].

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{patch, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection, PgExecutor, PgPool};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::Recheck;
use crate::json::Body;
use crate::path::Id;
use crate::state::AppState;

use super::access;
use super::membership::{self, Membership};
use super::permissions::Permission;
use super::roles;
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

/// The select list that builds a [`Channel`] from `channels c`. Every query
/// that returns one uses it (`INSERT … AS c` / `UPDATE … AS c` included).
macro_rules! channel_columns {
    () => {
        "c.id, c.server_id, c.category_id, c.name, c.kind, c.created_at, c.limited, \
         ARRAY(SELECT cr.role_id FROM channel_roles cr WHERE cr.channel_id = c.id \
               ORDER BY cr.role_id) AS role_ids"
    };
}
pub(crate) use channel_columns;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Channel {
    pub id: Uuid,
    pub server_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub name: String,
    #[sqlx(try_from = "String")]
    pub kind: ChannelKind,
    pub created_at: DateTime<Utc>,
    /// Seen only by the owner and holders of one of `role_ids` (v0.10). A
    /// limited channel whose last role was deleted is the owner's alone.
    /// Always false for DMs.
    pub limited: bool,
    /// The roles a limited channel is for; empty when it is not limited.
    pub role_ids: Vec<Uuid>,
}

pub async fn list_categories(db: &PgPool, server_id: Uuid) -> Result<Vec<Category>, ApiError> {
    Ok(sqlx::query_as::<_, Category>("SELECT id, server_id, name, created_at FROM categories WHERE server_id = $1 ORDER BY created_at, id")
    .bind(server_id)
    .fetch_all(db)
    .await?)
}

pub async fn list_channels(db: &PgPool, server_id: Uuid) -> Result<Vec<Channel>, ApiError> {
    Ok(sqlx::query_as::<_, Channel>(concat!(
        "SELECT ",
        channel_columns!(),
        " FROM channels c WHERE c.server_id = $1 ORDER BY c.created_at, c.id"
    ))
    .bind(server_id)
    .fetch_all(db)
    .await?)
}

pub async fn insert_category<'e>(
    db: impl PgExecutor<'e>,
    server_id: Uuid,
    name: &str,
) -> Result<Category, ApiError> {
    // `clock_timestamp()`, not `now()`: rows created in one transaction keep
    // their order, which `ORDER BY created_at` relies on.
    Ok(sqlx::query_as::<_, Category>("INSERT INTO categories (server_id, name, created_at) VALUES ($1, $2, clock_timestamp()) RETURNING id, server_id, name, created_at")
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
    Ok(sqlx::query_as::<_, Channel>(concat!(
        // See `insert_category` for `clock_timestamp()`.
        "INSERT INTO channels AS c (server_id, category_id, name, kind, created_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp()) \
         RETURNING ",
        channel_columns!()
    ))
    .bind(server_id)
    .bind(category_id)
    .bind(name)
    .bind(kind.name())
    .fetch_one(db)
    .await?)
}

pub async fn get<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    channel_id: Uuid,
) -> Result<Option<Channel>, ApiError> {
    Ok(sqlx::query_as::<_, Channel>(concat!(
        "SELECT ",
        channel_columns!(),
        " FROM channels c WHERE c.id = $1"
    ))
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
    /// Absent: everyone sees it. `[]` is the same; a non-empty list limits
    /// the channel to those roles.
    pub role_ids: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateChannelBody {
    pub name: Option<String>,
    /// `""` moves the channel out of its category.
    pub category_id: Option<String>,
    /// Absent: access is unchanged. `[]` opens the channel to everyone; a
    /// non-empty list limits it to those roles (full replacement).
    pub role_ids: Option<Vec<String>>,
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
    let role_ids = body
        .role_ids
        .and_then(|raw| validate::role_ids(&raw, &mut errors));
    validate::finish(errors)?;
    let (kind, name) = (kind.expect("validated"), name.expect("validated"));
    check_category(&state.db, server_id, category_id).await?;

    let Some(role_ids) = role_ids else {
        let channel = insert_channel(&state.db, server_id, category_id, &name, kind).await?;
        info!(server_id = %server_id, channel_id = %channel.id, kind = kind.name(), "channel created");
        return Ok((StatusCode::CREATED, Json(channel)).into_response());
    };

    // With `role_ids`: the access-change flow, so the channel is never
    // visible to everyone for a moment.
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user.id).await?;
    member.require(Permission::ManageChannels)?;
    roles::check_roles(&mut tx, server_id, &role_ids).await?;
    let before = access::snapshot(&mut tx, server_id).await?;
    let created = insert_channel(&mut *tx, server_id, category_id, &name, kind).await?;
    set_roles(&mut tx, server_id, created.id, &role_ids).await?;
    let channel = get(&mut *tx, created.id).await?.ok_or(ApiError::NotFound)?;
    let after = access::snapshot(&mut tx, server_id).await?;
    access::after_change(&state, server_id, &before, &after).await?;
    tx.commit().await?;
    info!(server_id = %server_id, channel_id = %channel.id, kind = kind.name(), limited = channel.limited, "channel created");
    access::announce(&state, server_id).await;
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
    let role_ids = body
        .role_ids
        .and_then(|raw| validate::role_ids(&raw, &mut errors));
    validate::finish(errors)?;
    if name.is_none() && category.is_none() && role_ids.is_none() {
        return Ok(Json(current));
    }
    let server_id = current.server_id.ok_or(ApiError::NotFound)?;
    if let Some(category_id) = category {
        check_category(&state.db, server_id, category_id).await?;
    }

    // A change of `role_ids` runs the access-change flow inside the
    // exclusive server lock; name and category alone need no lock.
    let mut tx = match role_ids {
        Some(_) => membership::lock_server(&state.db, server_id, true).await?,
        None => state.db.begin().await?,
    };
    let before = match &role_ids {
        Some(role_ids) => {
            // Again inside the lock: the caller's access may have changed.
            let (member, _) = channel_for_conn(&mut tx, channel_id, user.id).await?;
            member.require(Permission::ManageChannels)?;
            roles::check_roles(&mut tx, server_id, role_ids).await?;
            let before = access::snapshot(&mut tx, server_id).await?;
            set_roles(&mut tx, server_id, channel_id, role_ids).await?;
            Some(before)
        }
        None => None,
    };

    let updated = sqlx::query_as::<_, Channel>(concat!(
        "UPDATE channels AS c \
         SET name = COALESCE($2, c.name), \
             category_id = CASE WHEN $3 THEN $4 ELSE c.category_id END, \
             updated_at = now() \
         WHERE c.id = $1 \
         RETURNING ",
        channel_columns!()
    ))
    .bind(channel_id)
    .bind(name.as_deref())
    .bind(category.is_some())
    .bind(category.flatten())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    if let Some(before) = &before {
        let after = access::snapshot(&mut tx, server_id).await?;
        access::after_change(&state, server_id, before, &after).await?;
    }
    tx.commit().await?;
    if before.is_some() {
        info!(server_id = %server_id, channel_id = %channel_id, limited = updated.limited, roles = updated.role_ids.len(), "channel access changed");
        access::announce(&state, server_id).await;
    }
    Ok(Json(updated))
}

/// Replaces the roles a channel is limited to: `[]` opens it to everyone, a
/// non-empty list limits it.
async fn set_roles(
    db: &mut PgConnection,
    server_id: Uuid,
    channel_id: Uuid,
    role_ids: &[Uuid],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM channel_roles WHERE channel_id = $1")
        .bind(channel_id)
        .execute(&mut *db)
        .await?;
    sqlx::query(
        "INSERT INTO channel_roles (server_id, channel_id, role_id) \
         SELECT $1, $2, unnest($3::uuid[])",
    )
    .bind(server_id)
    .bind(channel_id)
    .bind(role_ids)
    .execute(&mut *db)
    .await?;
    sqlx::query("UPDATE channels SET limited = $2 WHERE id = $1")
        .bind(channel_id)
        .bind(!role_ids.is_empty())
        .execute(&mut *db)
        .await?;
    Ok(())
}

async fn delete_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(channel_id): Id,
) -> Result<StatusCode, ApiError> {
    let (_, channel) = channel_for(&state.db, channel_id, user.id).await?;
    let server_id = channel.server_id.ok_or(ApiError::NotFound)?;
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let (member, _) = channel_for_conn(&mut tx, channel_id, user.id).await?;
    member.require(Permission::ManageChannels)?;
    state.gateway.revoke_channel(server_id, channel_id).await?;

    sqlx::query("DELETE FROM channels WHERE id = $1")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    // After the row: writers that held it have committed their outbox rows.
    // Delivered later, a row would bring back the realtime keys dropped below.
    sqlx::query("DELETE FROM gateway_outbox WHERE channel_id = $1")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    state.gateway.drop_channel_log(channel_id).await;
    state
        .gateway
        .announce_recheck(Recheck::Channel(channel_id))
        .await;
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
/// caller is not part of, or one they cannot see, is a plain 404 — same as a
/// missing id.
pub async fn channel_for(
    db: &PgPool,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<(Membership, Channel), ApiError> {
    let mut conn = db.acquire().await?;
    channel_for_conn(&mut conn, channel_id, user_id).await
}

pub async fn channel_for_conn(
    db: &mut sqlx::PgConnection,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<(Membership, Channel), ApiError> {
    let channel = get(&mut *db, channel_id).await?.ok_or(ApiError::NotFound)?;
    let server_id = channel.server_id.ok_or(ApiError::NotFound)?;
    let member = membership::load(db, server_id, user_id).await?;
    if !member.sees(&channel) {
        return Err(ApiError::NotFound);
    }
    Ok((member, channel))
}
