//! Custom server roles (v0.10, #516). "Alle Mitglieder" is the base role,
//! `servers.member_permissions`, and no row here; the owner is fixed and holds
//! everything. Every route needs `manage_server` in the role's server, and a
//! server or role the caller is not part of is a `404`. There is no role
//! hierarchy: a `manage_server` holder edits any role and assigns any role,
//! themselves included.
//!
//! Writes run inside the exclusive server lock. Those that can change access
//! (a role's flags, deleting a role, a member's roles) go through
//! [`access`]; the rest publish the `{"k": "a"}` event alone.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{patch, post, put};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::{Id, Ids};
use crate::state::AppState;

use super::access;
use super::membership::{self, Role as MemberRole};
use super::permissions::{Permission, Permissions};
use super::validate;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/servers/{id}/roles", post(create_role))
        .route("/api/roles/{id}", patch(update_role).delete(delete_role))
        .route(
            "/api/servers/{id}/members/{user_id}/roles",
            put(set_member_roles),
        )
}

/// The select list that builds a [`Role`] from `server_roles`.
macro_rules! role_columns {
    () => {
        "id, server_id, name, color, permissions, position, created_at"
    };
}

/// A custom role as stored and sent.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Role {
    pub id: Uuid,
    pub server_id: Uuid,
    pub name: String,
    /// One of [`validate::ROLE_COLORS`].
    pub color: String,
    #[sqlx(try_from = "i32")]
    pub permissions: Permissions,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoleBody {
    #[serde(default)]
    pub name: String,
    pub color: Option<String>,
    pub permissions: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateRoleBody {
    pub name: Option<String>,
    pub color: Option<String>,
    /// Full replacement of the role's flag set, as names.
    pub permissions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct MemberRolesBody {
    pub role_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct MemberRoles {
    pub user_id: Uuid,
    pub role_ids: Vec<Uuid>,
}

/// The server's custom roles, by position.
pub async fn list<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    server_id: Uuid,
) -> Result<Vec<Role>, ApiError> {
    Ok(sqlx::query_as::<_, Role>(concat!(
        "SELECT ",
        role_columns!(),
        " FROM server_roles WHERE server_id = $1 ORDER BY position, id"
    ))
    .bind(server_id)
    .fetch_all(db)
    .await?)
}

/// `taken` when another role of the server already has the name, ignoring
/// case. `except` is the role being renamed.
async fn check_name_free(
    db: &mut PgConnection,
    server_id: Uuid,
    name: &str,
    except: Option<Uuid>,
    errors: &mut FieldErrors,
) -> Result<(), ApiError> {
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM server_roles \
         WHERE server_id = $1 AND lower(name) = lower($2) AND id IS DISTINCT FROM $3)",
    )
    .bind(server_id)
    .bind(name)
    .bind(except)
    .fetch_one(db)
    .await?;
    if taken {
        errors.insert("name", "taken");
    }
    Ok(())
}

async fn create_role(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<CreateRoleBody>,
) -> Result<Response, ApiError> {
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user.id).await?;
    member.require(Permission::ManageServer)?;

    let mut errors = FieldErrors::new();
    let name = validate::role_name(&body.name, &mut errors);
    let color = validate::role_color(body.color.as_deref(), &mut errors);
    let permissions = match &body.permissions {
        None => Some(Permissions::NONE),
        Some(names) => validate::permissions(names, "permissions", &mut errors),
    };
    if let Some(name) = &name {
        check_name_free(&mut tx, server_id, name, None, &mut errors).await?;
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM server_roles WHERE server_id = $1")
        .bind(server_id)
        .fetch_one(&mut *tx)
        .await?;
    if count >= validate::ROLES_MAX as i64 {
        errors.insert("roles", "too_many");
    }
    validate::finish(errors)?;
    let (name, color, permissions) = (
        name.expect("validated"),
        color.expect("validated"),
        permissions.expect("validated"),
    );

    let role = sqlx::query_as::<_, Role>(concat!(
        "INSERT INTO server_roles (server_id, name, color, permissions, position) \
         VALUES ($1, $2, $3, $4, \
                 (SELECT COALESCE(max(position) + 1, 0) FROM server_roles WHERE server_id = $1)) \
         RETURNING ",
        role_columns!()
    ))
    .bind(server_id)
    .bind(&name)
    .bind(color)
    .bind(permissions.bits())
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    info!(server_id = %server_id, role_id = %role.id, "role created");
    access::publish_changed(&state, server_id).await;
    Ok((StatusCode::CREATED, Json(role)).into_response())
}

