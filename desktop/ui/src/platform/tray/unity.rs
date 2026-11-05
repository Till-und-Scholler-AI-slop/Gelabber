//! Linux: the unread count on the launcher icon (Plasma's task manager,
//! Dash to Dock, Plank) through `com.canonical.Unity.LauncherEntry`, the
//! signal those docks listen for; the web's `setAppBadge` in the browser.

use std::collections::HashMap;
use std::sync::mpsc;

use zbus::blocking::{Connection, connection};
use zbus::zvariant::Value;

const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const PATH: &str = "/org/gelabber/LauncherEntry";

/// The badge's sender; a worker thread owns the bus connection.
pub struct Badge {
    tx: mpsc::Sender<u32>,
}

impl Badge {
    /// `bus`: a bus address for tests; the user's session bus when `None`.
    pub fn start(bus: Option<String>) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<u32>();
        std::thread::Builder::new()
            .name("gelabber-badge".into())
            .spawn(move || {
                let built = match bus {
                    Some(address) => connection::Builder::address(address.as_str())
                        .and_then(connection::Builder::build),
                    None => Connection::session(),
                };
                let conn = match built {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::info!("launcher badge: {e}");
                        return;
                    }
                };
                for count in rx {
                    if let Err(e) = conn.emit_signal(
                        None::<&str>,
                        PATH,
                        INTERFACE,
                        "Update",
                        &(app_uri(), properties(count)),
                    ) {
                        tracing::info!("launcher badge: {e}");
                    }
                }
            })
            .ok()?;
        Some(Self { tx })
    }

    /// Shows `count` on the icon; 0 hides the number.
    pub fn set(&self, count: u32) {
        let _ = self.tx.send(count);
    }
}

/// The desktop entry the dock matches the window with.
fn app_uri() -> String {
    format!("application://{}.desktop", crate::platform::APP_ID)
}

fn properties(count: u32) -> HashMap<&'static str, Value<'static>> {
    HashMap::from([
        ("count", Value::I64(i64::from(count))),
        ("count-visible", Value::Bool(count > 0)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::testing::private_bus;
    use std::time::Duration;
    use zbus::MatchRule;
    use zbus::blocking::MessageIterator;
    use zbus::zvariant::OwnedValue;

    #[test]
    fn the_count_goes_to_the_launcher() {
        let Some((_daemon, address)) = private_bus("badge") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let dock = connection::Builder::address(address.as_str())
            .unwrap()
            .build()
            .unwrap();
        let rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(INTERFACE)
            .unwrap()
            .member("Update")
            .unwrap()
            .build();
        let mut signals = MessageIterator::for_match_rule(rule, &dock, None).unwrap();
        let badge = Badge::start(Some(address)).unwrap();
        badge.set(3);
        badge.set(0);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for message in signals.by_ref().take(2) {
                let message = message.unwrap();
                let (uri, props): (String, HashMap<String, OwnedValue>) =
                    message.body().deserialize().unwrap();
                let count = i64::try_from(props["count"].clone()).unwrap();
                let visible = bool::try_from(props["count-visible"].clone()).unwrap();
                tx.send((uri, count, visible)).unwrap();
            }
        });
        let wait = Duration::from_secs(10);
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            ("application://gelabber.desktop".to_owned(), 3, true)
        );
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            ("application://gelabber.desktop".to_owned(), 0, false)
        );
    }
}
