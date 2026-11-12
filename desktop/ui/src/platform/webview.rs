//! The Tauri app's WebView storage, for installs that left no
//! `legacy-settings.json` (0.8.9 and older, or 0.8.10 never started): the
//! web client's localStorage read where the WebView keeps it, in the app's
//! data directory (`<local data>/<identifier>`, Tauri's `app_local_data_dir`).
//!
//! - Windows, WebView2: `EBWebView/<profile>/Local Storage/leveldb` (LevelDB,
//!   [`leveldb`]).
//! - Linux, WebKitGTK: `localstorage/<origin>.localstorage` (SQLite, [`webkit`]).
//! - macOS: none; Tauri leaves WKWebView's storage where Apple keeps it.
//!
//! Only the server's origin counts: the one `desktop.json` names, or else the
//! only origin that holds wanted entries. Nothing is written to the store.

#[cfg(any(windows, target_os = "linux", test))]
use std::collections::BTreeMap;
use std::path::Path;
#[cfg(any(windows, test))]
use std::path::PathBuf;

use serde_json::{Map, Value};

#[cfg(any(windows, test))]
pub(super) mod leveldb;
#[cfg(target_os = "linux")]
pub(super) mod webkit;

/// localStorage areas: origin to key to value.
#[cfg(any(windows, target_os = "linux", test))]
pub(super) type Areas = BTreeMap<String, BTreeMap<String, String>>;

#[cfg(any(windows, target_os = "linux", test))]
#[derive(Debug)]
pub(super) enum Failure {
    /// May pass by the next start: I/O and sharing errors, a busy database.
    Transient(String),
    /// Will not: a format this build does not know, a damaged file.
    Permanent(String),
}

#[derive(Debug, PartialEq)]
pub enum Found {
    /// The server's wanted entries (localStorage strings) and the WebView
    /// that kept them.
    Entries(Map<String, Value>, &'static str),
    Absent,
}

/// The WebView storage in the Tauri app's data directory `dir`. `Err` for a
/// failure that may pass (the import is tried again on the next start);
/// one that will not is logged and counts as no storage.
pub fn local_storage(dir: &Path, server: Option<&str>) -> Result<Found, String> {
    #[cfg(windows)]
    return chromium(dir, server);
    #[cfg(target_os = "linux")]
    return webkitgtk(dir, server);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (dir, server);
        Ok(Found::Absent)
    }
}

/// WebView2's store in `dir`.
#[cfg(any(windows, test))]
pub(super) fn chromium(dir: &Path, server: Option<&str>) -> Result<Found, String> {
    match chromium_store(&dir.join("EBWebView")) {
        Some(store) => choose(leveldb::areas(&store), server, "WebView2"),
        None => Ok(Found::Absent),
    }
}

/// WebKitGTK's store in `dir`.
#[cfg(target_os = "linux")]
pub(super) fn webkitgtk(dir: &Path, server: Option<&str>) -> Result<Found, String> {
    choose(
        webkit::areas(&dir.join("localstorage")),
        server,
        "WebKitGTK",
    )
}

/// `<profile>/Local Storage/leveldb` of the WebView2 user data folder,
/// `Default` first, then the others sorted: the profile's name is not
/// documented.
#[cfg(any(windows, test))]
fn chromium_store(user_data: &Path) -> Option<PathBuf> {
    let default = user_data.join("Default");
    let mut profiles = vec![default.clone()];
    if let Ok(listing) = std::fs::read_dir(user_data) {
        let mut others: Vec<PathBuf> = listing
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && *p != default)
            .collect();
        others.sort();
        profiles.extend(others);
    }
    let store = |profile: &Path| profile.join("Local Storage").join("leveldb"); // i18n-ok
    if let Some(profile) = profiles.iter().find(|p| store(p).is_dir()) {
        return Some(store(profile));
    }
    // Chromium's SQLite DOM storage (an experiment so far) is not read.
    if let Some(profile) = profiles.iter().find(|p| p.join("LocalStorage").exists()) {
        tracing::warn!(
            "{}: localStorage in SQLite, which this build does not read",
            profile.display()
        );
    }
    None
}

