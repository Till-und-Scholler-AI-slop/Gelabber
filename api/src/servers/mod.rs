//! `/api/servers/*`: servers, their members, categories, channels and invite
//! links (issue #4). Every route needs a session; every read goes through
//! the caller's membership, so a foreign server id is a `404`.
//!
//! Permission model, on purpose coarse: the owner may do everything, members
//! share one flag mask per server (`member_permissions`). No per-channel
//! overwrites, no further roles in v1.

pub mod channel;
pub mod invite;
pub mod membership;
pub mod permissions;
pub mod validate;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::Id;
use crate::state::AppState;

use self::channel::{Category, Channel, ChannelKind};
use self::membership::{Membership, Role};
use self::permissions::{Permission, Permissions};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/servers", get(list_servers).post(create_server))
        .route(
            "/api/servers/{id}",
            get(get_server).patch(update_server).delete(delete_server),
        )
        .route("/api/servers/{id}/leave", post(leave_server))
        .merge(channel::router())
        .merge(invite::router())
}

/// The `servers` row as stored.
#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct ServerRow {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    #[sqlx(try_from = "i32")]
    pub member_permissions: Permissions,
    pub created_at: DateTime<Utc>,
}

/// A server as one particular member sees it: the row plus *their* role and
/// effective flags. This is what the sidebar lists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServerView {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub role: Role,
    /// The caller's effective flags.
    pub permissions: Permissions,
    /// The mask every non-owner member holds (editable by `manage_server`).
    pub member_permissions: Permissions,
}

