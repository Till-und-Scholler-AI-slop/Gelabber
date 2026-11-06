//! `desktop.json`, shared with the Tauri app (desktop/app/src/main.rs
//! `Settings`): the chosen server and close-to-tray. Keys this app does not
//! know stay as they are.

use serde_json::{Map, Value};

const FILE: &str = "desktop.json";

fn load() -> Map<String, Value> {
    match super::paths::read_json(FILE) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn change(edit: impl FnOnce(&mut Map<String, Value>)) -> Result<(), String> {
    let mut map = load();
    edit(&mut map);
    super::paths::write_json(FILE, &Value::Object(map), false).map_err(|e| e.to_string())
}

/// The remembered server, as the setup screen stored it.
pub fn server() -> Option<String> {
    load()
        .get("server")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub fn set_server(origin: &str) -> Result<(), String> {
    change(|m| {
        m.insert("server".into(), Value::String(origin.to_owned()));
    })
}

/// Closing the window hides it in the tray; off by default.
pub fn close_to_tray() -> bool {
    load()
        .get("close_to_tray")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn set_close_to_tray(enabled: bool) -> Result<(), String> {
    change(|m| {
        m.insert("close_to_tray".into(), Value::Bool(enabled));
    })
}

/// The Tauri app's choice, first match wins: `--server <url>` (or
/// `--server=<url>`), `GELABBER_SERVER`, then `desktop.json`.
pub fn configured_server(
    args: impl IntoIterator<Item = String>,
    env: Option<String>,
) -> Option<String> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--server" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--server=") {
            return Some(value.to_owned());
        }
    }
    env.filter(|v| !v.is_empty()).or_else(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::paths::testing::TempConfig;

    #[test]
    fn the_tauri_apps_file_keeps_its_other_keys() {
        let config = TempConfig::new("settings");
        std::fs::write(
            config.dir.join(FILE),
            r#"{"server":"https://old.example","close_to_tray":true,"future":1}"#,
        )
        .unwrap();
        assert_eq!(server().as_deref(), Some("https://old.example"));
        assert!(close_to_tray());
        set_server("https://chat.example").unwrap();
        set_close_to_tray(false).unwrap();
        let back: Value =
            serde_json::from_slice(&std::fs::read(config.dir.join(FILE)).unwrap()).unwrap();
        assert_eq!(back["server"], "https://chat.example");
        assert_eq!(back["close_to_tray"], false);
        assert_eq!(back["future"], 1);
    }

    #[test]
    fn argument_then_environment_then_file() {
        let _config = TempConfig::new("precedence");
        set_server("https://file.example").unwrap();
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let env = Some("https://env.example".to_owned());
        assert_eq!(
            configured_server(args(&["--server", "https://arg.example"]), env.clone()).as_deref(),
            Some("https://arg.example")
        );
        assert_eq!(
            configured_server(args(&["--server=https://eq.example"]), env.clone()).as_deref(),
            Some("https://eq.example")
        );
        assert_eq!(
            configured_server(args(&[]), env).as_deref(),
            Some("https://env.example")
        );
        assert_eq!(
            configured_server(args(&[]), Some(String::new())).as_deref(),
            Some("https://file.example")
        );
    }
}
