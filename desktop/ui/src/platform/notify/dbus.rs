//! Linux: `org.freedesktop.Notifications` on the session bus (mako, KDE,
//! GNOME, dunst, swaync), as the Tauri app's notify/dbus.rs did it, through
//! zbus instead of GIO. A worker thread sends, a second one listens for
//! clicks; the service's id per tag lets a newer message replace the older.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, connection};
use zbus::message::Type;
use zbus::zvariant::Value;

use super::{Callback, Notice, Report, escape_markup, matches};

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const APP_NAME: &str = "Gelabber";
const CALL_TIMEOUT: Duration = Duration::from_secs(4);
const ANSWER_WAIT: Duration = Duration::from_secs(4);
const END_WAIT: Duration = Duration::from_secs(1);

enum Command {
    Show(Notice),
    Close(String, bool),
    Clear(mpsc::Sender<()>),
}

#[derive(Default)]
struct Shown {
    ids: HashMap<String, u32>,
    tokens: HashMap<u32, String>,
}

impl Shown {
    fn tag_of(&self, id: u32) -> Option<String> {
        self.ids
            .iter()
            .find(|(_, shown)| **shown == id)
            .map(|(tag, _)| tag.clone())
    }
}

pub struct Backend {
    commands: Mutex<mpsc::Sender<Command>>,
    ready: Arc<(Mutex<Option<bool>>, Condvar)>,
}

impl Backend {
    /// `bus`: a bus address for tests; the user's session bus when `None`.
    pub fn start(bus: Option<String>, report: Callback) -> Self {
        let (tx, rx) = mpsc::channel();
        let ready: Arc<(Mutex<Option<bool>>, Condvar)> = Arc::default();
        let answer = ready.clone();
        let spawned = std::thread::Builder::new()
            .name("gelabber-notify".into())
            .spawn(move || {
                let connected = connect(bus);
                let set = |ok: bool| {
                    let (state, cv) = &*answer;
                    if let Ok(mut s) = state.lock() {
                        *s = Some(ok);
                    }
                    cv.notify_all();
                };
                match connected {
                    Ok((conn, markup, actions)) => {
                        let shown = Arc::new(Mutex::new(Shown::default()));
                        listen(&conn, shown.clone(), report);
                        set(true);
                        work(&conn, markup, actions, &shown, rx);
                    }
                    Err(reason) => {
                        tracing::info!("notifications: {reason}");
                        set(false);
                    }
                }
            });
        if spawned.is_err() {
            let (state, _) = &*ready;
            if let Ok(mut s) = state.lock() {
                *s = Some(false);
            }
        }
        Self {
            commands: Mutex::new(tx),
            ready,
        }
    }

    fn state(&self, wait: Duration) -> Option<bool> {
        let (state, cv) = &*self.ready;
        let guard = state.lock().ok()?;
        let (guard, _) = cv.wait_timeout_while(guard, wait, |s| s.is_none()).ok()?;
        *guard
    }

    fn send(&self, command: Command) {
        if let Ok(tx) = self.commands.lock() {
            let _ = tx.send(command);
        }
    }
}

impl super::Backend for Backend {
    fn available(&self) -> bool {
        self.state(ANSWER_WAIT) == Some(true)
    }

    fn known(&self) -> Option<bool> {
        self.state(Duration::ZERO)
    }

    fn show(&self, notice: Notice) -> Result<(), String> {
        if self.state(Duration::ZERO) == Some(false) {
            return Err("no notification service on the session bus".into());
        }
        self.send(Command::Show(notice));
        Ok(())
    }

    fn close(&self, tag: &str, prefix: bool) {
        self.send(Command::Close(tag.to_owned(), prefix));
    }

    fn clear(&self) {
        if self.state(Duration::ZERO) != Some(true) {
            return;
        }
        let (done, closed) = mpsc::channel();
        self.send(Command::Clear(done));
        let _ = closed.recv_timeout(END_WAIT * 2);
    }
}

