//! Coarse permission flags (issue #4). The base role "Alle Mitglieder"
//! (`servers.member_permissions`) and every custom role (`server_roles`, v0.10)
//! hold one bitmask each; a member holds the union of the base role and their
//! roles, and the owner always has every flag. There are no per-channel
//! overwrites: a channel is only limited to roles or not (`channels.limited`).
//!
//! On the wire a set is a JSON array of names (`["send_messages", …]`); in
//! Postgres it is the `INTEGER` bitmask.

use std::fmt;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Permission {
    /// Rename the server, edit roles and member permissions, manage invites.
    ManageServer = 1,
    /// Create, rename, move and delete categories and channels.
    ManageChannels = 2,
    SendMessages = 4,
    SendFiles = 8,
    JoinVoice = 16,
    GoLive = 32,
    /// Delete someone else's message (own delete needs no flag).
    ManageMessages = 64,
    /// `@everyone` and `@here` notify; without it they are plain text.
    MentionEveryone = 128,
    /// Kick and ban members, list and lift bans.
    ModerateMembers = 256,
}

impl Permission {
    pub const ALL: [Permission; 9] = [
        Self::ManageServer,
        Self::ModerateMembers,
        Self::ManageChannels,
        Self::ManageMessages,
        Self::MentionEveryone,
        Self::SendMessages,
        Self::SendFiles,
        Self::JoinVoice,
        Self::GoLive,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::ManageServer => "manage_server",
            Self::ManageChannels => "manage_channels",
            Self::ManageMessages => "manage_messages",
            Self::MentionEveryone => "mention_everyone",
            Self::SendMessages => "send_messages",
            Self::SendFiles => "send_files",
            Self::JoinVoice => "join_voice",
            Self::GoLive => "go_live",
            Self::ModerateMembers => "moderate_members",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// A set of flags. `Default` is the empty set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Permissions(i32);

impl Permissions {
    pub const NONE: Permissions = Permissions(0);
    /// What a fresh server grants its members: talk, share, join voice, go
    /// live — but not manage anything. Mirrored by the column default.
    pub const DEFAULT_MEMBER: Permissions = Permissions(4 | 8 | 16 | 32);
    pub const ALL: Permissions = Permissions(1 | 2 | 4 | 8 | 16 | 32 | 64 | 128 | 256);

    pub fn contains(self, permission: Permission) -> bool {
        self.0 & permission as i32 != 0
    }

    pub fn with(self, permission: Permission) -> Self {
        Self(self.0 | permission as i32)
    }

    /// Both sets together: the base role plus a member's roles.
    pub fn union(self, other: Permissions) -> Self {
        Self(self.0 | other.0)
    }

    pub fn bits(self) -> i32 {
        self.0
    }

    pub fn iter(self) -> impl Iterator<Item = Permission> {
        Permission::ALL
            .into_iter()
            .filter(move |p| self.contains(*p))
    }

    /// Parses the JSON-array shape. Unknown names are reported back so the
    /// handler can turn them into a field error instead of a 400.
    pub fn from_names<S: AsRef<str>>(names: &[S]) -> Result<Self, String> {
        let mut set = Self::NONE;
        for name in names {
            let name = name.as_ref();
            match Permission::from_name(name) {
                Some(permission) => set = set.with(permission),
                None => return Err(name.to_owned()),
            }
        }
        Ok(set)
    }
}

/// Column → set. Unknown bits (a future migration) are dropped rather than
/// rejected so an older binary keeps working against a newer database.
impl From<i32> for Permissions {
    fn from(bits: i32) -> Self {
        Self(bits & Self::ALL.0)
    }
}

impl Serialize for Permissions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for permission in self.iter() {
            seq.serialize_element(permission.name())?;
        }
        seq.end()
    }
}

impl fmt::Display for Permissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.iter().map(Permission::name).collect();
        write!(f, "[{}]", names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for permission in Permission::ALL {
            assert_eq!(Permission::from_name(permission.name()), Some(permission));
        }
        assert_eq!(Permission::from_name("admin"), None);
    }

    #[test]
    fn all_holds_the_nine_flags_in_canonical_order() {
        assert_eq!(Permissions::ALL.bits(), 511);
        assert_eq!(
            Permission::ALL.iter().fold(0, |bits, p| bits | *p as i32),
            511
        );
        let names: Vec<&str> = Permissions::ALL.iter().map(Permission::name).collect();
        assert_eq!(
            names,
            [
                "manage_server",
                "moderate_members",
                "manage_channels",
                "manage_messages",
                "mention_everyone",
                "send_messages",
                "send_files",
                "join_voice",
                "go_live",
            ]
        );
        assert!(Permissions::from(256).contains(Permission::ModerateMembers));
        assert_eq!(Permissions::from(1024 | 256).bits(), 256);
    }

    #[test]
    fn default_member_can_talk_but_not_manage() {
        let set = Permissions::DEFAULT_MEMBER;
        assert!(set.contains(Permission::SendMessages));
        assert!(set.contains(Permission::SendFiles));
        assert!(set.contains(Permission::JoinVoice));
        assert!(set.contains(Permission::GoLive));
        assert!(!set.contains(Permission::ManageServer));
        assert!(!set.contains(Permission::ManageChannels));
        assert!(!set.contains(Permission::ModerateMembers));
        assert_eq!(set.bits(), 60, "matches the column default");
    }

    #[test]
    fn from_names_rejects_unknown() {
        assert_eq!(
            Permissions::from_names(&["send_messages", "go_live"]),
            Ok(Permissions::NONE
                .with(Permission::SendMessages)
                .with(Permission::GoLive))
        );
        assert_eq!(
            Permissions::from_names(&["send_messages", "nuke"]),
            Err("nuke".to_owned())
        );
        assert_eq!(Permissions::from_names::<&str>(&[]), Ok(Permissions::NONE));
    }

    #[test]
    fn serialises_as_name_array_and_masks_unknown_bits() {
        let json = serde_json::to_value(Permissions::from(1 | 4 | 1024)).unwrap();
        assert_eq!(json, serde_json::json!(["manage_server", "send_messages"]));
        assert_eq!(
            serde_json::to_value(Permissions::NONE).unwrap(),
            serde_json::json!([])
        );
    }
}
