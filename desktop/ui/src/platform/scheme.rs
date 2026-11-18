//! Linux: the desktop's light or dark preference, which Chromium reads for
//! `prefers-color-scheme` and winit does not report there, and its accent
//! colour for "System". The Settings portal's `org.freedesktop.appearance`
//! `color-scheme` and `accent-color`, read once and then followed through
//! `SettingChanged`.

use egui::Color32;
use zbus::blocking::{MessageIterator, connection};
use zbus::zvariant::{OwnedValue, Value};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";
const ACCENT: &str = "accent-color";

/// One of the desktop's appearance settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// The light or dark preference: dark or not.
    Dark(bool),
    /// The accent colour, `None` where the desktop sets none.
    Accent(Option<Color32>),
}

/// Calls `change` with each setting on a thread of its own: at once, then
/// on every change. `bus` is an address for tests, else the session bus.
/// Without a portal it never calls.
pub fn watch(bus: Option<String>, change: impl Fn(Change) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("gelabber-scheme".into())
        .spawn(move || {
            if let Err(e) = follow(bus.as_deref(), &change) {
                tracing::info!("color scheme: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::info!("color scheme: {e}");
    }
}

fn follow(bus: Option<&str>, change: &dyn Fn(Change)) -> zbus::Result<()> {
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
    let read = |method: &str, key: &str| {
        conn.call_method(
            Some(DESKTOP),
            PATH,
            Some(SETTINGS),
            method,
            &(NAMESPACE, key),
        )
        .and_then(|reply| reply.body().deserialize::<OwnedValue>())
    };
    let read = |key: &str| read("ReadOne", key).or_else(|_| read("Read", key));
    let first = read(KEY)?;
    if let Some(value) = scheme(&first) {
        change(Change::Dark(value == 1));
    }
    // Portals before version 2 (and desktops without one) have no accent.
    if let Ok(value) = read(ACCENT) {
        change(Change::Accent(accent(&value)));
    }
    for message in changes {
        let message = message?;
        let Ok((namespace, key, value)) =
            message.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        if namespace != NAMESPACE {
            continue;
        }
        if key == KEY
            && let Some(value) = scheme(&value)
        {
            change(Change::Dark(value == 1));
        } else if key == ACCENT {
            change(Change::Accent(accent(&value)));
        }
    }
    Ok(())
}

/// `(ddd)`, sRGB in 0..1; anything out of range means no accent (the
/// portal's documentation). Unwraps Read's extra variant.
fn accent(value: &Value<'_>) -> Option<Color32> {
    match value {
        Value::Value(inner) => accent(inner),
        Value::Structure(s) => {
            let channel = |v: &Value<'_>| match v {
                Value::F64(x) if (0.0..=1.0).contains(x) => Some((x * 255.0).round() as u8),
                _ => None,
            };
            match s.fields() {
                [r, g, b] => Some(Color32::from_rgb(channel(r)?, channel(g)?, channel(b)?)),
                _ => None,
            }
        }
        _ => None,
    }
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

    fn rgb(r: f64, g: f64, b: f64) -> Value<'static> {
        Value::Structure((r, g, b).into())
    }

    #[zbus::interface(name = "org.freedesktop.portal.Settings")]
    impl Portal {
        fn read_one(&self, namespace: String, key: String) -> zbus::fdo::Result<OwnedValue> {
            assert_eq!(namespace, NAMESPACE);
            let value = match key.as_str() {
                KEY => Value::from(1u32),
                ACCENT => rgb(0.2, 0.4, 1.0),
                _ => return Err(zbus::fdo::Error::Failed(key)),
            };
            OwnedValue::try_from(value).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
        }
    }

    /// The portal's dark preference and accent arrive, then a new accent,
    /// one out of range (none), and a switch to light; another setting's
    /// change is none of these.
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
        watch(Some(address), move |change| tx.send(change).unwrap());
        let wait = Duration::from_secs(10);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Change::Dark(true));
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            Change::Accent(Some(Color32::from_rgb(51, 102, 255)))
        );
        let changed = |key: &str, value: Value<'_>| {
            portal
                .emit_signal(
                    None::<&str>,
                    PATH,
                    SETTINGS,
                    "SettingChanged",
                    &(NAMESPACE, key, value),
                )
                .unwrap();
        };
        changed("contrast", Value::from(1u32));
        changed(ACCENT, rgb(1.0, 0.0, 0.0));
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            Change::Accent(Some(Color32::from_rgb(255, 0, 0)))
        );
        changed(ACCENT, rgb(2.0, 0.0, 0.0));
        assert_eq!(rx.recv_timeout(wait).unwrap(), Change::Accent(None));
        changed(KEY, Value::from(2u32));
        assert_eq!(rx.recv_timeout(wait).unwrap(), Change::Dark(false));
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
