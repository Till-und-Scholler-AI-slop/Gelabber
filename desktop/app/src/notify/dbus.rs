//! Linux: notifications through `org.freedesktop.Notifications` on the
//! session bus, the service every desktop runs (mako on Omarchy/Hyprland,
//! KDE Plasma, GNOME Shell, dunst, swaync). It shows them as the desktop
//! shows all others: its own look, its do-not-disturb, its history.
//!
//! One connection of its own on a thread of its own with a GLib loop, as the
//! push-to-talk portal has (`ptt/portal.rs`): the service answers a click
//! with signals. The service's id for each tag is kept, so that a newer
//! message of a conversation replaces the older one and a conversation that
//! is being read takes its notification down.
//!
//! A click on Wayland comes with an activation token (notification
//! specification 1.2, `ActivationToken`): without it Hyprland, KDE and GNOME
//! leave a window that asks for the focus where it is.

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::CString,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Duration,
};

use gio::{
    BusType, Cancellable, DBusCallFlags, DBusConnection, DBusConnectionFlags, DBusSignalFlags,
    glib::{self, ToVariant, Variant},
};
use tauri::{AppHandle, Manager};

use super::{Notice, Report, escape_markup, matches};

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const APP_NAME: &str = "Gelabber";
/// The desktop file's name (packaging/arch/gelabber-desktop.desktop), which
/// is also the icon's.
const APP_ID: &str = "gelabber-desktop";
/// How long a call to the service may take. One that starts on demand
/// (D-Bus activation, as mako and dunst do) answers within that.
const CALL_TIMEOUT_MS: i32 = 4_000;
/// How long the app waits for the first answer, and when it ends.
const ANSWER_WAIT: Duration = Duration::from_secs(4);
const END_WAIT: Duration = Duration::from_secs(1);

type Callback = Arc<dyn Fn(Report) + Send + Sync>;

pub struct Backend {
    context: glib::MainContext,
    /// Whether the service answered: `None` until it did or failed.
    ready: Arc<(Mutex<Option<bool>>, Condvar)>,
}

/// The service as the bus thread knows it.
struct Service {
    bus: DBusConnection,
    /// Bodies are markup there (`body-markup`): message text is escaped.
    markup: bool,
    /// A click reaches the app only through the default action.
    actions: bool,
    icon: String,
    /// The service's id for each tag shown.
    ids: HashMap<String, u32>,
    /// Activation tokens by id, sent just before the click itself.
    tokens: HashMap<u32, String>,
}

thread_local! {
    /// Set on the bus thread only.
    static CURRENT: RefCell<Option<Service>> = const { RefCell::new(None) };
}

fn with_service<T>(change: impl FnOnce(&mut Service) -> T) -> Option<T> {
    CURRENT.with(|service| service.borrow_mut().as_mut().map(change))
}

fn text(error: glib::Error) -> String {
    error.message().to_owned()
}

impl Backend {
    pub fn start(
        _app: &AppHandle,
        report: Box<dyn Fn(Report) + Send + Sync>,
    ) -> Result<Self, String> {
        Self::start_on(None, report)
    }

