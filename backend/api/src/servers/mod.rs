//! `/api/servers/*`: servers, their members, categories, channels and invite
//! links (issue #4). Every route needs a session; every read goes through
//! the caller's membership, so a foreign server id is a `404`.
//!
//! Permission model, on purpose coarse: the owner may do everything; every
//! other member holds the base role "Alle Mitglieder" (`member_permissions`)
//! plus their custom roles (`roles`, v0.10), and their flags are the union.
//! Kick/ban need `moderate_members`; deleting someone else's message needs
//! `manage_messages`. No per-channel overwrites: a channel is only limited to
//! roles or not.

pub mod access;
pub mod channel;
pub mod icon;
pub mod invite;
pub mod membership;
pub mod moderation;
pub mod permissions;
pub mod roles;
pub mod validate;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::{EventKind, Recheck, publish_server};
use crate::i18n;
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
        .route("/api/servers/{id}/transfer", post(transfer_server))
        .merge(channel::router())
        .merge(icon::router())
        .merge(invite::router())
        .merge(moderation::router())
        .merge(roles::router())
}

/// The `servers` row as stored.
#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct ServerRow {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    #[sqlx(try_from = "i32")]
    pub member_permissions: Permissions,
    /// Uploaded icon (`/api/server-icons/{server}/{file}`), see `icon`.
    pub icon_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A server as one particular member sees it: the row plus *their* role and
/// effective flags. This is what the sidebar lists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServerView {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    pub icon_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub role: Role,
    /// The caller's effective flags.
    pub permissions: Permissions,
    /// The mask every non-owner member holds (editable by `manage_server`).
    pub member_permissions: Permissions,
    /// An operator blocked the server: it stays in the list so the client
    /// can explain, and everything but leaving and deleting answers
    /// `403 server_blocked`.
    pub blocked: bool,
}