impl From<Membership> for ServerView {
    fn from(m: Membership) -> Self {
        let role = m.role();
        let permissions = m.permissions();
        Self {
            id: m.server.id,
            name: m.server.name,
            owner_id: m.server.owner_id,
            created_at: m.server.created_at,
            role,
            permissions,
            member_permissions: m.server.member_permissions,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Member {
    #[serde(flatten)]
    pub user: MemberRow,
    pub role: Role,
}

/// `GET /api/servers/{id}`: everything the client needs after a server
/// switch, in one round trip.
#[derive(Debug, Clone, Serialize)]
pub struct ServerDetail {
    #[serde(flatten)]
    pub server: ServerView,
    pub categories: Vec<Category>,
    pub channels: Vec<Channel>,
    pub members: Vec<Member>,
}

#[derive(Debug, Deserialize)]
pub struct CreateServerBody {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateServerBody {
    pub name: Option<String>,
    /// Full replacement of the member flag set, as names.
    pub member_permissions: Option<Vec<String>>,
}

async fn list_servers(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<ServerView>>, ApiError> {
    let rows = sqlx::query_as::<_, ServerRow>(
        "SELECT s.id, s.name, s.owner_id, s.member_permissions, s.created_at \
         FROM server_members m JOIN servers s ON s.id = m.server_id \
         WHERE m.user_id = $1 \
         ORDER BY m.joined_at, s.id",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let views = rows
        .into_iter()
        .map(|server| {
            ServerView::from(Membership {
                server,
                user_id: user.id,
            })
        })
        .collect();
    Ok(Json(views))
}

/// Creates the server with its owner membership and a first text channel
/// (`#allgemein` under "Textkanäle"), so it is usable the moment it exists.
/// Answers with the full detail so the client can render it without a
/// second request.
async fn create_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<CreateServerBody>,
) -> Result<Response, ApiError> {
    let mut errors = FieldErrors::new();
    let name = validate::name(&body.name, &mut errors);
    validate::finish(errors)?;
    let name = name.expect("validated");

    let mut tx = state.db.begin().await?;
    let server = sqlx::query_as::<_, ServerRow>(
        "INSERT INTO servers (name, owner_id, member_permissions) VALUES ($1, $2, $3) \
         RETURNING id, name, owner_id, member_permissions, created_at",
    )
    .bind(&name)
    .bind(user.id)
    .bind(Permissions::DEFAULT_MEMBER.bits())
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO server_members (server_id, user_id) VALUES ($1, $2)")
        .bind(server.id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let category = channel::insert_category(&mut *tx, server.id, "Textkanäle").await?;
    channel::insert_channel(
        &mut *tx,
        server.id,
        Some(category.id),
        "allgemein",
        ChannelKind::Text,
    )
    .await?;
    tx.commit().await?;
    info!(server_id = %server.id, owner_id = %user.id, "server created");

    let detail = load_detail(
        &state.db,
        Membership {
            server,
            user_id: user.id,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(detail)).into_response())
}

async fn get_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<ServerDetail>, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    Ok(Json(load_detail(&state.db, member).await?))
}

async fn load_detail(db: &PgPool, member: Membership) -> Result<ServerDetail, ApiError> {
    let server_id = member.server.id;
    let owner_id = member.server.owner_id;
    let (categories, channels, members) = tokio::try_join!(
        channel::list_categories(db, server_id),
        channel::list_channels(db, server_id),
        list_members(db, server_id),
    )?;
    let members = members
        .into_iter()
        .map(|user| Member {
            role: if user.user_id == owner_id {
                Role::Owner
            } else {
                Role::Member
            },
            user,
        })
        .collect();
    Ok(ServerDetail {
        server: member.into(),
        categories,
        channels,
        members,
    })
}

async fn list_members(db: &PgPool, server_id: Uuid) -> Result<Vec<MemberRow>, ApiError> {
    Ok(sqlx::query_as::<_, MemberRow>(
        "SELECT m.user_id, u.name, u.avatar_url, m.joined_at \
         FROM server_members m JOIN users u ON u.id = m.user_id \
         WHERE m.server_id = $1 \
         ORDER BY m.joined_at, m.user_id",
    )
    .bind(server_id)
    .fetch_all(db)
    .await?)
}

async fn update_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<UpdateServerBody>,
) -> Result<Json<ServerView>, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ManageServer)?;

    let mut errors = FieldErrors::new();
    let name = body.name.and_then(|raw| validate::name(&raw, &mut errors));
    let member_permissions = body
        .member_permissions
        .and_then(|names| validate::permissions(&names, &mut errors));
    validate::finish(errors)?;
    if name.is_none() && member_permissions.is_none() {
        return Ok(Json(member.into()));
    }

    let server = sqlx::query_as::<_, ServerRow>(
        "UPDATE servers \
         SET name = COALESCE($2, name), \
             member_permissions = COALESCE($3, member_permissions), \
             updated_at = now() \
         WHERE id = $1 \
         RETURNING id, name, owner_id, member_permissions, created_at",
    )
    .bind(server_id)
    .bind(name.as_deref())
    .bind(member_permissions.map(Permissions::bits))
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if let Some(flags) = member_permissions {
        info!(server_id = %server_id, member_permissions = %flags, "member permissions changed");
    }
    Ok(Json(
        Membership {
            server,
            user_id: user.id,
        }
        .into(),
    ))
}

/// Owner only. Members, categories, channels and invites go with it.
async fn delete_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<StatusCode, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require_owner()?;

    sqlx::query("DELETE FROM servers WHERE id = $1")
        .bind(server_id)
        .execute(&state.db)
        .await?;
    info!(server_id = %server_id, "server deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// Members can leave; the owner cannot (there is no ownership transfer in
/// v1 — delete the server instead).
async fn leave_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<StatusCode, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    if member.role() == Role::Owner {
        return Err(ApiError::Forbidden(
            "The owner cannot leave; delete the server instead.",
        ));
    }

    sqlx::query("DELETE FROM server_members WHERE server_id = $1 AND user_id = $2")
        .bind(server_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    info!(server_id = %server_id, user_id = %user.id, "member left");
    Ok(StatusCode::NO_CONTENT)
}
