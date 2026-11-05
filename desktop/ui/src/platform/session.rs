//! The sign-in across restarts: the session cookie per server origin in
//! the system's keyring (the Secret Service over the zbus the app already
//! has on Linux, the Credential Manager on Windows), else in `session.json`,
//! readable by the user only (0600). A locked keyring is not unlocked: that
//! would prompt in the middle of a frame, so the file serves then.

use gelabber_client::Client;
use serde_json::{Map, Value};

const FILE: &str = "session.json";

/// Where cookies go besides the file.
pub trait Keyring: Send + Sync {
    fn get(&self, origin: &str) -> Result<Option<String>, String>;
    /// Stores, or with `None` forgets, the cookie of `origin`.
    fn set(&self, origin: &str, cookie: Option<&str>) -> Result<(), String>;
}

/// The system keyring of this run, looked up once; never in tests, which
/// must not touch the user's.
fn system() -> Option<&'static dyn Keyring> {
    static KEYRING: std::sync::OnceLock<Option<Box<dyn Keyring>>> = std::sync::OnceLock::new();
    KEYRING
        .get_or_init(|| {
            if cfg!(test) {
                return None;
            }
            #[cfg(target_os = "linux")]
            {
                secret::SecretService::connect(None).map(|s| Box::new(s) as Box<dyn Keyring>)
            }
            #[cfg(windows)]
            {
                Some(Box::new(wincred::CredentialManager) as Box<dyn Keyring>)
            }
            #[cfg(not(any(target_os = "linux", windows)))]
            {
                None
            }
        })
        .as_deref()
}

