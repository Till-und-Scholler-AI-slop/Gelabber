//! Who the caller is inside a server, and what they may do there.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::error::ApiError;

use super::ServerRow;
use super::channel::Channel;
use super::permissions::{Permission, Permissions};

/// The select list that builds a [`Membership`], over `servers s` joined
/// with the caller's `server_members m`: the server row, the caller, their
/// custom role ids by position, the bit-or of those roles' flags and
/// whether an operator blocked the server.
macro_rules! membership_columns {
    () => {
        "s.id, s.name, s.owner_id, s.member_permissions, s.icon_url, s.created_at, \
         m.user_id, s.blocked_at IS NOT NULL AS blocked, \
         ARRAY(SELECT mr.role_id FROM server_member_roles mr \
                 JOIN server_roles r ON r.id = mr.role_id \
                WHERE mr.server_id = m.server_id AND mr.user_id = m.user_id \
                ORDER BY r.position, r.id) AS role_ids, \
         COALESCE((SELECT bit_or(r.permissions) FROM server_member_roles mr \
                     JOIN server_roles r ON r.id = mr.role_id \
                    WHERE mr.server_id = m.server_id AND mr.user_id = m.user_id), 0) \
             AS role_permissions"
    };
}
pub(crate) use membership_columns;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Member,
}

/// The caller's membership in one server. Built by [`load`], or in
/// `servers/mod.rs` for a user just shown to be a member (the server list,
/// a new server, an update after `load`): holding one means they were.
#[derive(Debug, Clone, FromRow)]
pub struct Membership {
    #[sqlx(flatten)]
    pub server: ServerRow,
    pub user_id: Uuid,
    /// An operator blocked the server (`servers.blocked_at`). Only
    /// [`load_even_blocked`] and the server list hand out such a membership.
    pub blocked: bool,
    /// The caller's custom roles, by position. "Alle Mitglieder" is implicit.
    pub role_ids: Vec<Uuid>,
    /// The union of those roles' flags.
    #[sqlx(try_from = "i32")]
    pub role_permissions: Permissions,
}

impl Membership {
    pub fn role(&self) -> Role {
        if self.server.owner_id == self.user_id {
            Role::Owner
        } else {
            Role::Member
        }
    }

    /// A membership that holds no custom role: the owner, or a member right
    /// after creating or joining the server.
    pub fn base(server: ServerRow, user_id: Uuid) -> Self {
        Self {
            server,
            user_id,
            blocked: false,
            role_ids: Vec::new(),
            role_permissions: Permissions::NONE,
        }
    }

    /// Effective flags: everything for the owner, the base role
    /// ("Alle Mitglieder", the server's member mask) plus the caller's roles
    /// for everybody else.
    pub fn permissions(&self) -> Permissions {
        match self.role() {
            Role::Owner => Permissions::ALL,
            Role::Member => self.server.member_permissions.union(self.role_permissions),
        }
    }

    /// Whether the caller may see the channel: the owner sees every channel,
    /// everybody else an unlimited one or one limited to a role they hold.
    /// The same rule as the SQL function `channel_visible_to`.
    pub fn sees(&self, channel: &Channel) -> bool {
        self.role() == Role::Owner
            || !channel.limited
            || channel.role_ids.iter().any(|id| self.role_ids.contains(id))
    }

    pub fn can(&self, permission: Permission) -> bool {
        self.permissions().contains(permission)
    }

    /// `403 forbidden` unless the flag is held. Later issues call this for
    /// `SendMessages`, `SendFiles`, `JoinVoice` and `GoLive`.
    pub fn require(&self, permission: Permission) -> Result<(), ApiError> {
        if self.can(permission) {
            Ok(())
        } else {
            Err(ApiError::Forbidden(denied_message(permission)))
        }
    }

    pub fn require_owner(&self) -> Result<(), ApiError> {
        if self.role() == Role::Owner {
            Ok(())
        } else {
            Err(ApiError::Forbidden("Only the server owner can do this."))
        }
    }
}

