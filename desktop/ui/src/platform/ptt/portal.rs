//! Linux: the key through the GlobalShortcuts portal of xdg-desktop-portal
//! (KDE, GNOME 48+, Hyprland), as desktop/app/src/ptt/portal.rs does it,
//! through zbus. One shortcut, `push-to-talk`, in a portal session on a
//! connection of its own; the portal ends the session with the connection.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use zbus::blocking::{Connection, MessageIterator, connection, fdo::DBusProxy};
use zbus::message::Type;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{MatchRule, Message};

use super::{Binding, Event, Report};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
/// Names an app that runs outside a sandbox (xdg-desktop-portal 1.19+).
const REGISTRY: &str = "org.freedesktop.host.portal.Registry";
const SHORTCUT: &str = "push-to-talk";
/// The signal the binding sends itself to end its wait.
const STOP_PATH: &str = "/io/gelabber/ptt";
const STOP_IFACE: &str = "io.gelabber.Ptt";

type Options = HashMap<String, OwnedValue>;

/// The portal session's thread; dropping it ends it.
pub struct Session {
    stop: Arc<AtomicBool>,
    conn: Arc<Mutex<Option<Connection>>>,
}

impl Binding for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(conn) = self.conn.lock().take() {
            wake(&conn);
        }
    }
}

/// A signal to our own name: the thread's wait for messages ends.
fn wake(conn: &Connection) {
    if let Some(me) = conn.unique_name().map(|n| n.to_string()) {
        let _ = conn.emit_signal(Some(me.as_str()), STOP_PATH, STOP_IFACE, "Stop", &());
    }
}

pub fn start(trigger: Option<&'static str>, report: Report) -> Session {
    start_on(None, trigger, report)
}

/// `bus`: a bus address for tests; the user's session bus when `None`.
pub(super) fn start_on(
    bus: Option<String>,
    trigger: Option<&'static str>,
    report: Report,
) -> Session {
    let stop = Arc::new(AtomicBool::new(false));
    let conn: Arc<Mutex<Option<Connection>>> = Arc::default();
    let session = Session {
        stop: stop.clone(),
        conn: conn.clone(),
    };
    let failed = report.clone();
    let spawned = std::thread::Builder::new()
        .name("gelabber-ptt-portal".into())
        .spawn(move || {
            if let Err(reason) = run(bus, &conn, &stop, trigger, &report)
                && !stop.load(Ordering::SeqCst)
            {
                tracing::info!("push-to-talk: {reason}");
                report(Event::Failed { reason });
            }
        });
    if let Err(e) = spawned {
        failed(Event::Failed {
            reason: e.to_string(),
        });
    }
    session
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn text_of(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(p) => Some(p.as_str().to_owned()),
        Value::Value(inner) => text_of(inner),
        _ => None,
    }
}

/// The trigger as the desktop describes it, from a list of shortcuts
/// (`a(sa{sv})`).
fn described(list: &Value<'_>) -> Option<String> {
    let Value::Array(items) = list else {
        return match list {
            Value::Value(inner) => described(inner),
            _ => None,
        };
    };
    items.iter().find_map(|item| {
        let Value::Structure(s) = item else {
            return None;
        };
        let [id, about] = s.fields() else { return None };
        if text_of(id).as_deref() != Some(SHORTCUT) {
            return None;
        }
        let Value::Dict(about) = about else {
            return None;
        };
        about
            .iter()
            .find(|(k, _)| text_of(k).as_deref() == Some("trigger_description"))
            .and_then(|(_, v)| text_of(v))
            .filter(|d| !d.trim().is_empty())
    })
}

struct Portal<'a> {
    conn: Connection,
    messages: MessageIterator,
    stop: &'a AtomicBool,
    requests: u32,
    /// Signals that came while waiting for an answer.
    held: Vec<Message>,
}

impl Portal<'_> {
    fn call<B>(&self, path: &str, iface: &str, method: &str, body: &B) -> Result<Message, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(DESKTOP), path, Some(iface), method, body)
            .map_err(err)
    }

    fn next(&mut self) -> Option<Message> {
        if !self.held.is_empty() {
            return Some(self.held.remove(0));
        }
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return None;
            }
            match self.messages.next()? {
                Ok(m) if m.message_type() == Type::Signal => return Some(m),
                _ => continue,
            }
        }
    }

    /// Calls a method that answers through a Request object, and waits.
    fn request<B>(&mut self, method: &str, body: impl FnOnce(&str) -> B) -> Result<Options, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.requests += 1;
        let token = format!("gelabber{}", self.requests);
        let sender = self
            .conn
            .unique_name()
            .ok_or("no D-Bus name")?
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("{PATH}/request/{sender}/{token}");
        self.call(PATH, SHORTCUTS, method, &body(&token))?;
        let mut later = Vec::new();
        let answer = loop {
            let Some(m) = self.next() else { break None };
            let header = m.header();
            let is_answer = header.interface().is_some_and(|i| i.as_str() == REQUEST)
                && header.member().is_some_and(|n| n.as_str() == "Response")
                && header.path().is_some_and(|p| p.as_str() == path);
            if is_answer {
                break m.body().deserialize::<(u32, Options)>().ok();
            }
            later.push(m.clone());
        };
        self.held.extend(later);
        match answer {
            Some((0, results)) => Ok(results),
            Some((1, _)) => Err("Abgebrochen.".into()),
            Some((code, _)) => Err(format!("Das Portal lehnt ab ({method}, {code}).")),
            None => Err("stopped".into()),
        }
    }
}

