//! What the web keeps in localStorage per account and this app keeps in
//! `native-state.json`: the last place, channel and DM (servers/lastPlace.ts,
//! lastChannel.ts, dms/lastDm.ts), unsent texts for a week
//! (messages/drafts.ts), and the window's size and position. Per-person
//! volumes are screens/voice/volume.rs's (`user-volumes.json`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
    /// Per-person volumes as 0.9.0–0.9.2 kept (and applied) them here. Moved
    /// into `user-volumes.json` at load, and kept here until that worked.
    #[serde(default, rename = "volumes", skip_serializing_if = "Option::is_none")]
    pub old_volumes: Option<Map<String, Value>>,
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

/// The window in the desktop's logical pixels (egui points at zoom 1); no
/// position where the desktop does not tell it (Wayland).
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

/// A file this build cannot read (a hand edit, a newer version's) is moved
/// to `native-state.json.unreadable` (`-2` and on when that is taken), so
/// the next save does not overwrite it.
pub fn load() -> Stored {
    let Some(path) = super::paths::file(FILE) else {
        return Stored::default();
    };
    let read =
        std::fs::read(&path).map_err(|e| (e.kind() != std::io::ErrorKind::NotFound, e.to_string()));
    let parsed =
        read.and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| (true, e.to_string())));
    match parsed {
        Ok(mut stored) => {
            move_old_volumes(&mut stored);
            stored
        }
        Err((there, e)) => {
            if there {
                // Each one kept: an earlier rescued copy is not replaced.
                let aside = (1..)
                    .map(|n| match n {
                        1 => path.with_file_name(format!("{FILE}.unreadable")),
                        n => path.with_file_name(format!("{FILE}.unreadable-{n}")),
                    })
                    .find(|p| !p.exists())
                    .unwrap_or_else(|| path.with_file_name(format!("{FILE}.unreadable")));
                tracing::warn!("state: {e}; kept as {}", aside.display());
                if let Err(e) = std::fs::rename(&path, &aside) {
                    tracing::warn!("state: {e}");
                }
            }
            Stored::default()
        }
    }
}

/// The volumes of [`Account::old_volumes`] into the sliders' store
/// (screens/voice/volume.rs) for each person it has no level for, with its
/// range and its mute; the old field goes once that file is written.
fn move_old_volumes(stored: &mut Stored) {
    use crate::screens::voice::volume;
    if stored.accounts.values().all(|a| a.old_volumes.is_none()) {
        return;
    }
    let Some(path) = super::paths::file(volume::FILE) else {
        return;
    };
    let mut all = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(all)) => all,
            _ => {
                tracing::warn!("state: {} does not read; volumes stay", path.display());
                return;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => {
            tracing::warn!("state: {}: {e}; volumes stay", path.display());
            return;
        }
    };
    for (id, account) in &stored.accounts {
        let Some(old) = &account.old_volumes else {
            continue;
        };
        let key = id.to_string();
        let entry = all
            .entry(key.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(people) = entry else {
            continue;
        };
        for (person, level) in old {
            if Uuid::parse_str(person).is_err() || people.contains_key(person) {
                continue;
            }
            let volume = level
                .get("volume")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .unwrap_or(1.0)
                .clamp(0.0, volume::MAX_PLAYBACK);
            let muted = level.get("muted").and_then(Value::as_bool) == Some(true);
            if (volume - 1.0).abs() > f64::EPSILON || muted {
                people.insert(
                    person.clone(),
                    serde_json::json!({ "volume": volume, "muted": muted }),
                );
            }
        }
        if people.is_empty() {
            all.remove(&key);
        }
    }
    let written = serde_json::to_vec_pretty(&Value::Object(all))
        .map_err(std::io::Error::other)
        .and_then(|bytes| super::paths::write_atomic(&path, &bytes, false));
    match written {
        Ok(()) => {
            for account in stored.accounts.values_mut() {
                account.old_volumes = None;
            }
        }
        Err(e) => tracing::warn!("state: {}: {e}; volumes stay", path.display()),
    }
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
    let mut window = load().window.and_then(Window::sane)?;
    // A monitor that is gone would open the window where nobody sees it.
    if !on_screen(&window) {
        window.x = None;
        window.y = None;
    }
    Some(window)
}

/// Whether a monitor shows any part of the window at its stored place.
#[cfg(windows)]
fn on_screen(window: &Window) -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect};
    let (Some(x), Some(y)) = (window.x, window.y) else {
        return true;
    };
    let rect = RECT {
        left: x as i32,
        top: y as i32,
        right: (x + window.width) as i32,
        bottom: (y + window.height) as i32,
    };
    // SAFETY: only reads the rectangle.
    !unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_null()
}

