//! Who the caller is inside a server, and what they may do there.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

use super::ServerRow;
use super::permissions::{Permission, Permissions};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Member,
}

/// The caller's membership in one server. Constructed only by [`load`], so
/// holding one proves the user *is* a member.
#[derive(Debug, Clone)]
pub struct Membership {
    pub server: ServerRow,
    pub user_id: Uuid,
}

impl Membership {
    pub fn role(&self) -> Role {
        if self.server.owner_id == self.user_id {
            Role::Owner
        } else {
            Role::Member
        }
    }

    /// Effective flags: everything for the owner, the server's member mask
    /// for everybody else.
    pub fn permissions(&self) -> Permissions {
        match self.role() {
            Role::Owner => Permissions::ALL,
            Role::Member => self.server.member_permissions,
        }
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
        Permission::SendMessages => "You need the send_messages permission.",
        Permission::SendFiles => "You need the send_files permission.",
        Permission::JoinVoice => "You need the join_voice permission.",
        Permission::GoLive => "You need the go_live permission.",
    }
}

/// Loads the server *through* the caller's membership. A server the caller
/// is not part of — or that does not exist — is a `404 not_found`.
pub async fn load(db: &PgPool, server_id: Uuid, user_id: Uuid) -> Result<Membership, ApiError> {
    let server = sqlx::query_as::<_, ServerRow>(
        "SELECT s.id, s.name, s.owner_id, s.member_permissions, s.created_at \
         FROM servers s \
         JOIN server_members m ON m.server_id = s.id AND m.user_id = $2 \
         WHERE s.id = $1",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Membership { server, user_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn membership(owner: Uuid, user: Uuid, member_permissions: Permissions) -> Membership {
        Membership {
            server: ServerRow {
                id: uid(100),
                name: "Test".into(),
                owner_id: owner,
                member_permissions,
                created_at: Utc::now(),
            },
            user_id: user,
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
}