fn options(entries: &[(&str, Value<'_>)]) -> HashMap<String, OwnedValue> {
    entries
        .iter()
        .filter_map(|(k, v)| {
            Some((
                (*k).to_owned(),
                OwnedValue::try_from(v.try_clone().ok()?).ok()?,
            ))
        })
        .collect()
}

fn run(
    bus: Option<String>,
    shared: &Mutex<Option<Connection>>,
    stop: &AtomicBool,
    trigger: Option<&'static str>,
    report: &Report,
) -> Result<(), String> {
    let builder = match &bus {
        Some(address) => connection::Builder::address(address.as_str()),
        None => connection::Builder::session(),
    }
    .map_err(err)?;
    let conn = builder.build().map_err(err)?;
    // Before any call, so no answer is missed.
    let messages = MessageIterator::from(&conn);
    let dbus = DBusProxy::new(&conn).map_err(err)?;
    for iface in [SHORTCUTS, REQUEST] {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(iface)
            .map_err(err)?
            .build();
        dbus.add_match_rule(rule).map_err(err)?;
    }
    *shared.lock() = Some(conn.clone());
    if stop.load(Ordering::SeqCst) {
        return Ok(());
    }
    let mut portal = Portal {
        conn,
        messages,
        stop,
        requests: 0,
        held: Vec::new(),
    };
    // Must be the connection's first portal call; older portals do not know it.
    let _ = portal.call(
        PATH,
        REGISTRY,
        "Register",
        &(
            crate::platform::APP_ID,
            HashMap::<String, OwnedValue>::new(),
        ),
    );
    let version = portal.call(
        PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(SHORTCUTS, "version"),
    );
    if version.is_err() {
        return Err(
            "Der Desktop bietet keine globalen Tasten an (GlobalShortcuts-Portal fehlt).".into(),
        );
    }
    let created = portal.request("CreateSession", |token| {
        (options(&[
            ("handle_token", Value::from(token)),
            ("session_handle_token", Value::from("gelabber_ptt")),
        ]),)
    })?;
    let session = created
        .get("session_handle")
        .and_then(|v| text_of(v))
        .ok_or("Das Portal nennt keine Sitzung.")?;
    let session_path = ObjectPath::try_from(session.as_str())
        .map_err(err)?
        .into_owned();

    let mut about = options(&[("description", Value::from("Push-to-Talk"))]);
    if let Some(trigger) = trigger
        && let Ok(v) = OwnedValue::try_from(Value::from(trigger))
    {
        about.insert("preferred_trigger".into(), v);
    }
    let bound = portal.request("BindShortcuts", |token| {
        (
            session_path.clone(),
            vec![(SHORTCUT, about)],
            // No parent window: a portal dialog cannot be tied to it here.
            "",
            options(&[("handle_token", Value::from(token))]),
        )
    });
    let mut held = false;
    let outcome = bound.map(|results| {
        report(Event::Bound {
            trigger: results.get("shortcuts").and_then(|v| described(v)),
        });
        while let Some(m) = portal.next() {
            let header = m.header();
            if header.interface().is_none_or(|i| i.as_str() != SHORTCUTS) {
                continue;
            }
            let Some(member) = header.member().map(|n| n.to_string()) else {
                continue;
            };
            let body = m.body();
            let Ok(signal) = body.deserialize::<zbus::zvariant::Structure<'_>>() else {
                continue;
            };
            let fields = signal.fields();
            if fields.first().and_then(text_of).as_deref() != Some(session.as_str()) {
                continue;
            }
            match member.as_str() {
                "Activated" | "Deactivated" => {
                    if fields.get(1).and_then(text_of).as_deref() != Some(SHORTCUT) {
                        continue;
                    }
                    let down = member == "Activated";
                    if down != held {
                        held = down;
                        report(if down { Event::Press } else { Event::Release });
                    }
                }
                // The user bound another key in the desktop's settings.
                "ShortcutsChanged" => report(Event::Bound {
                    trigger: fields.get(1).and_then(described),
                }),
                _ => {}
            }
        }
    });
    if held {
        report(Event::Release);
    }
    let _ = portal.call(&session, SESSION, "Close", &());
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::testing::private_bus;
    use std::sync::mpsc;
    use std::time::Duration;
    use zbus::object_server::SignalEmitter;

    struct Fake {
        log: Mutex<mpsc::Sender<String>>,
    }

    fn request_path(sender: &str, token: &str) -> String {
        format!(
            "{PATH}/request/{}/{token}",
            sender.trim_start_matches(':').replace('.', "_")
        )
    }

    #[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
    impl Fake {
        #[zbus(property, name = "version")]
        fn version(&self) -> u32 {
            1
        }

        async fn create_session(
            &self,
            #[zbus(header)] header: zbus::message::Header<'_>,
            #[zbus(connection)] conn: &zbus::Connection,
            options: HashMap<String, OwnedValue>,
        ) -> zbus::zvariant::OwnedObjectPath {
            let sender = header.sender().unwrap().to_string();
            let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let path = request_path(&sender, &token);
            let results = HashMap::from([(
                "session_handle",
                Value::from("/org/freedesktop/portal/desktop/session/1/gelabber_ptt"),
            )]);
            conn.emit_signal(
                Some(sender.as_str()),
                path.as_str(),
                REQUEST,
                "Response",
                &(0u32, results),
            )
            .await
            .unwrap();
            ObjectPath::try_from(path).unwrap().into()
        }

        async fn bind_shortcuts(
            &self,
            #[zbus(header)] header: zbus::message::Header<'_>,
            #[zbus(connection)] conn: &zbus::Connection,
            session: ObjectPath<'_>,
            shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
            _parent: String,
            options: HashMap<String, OwnedValue>,
        ) -> zbus::zvariant::OwnedObjectPath {
            let sender = header.sender().unwrap().to_string();
            let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let preferred = shortcuts[0]
                .1
                .get("preferred_trigger")
                .and_then(|v| String::try_from(v.try_clone().unwrap()).ok());
            self.log
                .lock()
                .send(format!("bind {} {:?}", shortcuts[0].0, preferred))
                .unwrap();
            let path = request_path(&sender, &token);
            let about = HashMap::from([("trigger_description", Value::from("F13"))]);
            let results = HashMap::from([("shortcuts", Value::from(vec![(SHORTCUT, about)]))]);
            conn.emit_signal(
                Some(sender.as_str()),
                path.as_str(),
                REQUEST,
                "Response",
                &(0u32, results),
            )
            .await
            .unwrap();
            // Press, a second press (no change) and release.
            let emitter = SignalEmitter::new(conn, PATH).unwrap();
            for member in ["Activated", "Activated", "Deactivated"] {
                let none = HashMap::<String, Value>::new();
                emitter
                    .emit(SHORTCUTS, member, &(session.clone(), SHORTCUT, 1u64, none))
                    .await
                    .unwrap();
            }
            ObjectPath::try_from(path).unwrap().into()
        }
    }

    struct Closing {
        log: Mutex<mpsc::Sender<String>>,
    }

    #[zbus::interface(name = "org.freedesktop.portal.Session")]
    impl Closing {
        fn close(&self) {
            self.log.lock().send("close".into()).unwrap();
        }
    }

    #[test]
    fn binds_reports_press_and_release_and_closes_the_session() {
        let Some((_daemon, address)) = private_bus("portal") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let (log, logged) = mpsc::channel();
        let _portal = connection::Builder::address(address.as_str())
            .unwrap()
            .name(DESKTOP)
            .unwrap()
            .serve_at(
                PATH,
                Fake {
                    log: Mutex::new(log.clone()),
                },
            )
            .unwrap()
            .serve_at(
                "/org/freedesktop/portal/desktop/session/1/gelabber_ptt",
                Closing {
                    log: Mutex::new(log),
                },
            )
            .unwrap()
            .build()
            .unwrap();
        let (events, got) = mpsc::channel();
        let events = Mutex::new(events);
        let session = start_on(
            Some(address),
            Some("F13"),
            Arc::new(move |e| {
                let _ = events.lock().send(e);
            }),
        );
        let wait = Duration::from_secs(10);
        assert_eq!(
            logged.recv_timeout(wait).unwrap(),
            "bind push-to-talk Some(\"F13\")"
        );
        assert_eq!(
            got.recv_timeout(wait).unwrap(),
            Event::Bound {
                trigger: Some("F13".into())
            }
        );
        assert_eq!(got.recv_timeout(wait).unwrap(), Event::Press);
        assert_eq!(got.recv_timeout(wait).unwrap(), Event::Release);
        drop(session);
        assert_eq!(logged.recv_timeout(wait).unwrap(), "close");
        assert!(got.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn without_the_portal_the_binding_fails_with_a_reason() {
        let Some((_daemon, address)) = private_bus("portal-none") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let (events, got) = mpsc::channel();
        let events = Mutex::new(events);
        let _session = start_on(
            Some(address),
            None,
            Arc::new(move |e| {
                let _ = events.lock().send(e);
            }),
        );
        let Event::Failed { reason } = got.recv_timeout(Duration::from_secs(10)).unwrap() else {
            panic!("not a failure");
        };
        assert!(reason.contains("GlobalShortcuts"));
    }
}