/// The connection and what the service can: markup bodies, actions.
fn connect(bus: Option<String>) -> Result<(Connection, bool, bool), String> {
    let builder = match bus {
        Some(address) => connection::Builder::address(address.as_str()),
        None => connection::Builder::session(),
    }
    .map_err(|e| e.to_string())?;
    let conn = builder
        .method_timeout(CALL_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    // Starts a service that runs on demand, and says what it can.
    let capabilities: Vec<String> = conn
        .call_method(Some(SERVICE), PATH, Some(SERVICE), "GetCapabilities", &())
        .and_then(|m| m.body().deserialize())
        .map_err(|e| format!("no notification service: {e}"))?;
    let can = |what: &str| capabilities.iter().any(|c| c == what);
    Ok((conn.clone(), can("body-markup"), can("actions")))
}

/// Clicks, activation tokens and closes, for the ids this app was given.
fn listen(conn: &Connection, shown: Arc<Mutex<Shown>>, report: Callback) {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(SERVICE)
        .and_then(|b| b.path(PATH))
        .map(|b| b.build());
    let iterator = match rule {
        Ok(rule) => MessageIterator::for_match_rule(rule, conn, Some(64)),
        Err(e) => Err(e),
    };
    let mut iterator = match iterator {
        Ok(it) => it,
        Err(e) => {
            tracing::warn!("notifications: no click reports: {e}");
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("gelabber-notify-signals".into())
        .spawn(move || {
            for message in iterator.by_ref() {
                let Ok(message) = message else { continue };
                let header = message.header();
                let Some(member) = header.member().map(|m| m.to_string()) else {
                    continue;
                };
                let body = message.body();
                match member.as_str() {
                    "ActivationToken" => {
                        if let Ok((id, token)) = body.deserialize::<(u32, String)>()
                            && let Ok(mut s) = shown.lock()
                            && s.ids.values().any(|shown| *shown == id)
                        {
                            s.tokens.insert(id, token);
                        }
                    }
                    "ActionInvoked" => {
                        let Ok((id, action)) = body.deserialize::<(u32, String)>() else {
                            continue;
                        };
                        if action != "default" {
                            continue;
                        }
                        let clicked = shown.lock().ok().and_then(|mut s| {
                            let tag = s.tag_of(id)?;
                            s.ids.remove(&tag);
                            Some((tag, s.tokens.remove(&id)))
                        });
                        if let Some((tag, activation)) = clicked {
                            report(Report::Clicked { tag, activation });
                        }
                    }
                    "NotificationClosed" => {
                        if let Ok((id, _reason)) = body.deserialize::<(u32, u32)>()
                            && let Ok(mut s) = shown.lock()
                        {
                            s.ids.retain(|_, shown| *shown != id);
                            s.tokens.remove(&id);
                        }
                    }
                    _ => {}
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("notifications: {e}");
    }
}

fn work(
    conn: &Connection,
    markup: bool,
    actions: bool,
    shown: &Mutex<Shown>,
    commands: mpsc::Receiver<Command>,
) {
    let icon = icon();
    let close = |id: u32| {
        let _ = conn.call_method(
            Some(SERVICE),
            PATH,
            Some(SERVICE),
            "CloseNotification",
            &(id,),
        );
    };
    for command in commands {
        match command {
            Command::Show(notice) => {
                let replaces = shown
                    .lock()
                    .ok()
                    .and_then(|s| s.ids.get(&notice.tag).copied())
                    .unwrap_or(0);
                let body = if markup {
                    escape_markup(&notice.body)
                } else {
                    notice.body.clone()
                };
                let actions: Vec<&str> = if actions {
                    vec!["default", "Öffnen"]
                } else {
                    Vec::new()
                };
                let answer = conn
                    .call_method(
                        Some(SERVICE),
                        PATH,
                        Some(SERVICE),
                        "Notify",
                        &(
                            APP_NAME,
                            replaces,
                            icon.as_str(),
                            notice.title.as_str(),
                            body.as_str(),
                            actions,
                            hints(),
                            -1i32,
                        ),
                    )
                    .and_then(|m| m.body().deserialize::<u32>());
                match answer {
                    Ok(id) => {
                        if let Ok(mut s) = shown.lock() {
                            s.ids.insert(notice.tag, id);
                        }
                    }
                    Err(e) => tracing::warn!("notification: {e}"),
                }
            }
            Command::Close(tag, prefix) => {
                let closing: Vec<u32> = shown
                    .lock()
                    .map(|mut s| {
                        let ids: Vec<u32> = s
                            .ids
                            .iter()
                            .filter(|(t, _)| matches(t, &tag, prefix))
                            .map(|(_, id)| *id)
                            .collect();
                        s.ids.retain(|t, _| !matches(t, &tag, prefix));
                        for id in &ids {
                            s.tokens.remove(id);
                        }
                        ids
                    })
                    .unwrap_or_default();
                for id in closing {
                    close(id);
                }
            }
            Command::Clear(done) => {
                let all: Vec<u32> = shown
                    .lock()
                    .map(|mut s| {
                        s.tokens.clear();
                        s.ids.drain().map(|(_, id)| id).collect()
                    })
                    .unwrap_or_default();
                for id in all {
                    close(id);
                }
                let _ = done.send(());
            }
        }
    }
}

/// The app it is from (GNOME and KDE group by it), a chat message, normal
/// urgency, and no sound from the service: the app plays its own cue.
fn hints() -> HashMap<&'static str, Value<'static>> {
    HashMap::from([
        ("desktop-entry", Value::from(crate::platform::APP_ID)),
        ("category", Value::from("im.received")),
        ("urgency", Value::U8(1)),
        ("suppress-sound", Value::Bool(true)),
        ("sender-pid", Value::I64(i64::from(std::process::id()))),
    ])
}

/// The icon by name where a package installed it into the icon theme,
/// else the `icon.png` next to the binary.
fn icon() -> String {
    let id = crate::platform::APP_ID;
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        });
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let dirs = home.into_iter().chain(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from),
    );
    let themed = |dir: std::path::PathBuf| {
        std::fs::read_dir(dir.join("icons/hicolor")).is_ok_and(|sizes| {
            sizes.flatten().any(|size| {
                let apps = size.path().join("apps");
                apps.join(format!("{id}.png")).is_file() || apps.join(format!("{id}.svg")).is_file()
            })
        })
    };
    if dirs.into_iter().any(themed) {
        return id.to_owned();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("icon.png")))
        .filter(|f| f.is_file())
        .and_then(|f| url::Url::from_file_path(f).ok())
        .map(String::from)
        .unwrap_or_else(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::notify::Backend as _;
    use crate::platform::testing::{Daemon, private_bus};
    use zbus::blocking::Connection;

    #[derive(Debug, PartialEq)]
    enum Call {
        Notify {
            replaces: u32,
            summary: String,
            body: String,
            actions: Vec<String>,
            desktop_entry: Option<String>,
        },
        Close(u32),
    }

    struct Fake {
        calls: Mutex<mpsc::Sender<Call>>,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Fake {
        fn get_capabilities(&self) -> Vec<String> {
            vec!["body".into(), "body-markup".into(), "actions".into()]
        }

        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            _app: String,
            replaces: u32,
            _icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, zbus::zvariant::OwnedValue>,
            _timeout: i32,
        ) -> u32 {
            let desktop_entry = hints
                .get("desktop-entry")
                .and_then(|v| String::try_from(v.clone()).ok());
            self.calls
                .lock()
                .unwrap()
                .send(Call::Notify {
                    replaces,
                    summary,
                    body,
                    actions,
                    desktop_entry,
                })
                .unwrap();
            7
        }

        fn close_notification(&self, id: u32) {
            self.calls.lock().unwrap().send(Call::Close(id)).unwrap();
        }
    }

    /// The user clicks notification 7, after another app's notification 3.
    fn click(service: &Connection) {
        // After the answer to Notify, as a real click comes.
        std::thread::sleep(Duration::from_millis(200));
        let emit = |member: &str, id: u32, text: &str| {
            service
                .emit_signal(None::<&str>, PATH, SERVICE, member, &(id, text))
                .unwrap();
        };
        emit("ActionInvoked", 3, "default");
        emit("ActivationToken", 7, "token-1");
        emit("ActionInvoked", 7, "default");
    }

    fn serve(address: &str, calls: mpsc::Sender<Call>) -> Connection {
        connection::Builder::address(address)
            .unwrap()
            .name(SERVICE)
            .unwrap()
            .serve_at(
                PATH,
                Fake {
                    calls: Mutex::new(calls),
                },
            )
            .unwrap()
            .build()
            .unwrap()
    }

    fn notice(title: &str, body: &str) -> Notice {
        Notice {
            tag: "gelabber:u1:c1".into(),
            title: title.into(),
            body: body.into(),
        }
    }

    #[test]
    fn notifications_replace_report_the_click_and_close() {
        let Some((_daemon, address)): Option<(Daemon, String)> = private_bus("notify") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let (calls, called) = mpsc::channel();
        let service = serve(&address, calls);
        let (reports, reported) = mpsc::channel();
        let reports = Mutex::new(reports);
        let backend = Backend::start(
            Some(address),
            Arc::new(move |report| {
                let Report::Clicked { tag, activation } = report;
                let _ = reports.lock().unwrap().send((tag, activation));
            }),
        );
        assert!(backend.available());
        let wait = Duration::from_secs(10);
        backend
            .show(notice("Anna · #allgemein", "<3 & mehr"))
            .unwrap();
        assert_eq!(
            called.recv_timeout(wait).unwrap(),
            Call::Notify {
                replaces: 0,
                summary: "Anna · #allgemein".into(),
                body: "&lt;3 &amp; mehr".into(),
                actions: vec!["default".into(), "Öffnen".into()],
                desktop_entry: Some(crate::platform::APP_ID.into()),
            }
        );
        backend.show(notice("click", "")).unwrap();
        let Call::Notify { replaces, .. } = called.recv_timeout(wait).unwrap() else {
            panic!("not a notification");
        };
        assert_eq!(replaces, 7);
        click(&service);
        assert_eq!(
            reported.recv_timeout(wait).unwrap(),
            ("gelabber:u1:c1".to_owned(), Some("token-1".to_owned()))
        );
        assert!(reported.recv_timeout(Duration::from_millis(300)).is_err());
        // Clicked means gone; a close of the account takes the next one down.
        backend.show(notice("Anna", "noch was")).unwrap();
        let Call::Notify { replaces, .. } = called.recv_timeout(wait).unwrap() else {
            panic!("not a notification");
        };
        assert_eq!(replaces, 0);
        backend.close("gelabber:u2:", true);
        backend.close("gelabber:u1:", true);
        assert_eq!(called.recv_timeout(wait).unwrap(), Call::Close(7));
        backend.clear();
        assert!(called.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn without_a_service_notifications_are_unavailable() {
        let Some((_daemon, address)) = private_bus("notify-none") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let backend = Backend::start(Some(address), Arc::new(|_| {}));
        assert!(!backend.available());
        assert!(backend.show(notice("Anna", "hallo")).is_err());
    }
}
