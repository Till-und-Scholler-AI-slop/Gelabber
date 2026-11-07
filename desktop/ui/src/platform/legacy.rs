//! The first start after the Tauri app: what it kept comes over once, while
//! this app has no device settings of its own (`settings.json` of the config
//! directory). Three sources, each optional:
//!
//! - `legacy-settings.json` in the config directory, which the Tauri app's
//!   `settings_export` writes (docs/v0.9-plan.md, M1): the web client's
//!   localStorage entries as `{"version": 1, "localStorage": {key: value}}`,
//!   values as the page stored them (strings) or already parsed. Without a
//!   readable one (0.8.9 and older), the same entries straight from the
//!   WebView's storage ([`super::webview`]). Taken ([`KEYS`], [`PREFIXES`]):
//!   `gelabber.media` (voice/settings.ts, zustand's `{"state": …}`),
//!   `gelabber.media.source-audio-share`, `gelabber.interface`
//!   (interface/preferences.ts), `gelabber.user-volumes:<account>`
//!   (voice/userVolume.ts) and `gelabber:chat-draft:<account>:<channel>`
//!   (messages/drafts.ts, a week at most). Themes come back with the account.
//! - `<config base>/gelabber/settings.json` and `…/gelabber/server` of the
//!   native builds before 0.9 (the same device settings, the server's origin).
//!
//! Nothing is overwritten: device settings only into a missing
//! `settings.json`, the server only where `desktop.json` has none, volumes and
//! drafts only for accounts without them. The sources stay where they are.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use uuid::Uuid;

use super::paths::write_atomic;
use super::state::{DRAFT_MAX_AGE_MS, Draft, Stored, UserVolume};
use super::webview::{self, Found};

pub const LEGACY_FILE: &str = "legacy-settings.json";
const SETTINGS: &str = "settings.json";
const DESKTOP: &str = "desktop.json";
const STATE: &str = "native-state.json";

/// The web client's localStorage keys taken whole, and the prefixes of
/// those taken per account.
pub const KEYS: [&str; 3] = [
    "gelabber.media",
    "gelabber.media.source-audio-share",
    "gelabber.interface",
];
pub const PREFIXES: [&str; 2] = ["gelabber.user-volumes:", "gelabber:chat-draft:"];

/// Whether the import takes the localStorage entry `key`.
pub fn wanted(key: &str) -> bool {
    KEYS.contains(&key) || PREFIXES.iter().any(|p| key.starts_with(p))
}

/// What one import took over, for the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub settings: usize,
    pub server: bool,
    pub volumes: usize,
    pub drafts: usize,
    /// Where the web client's entries came from: the export or a WebView.
    pub from: Option<&'static str>,
}

impl fmt::Display for Imported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} device settings, {} server, {} volumes, {} drafts",
            self.settings,
            if self.server { "the" } else { "no" },
            self.volumes,
            self.drafts
        )?;
        match self.from {
            Some(from) => write!(f, " from {from}"),
            None => Ok(()),
        }
    }
}

/// The import for the app's own directories; `None` when there was nothing
/// to take or the app has its settings already.
pub fn import() -> Result<Option<Imported>, String> {
    let Some(config) = super::paths::config_dir() else {
        return Ok(None);
    };
    // Tests and parity runs choose their directory; the user's stays out.
    let replaced =
        cfg!(test) || std::env::var_os("GELABBER_CONFIG_DIR").is_some_and(|d| !d.is_empty());
    let (old, webview) = if replaced {
        (None, None)
    } else {
        (
            super::paths::base_dir().as_deref().map(old_dir),
            super::paths::local_data_base().map(|b| b.join(super::paths::IDENTIFIER)),
        )
    };
    import_into(&config, old.as_deref(), webview.as_deref(), now_ms())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json(path: &Path, value: &impl serde::Serialize, private: bool) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    write_atomic(path, &bytes, private).map_err(|e| format!("{}: {e}", path.display()))
}

/// localStorage holds strings; an export may also hand the parsed value.
fn stored(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => serde_json::from_str(text).ok(),
        other => Some(other.clone()),
    }
}

