//! The web client's local settings for the native app (docs/v0.9-plan.md
//! §9.7): [`EXPORT`] hands the server page's localStorage entries that the
//! native app takes over (desktop/ui/src/platform/legacy.rs) to
//! [`settings_export`], which keeps them in `legacy-settings.json` of the
//! config directory. The native app's first start reads them from there.

use std::collections::BTreeMap;
use std::fs;

use tauri::{AppHandle, Manager};

/// The entries legacy.rs takes: voice settings, Live sound, interface.
pub const KEYS: &[&str] = &[
    "gelabber.media",
    "gelabber.media.source-audio-share",
    "gelabber.interface",
];
/// Per account (volumes) and per account and channel (drafts).
pub const PREFIXES: &[&str] = &["gelabber.user-volumes:", "gelabber:chat-draft:"];
/// More than any real set of drafts; a page cannot fill the disk.
const MAX_BYTES: usize = 2 * 1024 * 1024;
const FILE: &str = "legacy-settings.json";

/// On the server's pages: the entries on load, when the window is hidden
/// and every minute, only when they changed. Errors stay quiet; the setup
/// page has no grant and gets refused.
pub const EXPORT: &str = r#"(() => {
  if (window.top !== window || !window.__TAURI_INTERNALS__) return;
  const keys = ["gelabber.media", "gelabber.media.source-audio-share", "gelabber.interface"];
  const prefixes = ["gelabber.user-volumes:", "gelabber:chat-draft:"];
  let last = null;
  const send = () => {
    const entries = {};
    try {
      const names = [];
      for (let i = 0; i < localStorage.length; i++) names.push(localStorage.key(i));
      for (const key of names.sort()) {
        if (key && (keys.includes(key) || prefixes.some((p) => key.startsWith(p)))) {
          entries[key] = localStorage.getItem(key);
        }
      }
    } catch {
      return;
    }
    const text = JSON.stringify(entries);
    if (text === last) return;
    last = text;
    window.__TAURI_INTERNALS__.invoke("settings_export", { entries }).catch(() => {
      last = null;
    });
  };
  addEventListener("load", send, { once: true });
  addEventListener("pagehide", send);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") send();
  });
  setInterval(send, 60_000);
})();
"#;

fn wanted(key: &str) -> bool {
    KEYS.contains(&key) || PREFIXES.iter().any(|prefix| key.starts_with(prefix))
}

/// The file legacy.rs reads: `{"version": 1, "localStorage": {key: value}}`
/// with the wanted entries only.
fn document(entries: BTreeMap<String, String>) -> Result<Vec<u8>, String> {
    let kept: BTreeMap<String, String> = entries.into_iter().filter(|(k, _)| wanted(k)).collect();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "localStorage": kept,
    }))
    .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "{FILE}: {} bytes, more than {MAX_BYTES}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// Keeps the page's entries for the native app, whole or not at all.
#[tauri::command]
pub fn settings_export(app: AppHandle, entries: BTreeMap<String, String>) -> Result<(), String> {
    let bytes = document(entries)?;
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(FILE);
    let partial = path.with_extension("json.partial");
    fs::write(&partial, bytes).map_err(|e| e.to_string())?;
    fs::rename(&partial, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_entries_the_native_app_takes_are_kept() {
        let entries = BTreeMap::from([
            ("gelabber.media".to_owned(), r#"{"state":{}}"#.to_owned()),
            (
                "gelabber.media.source-audio-share".to_owned(),
                "off".to_owned(),
            ),
            ("gelabber.interface".to_owned(), "{}".to_owned()),
            ("gelabber.user-volumes:a".to_owned(), "{}".to_owned()),
            ("gelabber:chat-draft:a:c".to_owned(), "{}".to_owned()),
            ("gelabber.theme.paint".to_owned(), "{}".to_owned()),
            ("session".to_owned(), "secret".to_owned()),
        ]);
        let doc: serde_json::Value = serde_json::from_slice(&document(entries).unwrap()).unwrap();
        assert_eq!(doc["version"], 1);
        let kept = doc["localStorage"].as_object().unwrap();
        assert_eq!(kept.len(), 5);
        assert!(!kept.contains_key("gelabber.theme.paint") && !kept.contains_key("session"));
        assert_eq!(kept["gelabber.media.source-audio-share"], "off");
    }

    #[test]
    fn a_page_cannot_write_more_than_the_cap() {
        let big = BTreeMap::from([("gelabber:chat-draft:a:c".to_owned(), "x".repeat(MAX_BYTES))]);
        assert!(document(big).is_err());
    }

    /// Handled, and granted to the server origin only.
    #[test]
    fn the_export_command_is_handled_and_granted_to_the_server() {
        // media.rs takes commands.rs in for its tests.
        let main = include_str!("main.rs");
        let setup = include_str!("../capabilities/setup.json");
        for command in crate::media::EXPORT_COMMANDS {
            assert!(main.contains(&format!("export::{command},")), "{command}");
            let permission = format!("\"allow-{}\"", command.replace('_', "-"));
            assert!(
                main.contains(&format!(".permission({permission})")),
                "{command}"
            );
            assert!(!setup.contains(&permission), "{command}");
        }
        assert!(main.contains(".initialization_script(export::EXPORT)"));
    }

    /// The script asks for what the command keeps.
    #[test]
    fn the_script_names_the_same_entries() {
        for key in KEYS.iter().chain(PREFIXES) {
            assert!(EXPORT.contains(&format!("\"{key}\"")), "{key}");
        }
    }
}