    /// `bus`: the session bus's address; the user's own when `None`.
    fn start_on(
        bus: Option<String>,
        report: Box<dyn Fn(Report) + Send + Sync>,
    ) -> Result<Self, String> {
        let context = glib::MainContext::new();
        let ready: Arc<(Mutex<Option<bool>>, Condvar)> = Arc::default();
        let backend = Self {
            context: context.clone(),
            ready: ready.clone(),
        };
        let report: Callback = Arc::from(report);
        let icon = icon();
        thread::Builder::new()
            .name("gelabber-notify".into())
            .spawn(move || {
                let main_loop = glib::MainLoop::new(Some(&context), false);
                let outcome = context
                    .with_thread_default(|| {
                        let connected = connect(bus, icon, &report);
                        let available = connected.is_ok();
                        match connected {
                            Ok(service) => {
                                CURRENT.with(|current| *current.borrow_mut() = Some(service))
                            }
                            Err(reason) => eprintln!("gelabber: notifications: {reason}"),
                        }
                        let (state, answered) = &*ready;
                        if let Ok(mut state) = state.lock() {
                            *state = Some(available);
                        }
                        answered.notify_all();
                        // For the app's lifetime: the service keeps reporting
                        // clicks, the page keeps showing notifications.
                        if available {
                            main_loop.run();
                        }
                    })
                    .map_err(|e| e.to_string());
                if let Err(reason) = outcome {
                    eprintln!("gelabber: notifications: {reason}");
                    let (state, answered) = &*ready;
                    if let Ok(mut state) = state.lock() {
                        *state = Some(false);
                    }
                    answered.notify_all();
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(backend)
    }

    /// The service's answer, waiting for it at most `wait`.
    fn state(&self, wait: Duration) -> Option<bool> {
        let (state, answered) = &*self.ready;
        let state = state.lock().ok()?;
        let (state, _) = answered
            .wait_timeout_while(state, wait, |state| state.is_none())
            .ok()?;
        *state
    }

    pub fn available(&self) -> bool {
        self.state(ANSWER_WAIT) == Some(true)
    }

    pub fn show(&self, notice: Notice) -> Result<(), String> {
        if self.state(Duration::ZERO) == Some(false) {
            return Err("no notification service on the session bus".into());
        }
        // Queued until the thread runs its loop, should it not yet.
        self.context.invoke(move || {
            with_service(|service| service.show(notice));
        });
        Ok(())
    }

    pub fn close(&self, tag: &str, prefix: bool) {
        let tag = tag.to_owned();
        self.context.invoke(move || {
            with_service(|service| service.close(&tag, prefix, false));
        });
    }

    /// Waits a moment for the service to take them down: the app is ending.
    pub fn clear(&self) {
        if self.state(Duration::ZERO) != Some(true) {
            return;
        }
        let (done, closed) = mpsc::channel();
        self.context.invoke(move || {
            with_service(|service| service.close("", true, true));
            let _ = done.send(());
        });
        let _ = closed.recv_timeout(END_WAIT);
    }
}

fn connect(bus: Option<String>, icon: String, report: &Callback) -> Result<Service, String> {
    let address = match bus {
        Some(address) => address,
        None => gio::dbus_address_get_for_bus_sync(BusType::Session, None::<&Cancellable>)
            .map_err(text)?
            .into(),
    };
    let bus = DBusConnection::for_address_sync(
        &address,
        DBusConnectionFlags::AUTHENTICATION_CLIENT | DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        None::<&Cancellable>,
    )
    .map_err(text)?;
    // Starts a service that runs on demand, and says what it can.
    let capabilities = bus
        .call_sync(
            Some(SERVICE),
            PATH,
            SERVICE,
            "GetCapabilities",
            None,
            None,
            DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            None::<&Cancellable>,
        )
        .map_err(|error| format!("no notification service: {}", text(error)))?
        .get::<(Vec<String>,)>()
        .map(|(capabilities,)| capabilities)
        .unwrap_or_default();
    let can = |what: &str| capabilities.iter().any(|capability| capability == what);
    subscribe(&bus, report);
    Ok(Service {
        markup: can("body-markup"),
        actions: can("actions"),
        bus,
        icon,
        ids: HashMap::new(),
        tokens: HashMap::new(),
    })
}

/// The service's signals, for every app on the bus; only the ids this app
/// was given count. The connection lives as long as the app, the
/// subscriptions with it.
fn subscribe(bus: &DBusConnection, report: &Callback) {
    let on = |member: &str, handle: Box<dyn Fn(&Variant)>| {
        bus.signal_subscribe(
            // The service's unique name is not known here, nor needed: the
            // ids are.
            None,
            Some(SERVICE),
            Some(member),
            Some(PATH),
            None,
            DBusSignalFlags::NONE,
            move |_, _, _, _, _, parameters| handle(parameters),
        );
    };
    on(
        "ActivationToken",
        Box::new(|parameters| {
            if let Some((id, token)) = parameters.get::<(u32, String)>() {
                with_service(|service| {
                    if service.ids.values().any(|shown| *shown == id) {
                        service.tokens.insert(id, token);
                    }
                });
            }
        }),
    );
    let report = report.clone();
    on(
        "ActionInvoked",
        Box::new(move |parameters| {
            let Some((id, action)) = parameters.get::<(u32, String)>() else {
                return;
            };
            if action != "default" {
                return;
            }
            let clicked = with_service(|service| {
                let tag = service.tag_of(id)?;
                service.ids.remove(&tag);
                Some((tag, service.tokens.remove(&id)))
            })
            .flatten();
            // Outside the service's borrow: the report goes to the main
            // thread, and nothing comes back here from it.
            if let Some((tag, activation)) = clicked {
                report(Report::Clicked { tag, activation });
            }
        }),
    );
    on(
        "NotificationClosed",
        Box::new(|parameters| {
            if let Some((id, _reason)) = parameters.get::<(u32, u32)>() {
                with_service(|service| {
                    service.ids.retain(|_, shown| *shown != id);
                    service.tokens.remove(&id);
                });
            }
        }),
    );
}

impl Service {
    fn tag_of(&self, id: u32) -> Option<String> {
        self.ids
            .iter()
            .find(|(_, shown)| **shown == id)
            .map(|(tag, _)| tag.clone())
    }

    fn show(&mut self, notice: Notice) {
        let replaces = self.ids.get(&notice.tag).copied().unwrap_or(0);
        let body = if self.markup {
            escape_markup(&notice.body)
        } else {
            notice.body
        };
        let actions: Vec<String> = if self.actions {
            vec!["default".into(), "Öffnen".into()]
        } else {
            Vec::new()
        };
        let arguments = (
            APP_NAME,
            replaces,
            self.icon.as_str(),
            notice.title.as_str(),
            body.as_str(),
            actions,
            hints(),
            // The service's own time on screen.
            -1i32,
        )
            .to_variant();
        let answer = self.bus.call_sync(
            Some(SERVICE),
            PATH,
            SERVICE,
            "Notify",
            Some(&arguments),
            None,
            DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            None::<&Cancellable>,
        );
        match answer.map(|answer| answer.get::<(u32,)>()) {
            Ok(Some((id,))) => {
                self.ids.insert(notice.tag, id);
            }
            Ok(None) => eprintln!("gelabber: notification: no id in the answer"),
            Err(error) => eprintln!("gelabber: notification: {}", text(error)),
        }
    }

    /// `wait`: when the app ends, the closes must reach the service first.
    fn close(&mut self, tag: &str, prefix: bool, wait: bool) {
        let closing: Vec<u32> = self
            .ids
            .iter()
            .filter(|(shown, _)| matches(shown, tag, prefix))
            .map(|(_, id)| *id)
            .collect();
        self.ids.retain(|shown, _| !matches(shown, tag, prefix));
        for id in closing {
            self.tokens.remove(&id);
            let arguments = (id,).to_variant();
            if wait {
                let _ = self.bus.call_sync(
                    Some(SERVICE),
                    PATH,
                    SERVICE,
                    "CloseNotification",
                    Some(&arguments),
                    None,
                    DBusCallFlags::NONE,
                    END_WAIT.as_millis() as i32,
                    None::<&Cancellable>,
                );
            } else {
                self.bus.call(
                    Some(SERVICE),
                    PATH,
                    SERVICE,
                    "CloseNotification",
                    Some(&arguments),
                    None,
                    DBusCallFlags::NONE,
                    CALL_TIMEOUT_MS,
                    None::<&Cancellable>,
                    |_| {},
                );
            }
        }
    }
}

/// The same for every message: the app it is from (GNOME and KDE group and
/// configure by it), a chat message, normal urgency, and no sound from the
/// service, since the page plays its own cue.
fn hints() -> HashMap<String, Variant> {
    HashMap::from([
        ("desktop-entry".to_owned(), APP_ID.to_variant()),
        ("category".to_owned(), "im.received".to_variant()),
        ("urgency".to_owned(), 1u8.to_variant()),
        ("suppress-sound".to_owned(), true.to_variant()),
        (
            "sender-pid".to_owned(),
            i64::from(std::process::id()).to_variant(),
        ),
    ])
}

/// The icon by name where the package installed it into the icon theme
/// (pacman), else the `icon.png` next to the binary (the plain archive).
fn icon() -> String {
    let dirs = data_dirs(
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var("XDG_DATA_DIRS").ok(),
    );
    if dirs.iter().any(|dir| themed(dir)) {
        return APP_ID.to_owned();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("icon.png")))
        .filter(|file| file.is_file())
        .and_then(|file| url::Url::from_file_path(file).ok())
        .map(String::from)
        .unwrap_or_else(|| APP_ID.to_owned())
}

/// Where the icon theme looks, in the base directory specification's order.
fn data_dirs(
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
    data_dirs: Option<String>,
) -> Vec<PathBuf> {
    let home = data_home.or_else(|| home.map(|home| home.join(".local/share")));
    let system = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    home.into_iter()
        .chain(
            system
                .split(':')
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from),
        )
        .collect()
}

/// Whether `dir`'s hicolor theme has the app icon, at any size.
fn themed(dir: &Path) -> bool {
    let Ok(sizes) = std::fs::read_dir(dir.join("icons/hicolor")) else {
        return false;
    };
    sizes.flatten().any(|size| {
        let apps = size.path().join("apps");
        apps.join(format!("{APP_ID}.png")).is_file() || apps.join(format!("{APP_ID}.svg")).is_file()
    })
}

/// Lets the window come to the front with the token of the click (main
/// thread). Wayland takes it through GDK's display, X11 as the window's
/// startup id.
pub fn activate(app: &AppHandle, token: &str) {
    use gtk::prelude::{GtkWindowExt, ObjectExt, WidgetExt};
    let Some(window) = app.get_webview_window(crate::WINDOW) else {
        return;
    };
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    let display = gtk_window.display();
    if display.type_().name() == "GdkWaylandDisplay" {
        wayland_activation(&display, token);
    } else {
        gtk_window.set_startup_id(token);
    }
}

/// `gdk_wayland_display_set_startup_notification_id`: the next focus of a
/// window activates it with this token (xdg-activation). Looked up, not
/// linked: a GDK built without Wayland has no such function, and then there
/// is no Wayland display to get here either.
fn wayland_activation(display: &gtk::gdk::Display, token: &str) {
    use gtk::glib::translate::ToGlibPtr;
    type SetToken = unsafe extern "C" fn(*mut gtk::gdk::ffi::GdkDisplay, *const std::ffi::c_char);
    let Ok(token) = CString::new(token) else {
        return;
    };
    let this = libloading::os::unix::Library::this();
    // SAFETY: the symbol is GDK's (loaded with GTK), with this signature
    // since GTK 3.22; the display is a Wayland one, as the function needs.
    unsafe {
        if let Ok(set) = this.get::<SetToken>(b"gdk_wayland_display_set_startup_notification_id\0")
        {
            set(display.to_glib_none().0, token.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, PATH, SERVICE, data_dirs, hints};
    use crate::notify::{Notice, Report};
    use gio::{
        Cancellable, DBusConnection, DBusConnectionFlags, DBusNodeInfo,
        glib::{self, ToVariant, Variant},
    };
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader},
        path::PathBuf,
        process::{Command, Stdio},
        sync::{Mutex, mpsc},
        time::Duration,
    };

    #[test]
    fn the_icon_is_looked_for_where_the_icon_theme_looks() {
        assert_eq!(
            data_dirs(None, Some("/home/a".into()), None),
            [
                PathBuf::from("/home/a/.local/share"),
                "/usr/local/share".into(),
                "/usr/share".into()
            ]
        );
        assert_eq!(
            data_dirs(
                Some("/d".into()),
                Some("/home/a".into()),
                Some("/x::/y".into())
            ),
            [PathBuf::from("/d"), "/x".into(), "/y".into()]
        );
    }

    #[test]
    fn hints_name_the_app_and_keep_the_service_quiet() {
        let hints = hints();
        assert_eq!(hints["desktop-entry"].str(), Some("gelabber-desktop"));
        assert_eq!(hints["category"].str(), Some("im.received"));
        assert_eq!(hints["urgency"].get::<u8>(), Some(1));
        assert_eq!(hints["suppress-sound"].get::<bool>(), Some(true));
        assert!(hints["sender-pid"].get::<i64>().is_some());
    }

    const SERVICE_XML: &str = r#"<node>
      <interface name="org.freedesktop.Notifications">
        <method name="GetCapabilities"><arg type="as" direction="out"/></method>
        <method name="Notify">
          <arg type="s" direction="in"/><arg type="u" direction="in"/>
          <arg type="s" direction="in"/><arg type="s" direction="in"/>
          <arg type="s" direction="in"/><arg type="as" direction="in"/>
          <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
          <arg type="u" direction="out"/>
        </method>
        <method name="CloseNotification"><arg type="u" direction="in"/></method>
      </interface>
    </node>"#;

    /// Ends the bus with the test, also one that failed.
    struct Daemon(std::process::Child);

    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A session bus without service files: no real notification service
    /// starts on it, even where one is installed.
    const BUS_CONFIG: &str = r#"<busconfig>
      <type>session</type>
      <listen>unix:tmpdir=/tmp</listen>
      <auth>EXTERNAL</auth>
      <policy context="default">
        <allow send_destination="*" eavesdrop="true"/>
        <allow eavesdrop="true"/>
        <allow own="*"/>
      </policy>
    </busconfig>"#;

    /// A bus of its own, or none where dbus-daemon is missing.
    fn private_bus(name: &str) -> Option<(Daemon, String)> {
        let config = std::env::temp_dir().join(format!(
            "gelabber-notify-bus-{}-{name}.conf",
            std::process::id()
        ));
        std::fs::write(&config, BUS_CONFIG).ok()?;
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let output = daemon.stdout.take();
        let daemon = Daemon(daemon);
        let mut address = String::new();
        BufReader::new(output?).read_line(&mut address).ok()?;
        let _ = std::fs::remove_file(config);
        Some((daemon, address.trim().to_owned()))
    }

    #[derive(Debug, PartialEq)]
    enum Call {
        Notify {
            app: String,
            replaces: u32,
            summary: String,
            body: String,
            actions: Vec<String>,
            desktop_entry: Option<String>,
        },
        Close(u32),
    }

    /// The service's side, as far as the app uses it: markup and actions,
    /// id 7 for everything, and a click (with its activation token) on the
    /// notification titled "click".
    fn fake_service(address: String, named: mpsc::Sender<()>, calls: mpsc::Sender<Call>) {
        std::thread::spawn(move || {
            let context = glib::MainContext::new();
            context
                .with_thread_default(|| {
                    let bus = DBusConnection::for_address_sync(
                        &address,
                        DBusConnectionFlags::AUTHENTICATION_CLIENT
                            | DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                        None,
                        None::<&Cancellable>,
                    )
                    .unwrap();
                    let info = DBusNodeInfo::for_xml(SERVICE_XML).unwrap();
                    let interface = info.lookup_interface(SERVICE).unwrap();
                    let calls = Mutex::new(calls);
                    bus.register_object(
                        PATH,
                        &interface,
                        move |bus, _, _, _, method, parameters, invocation| match method {
                            "GetCapabilities" => invocation.return_value(Some(
                                &(vec!["body", "body-markup", "actions"],).to_variant(),
                            )),
                            "Notify" => {
                                let hints = parameters
                                    .child_value(6)
                                    .get::<HashMap<String, Variant>>()
                                    .unwrap();
                                let summary = parameters.child_value(3).get::<String>().unwrap();
                                calls
                                    .lock()
                                    .unwrap()
                                    .send(Call::Notify {
                                        app: parameters.child_value(0).get().unwrap(),
                                        replaces: parameters.child_value(1).get().unwrap(),
                                        summary: summary.clone(),
                                        body: parameters.child_value(4).get().unwrap(),
                                        actions: parameters.child_value(5).get().unwrap(),
                                        desktop_entry: hints
                                            .get("desktop-entry")
                                            .and_then(|entry| entry.get()),
                                    })
                                    .unwrap();
                                invocation.return_value(Some(&(7u32,).to_variant()));
                                if summary == "click" {
                                    let emit = |name: &str, args: Variant| {
                                        bus.emit_signal(None, PATH, SERVICE, name, Some(&args))
                                            .unwrap();
                                    };
                                    // Another app's notification first: not ours.
                                    emit("ActionInvoked", (3u32, "default").to_variant());
                                    emit("ActivationToken", (7u32, "token-1").to_variant());
                                    emit("ActionInvoked", (7u32, "default").to_variant());
                                }
                            }
                            _ => {
                                let id = parameters.child_value(0).get::<u32>().unwrap();
                                calls.lock().unwrap().send(Call::Close(id)).unwrap();
                                invocation.return_value(None);
                            }
                        },
                        |_, _, _, _, _| 0u32.to_variant(),
                        |_, _, _, _, _, _| false,
                    )
                    .unwrap();
                    bus.call_sync(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "RequestName",
                        Some(&(SERVICE, 0u32).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        -1,
                        None::<&Cancellable>,
                    )
                    .unwrap();
                    named.send(()).unwrap();
                    glib::MainLoop::new(Some(&context), false).run();
                })
                .unwrap();
        });
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
        let Some((_daemon, address)) = private_bus("service") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let (named, ready) = mpsc::channel();
        let (calls, called) = mpsc::channel();
        fake_service(address.clone(), named, calls);
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        let (reports, reported) = mpsc::channel();
        let reports = Mutex::new(reports);
        let backend = Backend::start_on(
            Some(address),
            Box::new(move |report| {
                let Report::Clicked { tag, activation } = report;
                let _ = reports.lock().unwrap().send((tag, activation));
            }),
        )
        .unwrap();
        assert!(backend.available());
        let wait = Duration::from_secs(10);

        backend
            .show(notice("Anna · #allgemein", "<3 & mehr"))
            .unwrap();
        assert_eq!(
            called.recv_timeout(wait).unwrap(),
            Call::Notify {
                app: "Gelabber".into(),
                replaces: 0,
                summary: "Anna · #allgemein".into(),
                body: "&lt;3 &amp; mehr".into(),
                actions: vec!["default".into(), "Öffnen".into()],
                desktop_entry: Some("gelabber-desktop".into()),
            }
        );
        // The next message of the conversation replaces it, and is clicked.
        backend.show(notice("click", "")).unwrap();
        let Call::Notify { replaces, .. } = called.recv_timeout(wait).unwrap() else {
            panic!("not a notification");
        };
        assert_eq!(replaces, 7);
        assert_eq!(
            reported.recv_timeout(wait).unwrap(),
            ("gelabber:u1:c1".to_owned(), Some("token-1".to_owned()))
        );
        assert!(reported.recv_timeout(Duration::from_millis(300)).is_err());

        // Clicked means gone: a new one replaces nothing. A close of the
        // account takes it down.
        backend.show(notice("Anna", "noch was")).unwrap();
        let Call::Notify { replaces, .. } = called.recv_timeout(wait).unwrap() else {
            panic!("not a notification");
        };
        assert_eq!(replaces, 0);
        backend.close("gelabber:u2:", true);
        backend.close("gelabber:u1:", true);
        assert_eq!(called.recv_timeout(wait).unwrap(), Call::Close(7));
        // Nothing left for the end of the app.
        backend.clear();
        assert!(called.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn without_a_service_the_page_keeps_its_own_notifications() {
        let Some((_daemon, address)) = private_bus("none") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let backend = Backend::start_on(Some(address), Box::new(|_| {})).unwrap();
        assert!(!backend.available());
        assert!(backend.show(notice("Anna", "hallo")).is_err());
    }
}