fn load() -> Map<String, Value> {
    match super::paths::read_json(FILE) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The remembered cookie of `origin`.
pub fn stored(origin: &str) -> Option<String> {
    stored_in(system(), origin)
}

/// Stores or (with `None`) forgets the cookie of `origin`.
pub fn store(origin: &str, cookie: Option<&str>) -> Result<(), String> {
    store_in(system(), origin, cookie)
}

/// The keyring first, then the file (an older run's, or a locked keyring's).
fn stored_in(keyring: Option<&dyn Keyring>, origin: &str) -> Option<String> {
    if let Some(Ok(Some(cookie))) = keyring.map(|k| k.get(origin)) {
        return Some(cookie);
    }
    load()
        .get(origin)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Into the keyring when it takes it (and out of the file then), else
/// into the file.
fn store_in(
    keyring: Option<&dyn Keyring>,
    origin: &str,
    cookie: Option<&str>,
) -> Result<(), String> {
    let kept = match keyring.map(|k| k.set(origin, cookie)) {
        Some(Ok(())) => true,
        Some(Err(e)) => {
            tracing::info!("keyring: {e}; the session goes to {FILE}");
            false
        }
        None => false,
    };
    let mut map = load();
    let before = map.clone();
    match cookie {
        Some(value) if !kept => {
            map.insert(origin.to_owned(), Value::String(value.to_owned()));
        }
        _ => {
            map.remove(origin);
        }
    }
    if map == before && super::paths::file(FILE).is_some_and(|p| p.exists()) {
        return Ok(());
    }
    super::paths::write_json(FILE, &Value::Object(map), true).map_err(|e| e.to_string())
}

/// The Secret Service (`org.freedesktop.secrets`: GNOME Keyring, KWallet,
/// KeePassXC) with a plain session: the bus between the app and the daemon
/// is the user's own.
#[cfg(target_os = "linux")]
pub mod secret {
    use std::collections::HashMap;

    use zbus::blocking::{Connection, connection};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    const SERVICE: &str = "org.freedesktop.secrets";
    const PATH: &str = "/org/freedesktop/secrets";
    const IFACE: &str = "org.freedesktop.Secret.Service";
    const COLLECTION: &str = "org.freedesktop.Secret.Collection";
    const ITEM: &str = "org.freedesktop.Secret.Item";

    /// `(session, parameters, value, content type)`.
    type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

    pub struct SecretService {
        conn: Connection,
        session: OwnedObjectPath,
    }

    fn attributes(origin: &str) -> HashMap<&str, &str> {
        HashMap::from([("application", crate::platform::APP_ID), ("origin", origin)])
    }

    fn err(e: impl std::fmt::Display) -> String {
        e.to_string()
    }

    impl SecretService {
        /// `bus`: a bus address for tests; the user's session bus when
        /// `None`. `None` when no Secret Service answers.
        pub fn connect(bus: Option<&str>) -> Option<Self> {
            let builder = match bus {
                Some(address) => connection::Builder::address(address).ok()?,
                None => connection::Builder::session().ok()?,
            };
            let conn = builder.build().ok()?;
            let reply = conn
                .call_method(
                    Some(SERVICE),
                    PATH,
                    Some(IFACE),
                    "OpenSession",
                    &("plain", Value::from("")),
                )
                .ok()?;
            let (_, session): (OwnedValue, OwnedObjectPath) = reply.body().deserialize().ok()?;
            Some(Self { conn, session })
        }

        fn call<B>(
            &self,
            path: &str,
            iface: &str,
            method: &str,
            body: &B,
        ) -> Result<zbus::Message, String>
        where
            B: serde::Serialize + zbus::zvariant::DynamicType,
        {
            self.conn
                .call_method(Some(SERVICE), path, Some(iface), method, body)
                .map_err(err)
        }

        /// Our items: unlocked and locked.
        fn search(
            &self,
            origin: &str,
        ) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>), String> {
            self.call(PATH, IFACE, "SearchItems", &(attributes(origin),))?
                .body()
                .deserialize()
                .map_err(err)
        }
    }

    impl super::Keyring for SecretService {
        fn get(&self, origin: &str) -> Result<Option<String>, String> {
            let (unlocked, _) = self.search(origin)?;
            if unlocked.is_empty() {
                return Ok(None);
            }
            let secrets: HashMap<OwnedObjectPath, Secret> = self
                .call(PATH, IFACE, "GetSecrets", &(unlocked, &self.session))?
                .body()
                .deserialize()
                .map_err(err)?;
            Ok(secrets
                .into_values()
                .next()
                .and_then(|(_, _, value, _)| String::from_utf8(value).ok()))
        }

        fn set(&self, origin: &str, cookie: Option<&str>) -> Result<(), String> {
            let Some(cookie) = cookie else {
                let (unlocked, locked) = self.search(origin)?;
                if !locked.is_empty() {
                    return Err("the keyring is locked".into());
                }
                for item in unlocked {
                    self.call(item.as_str(), ITEM, "Delete", &())?;
                }
                return Ok(());
            };
            let collection: OwnedObjectPath = self
                .call(PATH, IFACE, "ReadAlias", &("default",))?
                .body()
                .deserialize()
                .map_err(err)?;
            if collection.as_str() == "/" {
                return Err("no default collection".into());
            }
            let locked: bool = self
                .conn
                .call_method(
                    Some(SERVICE),
                    collection.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &(COLLECTION, "Locked"),
                )
                .map_err(err)?
                .body()
                .deserialize::<OwnedValue>()
                .map_err(err)
                .and_then(|v| bool::try_from(v).map_err(err))?;
            if locked {
                return Err("the keyring is locked".into());
            }
            let label = format!("Gelabber-Anmeldung ({origin})");
            let properties: HashMap<&str, Value<'_>> = HashMap::from([
                ("org.freedesktop.Secret.Item.Label", Value::from(label)),
                (
                    "org.freedesktop.Secret.Item.Attributes",
                    Value::from(attributes(origin)),
                ),
            ]);
            let secret: Secret = (
                self.session.clone(),
                Vec::new(),
                cookie.as_bytes().to_vec(),
                "text/plain".into(),
            );
            let (_, prompt): (OwnedObjectPath, OwnedObjectPath) = self
                .call(
                    collection.as_str(),
                    COLLECTION,
                    "CreateItem",
                    &(properties, secret, true),
                )?
                .body()
                .deserialize()
                .map_err(err)?;
            if prompt.as_str() != "/" {
                return Err("the keyring wants a prompt".into());
            }
            Ok(())
        }
    }
}