/// The server's area among `areas`, with the keys the import takes.
#[cfg(any(windows, target_os = "linux", test))]
fn choose(
    areas: Result<Areas, Failure>,
    server: Option<&str>,
    source: &'static str,
) -> Result<Found, String> {
    let areas = match areas {
        Ok(areas) => areas,
        Err(Failure::Transient(e)) => return Err(format!("{source} localStorage: {e}")),
        Err(Failure::Permanent(e)) => {
            tracing::warn!("{source} localStorage left out: {e}");
            return Ok(Found::Absent);
        }
    };
    let mut candidates: BTreeMap<String, Map<String, Value>> = areas
        .into_iter()
        .filter(|(origin, _)| origin.starts_with("http://") || origin.starts_with("https://"))
        .map(|(origin, entries)| {
            let wanted = entries
                .into_iter()
                .filter(|(key, _)| super::legacy::wanted(key))
                .map(|(key, value)| (key, Value::String(value)))
                .collect::<Map<_, _>>();
            (origin, wanted)
        })
        .filter(|(_, wanted)| !wanted.is_empty())
        .collect();
    let server = server.and_then(server_origin);
    if let Some(entries) = server.as_ref().and_then(|s| candidates.remove(s)) {
        return Ok(Found::Entries(entries, source));
    }
    // GELABBER_SERVER and --server left no trace in desktop.json.
    if candidates.len() == 1
        && let Some((origin, entries)) = candidates.pop_first()
    {
        tracing::info!("{source} localStorage of {origin}, the only server in it");
        return Ok(Found::Entries(entries, source));
    }
    if !candidates.is_empty() {
        let origins: Vec<_> = candidates.keys().map(String::as_str).collect();
        tracing::warn!(
            "{source} localStorage of {}, none of them {}: nothing taken",
            origins.join(", "),
            server.as_deref().unwrap_or("a configured server")
        );
    }
    Ok(Found::Absent)
}

/// The origin the Tauri app's page ran on, as 0.8 stored it
/// (`origin().ascii_serialization()`, http or https).
#[cfg(any(windows, target_os = "linux", test))]
fn server_origin(server: &str) -> Option<String> {
    let url = url::Url::parse(server.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn areas(origins: &[(&str, &[(&str, &str)])]) -> Areas {
        origins
            .iter()
            .map(|(origin, entries)| {
                let entries = entries.iter().map(|(k, v)| (k.to_string(), v.to_string()));
                (origin.to_string(), entries.collect())
            })
            .collect()
    }

    fn keys(found: Found) -> Vec<String> {
        match found {
            Found::Entries(entries, _) => entries.keys().cloned().collect(),
            Found::Absent => Vec::new(),
        }
    }

    #[test]
    fn the_server_is_matched_by_its_origin() {
        let store = areas(&[
            ("https://chat.example.org", &[("gelabber.interface", "{}")]),
            ("https://other.example", &[("gelabber.media", "{}")]),
        ]);
        // Path, default port and case do not change the origin.
        let found = choose(Ok(store), Some("HTTPS://Chat.Example.org:443/raum"), "test").unwrap();
        assert_eq!(keys(found), ["gelabber.interface"]);
    }

    #[test]
    fn other_keys_and_other_schemes_stay_out() {
        let store = areas(&[
            ("tauri://localhost", &[("gelabber.interface", "{}")]),
            (
                "http://127.0.0.1:8123",
                &[
                    ("gelabber.theme.paint", "{}"),
                    ("gelabber.media", "{}"),
                    ("session", "x"),
                ],
            ),
        ]);
        // The only http(s) origin with wanted keys, without a server.
        assert_eq!(
            keys(choose(Ok(store), None, "test").unwrap()),
            ["gelabber.media"]
        );
        let only_unwanted = areas(&[("http://a.example", &[("gelabber.theme.paint", "{}")])]);
        assert_eq!(
            choose(Ok(only_unwanted), None, "test").unwrap(),
            Found::Absent
        );
    }

    #[test]
    fn a_failure_that_passes_is_an_error_one_that_stays_is_no_store() {
        let transient = Failure::Transient("busy".into());
        assert!(choose(Err(transient), None, "test").is_err());
        let permanent = Failure::Permanent("schema 2".into());
        assert_eq!(choose(Err(permanent), None, "test").unwrap(), Found::Absent);
    }

    #[test]
    fn webview2_profiles_default_first() {
        let root =
            std::env::temp_dir().join(format!("gelabber-webview-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = |profile: &str| root.join(profile).join("Local Storage").join("leveldb");
        // SQLite DOM storage alone is not read.
        std::fs::create_dir_all(root.join("Default")).unwrap();
        std::fs::write(root.join("Default").join("LocalStorage"), b"").unwrap();
        assert_eq!(chromium_store(&root), None);
        std::fs::create_dir_all(store("Profile 1")).unwrap();
        assert_eq!(chromium_store(&root), Some(store("Profile 1")));
        std::fs::create_dir_all(store("Default")).unwrap();
        assert_eq!(chromium_store(&root), Some(store("Default")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