impl From<Membership> for ServerView {
    fn from(m: Membership) -> Self {
        let role = m.role();
        let permissions = m.permissions();
        let blocked = m.blocked;
        Self {
            id: m.server.id,
            name: m.server.name,
            owner_id: m.server.owner_id,
            icon_url: m.server.icon_url,
            created_at: m.server.created_at,
            role,
            permissions,
            member_permissions: m.server.member_permissions,
            blocked,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub joined_at: DateTime<Utc>,
    /// The parts of the profile card the member list shows; the rest comes
    /// from `GET /api/users/{id}` when a card opens.
    pub status_text: Option<String>,
    pub accent_color: Option<String>,
    pub avatar_decoration: Option<String>,
    /// The member's custom roles, by position.
    pub role_ids: Vec<Uuid>,
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
    /// The server's custom roles, by position. Every member sees them.
    pub roles: Vec<roles::Role>,
}

#[derive(Debug, Deserialize)]
pub struct CreateServerBody {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct TransferBody {
    #[serde(default)]
    pub user_id: Option<String>,
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
    let rows = sqlx::query_as::<_, Membership>(concat!(
        "SELECT ",
        membership::membership_columns!(),
        " FROM server_members m JOIN servers s ON s.id = m.server_id \
         WHERE m.user_id = $1 \
         ORDER BY m.joined_at, s.id"
    ))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let views = rows.into_iter().map(ServerView::from).collect();
    Ok(Json(views))
}

/// Creates the server with its owner membership, a first text channel
/// (`#allgemein` under "Textkanäle") and a voice room ("Wohnzimmer" under
/// "Sprachkanäle"), named in the creator's language
/// ([`i18n::default_channels`]), so it is usable the moment it exists.
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
    // Waits for an account deletion that holds the row and then refuses: a
    // deleted account must not end up owning a server (`account::delete_me`).
    if !account_alive(&mut tx, user.id).await? {
        return Err(ApiError::Unauthenticated);
    }
    let server = sqlx::query_as::<_, ServerRow>(
        "INSERT INTO servers (name, owner_id, member_permissions) VALUES ($1, $2, $3) \
         RETURNING id, name, owner_id, member_permissions, icon_url, created_at",
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
    let names = i18n::default_channels(user.locale);
    let category = channel::insert_category(&mut *tx, server.id, names.text_category).await?;
    channel::insert_channel(
        &mut *tx,
        server.id,
        Some(category.id),
        names.text_channel,
        ChannelKind::Text,
    )
    .await?;
    let voice = channel::insert_category(&mut *tx, server.id, names.voice_category).await?;
    channel::insert_channel(
        &mut *tx,
        server.id,
        Some(voice.id),
        names.voice_channel,
        ChannelKind::Voice,
    )
    .await?;
    tx.commit().await?;
    info!(server_id = %server.id, owner_id = %user.id, "server created");

    let detail = load_detail(&state.db, Membership::base(server, user.id)).await?;
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
    let (categories, channels, members, roles) = tokio::try_join!(
        channel::list_categories(db, server_id),
        channel::list_channels(db, server_id),
        list_members(db, server_id),
        roles::list(db, server_id),
    )?;
    let (categories, channels) = visible_to(&member, categories, channels);
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
        roles,
    })
}

/// What of the server's layout the caller sees: the channels [`Membership::sees`]
/// allows, and the categories minus those that hold channels, all of them
/// hidden. Empty categories stay, so `manage_channels` holders can fill them.
fn visible_to(
    member: &Membership,
    categories: Vec<Category>,
    channels: Vec<Channel>,
) -> (Vec<Category>, Vec<Channel>) {
    let (channels, hidden): (Vec<Channel>, Vec<Channel>) = channels
        .into_iter()
        .partition(|channel| member.sees(channel));
    let categories = categories
        .into_iter()
        .filter(|category| {
            let holds = |list: &[Channel]| {
                list.iter()
                    .any(|channel| channel.category_id == Some(category.id))
            };
            holds(&channels) || !holds(&hidden)
        })
        .collect();
    (categories, channels)
}

async fn list_members(db: &PgPool, server_id: Uuid) -> Result<Vec<MemberRow>, ApiError> {
    Ok(sqlx::query_as::<_, MemberRow>(
        "SELECT m.user_id, u.name, u.avatar_url, m.joined_at, u.status_text, u.accent_color, \
             u.avatar_decoration, \
             ARRAY(SELECT mr.role_id FROM server_member_roles mr \
                     JOIN server_roles r ON r.id = mr.role_id \
                    WHERE mr.server_id = m.server_id AND mr.user_id = m.user_id \
                    ORDER BY r.position, r.id) AS role_ids \
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
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user.id).await?;
    member.require(Permission::ManageServer)?;

    let mut errors = FieldErrors::new();
    let name = body.name.and_then(|raw| validate::name(&raw, &mut errors));
    let member_permissions = body
        .member_permissions
        .and_then(|names| validate::permissions(&names, "member_permissions", &mut errors));
    validate::finish(errors)?;
    if name.is_none() && member_permissions.is_none() {
        return Ok(Json(member.into()));
    }

    // Only the base role's flags change access; a rename alone does not.
    let before = match member_permissions {
        Some(_) => Some(access::snapshot(&mut tx, server_id).await?),
        None => None,
    };
    let server = sqlx::query_as::<_, ServerRow>(
        "UPDATE servers \
         SET name = COALESCE($2, name), \
             member_permissions = COALESCE($3, member_permissions), \
             updated_at = now() \
         WHERE id = $1 \
         RETURNING id, name, owner_id, member_permissions, icon_url, created_at",
    )
    .bind(server_id)
    .bind(name.as_deref())
    .bind(member_permissions.map(Permissions::bits))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    if let Some(flags) = member_permissions {
        info!(server_id = %server_id, member_permissions = %flags, "member permissions changed");
    }
    if let Some(before) = &before {
        let after = access::snapshot(&mut tx, server_id).await?;
        access::after_change(&state, server_id, before, &after).await?;
    }
    tx.commit().await?;
    if before.is_some() {
        access::announce(&state, server_id).await;
    }
    Ok(Json(Membership { server, ..member }.into()))
}

/// Owner only. Members, categories, channels and invites go with it. A
/// blocked server can still be deleted.
async fn delete_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = membership::lock_server_bounded(&state.db, server_id).await?;
    let member = membership::load_even_blocked(&mut *tx, server_id, user.id).await?;
    member.require_owner()?;

    let members: Vec<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM server_members WHERE server_id = $1")
            .bind(server_id)
            .fetch_all(&mut *tx)
            .await?;
    let channel_ids = moderation::channel_ids(&mut tx, server_id).await?;
    moderation::after_removal_with(&state, server_id, &members, &channel_ids, "not_found").await?;
    let icon =
        icon::schedule_removal(&mut tx, server_id, member.server.icon_url.as_deref()).await?;
    sqlx::query("DELETE FROM servers WHERE id = $1")
        .bind(server_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some(key) = icon {
        crate::avatars::discard(&state, &key).await;
    }
    state
        .gateway
        .announce_recheck(Recheck::Server(server_id))
        .await;
    info!(server_id = %server_id, "server deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// Members can leave; the owner cannot and must transfer the server
/// ([`transfer_server`]) or delete it first. A blocked server can still be
/// left.
async fn leave_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load_even_blocked(&mut *tx, server_id, user.id).await?;
    if member.role() == Role::Owner {
        return Err(ApiError::Forbidden(
            "The owner cannot leave; delete the server instead.",
        ));
    }

    sqlx::query("DELETE FROM server_members WHERE server_id = $1 AND user_id = $2")
        .bind(server_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    moderation::after_removal(&state, &mut tx, server_id, user.id, "not_found").await?;
    tx.commit().await?;
    state.gateway.announce_recheck(Recheck::User(user.id)).await;
    info!(server_id = %server_id, user_id = %user.id, "member left");
    Ok(StatusCode::NO_CONTENT)
}

/// Owner only: another member becomes the owner (#519 §4.1). The previous
/// owner stays a member with the base role plus the roles they already
/// hold. A blocked server cannot be transferred (`load` answers
/// `403 server_blocked`); it can still be deleted.
async fn transfer_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<TransferBody>,
) -> Result<Json<ServerView>, ApiError> {
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let member = membership::load(&mut *tx, server_id, user.id).await?;
    member.require_owner()?;

    let raw = body.user_id.as_deref().map(str::trim).unwrap_or_default();
    let target_id = match raw.parse::<Uuid>() {
        _ if raw.is_empty() => Err("required"),
        Ok(id) if id != user.id => Ok(id),
        _ => Err("invalid"),
    }
    .map_err(|code| ApiError::Validation(FieldErrors::from([("user_id", code)])))?;
    let is_member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
    )
    .bind(server_id)
    .bind(target_id)
    .fetch_one(&mut *tx)
    .await?;
    // An account under deletion has left every server already; one that is
    // being deleted right now is waited for. A disabled account cannot sign
    // in to manage the server, so it is refused like a non-member.
    if !is_member || !account_usable(&mut tx, target_id).await? {
        return Err(ApiError::NotFound);
    }

    let server = sqlx::query_as::<_, ServerRow>(
        "UPDATE servers SET owner_id = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, name, owner_id, member_permissions, icon_url, created_at",
    )
    .bind(server_id)
    .bind(target_id)
    .fetch_one(&mut *tx)
    .await?;
    // The former owner now holds only the base role and their own roles.
    if access::former_owner_lost_voice(&mut tx, server_id, user.id).await? {
        state
            .gateway
            .revoke_voice_access(&[user.id], server_id)
            .await?;
    }
    tx.commit().await?;
    state
        .gateway
        .announce_recheck(Recheck::Server(server_id))
        .await;
    if let Err(err) = publish_server(
        &state,
        server_id,
        EventKind::E,
        None,
        Some(serde_json::json!({ "k": "o", "owner_id": target_id })),
    )
    .await
    {
        warn!(error = err.code(), %server_id, "ownership event not published");
    }
    info!(server_id = %server_id, from = %user.id, to = %target_id, "server ownership transferred");
    Ok(Json(Membership { server, ..member }.into()))
}

/// Key-share locks the account's row until commit unless it is under
/// deletion (`users.deleted_at`). The deletion takes the row `FOR UPDATE`,
/// so whichever comes second waits and then sees the other's result.
async fn account_alive(tx: &mut sqlx::PgConnection, user_id: Uuid) -> Result<bool, ApiError> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR KEY SHARE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(found.is_some())
}

/// [`account_alive`] for an account that must also be able to sign in:
/// not disabled by an operator (`users.disabled_at`).
async fn account_usable(tx: &mut sqlx::PgConnection, user_id: Uuid) -> Result<bool, ApiError> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL AND disabled_at IS NULL \
         FOR KEY SHARE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(found.is_some())
}