/// The Windows Credential Manager: one generic credential per origin.
#[cfg(windows)]
mod wincred {
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    pub struct CredentialManager;

    fn target(origin: &str) -> Vec<u16> {
        format!("Gelabber/{origin}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    impl super::Keyring for CredentialManager {
        fn get(&self, origin: &str) -> Result<Option<String>, String> {
            let name = target(origin);
            let mut found: *mut CREDENTIALW = std::ptr::null_mut();
            // SAFETY: a valid NUL-terminated name; CredFree releases `found`.
            let ok = unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) };
            if ok == 0 || found.is_null() {
                return Ok(None);
            }
            // SAFETY: CredReadW filled `found` with a blob of that size.
            let value = unsafe {
                let c = &*found;
                let bytes =
                    std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
                let text = String::from_utf8(bytes.to_vec()).ok();
                CredFree(found.cast());
                text
            };
            Ok(value)
        }

        fn set(&self, origin: &str, cookie: Option<&str>) -> Result<(), String> {
            let mut name = target(origin);
            let Some(cookie) = cookie else {
                // SAFETY: a valid NUL-terminated name. A missing one is fine.
                unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) };
                return Ok(());
            };
            let mut blob = cookie.as_bytes().to_vec();
            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: name.as_mut_ptr(),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            // SAFETY: the buffers outlive the call.
            if unsafe { CredWriteW(&credential, 0) } == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(())
        }
    }
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

    /// A Secret Service on a bus of its own: the cookie goes there, out of
    /// the file, and comes back; a locked collection leaves it in the file.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_keyring_holds_the_cookie() {
        use std::collections::HashMap;
        use std::sync::Arc;
        use zbus::object_server::SignalEmitter;
        use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

        type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);
        #[derive(Default)]
        struct Store {
            items: HashMap<String, (HashMap<String, String>, Vec<u8>)>,
            locked: bool,
            next: u32,
        }
        type Shared = Arc<parking_lot::Mutex<Store>>;
        struct Service(Shared);
        struct Collection(Shared);
        struct Item(Shared, String);

        #[zbus::interface(name = "org.freedesktop.Secret.Service")]
        impl Service {
            fn open_session(
                &self,
                _algorithm: String,
                _input: OwnedValue,
            ) -> (OwnedValue, OwnedObjectPath) {
                (
                    OwnedValue::try_from(Value::from("")).unwrap(),
                    ObjectPath::try_from("/org/freedesktop/secrets/session/1")
                        .unwrap()
                        .into(),
                )
            }
            fn read_alias(&self, _name: String) -> OwnedObjectPath {
                ObjectPath::try_from("/org/freedesktop/secrets/collection/login")
                    .unwrap()
                    .into()
            }
            fn search_items(
                &self,
                attributes: HashMap<String, String>,
            ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
                let store = self.0.lock();
                let found: Vec<OwnedObjectPath> = store
                    .items
                    .iter()
                    .filter(|(_, (a, _))| attributes.iter().all(|(k, v)| a.get(k) == Some(v)))
                    .map(|(p, _)| ObjectPath::try_from(p.as_str()).unwrap().into())
                    .collect();
                if store.locked {
                    (Vec::new(), found)
                } else {
                    (found, Vec::new())
                }
            }
            fn get_secrets(
                &self,
                items: Vec<OwnedObjectPath>,
                session: OwnedObjectPath,
            ) -> HashMap<OwnedObjectPath, Secret> {
                let store = self.0.lock();
                items
                    .into_iter()
                    .filter_map(|p| {
                        let (_, value) = store.items.get(p.as_str())?;
                        Some((
                            p,
                            (
                                session.clone(),
                                Vec::new(),
                                value.clone(),
                                "text/plain".into(),
                            ),
                        ))
                    })
                    .collect()
            }
        }

        #[zbus::interface(name = "org.freedesktop.Secret.Collection")]
        impl Collection {
            #[zbus(property)]
            fn locked(&self) -> bool {
                self.0.lock().locked
            }
            async fn create_item(
                &self,
                #[zbus(object_server)] server: &zbus::ObjectServer,
                properties: HashMap<String, OwnedValue>,
                secret: Secret,
                replace: bool,
            ) -> (OwnedObjectPath, OwnedObjectPath) {
                let attributes: HashMap<String, String> = HashMap::try_from(
                    properties["org.freedesktop.Secret.Item.Attributes"]
                        .try_clone()
                        .unwrap(),
                )
                .unwrap();
                let path = {
                    let mut store = self.0.lock();
                    assert!(replace);
                    store.items.retain(|_, (a, _)| *a != attributes);
                    store.next += 1;
                    let path = format!("/org/freedesktop/secrets/collection/login/{}", store.next);
                    store.items.insert(path.clone(), (attributes, secret.2));
                    path
                };
                server
                    .at(path.as_str(), Item(self.0.clone(), path.clone()))
                    .await
                    .unwrap();
                (
                    ObjectPath::try_from(path).unwrap().into(),
                    ObjectPath::try_from("/").unwrap().into(),
                )
            }
            #[zbus(signal)]
            async fn item_created(
                emitter: &SignalEmitter<'_>,
                item: ObjectPath<'_>,
            ) -> zbus::Result<()>;
        }

        #[zbus::interface(name = "org.freedesktop.Secret.Item")]
        impl Item {
            fn delete(&self) -> OwnedObjectPath {
                self.0.lock().items.remove(&self.1);
                ObjectPath::try_from("/").unwrap().into()
            }
        }

        let Some((_daemon, address)) = crate::platform::testing::private_bus("secrets") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let _config = TempConfig::new("keyring");
        let shared: Shared = Arc::default();
        let _service = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.freedesktop.secrets")
            .unwrap()
            .serve_at("/org/freedesktop/secrets", Service(shared.clone()))
            .unwrap()
            .serve_at(
                "/org/freedesktop/secrets/collection/login",
                Collection(shared.clone()),
            )
            .unwrap()
            .build()
            .unwrap();
        let keyring = secret::SecretService::connect(Some(&address)).expect("the fake service");
        let keyring: Option<&dyn Keyring> = Some(&keyring);
        // An older run's cookie in the file moves into the keyring.
        store_in(None, "http://127.0.0.1:9", Some("old")).unwrap();
        assert_eq!(
            stored_in(keyring, "http://127.0.0.1:9").as_deref(),
            Some("old")
        );
        store_in(keyring, "http://127.0.0.1:9", Some("abc123")).unwrap();
        assert_eq!(
            stored_in(keyring, "http://127.0.0.1:9").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            stored_in(None, "http://127.0.0.1:9"),
            None,
            "not in the file"
        );
        assert_eq!(shared.lock().items.len(), 1);
        // A renewal replaces the item; signing out removes it.
        store_in(keyring, "http://127.0.0.1:9", Some("def456")).unwrap();
        assert_eq!(shared.lock().items.len(), 1);
        assert_eq!(
            stored_in(keyring, "http://127.0.0.1:9").as_deref(),
            Some("def456")
        );
        store_in(keyring, "http://127.0.0.1:9", None).unwrap();
        assert!(shared.lock().items.is_empty());
        assert_eq!(stored_in(keyring, "http://127.0.0.1:9"), None);
        // Locked: no prompt, the file keeps it.
        shared.lock().locked = true;
        store_in(keyring, "https://other.example", Some("zzz")).unwrap();
        assert!(shared.lock().items.is_empty());
        assert_eq!(
            stored_in(keyring, "https://other.example").as_deref(),
            Some("zzz")
        );
        // No service on the bus: no keyring at all.
        let (_empty, other) = crate::platform::testing::private_bus("secrets-none").unwrap();
        assert!(secret::SecretService::connect(Some(&other)).is_none());
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
