//! The sign-in across restarts: the session cookie per server origin in
//! `session.json`, readable by the user only (0600). No OS keyring: the
//! maintained crates for it pull in a second D-Bus stack and crypto on Linux,
//! and the cookie is as strong as the browser's own cookie file.

use gelabber_client::Client;
use serde_json::{Map, Value};

const FILE: &str = "session.json";

fn load() -> Map<String, Value> {
    match super::paths::read_json(FILE) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The remembered cookie of `origin`.
pub fn stored(origin: &str) -> Option<String> {
    load()
        .get(origin)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Stores or (with `None`) forgets the cookie of `origin`.
pub fn store(origin: &str, cookie: Option<&str>) -> Result<(), String> {
    let mut map = load();
    match cookie {
        Some(value) => map.insert(origin.to_owned(), Value::String(value.to_owned())),
        None => map.remove(origin),
    };
    super::paths::write_json(FILE, &Value::Object(map), true).map_err(|e| e.to_string())
}

/// Follows the client's cookie jar and writes when the cookie changes
/// (sign-in, renewal, sign-out).
pub struct Keeper {
    origin: String,
    saved: Option<String>,
}

impl Keeper {
    /// Puts the remembered cookie into a new client before its first request.
    pub fn restore(client: &Client) -> Self {
        let origin = client.http().base().origin().ascii_serialization();
        let saved = stored(&origin);
        if let Some(cookie) = &saved {
            client.http().restore_session_cookie(cookie);
        }
        Self { origin, saved }
    }

    pub fn tick(&mut self, client: &Client) {
        let now = client.http().session_cookie();
        if now != self.saved {
            match store(&self.origin, now.as_deref()) {
                Ok(()) => self.saved = now,
                Err(e) => {
                    tracing::warn!("session: {e}");
                    // Not again every frame.
                    self.saved = now;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::paths::testing::TempConfig;

    /// Against the seeded stack (web/scripts/parity), ignored by default:
    /// `GELABBER_TEST_SERVER=http://127.0.0.1:5180 cargo test -p gelabber-ui
    /// signed_in_after_a_restart -- --ignored`. Signs out again at the end.
    #[test]
    #[ignore]
    fn signed_in_after_a_restart() {
        let Ok(server) = std::env::var("GELABBER_TEST_SERVER") else {
            return;
        };
        // Read at run time: the seed writes it, git ignores it.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../web/scripts/parity/fixture.json"
        );
        let fixture: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let email = fixture["users"]["jonas"]["email"].as_str().unwrap();
        let password = fixture["password"].as_str().unwrap();
        let _config = TempConfig::new("live-session");
        let first = Client::new(&server).unwrap();
        let mut keeper = Keeper::restore(&first);
        first.login_blocking(email, password).unwrap();
        keeper.tick(&first);
        drop(first);
        // The next start of the app.
        let second = Client::new(&server).unwrap();
        let mut keeper = Keeper::restore(&second);
        let http = second.http().clone();
        let session = second
            .block_on(async move { http.session().await })
            .unwrap();
        assert_eq!(session["user"]["email"], email);
        let http = second.http().clone();
        second.block_on(async move { http.logout().await }).unwrap();
        keeper.tick(&second);
        let origin = second.http().base().origin().ascii_serialization();
        assert_eq!(stored(&origin), None);
    }

    #[test]
    fn a_restored_cookie_reaches_the_jar_and_changes_are_written() {
        let config = TempConfig::new("session");
        store("http://127.0.0.1:9", Some("abc123")).unwrap();
        store("https://other.example", Some("zzz")).unwrap();
        let client = Client::new("http://127.0.0.1:9").unwrap();
        let mut keeper = Keeper::restore(&client);
        assert_eq!(client.http().session_cookie().as_deref(), Some("abc123"));
        assert!(client.http().has_session_cookie());
        keeper.tick(&client);
        // A renewal from the server.
        client.http().restore_session_cookie("def456");
        keeper.tick(&client);
        assert_eq!(stored("http://127.0.0.1:9").as_deref(), Some("def456"));
        // Another server's cookie is never sent here, and stays stored.
        assert_eq!(stored("https://other.example").as_deref(), Some("zzz"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config.dir.join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Junk is not put into the jar.
        let fresh = Client::new("http://127.0.0.1:9").unwrap();
        fresh.http().restore_session_cookie("a;b");
        assert_eq!(fresh.http().session_cookie(), None);
    }
}
