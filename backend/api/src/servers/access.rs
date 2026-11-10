//! Access changes take effect at once (v0.10 roles, #516 §3). A write that
//! can change what members may do or see (the base role's flags, a role's
//! flags, deleting a role, a member's roles, a channel's roles) runs inside
//! the exclusive server lock as:
//!
//! 1. `before = snapshot(tx)`
//! 2. the write
//! 3. `after = snapshot(tx)`
//! 4. [`after_change`]
//! 5. commit
//! 6. [`announce`]
//!
//! [`after_change`] cuts voice that is no longer allowed before the commit;
//! [`announce`] makes every socket of the server recheck its topics and seats
//! and tells clients to refetch the server.

use std::collections::{HashMap, HashSet};

use sqlx::PgConnection;
use tracing::warn;
use uuid::Uuid;

use crate::error::ApiError;
use crate::gateway::{EventKind, Recheck, publish_server};
use crate::state::AppState;

use super::permissions::{Permission, Permissions};

/// What every non-owner member of a server may do and where they may talk:
/// their effective flags and the ids of the voice rooms visible to them.
/// Text channels are left to the recheck. The owner always holds everything
/// and is left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot(HashMap<Uuid, (Permissions, HashSet<Uuid>)>);

impl Snapshot {
    /// Members who held `JoinVoice` or `GoLive` before and lack it after.
    fn lost_voice_rights(&self, after: &Snapshot) -> Vec<Uuid> {
        const VOICE: [Permission; 2] = [Permission::JoinVoice, Permission::GoLive];
        self.0
            .iter()
            .filter(|(user, (before, _))| {
                after.0.get(*user).is_some_and(|(now, _)| {
                    VOICE
                        .iter()
                        .any(|flag| before.contains(*flag) && !now.contains(*flag))
                })
            })
            .map(|(user, _)| *user)
            .collect()
    }

    /// Members who saw a voice room before and do not after.
    fn lost_voice_rooms(&self, after: &Snapshot) -> Vec<Uuid> {
        self.0
            .iter()
            .filter(|(user, (_, saw))| {
                after
                    .0
                    .get(*user)
                    .is_some_and(|(_, sees)| !saw.is_subset(sees))
            })
            .map(|(user, _)| *user)
            .collect()
    }
}

/// Reads the [`Snapshot`] in one query. Run it inside the exclusive server
/// lock, before and after the write.
pub async fn snapshot(db: &mut PgConnection, server_id: Uuid) -> Result<Snapshot, ApiError> {
    let rows: Vec<(Uuid, i32, Vec<Uuid>)> = sqlx::query_as(
        "SELECT m.user_id, server_member_bits(m.server_id, m.user_id), \
             ARRAY(SELECT c.id FROM channels c \
                    WHERE c.server_id = $1 AND c.kind = 'voice' \
                      AND channel_visible_to(c.id, m.user_id)) \
         FROM server_members m JOIN servers s ON s.id = m.server_id \
         WHERE m.server_id = $1 AND m.user_id <> s.owner_id",
    )
    .bind(server_id)
    .fetch_all(db)
    .await?;
    Ok(Snapshot(
        rows.into_iter()
            .map(|(user, bits, channels)| {
                (
                    user,
                    (Permissions::from(bits), channels.into_iter().collect()),
                )
            })
            .collect(),
    ))
}

/// Cuts the voice access the change took away, before the commit: members
/// who lost `JoinVoice` or `GoLive`, and members who lost sight of a voice
/// room, seated in it or not. Both rotate the member's media authority and
/// leave their seats (`revoke_voice_access`). The rotation also voids an
/// unused ticket and a media connection without a gateway seat, which the
/// media service checks by authority alone. Text topics are left to the
/// recheck that [`announce`] starts.
pub async fn after_change(
    state: &AppState,
    server_id: Uuid,
    before: &Snapshot,
    after: &Snapshot,
) -> Result<(), ApiError> {
    let mut users = before.lost_voice_rights(after);
    users.extend(before.lost_voice_rooms(after));
    if users.is_empty() {
        return Ok(());
    }
    users.sort_unstable();
    users.dedup();
    state.gateway.revoke_voice_access(&users, server_id).await
}

/// After the commit: every socket of the server rechecks its topics and
/// seats, and clients get the server event `{"k": "a"}` ("access changed")
/// to refetch the server detail and list.
pub async fn announce(state: &AppState, server_id: Uuid) {
    state
        .gateway
        .announce_recheck(Recheck::Server(server_id))
        .await;
    publish_changed(state, server_id).await;
}

/// The `{"k": "a"}` event alone, for changes that alter no access: a role
/// created, renamed or recoloured. Other managers' lists and chips refresh.
pub async fn publish_changed(state: &AppState, server_id: Uuid) {
    if let Err(err) = publish_server(
        state,
        server_id,
        EventKind::E,
        None,
        Some(serde_json::json!({ "k": "a" })),
    )
    .await
    {
        warn!(error = err.code(), %server_id, "access change event not published");
    }
}

/// Everyone in the server who holds `permission`: the owner, plus every
/// member whose base role or custom roles grant it. By join order. Nobody
/// in a blocked server.
pub async fn holders<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    server_id: Uuid,
    permission: Permission,
) -> Result<Vec<Uuid>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT m.user_id FROM server_members m JOIN servers s ON s.id = m.server_id \
         WHERE m.server_id = $1 AND s.blocked_at IS NULL \
           AND (m.user_id = s.owner_id OR server_member_bits(m.server_id, m.user_id) & $2 <> 0) \
         ORDER BY m.joined_at, m.user_id",
    )
    .bind(server_id)
    .bind(permission as i32)
    .fetch_all(db)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn snap(entries: &[(u128, Permissions, &[u128])]) -> Snapshot {
        Snapshot(
            entries
                .iter()
                .map(|(user, bits, seen)| {
                    (uid(*user), (*bits, seen.iter().map(|c| uid(*c)).collect()))
                })
                .collect(),
        )
    }

    #[test]
    fn voice_rights_lost_are_join_voice_or_go_live() {
        let full = Permissions::DEFAULT_MEMBER;
        let no_live = Permissions::NONE
            .with(Permission::SendMessages)
            .with(Permission::JoinVoice);
        let before = snap(&[(1, full, &[]), (2, full, &[]), (3, no_live, &[])]);
        let after = snap(&[
            (1, no_live, &[]),
            (2, full.with(Permission::ManageServer), &[]),
            (3, Permissions::NONE.with(Permission::JoinVoice), &[]),
        ]);
        assert_eq!(before.lost_voice_rights(&after), vec![uid(1)]);
        assert!(
            after.lost_voice_rights(&before).is_empty(),
            "gains cut nothing"
        );
    }

    #[test]
    fn lost_voice_rooms_are_seen_before_and_hidden_after() {
        let bits = Permissions::DEFAULT_MEMBER;
        let before = snap(&[(1, bits, &[10, 11]), (2, bits, &[10]), (9, bits, &[10])]);
        let after = snap(&[(1, bits, &[11]), (2, bits, &[10, 11])]);
        assert_eq!(before.lost_voice_rooms(&after), vec![uid(1)]);
        assert_eq!(after.lost_voice_rooms(&before), vec![uid(2)], "reversed");
    }
}
