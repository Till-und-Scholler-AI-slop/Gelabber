//! What the web keeps in localStorage per account and this app keeps in
//! `native-state.json`: the last place, channel and DM (servers/lastPlace.ts,
//! lastChannel.ts, dms/lastDm.ts), unsent texts for a week
//! (messages/drafts.ts), and the window's size and position. Per-person
//! volumes are screens/voice/volume.rs's (`user-volumes.json`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::screens::shell::{Memory, Place};

const FILE: &str = "native-state.json";
/// `DRAFT_MAX_AGE_MS` of messages/drafts.ts.
pub const DRAFT_MAX_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub accounts: HashMap<Uuid, Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<StoredPlace>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channels: HashMap<Uuid, Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dm: Option<Uuid>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub drafts: HashMap<Uuid, Draft>,
}

/// `Place` of lastPlace.ts, as it stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum StoredPlace {
    Server(Uuid),
    Dm(Uuid),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
    /// Milliseconds since 1970 of the last change.
    pub at: i64,
}

/// The window in logical points; no position where the desktop does not
/// tell it (Wayland).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub width: f32,
    pub height: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    #[serde(default)]
    pub maximized: bool,
}

/// Smallest window the Tauri app allowed.
pub const MIN_SIZE: [f32; 2] = [480.0, 360.0];
pub const DEFAULT_SIZE: [f32; 2] = [1280.0, 800.0];

impl Window {
    /// Sizes a damaged file or another screen could make useless are
    /// brought back into range.
    pub fn sane(self) -> Option<Self> {
        let ok = |v: f32| v.is_finite();
        if !ok(self.width) || !ok(self.height) {
            return None;
        }
        let finite = |v: Option<f32>| v.filter(|v| ok(*v) && v.abs() < 100_000.0);
        Some(Self {
            width: self.width.clamp(MIN_SIZE[0], 16_384.0),
            height: self.height.clamp(MIN_SIZE[1], 16_384.0),
            x: finite(self.x),
            y: finite(self.y),
            maximized: self.maximized,
        })
    }
}

pub fn load() -> Stored {
    serde_json::from_value(super::paths::read_json(FILE)).unwrap_or_default()
}

/// Off the UI thread, in order; a failed write only loses the newest change.
pub fn save(stored: &Stored) {
    let (Ok(bytes), Some(path)) = (serde_json::to_vec_pretty(stored), super::paths::file(FILE))
    else {
        return;
    };
    super::paths::write_latest(path, bytes, true);
}

/// The stored window for the viewport builder.
pub fn window() -> Option<Window> {
    load().window.and_then(Window::sane)
}

impl From<&Memory> for Account {
    fn from(m: &Memory) -> Self {
        Self {
            place: m.place.map(|p| match p {
                Place::Server(id) => StoredPlace::Server(id),
                Place::Dm(id) => StoredPlace::Dm(id),
            }),
            channels: m.channels.clone(),
            dm: m.dm,
            drafts: HashMap::new(),
        }
    }
}

impl Account {
    pub fn memory(&self) -> Memory {
        Memory {
            place: self.place.map(|p| match p {
                StoredPlace::Server(id) => Place::Server(id),
                StoredPlace::Dm(id) => Place::Dm(id),
            }),
            channels: self.channels.clone(),
            dm: self.dm,
        }
    }

    /// The places of `memory`, leaving the drafts as they are.
    pub fn set_memory(&mut self, memory: &Memory) {
        let places = Account::from(memory);
        self.place = places.place;
        self.channels = places.channels;
        self.dm = places.dm;
    }

    /// The drafts that are not older than a week at `now`.
    pub fn fresh_drafts(&self, now: i64) -> HashMap<Uuid, String> {
        self.drafts
            .iter()
            .filter(|(_, d)| now - d.at < DRAFT_MAX_AGE_MS && !d.text.is_empty())
            .map(|(id, d)| (*id, d.text.clone()))
            .collect()
    }

    /// Takes the texts on screen: a changed text gets `now`, a sent one goes;
    /// expired ones are dropped.
    pub fn set_drafts(&mut self, shown: &HashMap<Uuid, String>, now: i64) {
        self.drafts.retain(|id, d| {
            shown.get(id).is_some_and(|t| *t == d.text) && now - d.at < DRAFT_MAX_AGE_MS
        });
        for (id, text) in shown {
            self.drafts.entry(*id).or_insert_with(|| Draft {
                text: text.clone(),
                at: now,
            });
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_round_trip_in_the_webs_shape() {
        let (s, c, d) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let memory = Memory {
            place: Some(Place::Server(s)),
            channels: HashMap::from([(s, c)]),
            dm: Some(d),
        };
        let account = Account::from(&memory);
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["place"]["kind"], "server");
        assert_eq!(json["place"]["id"], s.to_string());
        let back: Account = serde_json::from_value(json).unwrap();
        let again = back.memory();
        assert_eq!(again.place, Some(Place::Server(s)));
        assert_eq!(again.channels.get(&s), Some(&c));
        assert_eq!(again.dm, Some(d));
    }

    #[test]
    fn drafts_keep_their_time_until_changed_and_expire_after_a_week() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut account = Account::default();
        account.set_drafts(&HashMap::from([(a, "hallo".into())]), 1_000);
        account.set_drafts(
            &HashMap::from([(a, "hallo".into()), (b, "x".into())]),
            2_000,
        );
        assert_eq!(account.drafts[&a].at, 1_000);
        account.set_drafts(
            &HashMap::from([(a, "hallo du".into()), (b, "x".into())]),
            3_000,
        );
        assert_eq!(account.drafts[&a].at, 3_000);
        // Sent: gone.
        account.set_drafts(&HashMap::from([(b, "x".into())]), 4_000);
        assert!(!account.drafts.contains_key(&a));
        assert_eq!(account.fresh_drafts(4_000).len(), 1);
        assert!(account.fresh_drafts(2_000 + DRAFT_MAX_AGE_MS).is_empty());
    }

    #[test]
    fn windows_come_back_in_range() {
        let w = Window {
            width: 100.0,
            height: f32::INFINITY,
            x: None,
            y: None,
            maximized: false,
        };
        assert_eq!(w.sane(), None);
        let w = Window {
            width: 100.0,
            height: 20_000.0,
            x: Some(f32::NAN),
            y: Some(40.0),
            maximized: true,
        }
        .sane()
        .unwrap();
        assert_eq!(
            (w.width, w.height, w.x, w.y),
            (480.0, 16_384.0, None, Some(40.0))
        );
        // Unknown files read as empty, not as an error.
        let s: Stored = serde_json::from_str(r#"{"accounts":{},"other":1}"#).unwrap();
        assert_eq!(s, Stored::default());
    }
}