/// The web's localStorage entries of `legacy-settings.json`.
fn entries(legacy: &Value) -> Map<String, Value> {
    match legacy.get("localStorage").unwrap_or(legacy) {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    }
}

/// Device settings with the names settings.json uses (screens/settings/prefs.rs
/// takes the web's), from the media and interface entries.
fn device_settings(entries: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let media = entries.get("gelabber.media").and_then(stored);
    // zustand's persist middleware wraps the settings in `state`.
    let media = media.as_ref().map(|m| m.get("state").unwrap_or(m));
    if let Some(Value::Object(media)) = media {
        out.extend(media.clone());
    }
    let valid_share = |v: &Value| matches!(v.as_str(), Some("auto" | "on" | "off"));
    if !out.get("sourceAudioShare").is_some_and(valid_share)
        && let Some(share) = entries
            .get("gelabber.media.source-audio-share")
            .filter(|v| valid_share(v))
    {
        out.insert("sourceAudioShare".into(), share.clone());
    }
    if let Some(Value::Object(interface)) = entries.get("gelabber.interface").and_then(stored) {
        for key in ["uiScale", "compactRooms", "reducedMotion"] {
            if let Some(value) = interface.get(key) {
                out.insert(key.into(), value.clone());
            }
        }
    }
    out
}

/// What an account brings: per-person volumes and drafts by channel.
type Taken = (HashMap<Uuid, UserVolume>, HashMap<Uuid, Draft>);

/// Volumes and fresh drafts per account.
fn account_data(entries: &Map<String, Value>, now: i64) -> HashMap<Uuid, Taken> {
    let mut out: HashMap<Uuid, Taken> = HashMap::new();
    for (key, value) in entries {
        if let Some(account) = key.strip_prefix("gelabber.user-volumes:") {
            let (Ok(account), Some(Value::Object(users))) =
                (Uuid::parse_str(account), stored(value))
            else {
                continue;
            };
            for (user, volume) in users {
                let (Ok(user), Ok(volume)) = (
                    Uuid::parse_str(&user),
                    serde_json::from_value::<UserVolume>(volume),
                ) else {
                    continue;
                };
                if volume.volume.is_finite() {
                    out.entry(account).or_default().0.insert(user, volume);
                }
            }
        } else if let Some(rest) = key.strip_prefix("gelabber:chat-draft:") {
            // Both ids went through encodeURIComponent, which leaves a UUID as it is.
            let Some((account, channel)) = rest.split_once(':') else {
                continue;
            };
            let (Ok(account), Ok(channel), Some(draft)) = (
                Uuid::parse_str(account),
                Uuid::parse_str(channel),
                stored(value).and_then(|v| serde_json::from_value::<Draft>(v).ok()),
            ) else {
                continue;
            };
            if !draft.text.is_empty() && now - draft.at < DRAFT_MAX_AGE_MS {
                out.entry(account).or_default().1.insert(channel, draft);
            }
        }
    }
    out
}

/// The import into `config`, with `old` the directory of native builds
/// before 0.9 and `webview` the Tauri app's data directory; `now` in
/// milliseconds since 1970 (drafts expire).
pub fn import_into(
    config: &Path,
    old: Option<&Path>,
    webview: Option<&Path>,
    now: i64,
) -> Result<Option<Imported>, String> {
    import_from(
        config,
        old,
        |server| match webview {
            Some(dir) => webview::local_storage(dir, server),
            None => Ok(Found::Absent),
        },
        now,
    )
}

/// The server the Tauri app's `desktop.json` names, whose origin the
/// WebView's storage is read for.
fn desktop_server(config: &Path) -> Option<String> {
    let desktop = read_json(&config.join(DESKTOP))?;
    desktop.get("server")?.as_str().map(str::to_owned)
}

