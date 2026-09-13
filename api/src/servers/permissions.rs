//! Coarse permission flags (issue #4). One bitmask per server describes what
//! a *member* may do; the owner always has every flag. There are no roles
//! beyond owner/member and no per-channel overwrites in v1.
//!
//! On the wire a set is a JSON array of names (`["send_messages", …]`); in
//! Postgres it is the `INTEGER` bitmask.

use std::fmt;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Permission {
    /// Rename the server, edit member permissions, manage invites.
    ManageServer = 1,
    /// Create, rename, move and delete categories and channels.
    ManageChannels = 2,
    SendMessages = 4,
    SendFiles = 8,
    JoinVoice = 16,
    GoLive = 32,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Self::ManageServer,
        Self::ManageChannels,
        Self::SendMessages,
        Self::SendFiles,
        Self::JoinVoice,
        Self::GoLive,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::ManageServer => "manage_server",
            Self::ManageChannels => "manage_channels",
            Self::SendMessages => "send_messages",
            Self::SendFiles => "send_files",
            Self::JoinVoice => "join_voice",
            Self::GoLive => "go_live",
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
    pub const ALL: Permissions = Permissions(1 | 2 | 4 | 8 | 16 | 32);

    pub fn contains(self, permission: Permission) -> bool {
        self.0 & permission as i32 != 0
    }

    pub fn with(self, permission: Permission) -> Self {
        Self(self.0 | permission as i32)
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
    fn default_member_can_talk_but_not_manage() {
        let set = Permissions::DEFAULT_MEMBER;
        assert!(set.contains(Permission::SendMessages));
        assert!(set.contains(Permission::SendFiles));
        assert!(set.contains(Permission::JoinVoice));
        assert!(set.contains(Permission::GoLive));
        assert!(!set.contains(Permission::ManageServer));
        assert!(!set.contains(Permission::ManageChannels));
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