fn denied_message(permission: Permission) -> &'static str {
    match permission {
        Permission::ManageServer => "You need the manage_server permission.",
        Permission::ManageChannels => "You need the manage_channels permission.",
        Permission::ManageMessages => "You need the manage_messages permission.",
        Permission::SendMessages => "You need the send_messages permission.",
        Permission::SendFiles => "You need the send_files permission.",
        Permission::JoinVoice => "You need the join_voice permission.",
        Permission::GoLive => "You need the go_live permission.",
        Permission::MentionEveryone => "You need the mention_everyone permission.",
        Permission::ModerateMembers => "You need the moderate_members permission.",
    }
}

/// Loads the server *through* the caller's membership. A server the caller
/// is not part of — or that does not exist — is a `404 not_found`; a server
/// an operator blocked is a `403 server_blocked`.
pub async fn load<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<Membership, ApiError> {
    let member = load_even_blocked(db, server_id, user_id).await?;
    if member.blocked {
        return Err(ApiError::ServerBlocked);
    }
    Ok(member)
}

/// [`load`] that also hands out a blocked server's membership. Only for
/// what a blocked server still allows: its owner deletes it, a member
/// leaves it.
pub async fn load_even_blocked<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<Membership, ApiError> {
    sqlx::query_as::<_, Membership>(concat!(
        "SELECT ",
        membership_columns!(),
        " FROM servers s \
         JOIN server_members m ON m.server_id = s.id AND m.user_id = $2 \
         WHERE s.id = $1"
    ))
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Serialize joins, removals, deletes and permission changes on the server row.
/// Readers hold a shared lock until socket registration / ticket mint completes.
pub async fn lock_server(
    db: &PgPool,
    server_id: Uuid,
    exclusive: bool,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, ApiError> {
    let mut tx = db.begin().await?;
    lock_server_conn(&mut tx, server_id, exclusive).await?;
    Ok(tx)
}

/// [`lock_server`] exclusively for kick, ban and server delete, waiting at
/// most 5 s. Readers hold the row shared only briefly, so a longer wait means
/// someone keeps it held (a member stalling their own ban); the caller gets
/// a retryable answer. The rest of the transaction waits as usual.
pub async fn lock_server_bounded(
    db: &PgPool,
    server_id: Uuid,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, ApiError> {
    let mut tx = db.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    let locked = sqlx::query_scalar::<_, Uuid>("SELECT id FROM servers WHERE id = $1 FOR UPDATE")
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await;
    match locked {
        Ok(Some(_)) => {}
        Ok(None) => return Err(ApiError::NotFound),
        // 55P03 lock_not_available: the timeout ran out.
        Err(sqlx::Error::Database(err)) if err.code().as_deref() == Some("55P03") => {
            return Err(ApiError::RateLimited { retry_after: 1 });
        }
        Err(err) => return Err(err.into()),
    }
    sqlx::query("SET LOCAL lock_timeout TO DEFAULT")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

pub async fn lock_server_conn(
    db: &mut sqlx::PgConnection,
    server_id: Uuid,
    exclusive: bool,
) -> Result<(), ApiError> {
    let query = if exclusive {
        "SELECT id FROM servers WHERE id = $1 FOR UPDATE"
    } else {
        "SELECT id FROM servers WHERE id = $1 FOR SHARE"
    };
    sqlx::query_scalar::<_, Uuid>(query)
        .bind(server_id)
        .fetch_optional(&mut *db)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(())
}

/// A rejoin is a new authorization, even if it happened between two heartbeats.
/// A blocked server has none, like one the user left.
pub async fn revision<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<Option<(chrono::DateTime<chrono::Utc>, i32)>, ApiError> {
    Ok(sqlx::query_as("SELECT m.joined_at, CASE WHEN s.owner_id = m.user_id THEN $3 ELSE server_member_bits(m.server_id, m.user_id) END FROM server_members m JOIN servers s ON s.id = m.server_id WHERE m.server_id = $1 AND m.user_id = $2 AND s.blocked_at IS NULL")
        .bind(server_id).bind(user_id).bind(crate::servers::permissions::Permissions::ALL.bits()).fetch_optional(db).await?)
}

/// [`revision`] for several servers in one query. A server the user is no
/// longer a member of (or that is gone or blocked) has no entry.
pub async fn revisions<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    user_id: Uuid,
    server_ids: &[Uuid],
) -> Result<HashMap<Uuid, (chrono::DateTime<chrono::Utc>, i32)>, ApiError> {
    let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>, i32)> = sqlx::query_as("SELECT m.server_id, m.joined_at, CASE WHEN s.owner_id = m.user_id THEN $3 ELSE server_member_bits(m.server_id, m.user_id) END FROM server_members m JOIN servers s ON s.id = m.server_id WHERE m.user_id = $1 AND m.server_id = ANY($2) AND s.blocked_at IS NULL")
        .bind(user_id).bind(server_ids).bind(crate::servers::permissions::Permissions::ALL.bits()).fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .map(|(server_id, joined_at, permissions)| (server_id, (joined_at, permissions)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn membership(owner: Uuid, user: Uuid, member_permissions: Permissions) -> Membership {
        Membership::base(
            ServerRow {
                id: uid(100),
                name: "Test".into(),
                owner_id: owner,
                member_permissions,
                icon_url: None,
                created_at: Utc::now(),
            },
            user,
        )
    }

    fn channel(limited: bool, role_ids: Vec<Uuid>) -> Channel {
        Channel {
            id: uid(200),
            server_id: Some(uid(100)),
            category_id: None,
            name: "spiele".into(),
            kind: super::super::channel::ChannelKind::Text,
            created_at: Utc::now(),
            limited,
            role_ids,
        }
    }

    #[test]
    fn owner_has_everything() {
        let owner = uid(1);
        let m = membership(owner, owner, Permissions::NONE);
        assert_eq!(m.role(), Role::Owner);
        assert_eq!(m.permissions(), Permissions::ALL);
        assert!(m.require(Permission::ManageServer).is_ok());
        assert!(m.require_owner().is_ok());
    }

    #[test]
    fn member_is_bound_by_the_server_mask() {
        let m = membership(uid(1), uid(2), Permissions::DEFAULT_MEMBER);
        assert_eq!(m.role(), Role::Member);
        assert!(m.require(Permission::SendMessages).is_ok());
        assert!(matches!(
            m.require(Permission::ManageChannels),
            Err(ApiError::Forbidden(_))
        ));
        assert!(matches!(m.require_owner(), Err(ApiError::Forbidden(_))));

        let promoted = membership(
            uid(1),
            uid(2),
            Permissions::DEFAULT_MEMBER.with(Permission::ManageChannels),
        );
        assert!(promoted.require(Permission::ManageChannels).is_ok());
        assert!(promoted.require(Permission::ManageServer).is_err());
    }

    #[test]
    fn member_holds_the_union_of_base_and_roles() {
        let mut m = membership(uid(1), uid(2), Permissions::DEFAULT_MEMBER);
        m.role_ids = vec![uid(10)];
        m.role_permissions = Permissions::NONE
            .with(Permission::ModerateMembers)
            .with(Permission::ManageMessages);
        let flags = m.permissions();
        assert!(
            flags.contains(Permission::SendMessages),
            "from the base role"
        );
        assert!(flags.contains(Permission::ModerateMembers), "from the role");
        assert!(flags.contains(Permission::ManageMessages), "from the role");
        assert!(!flags.contains(Permission::ManageServer));
        assert_eq!(flags.bits(), 60 | 64 | 256);
        assert!(m.require(Permission::ModerateMembers).is_ok());

        // Roles add nothing for the owner, who already holds everything.
        let mut owner = membership(uid(1), uid(1), Permissions::NONE);
        owner.role_permissions = Permissions::NONE.with(Permission::SendFiles);
        assert_eq!(owner.permissions(), Permissions::ALL);
    }

    #[test]
    fn sees_unlimited_channels_and_those_of_a_held_role() {
        let (owner, user) = (uid(1), uid(2));
        let (role, other) = (uid(10), uid(11));
        let mut m = membership(owner, user, Permissions::DEFAULT_MEMBER);
        assert!(m.sees(&channel(false, vec![])));
        assert!(!m.sees(&channel(true, vec![role])));
        assert!(!m.sees(&channel(true, vec![])), "no roles left: owner only");

        m.role_ids = vec![other, role];
        assert!(m.sees(&channel(true, vec![role])));
        assert!(!m.sees(&channel(true, vec![uid(12)])));

        let o = membership(owner, owner, Permissions::NONE);
        assert!(o.sees(&channel(true, vec![role])));
        assert!(o.sees(&channel(true, vec![])));
    }
}