/// X11 window managers keep a window on screen; Wayland ignores positions.
#[cfg(not(windows))]
fn on_screen(_: &Window) -> bool {
    true
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
            old_volumes: None,
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

    #[test]
    fn a_file_this_build_cannot_read_is_moved_aside_not_overwritten() {
        let config = super::super::paths::testing::TempConfig::new("state-unreadable");
        let path = config.dir.join(FILE);
        let text =
            r#"{"accounts":{"0f9a4c1e-8f1b-4f3e-9d55-2b7a1c3e5d01":{"place":{"kind":"planet"}}}}"#;
        std::fs::write(&path, text).unwrap();
        assert_eq!(load(), Stored::default());
        assert!(!path.exists());
        let aside = config.dir.join(format!("{FILE}.unreadable"));
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), text);
        // A second one does not replace the first.
        std::fs::write(&path, "{halb").unwrap();
        assert_eq!(load(), Stored::default());
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), text);
        let second = config.dir.join(format!("{FILE}.unreadable-2"));
        assert_eq!(std::fs::read_to_string(second).unwrap(), "{halb");
        drop(config);
    }

    /// 0.9.0–0.9.2 kept imported per-person volumes in this file and applied
    /// them; they move once into the sliders' store, without replacing a
    /// level set there, and only then leave this file.
    #[test]
    fn old_volumes_move_into_the_sliders_store_once() {
        let config = super::super::paths::testing::TempConfig::new("state-old-volumes");
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let (x, y, z) = (
            Uuid::from_u128(11),
            Uuid::from_u128(12),
            Uuid::from_u128(13),
        );
        let old = serde_json::json!({"accounts": {
            a.to_string(): {"dm": z, "volumes": {
                x.to_string(): {"volume": 0.5, "muted": true},
                y.to_string(): {"volume": 2.0},
                z.to_string(): {"volume": 9.0},
            }},
            b.to_string(): {"volumes": {x.to_string(): {"volume": 1.5}}},
        }});
        std::fs::write(config.dir.join(FILE), old.to_string()).unwrap();
        let volumes = config.dir.join(crate::screens::voice::volume::FILE);
        let own =
            serde_json::json!({a.to_string(): {y.to_string(): {"volume": 3.0, "muted": false}}});
        std::fs::write(&volumes, own.to_string()).unwrap();

        let stored = load();
        let moved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&volumes).unwrap()).unwrap();
        assert_eq!(
            moved,
            serde_json::json!({
                a.to_string(): {
                    x.to_string(): {"volume": 0.5, "muted": true},
                    y.to_string(): {"volume": 3.0, "muted": false},
                    z.to_string(): {"volume": 4.0, "muted": false},
                },
                b.to_string(): {x.to_string(): {"volume": 1.5, "muted": false}},
            })
        );
        assert_eq!(stored.accounts[&a].dm, Some(z));
        // The next save leaves the old field out.
        save(&stored);
        assert!(super::super::paths::flush(std::time::Duration::from_secs(
            5
        )));
        let text = std::fs::read_to_string(config.dir.join(FILE)).unwrap();
        assert!(!text.contains("volumes"), "{text}");
        drop(config);
    }

    /// A sliders' store that does not read is left alone, and so is the old
    /// field: the move is tried again at the next start.
    #[test]
    fn old_volumes_stay_while_the_store_does_not_read() {
        let config = super::super::paths::testing::TempConfig::new("state-old-volumes-kept");
        let (a, x) = (Uuid::from_u128(1), Uuid::from_u128(11));
        let old = serde_json::json!({"accounts": {a.to_string(): {"volumes": {
            x.to_string(): {"volume": 0.5},
        }}}});
        std::fs::write(config.dir.join(FILE), old.to_string()).unwrap();
        let volumes = config.dir.join(crate::screens::voice::volume::FILE);
        std::fs::write(&volumes, "{halb").unwrap();
        let stored = load();
        assert_eq!(std::fs::read_to_string(&volumes).unwrap(), "{halb");
        save(&stored);
        assert!(super::super::paths::flush(std::time::Duration::from_secs(
            5
        )));
        let text = std::fs::read_to_string(config.dir.join(FILE)).unwrap();
        assert!(text.contains("volumes"), "{text}");
        drop(config);
    }
}
