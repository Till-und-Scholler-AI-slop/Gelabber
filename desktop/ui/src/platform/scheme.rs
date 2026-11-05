//! Linux: the desktop's light or dark preference, which Chromium reads for
//! `prefers-color-scheme` and winit does not report there. The Settings
//! portal's `org.freedesktop.appearance` `color-scheme`, read once and then
//! followed through `SettingChanged`.

use zbus::blocking::{MessageIterator, connection};
use zbus::zvariant::{OwnedValue, Value};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// Calls `dark` with each preference on a thread of its own: at once, then
/// on every change. `bus` is an address for tests, else the session bus.
/// Without a portal it never calls.
pub fn watch(bus: Option<String>, dark: impl Fn(bool) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("gelabber-scheme".into())
        .spawn(move || {
            if let Err(e) = follow(bus.as_deref(), &dark) {
                tracing::info!("color scheme: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::info!("color scheme: {e}");
    }
}

fn follow(bus: Option<&str>, dark: &dyn Fn(bool)) -> zbus::Result<()> {
    let conn = match bus {
        Some(address) => connection::Builder::address(address)?.build()?,
        None => connection::Builder::session()?.build()?,
    };
    // Subscribed before the read, so no change falls between the two.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(SETTINGS)?
        .member("SettingChanged")?
        .path(PATH)?
        .build();
    let changes = MessageIterator::for_match_rule(rule, &conn, None)?;
    // ReadOne since version 2 of the portal, Read (a variant in a variant)
    // before.
    let read = |method: &str| {
        conn.call_method(
            Some(DESKTOP),
            PATH,
            Some(SETTINGS),
            method,
            &(NAMESPACE, KEY),
        )
        .and_then(|reply| reply.body().deserialize::<OwnedValue>())
    };
    let first = read("ReadOne").or_else(|_| read("Read"))?;
    if let Some(value) = scheme(&first) {
        dark(value == 1);
    }
    for message in changes {
        let message = message?;
        let Ok((namespace, key, value)) =
            message.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        if namespace == NAMESPACE
            && key == KEY
            && let Some(value) = scheme(&value)
        {
            dark(value == 1);
        }
    }
    Ok(())
}

/// 0 no preference, 1 dark, 2 light; unwraps Read's extra variant.
fn scheme(value: &Value<'_>) -> Option<u32> {
    match value {
        Value::U32(n) => Some(*n),
        Value::Value(inner) => scheme(inner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::platform::testing::private_bus;

    struct Portal;

    #[zbus::interface(name = "org.freedesktop.portal.Settings")]
    impl Portal {
        fn read_one(&self, namespace: String, key: String) -> OwnedValue {
            assert_eq!((namespace.as_str(), key.as_str()), (NAMESPACE, KEY));
            OwnedValue::from(1u32)
        }
    }

    /// The portal's dark preference arrives, then a switch to light, and a
    /// change of another setting is no switch.
    #[test]
    fn the_desktop_preference_is_followed() {
        let Some((_daemon, address)) = private_bus("scheme") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let portal = connection::Builder::address(address.as_str())
            .unwrap()
            .name(DESKTOP)
            .unwrap()
            .serve_at(PATH, Portal)
            .unwrap()
            .build()
            .unwrap();
        let (tx, rx) = mpsc::channel();
        watch(Some(address), move |dark| tx.send(dark).unwrap());
        let wait = Duration::from_secs(10);
        assert!(rx.recv_timeout(wait).unwrap());
        let changed = |key: &str, value: u32| {
            portal
                .emit_signal(
                    None::<&str>,
                    PATH,
                    SETTINGS,
                    "SettingChanged",
                    &(NAMESPACE, key, Value::from(value)),
                )
                .unwrap();
        };
        changed("accent-color", 1);
        changed(KEY, 2);
        assert!(!rx.recv_timeout(wait).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