fn import_from(
    config: &Path,
    old: Option<&Path>,
    webview: impl FnOnce(Option<&str>) -> Result<Found, String>,
    now: i64,
) -> Result<Option<Imported>, String> {
    let settings_path = config.join(SETTINGS);
    if settings_path.exists() {
        return Ok(None);
    }
    // The export is a filtered snapshot of the same storage, so it wins
    // whole; an Err here returns before anything is written.
    let (legacy, from) = match read_json(&config.join(LEGACY_FILE)) {
        Some(legacy) => (Some(legacy), Some(LEGACY_FILE)),
        None => match webview(desktop_server(config).as_deref())? {
            Found::Entries(entries, source) => (
                Some(serde_json::json!({ "localStorage": entries })),
                Some(source),
            ),
            Found::Absent => (None, None),
        },
    };
    let old_settings = old.and_then(|dir| read_json(&dir.join(SETTINGS)));
    let old_server = old
        .and_then(|dir| std::fs::read_to_string(dir.join("server")).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if legacy.is_none() && old_settings.is_none() && old_server.is_none() {
        return Ok(None);
    }
    let mut imported = Imported {
        from,
        ..Imported::default()
    };

    let entries = legacy.as_ref().map(entries).unwrap_or_default();
    // The Tauri app's export is the newer one: it goes over the older build's.
    let mut device = match old_settings {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    device.extend(device_settings(&entries));
    imported.settings = device.len();
    // Written even when empty: settings.json marks the import as done.
    write_json(&settings_path, &Value::Object(device), false)?;

    if let Some(server) = old_server {
        let desktop_path = config.join(DESKTOP);
        // The Tauri app's file too: one that does not read as an object stays.
        let desktop = match read_json(&desktop_path) {
            Some(Value::Object(map)) => Some(map),
            None if !desktop_path.exists() => Some(Map::new()),
            _ => None,
        };
        let has_server = |desktop: &Map<String, Value>| {
            desktop
                .get("server")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        };
        if let Some(mut desktop) = desktop.filter(|d| !has_server(d)) {
            desktop.insert("server".into(), Value::String(server));
            write_json(&desktop_path, &Value::Object(desktop), false)?;
            imported.server = true;
        }
    }

    let accounts = account_data(&entries, now);
    if !accounts.is_empty() {
        let state_path = config.join(STATE);
        // A state file this build cannot read is left alone, not replaced.
        let state: Option<Stored> = match read_json(&state_path) {
            Some(value) => serde_json::from_value(value).ok(),
            None if !state_path.exists() => Some(Stored::default()),
            None => None,
        };
        if let Some(mut state) = state {
            for (id, (volumes, drafts)) in accounts {
                let account = state.accounts.entry(id).or_default();
                if account.volumes.is_empty() {
                    imported.volumes += volumes.len();
                    account.volumes = volumes;
                }
                if account.drafts.is_empty() {
                    imported.drafts += drafts.len();
                    account.drafts = drafts;
                }
            }
            write_json(&state_path, &state, true)?;
        }
    }
    Ok(Some(imported))
}

/// `<config base>/gelabber`, where native builds before 0.9 kept their files.
pub fn old_dir(base: &Path) -> PathBuf {
    base.join("gelabber")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The Tauri app's export holds the web client's localStorage: every
    /// key taken here is one the web client stores under that name.
    #[test]
    fn the_keys_taken_here_are_the_web_clients() {
        let this = include_str!("legacy.rs");
        for (key, web) in [
            (
                "\"gelabber.media\"",
                include_str!("../../../../web/src/voice/settings.ts"),
            ),
            (
                "\"gelabber.media.source-audio-share\"",
                include_str!("../../../../web/src/voice/settings.ts"),
            ),
            (
                "\"gelabber.interface\"",
                include_str!("../../../../web/src/interface/preferences.ts"),
            ),
            (
                "\"gelabber.user-volumes:\"",
                include_str!("../../../../web/src/voice/userVolume.ts"),
            ),
            (
                "\"gelabber:chat-draft:\"",
                include_str!("../../../../web/src/messages/drafts.ts"),
            ),
        ] {
            assert!(this.contains(key), "{key} is not taken here");
            assert!(
                web.contains(&format!("= {key};")),
                "{key} is not the web client's"
            );
        }
        assert_eq!(LEGACY_FILE, "legacy-settings.json");
    }

    const ACCOUNT: &str = "0f9a4c1e-8f1b-4f3e-9d55-2b7a1c3e5d01";
    const FRIEND: &str = "6b0e2a77-3c44-4d0a-a1c2-9e8f7d6c5b4a";
    const CHANNEL: &str = "c2d3e4f5-a6b7-4c8d-9e0f-1a2b3c4d5e6f";
    const NOW: i64 = 1_800_000_000_000;

    struct Dirs {
        root: PathBuf,
        config: PathBuf,
        old: PathBuf,
    }

    impl Dirs {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "gelabber-legacy-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            let config = root.join("io.github.till-und-scholler-ai-slop.gelabber");
            let old = old_dir(&root);
            std::fs::create_dir_all(&config).unwrap();
            Self { root, config, old }
        }

        fn json(&self, name: &str) -> Value {
            read_json(&self.config.join(name)).unwrap_or(Value::Null)
        }

        /// The Tauri app's data directory (`<local data>/<identifier>`).
        fn local(&self) -> PathBuf {
            self.root.join("local")
        }

        fn server(&self, server: &str) {
            write_json(
                &self.config.join(DESKTOP),
                &json!({ "server": server }),
                false,
            )
            .unwrap();
        }
    }

    /// The WebView stores of the fixtures (tests/fixtures/webview): WebView2's
    /// format everywhere, WebKitGTK's on Linux, where its reader is built.
    #[derive(Clone, Copy, Debug)]
    enum Store {
        Chromium,
        #[cfg(target_os = "linux")]
        WebKit,
    }

    const STORES: &[Store] = &[
        Store::Chromium,
        #[cfg(target_os = "linux")]
        Store::WebKit,
    ];

    impl Store {
        /// Where the store goes in the Tauri app's data directory.
        fn dir(self, dirs: &Dirs) -> PathBuf {
            match self {
                Store::Chromium => dirs.local().join("EBWebView/Default/Local Storage/leveldb"),
                #[cfg(target_os = "linux")]
                Store::WebKit => dirs.local().join("localstorage"),
            }
        }

        fn fill(self, dirs: &Dirs) -> PathBuf {
            let name = match self {
                Store::Chromium => "chromium",
                #[cfg(target_os = "linux")]
                Store::WebKit => "webkitgtk",
            };
            let from = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/webview")
                .join(name);
            let to = self.dir(dirs);
            std::fs::create_dir_all(&to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                if entry.file_name() != ".gitignore" {
                    std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
                }
            }
            to
        }

        fn source(self) -> &'static str {
            match self {
                Store::Chromium => "WebView2",
                #[cfg(target_os = "linux")]
                Store::WebKit => "WebKitGTK",
            }
        }

        /// The import with this store as the WebView's, as the platform's
        /// `local_storage` would read it.
        fn import(self, dirs: &Dirs) -> Result<Option<Imported>, String> {
            let local = dirs.local();
            import_from(
                &dirs.config,
                Some(&dirs.old),
                |server| match self {
                    Store::Chromium => webview::chromium(&local, server),
                    #[cfg(target_os = "linux")]
                    Store::WebKit => webview::webkitgtk(&local, server),
                },
                NOW,
            )
        }
    }

    /// What the export of [`export`] and the stores of the fixture page
    /// (port 8123) give, `draft` the text of the fresh draft.
    fn assert_taken(dirs: &Dirs, imported: &Imported, draft: &str) {
        assert_eq!(imported.volumes, 1);
        assert_eq!(imported.drafts, 1);
        assert!(!imported.server);

        let settings = dirs.json(SETTINGS);
        assert_eq!(settings["inputGain"], 1.5);
        assert_eq!(settings["voiceInput"], "ptt");
        assert_eq!(settings["pttKey"], "F13");
        assert_eq!(settings["sourceAudioShare"], "off");
        assert_eq!(settings["uiScale"], "large");
        assert_eq!(settings["compactRooms"], true);
        assert!(settings.get("theme").is_none());

        let state: Stored = serde_json::from_value(dirs.json(STATE)).unwrap();
        let account = &state.accounts[&Uuid::parse_str(ACCOUNT).unwrap()];
        let friend = Uuid::parse_str(FRIEND).unwrap();
        assert_eq!(
            account.volumes[&friend],
            UserVolume {
                volume: 0.5,
                muted: true
            }
        );
        assert_eq!(
            account.drafts[&Uuid::parse_str(CHANNEL).unwrap()].text,
            draft
        );
        assert!(!account.drafts.contains_key(&friend));
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn export() -> Value {
        // As the page's localStorage holds them: strings of JSON.
        let media = json!({"state": {"inputGain": 1.5, "voiceInput": "ptt", "pttKey": "F13",
            "shareSourceAudio": false}, "version": 3});
        let volumes = json!({FRIEND: {"volume": 0.5, "muted": true}, "kaputt": {"volume": 1}});
        let draft = json!({"text": "bis gleich", "at": NOW - 60_000});
        let old_draft = json!({"text": "vor Wochen", "at": NOW - DRAFT_MAX_AGE_MS - 1});
        json!({"version": 1, "localStorage": {
            "gelabber.media": media.to_string(),
            "gelabber.media.source-audio-share": "off",
            "gelabber.interface": json!({"uiScale": "large", "compactRooms": true}).to_string(),
            format!("gelabber.user-volumes:{ACCOUNT}"): volumes.to_string(),
            format!("gelabber:chat-draft:{ACCOUNT}:{CHANNEL}"): draft.to_string(),
            format!("gelabber:chat-draft:{ACCOUNT}:{FRIEND}"): old_draft.to_string(),
            "gelabber.theme.paint": "{}",
        }})
    }

    #[test]
    fn the_tauri_apps_export_becomes_settings_volumes_and_drafts() {
        let dirs = Dirs::new("export");
        write_json(&dirs.config.join(LEGACY_FILE), &export(), false).unwrap();
        let imported = import_into(&dirs.config, Some(&dirs.old), None, NOW)
            .unwrap()
            .unwrap();
        assert_taken(&dirs, &imported, "bis gleich");
        assert_eq!(imported.from, Some(LEGACY_FILE));
        // The source stays for the Tauri app.
        assert!(dirs.config.join(LEGACY_FILE).exists());
    }

    #[test]
    fn the_webview_store_becomes_settings_volumes_and_drafts() {
        for &store in STORES {
            let dirs = Dirs::new("webview");
            store.fill(&dirs);
            dirs.server("http://127.0.0.1:8123/");
            let imported = store.import(&dirs).unwrap().unwrap();
            assert_taken(&dirs, &imported, "bis gleich 👋");
            assert_eq!(imported.from, Some(store.source()), "{store:?}");
            assert_eq!(
                imported.to_string(),
                format!(
                    "7 device settings, no server, 1 volumes, 1 drafts from {}",
                    store.source()
                )
            );
            // Nothing in the store that the import does not take.
            let settings = dirs.json(SETTINGS);
            assert!(settings.get("greeting").is_none(), "{store:?}");
        }
    }

    #[test]
    fn the_export_goes_before_the_webview() {
        for &store in STORES {
            let dirs = Dirs::new("export-first");
            store.fill(&dirs);
            // 8124's store has inputGain 0.75, the export 1.5.
            dirs.server("http://127.0.0.1:8124");
            write_json(&dirs.config.join(LEGACY_FILE), &export(), false).unwrap();
            let imported = store.import(&dirs).unwrap().unwrap();
            assert_taken(&dirs, &imported, "bis gleich");
            assert_eq!(imported.from, Some(LEGACY_FILE));
        }
    }

    #[test]
    fn an_unreadable_export_falls_back_to_the_webview() {
        for &store in STORES {
            let dirs = Dirs::new("export-damaged");
            store.fill(&dirs);
            dirs.server("http://127.0.0.1:8123");
            std::fs::write(dirs.config.join(LEGACY_FILE), "{halb").unwrap();
            let imported = store.import(&dirs).unwrap().unwrap();
            assert_taken(&dirs, &imported, "bis gleich 👋");
            assert_eq!(imported.from, Some(store.source()));
        }
    }

    #[test]
    fn only_the_configured_servers_origin() {
        for &store in STORES {
            let dirs = Dirs::new("origin");
            store.fill(&dirs);
            dirs.server("http://127.0.0.1:8124");
            store.import(&dirs).unwrap().unwrap();
            let settings = dirs.json(SETTINGS);
            assert_eq!(settings["inputGain"], 0.75, "{store:?}");
            assert_eq!(settings["uiScale"], "small");
            assert_eq!(settings["sourceAudioShare"], "on");

            // Two servers in the store and none configured: nothing.
            let dirs = Dirs::new("origin-none");
            store.fill(&dirs);
            assert_eq!(store.import(&dirs).unwrap(), None, "{store:?}");
            assert!(!dirs.config.join(SETTINGS).exists());
            // Neither of them the configured one: nothing either.
            dirs.server("https://chat.example.org");
            assert_eq!(store.import(&dirs).unwrap(), None, "{store:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_only_server_in_the_store_is_taken_without_one_configured() {
        let dirs = Dirs::new("origin-one");
        let store = Store::WebKit.fill(&dirs);
        for wal in ["", "-wal"] {
            std::fs::remove_file(store.join(format!("http_127.0.0.1_8124.localstorage{wal}")))
                .unwrap();
        }
        let imported = Store::WebKit.import(&dirs).unwrap().unwrap();
        assert_taken(&dirs, &imported, "bis gleich 👋");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_damaged_database_of_another_server_leaves_the_servers() {
        let dirs = Dirs::new("origin-damaged");
        let store = Store::WebKit.fill(&dirs);
        std::fs::write(store.join("http_127.0.0.1_8124.localstorage"), [7u8; 8192]).unwrap();
        std::fs::write(store.join("https_old.example_0.localstorage"), b"").unwrap();
        dirs.server("http://127.0.0.1:8123");
        let imported = Store::WebKit.import(&dirs).unwrap().unwrap();
        assert_taken(&dirs, &imported, "bis gleich 👋");
    }

    #[cfg(unix)]
    #[test]
    fn a_store_that_cannot_be_read_now_writes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        for &store in STORES {
            let dirs = Dirs::new("locked");
            let dir = store.fill(&dirs);
            dirs.server("http://127.0.0.1:8123");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
            let locked = std::fs::read_dir(&dir).is_err();
            let first = store.import(&dirs);
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            if !locked {
                eprintln!("{store:?}: mode 000 does not lock this user (root) out; skipped");
                continue;
            }
            assert!(first.is_err(), "{store:?}: {first:?}");
            assert!(!dirs.config.join(SETTINGS).exists());
            assert!(!dirs.config.join(STATE).exists());
            // The next start takes it.
            let imported = store.import(&dirs).unwrap().unwrap();
            assert_taken(&dirs, &imported, "bis gleich 👋");
        }
    }

    #[test]
    fn only_once_and_never_over_the_apps_own_settings() {
        let dirs = Dirs::new("once");
        write_json(&dirs.config.join(LEGACY_FILE), &export(), false).unwrap();
        assert!(
            import_into(&dirs.config, None, None, NOW)
                .unwrap()
                .is_some()
        );
        // A setting changed in this app afterwards stays.
        write_json(
            &dirs.config.join(SETTINGS),
            &json!({"inputGain": 0.7}),
            false,
        )
        .unwrap();
        assert_eq!(import_into(&dirs.config, None, None, NOW).unwrap(), None);
        assert_eq!(dirs.json(SETTINGS)["inputGain"], 0.7);
    }

    #[test]
    fn the_old_native_files_give_settings_and_the_server() {
        let dirs = Dirs::new("old");
        std::fs::create_dir_all(&dirs.old).unwrap();
        std::fs::write(dirs.old.join("server"), "https://chat.example.org\n").unwrap();
        std::fs::write(
            dirs.old.join(SETTINGS),
            r#"{"outputVolume":2.0,"callSounds":false}"#,
        )
        .unwrap();
        write_json(
            &dirs.config.join(DESKTOP),
            &json!({"close_to_tray": true}),
            false,
        )
        .unwrap();
        let imported = import_into(&dirs.config, Some(&dirs.old), None, NOW)
            .unwrap()
            .unwrap();
        assert_eq!(
            imported,
            Imported {
                settings: 2,
                server: true,
                volumes: 0,
                drafts: 0,
                from: None
            }
        );
        assert_eq!(dirs.json(SETTINGS)["outputVolume"], 2.0);
        let desktop = dirs.json(DESKTOP);
        assert_eq!(desktop["server"], "https://chat.example.org");
        assert_eq!(desktop["close_to_tray"], true);
        assert!(!dirs.config.join(STATE).exists());
    }

    #[test]
    fn a_chosen_server_and_existing_account_data_stay() {
        let dirs = Dirs::new("keep");
        std::fs::create_dir_all(&dirs.old).unwrap();
        std::fs::write(dirs.old.join("server"), "https://old.example").unwrap();
        write_json(&dirs.config.join(LEGACY_FILE), &export(), false).unwrap();
        write_json(
            &dirs.config.join(DESKTOP),
            &json!({"server": "https://chosen.example"}),
            false,
        )
        .unwrap();
        let mut state = Stored::default();
        let account = Uuid::parse_str(ACCOUNT).unwrap();
        let other = Uuid::new_v4();
        state.accounts.entry(account).or_default().volumes.insert(
            other,
            UserVolume {
                volume: 2.0,
                muted: false,
            },
        );
        write_json(&dirs.config.join(STATE), &state, true).unwrap();
        let imported = import_into(&dirs.config, Some(&dirs.old), None, NOW)
            .unwrap()
            .unwrap();
        assert!(!imported.server);
        assert_eq!(imported.volumes, 0);
        assert_eq!(imported.drafts, 1);
        assert_eq!(dirs.json(DESKTOP)["server"], "https://chosen.example");
        let back: Stored = serde_json::from_value(dirs.json(STATE)).unwrap();
        assert_eq!(back.accounts[&account].volumes.len(), 1);
        assert!(back.accounts[&account].volumes.contains_key(&other));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dirs.config.join(STATE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn files_that_do_not_read_stay_as_they_are() {
        let dirs = Dirs::new("damaged");
        std::fs::create_dir_all(&dirs.old).unwrap();
        std::fs::write(dirs.old.join("server"), "https://old.example").unwrap();
        write_json(&dirs.config.join(LEGACY_FILE), &export(), false).unwrap();
        std::fs::write(dirs.config.join(DESKTOP), "{halb").unwrap();
        std::fs::write(dirs.config.join(STATE), r#"{"accounts": 3}"#).unwrap();
        let imported = import_into(&dirs.config, Some(&dirs.old), None, NOW)
            .unwrap()
            .unwrap();
        assert!(!imported.server);
        assert_eq!((imported.volumes, imported.drafts), (0, 0));
        assert_eq!(
            std::fs::read_to_string(dirs.config.join(DESKTOP)).unwrap(),
            "{halb"
        );
        assert_eq!(
            std::fs::read_to_string(dirs.config.join(STATE)).unwrap(),
            r#"{"accounts": 3}"#
        );
        assert!(dirs.config.join(SETTINGS).exists());
    }

    #[test]
    fn nothing_to_take_writes_nothing() {
        let dirs = Dirs::new("none");
        assert_eq!(
            import_into(&dirs.config, Some(&dirs.old), None, NOW).unwrap(),
            None
        );
        assert!(!dirs.config.join(SETTINGS).exists());
        // A damaged export counts as none.
        std::fs::write(dirs.config.join(LEGACY_FILE), "{kaputt").unwrap();
        assert_eq!(import_into(&dirs.config, None, None, NOW).unwrap(), None);
        // A flat object of entries, without the wrapper, is read too.
        write_json(
            &dirs.config.join(LEGACY_FILE),
            &json!({"gelabber.interface": {"reducedMotion": true}}),
            false,
        )
        .unwrap();
        let imported = import_into(&dirs.config, None, None, NOW).unwrap().unwrap();
        assert_eq!(imported.settings, 1);
        assert_eq!(dirs.json(SETTINGS)["reducedMotion"], true);
    }

    #[test]
    fn the_apps_own_import_stays_out_of_the_users_directories() {
        let config = crate::platform::paths::testing::TempConfig::new("legacy");
        write_json(&config.dir.join(LEGACY_FILE), &export(), false).unwrap();
        let imported = import().unwrap().unwrap();
        assert_eq!(imported.volumes, 1);
        assert!(config.dir.join(SETTINGS).exists());
    }
}