/// The role's server id, read before the lock. A missing role is a `404`.
async fn role_server(state: &AppState, role_id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar("SELECT server_id FROM server_roles WHERE id = $1")
        .bind(role_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

/// The role again, inside the lock: it may have gone in the meantime.
async fn locked_role(db: &mut PgConnection, role_id: Uuid) -> Result<Role, ApiError> {
    sqlx::query_as::<_, Role>(concat!(
        "SELECT ",
        role_columns!(),
        " FROM server_roles WHERE id = $1"
    ))
    .bind(role_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

async fn update_role(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(role_id): Id,
    Body(body): Body<UpdateRoleBody>,
) -> Result<Json<Role>, ApiError> {
    let server_id = role_server(&state, role_id).await?;
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user.id).await?;
    let current = locked_role(&mut tx, role_id).await?;
    member.require(Permission::ManageServer)?;

    let mut errors = FieldErrors::new();
    let name = body
        .name
        .and_then(|raw| validate::role_name(&raw, &mut errors));
    let color = body
        .color
        .and_then(|raw| validate::role_color(Some(&raw), &mut errors));
    let permissions = body
        .permissions
        .and_then(|names| validate::permissions(&names, "permissions", &mut errors));
    if let Some(name) = &name {
        check_name_free(&mut tx, server_id, name, Some(role_id), &mut errors).await?;
    }
    validate::finish(errors)?;
    if name.is_none() && color.is_none() && permissions.is_none() {
        return Ok(Json(current));
    }

    let flags_changed = permissions.is_some_and(|flags| flags != current.permissions);
    let before = if flags_changed {
        Some(access::snapshot(&mut tx, server_id).await?)
    } else {
        None
    };
    let role = sqlx::query_as::<_, Role>(concat!(
        "UPDATE server_roles \
         SET name = COALESCE($2, name), \
             color = COALESCE($3, color), \
             permissions = COALESCE($4, permissions), \
             updated_at = now() \
         WHERE id = $1 \
         RETURNING ",
        role_columns!()
    ))
    .bind(role_id)
    .bind(name.as_deref())
    .bind(color)
    .bind(permissions.map(Permissions::bits))
    .fetch_one(&mut *tx)
    .await?;
    if let Some(before) = &before {
        let after = access::snapshot(&mut tx, server_id).await?;
        access::after_change(&state, server_id, before, &after).await?;
    }
    tx.commit().await?;
    if flags_changed {
        info!(server_id = %server_id, role_id = %role_id, permissions = %role.permissions, "role permissions changed");
        access::announce(&state, server_id).await;
    } else {
        access::publish_changed(&state, server_id).await;
    }
    Ok(Json(role))
}

/// Members lose the role's flags; its assignments and channel rows go by
/// cascade. A channel limited to it stays limited (then maybe the owner's
/// alone).
async fn delete_role(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(role_id): Id,
) -> Result<StatusCode, ApiError> {
    let server_id = role_server(&state, role_id).await?;
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user.id).await?;
    locked_role(&mut tx, role_id).await?;
    member.require(Permission::ManageServer)?;

    let before = access::snapshot(&mut tx, server_id).await?;
    sqlx::query("DELETE FROM server_roles WHERE id = $1")
        .bind(role_id)
        .execute(&mut *tx)
        .await?;
    let after = access::snapshot(&mut tx, server_id).await?;
    access::after_change(&state, server_id, &before, &after).await?;
    tx.commit().await?;
    info!(server_id = %server_id, role_id = %role_id, "role deleted");
    access::announce(&state, server_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Full replacement of a member's custom roles. The owner holds every right
/// already and gets none.
async fn set_member_roles(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Ids(server_id, target_id): Ids,
    Body(body): Body<MemberRolesBody>,
) -> Result<Json<MemberRoles>, ApiError> {
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let actor = membership::load(&mut *tx, server_id, user.id).await?;
    actor.require(Permission::ManageServer)?;
    let target = membership::load(&mut *tx, server_id, target_id).await?;
    if target.role() == MemberRole::Owner {
        return Err(ApiError::Forbidden(
            "The owner always has every permission.",
        ));
    }

    let mut errors = FieldErrors::new();
    let role_ids = match &body.role_ids {
        Some(raw) => validate::role_ids(raw, &mut errors),
        None => {
            errors.insert("role_ids", "invalid");
            None
        }
    };
    validate::finish(errors)?;
    let role_ids = role_ids.expect("validated");
    check_roles(&mut tx, server_id, &role_ids).await?;

    let before = access::snapshot(&mut tx, server_id).await?;
    sqlx::query("DELETE FROM server_member_roles WHERE server_id = $1 AND user_id = $2")
        .bind(server_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO server_member_roles (server_id, user_id, role_id) \
         SELECT $1, $2, unnest($3::uuid[])",
    )
    .bind(server_id)
    .bind(target_id)
    .bind(&role_ids)
    .execute(&mut *tx)
    .await?;
    let after = access::snapshot(&mut tx, server_id).await?;
    access::after_change(&state, server_id, &before, &after).await?;
    let role_ids = membership::load(&mut *tx, server_id, target_id)
        .await?
        .role_ids;
    tx.commit().await?;
    info!(server_id = %server_id, user_id = %target_id, roles = role_ids.len(), "member roles set");
    access::announce(&state, server_id).await;
    Ok(Json(MemberRoles {
        user_id: target_id,
        role_ids,
    }))
}

/// `422 role_ids invalid` unless every id is a role of the server. A foreign
/// role is reported the same as an unknown one.
pub async fn check_roles(
    db: &mut PgConnection,
    server_id: Uuid,
    role_ids: &[Uuid],
) -> Result<(), ApiError> {
    if role_ids.is_empty() {
        return Ok(());
    }
    let found: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM server_roles WHERE server_id = $1 AND id = ANY($2)",
    )
    .bind(server_id)
    .bind(role_ids)
    .fetch_one(db)
    .await?;
    if found == role_ids.len() as i64 {
        Ok(())
    } else {
        Err(ApiError::Validation(FieldErrors::from([(
            "role_ids", "invalid",
        )])))
    }
}
