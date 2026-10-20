//! Linux: the bound key through the GlobalShortcuts portal of
//! xdg-desktop-portal (KDE, GNOME 48+, Hyprland), which works on Wayland
//! and X11 alike.
//!
//! One shortcut, `push-to-talk`, in a portal session of its own on a D-Bus
//! connection of its own: the portal ends the session with the connection.
//! The desktop decides the key. KDE and GNOME ask the user once, with the
//! page's key as the suggestion, and remember the answer; Hyprland takes
//! `bind = , <key>, global, gelabber-desktop:push-to-talk` in
//! `hyprland.conf` (`hyprctl globalshortcuts` lists the name).
//!
//! The portal answers through signals, so the session runs a GLib loop on a
//! thread of its own.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use gio::{
    BusType, Cancellable, DBusCallFlags, DBusConnection, DBusConnectionFlags, DBusSignalFlags,
    glib::{self, ToVariant, Variant, variant::ObjectPath},
};

use super::{Binding, Event, Report};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
/// Names an app that runs outside a sandbox (xdg-desktop-portal 1.19+).
const REGISTRY: &str = "org.freedesktop.host.portal.Registry";
/// The desktop file's name (packaging/arch/gelabber-desktop.desktop).
const APP_ID: &str = "gelabber-desktop";
const SHORTCUT: &str = "push-to-talk";

type Options = HashMap<String, Variant>;

/// The portal session's thread; dropping this ends it.
pub struct Session {
    stop: Arc<AtomicBool>,
    context: glib::MainContext,
    main_loop: glib::MainLoop,
}

impl Binding for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Through the session's own context: a quit sent before its loop
        // runs again would be lost, this waits for it.
        let main_loop = self.main_loop.clone();
        self.context.invoke(move || main_loop.quit());
    }
}

pub fn start(trigger: Option<&'static str>, report: Report) -> Session {
    start_on(None, trigger, report)
}

/// `bus`: the session bus's address; the user's own when `None`.
fn start_on(bus: Option<String>, trigger: Option<&'static str>, report: Report) -> Session {
    let context = glib::MainContext::new();
    let main_loop = glib::MainLoop::new(Some(&context), false);
    let stop = Arc::new(AtomicBool::new(false));
    let session = Session {
        stop: stop.clone(),
        context: context.clone(),
        main_loop: main_loop.clone(),
    };
    let failed = report.clone();
    let spawned = thread::Builder::new()
        .name("gelabber-ptt-portal".into())
        .spawn(move || {
            let outcome = context
                .with_thread_default(|| run(bus, &main_loop, &stop, trigger, &report))
                .map_err(|e| e.to_string())
                .and_then(|outcome| outcome);
            if let Err(reason) = outcome
                && !stop.load(Ordering::SeqCst)
            {
                eprintln!("gelabber: push-to-talk: {reason}");
                report(Event::Failed { reason });
            }
        });
    if let Err(error) = spawned {
        failed(Event::Failed {
            reason: error.to_string(),
        });
    }
    session
}

fn text(error: glib::Error) -> String {
    error.message().to_owned()
}

fn options<const N: usize>(entries: [(&str, Variant); N]) -> Options {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

/// The trigger as the desktop describes it, from a list of shortcuts.
fn described(shortcuts: &[(String, Options)]) -> Option<String> {
    shortcuts
        .iter()
        .find(|(id, _)| id == SHORTCUT)
        .and_then(|(_, about)| about.get("trigger_description"))
        .and_then(|description| description.str().map(str::to_owned))
        .filter(|description| !description.trim().is_empty())
}

struct Portal<'a> {
    bus: DBusConnection,
    main_loop: &'a glib::MainLoop,
    stop: &'a AtomicBool,
    requests: Cell<u32>,
}

impl Portal<'_> {
    fn call(
        &self,
        path: &str,
        interface: &str,
        method: &str,
        args: &Variant,
    ) -> Result<Variant, String> {
        self.bus
            .call_sync(
                Some(DESKTOP),
                path,
                interface,
                method,
                Some(args),
                None,
                DBusCallFlags::NONE,
                -1,
                None::<&Cancellable>,
            )
            .map_err(text)
    }

    /// Runs the loop until the stop or until `done` holds.
    fn wait(&self, done: impl Fn() -> bool) {
        while !done() && !self.stop.load(Ordering::SeqCst) {
            self.main_loop.run();
        }
    }

    /// Calls a method that answers through a Request object and waits for
    /// that answer: the results, or why there are none.
    fn request(&self, method: &str, args: impl FnOnce(&str) -> Variant) -> Result<Options, String> {
        let number = self.requests.get() + 1;
        self.requests.set(number);
        let token = format!("gelabber{number}");
        // The Request object's path is known before the call, so the answer
        // cannot come before the subscription.
        let sender = self.bus.unique_name().ok_or("no D-Bus name")?;
        let sender = sender.trim_start_matches(':').replace('.', "_");
        let path = format!("{PATH}/request/{sender}/{token}");
        let answer: Rc<RefCell<Option<(u32, Options)>>> = Rc::default();
        let subscription = {
            let answer = answer.clone();
            let main_loop = self.main_loop.clone();
            self.bus.signal_subscribe(
                None,
                Some(REQUEST),
                Some("Response"),
                Some(&path),
                None,
                DBusSignalFlags::NONE,
                move |_, _, _, _, _, parameters| {
                    *answer.borrow_mut() = parameters.get();
                    main_loop.quit();
                },
            )
        };
        let called = self.call(PATH, SHORTCUTS, method, &args(&token));
        if called.is_ok() {
            self.wait(|| answer.borrow().is_some());
        }
        self.bus.signal_unsubscribe(subscription);
        called?;
        match answer.take() {
            Some((0, results)) => Ok(results),
            Some((1, _)) => Err("Abgebrochen.".into()),
            Some((code, _)) => Err(format!("Das Portal lehnt ab ({method}, {code}).")),
            None => Err("stopped".into()),
        }
    }

    /// A signal of `session`. Matched here rather than by the bus: `arg0`
    /// match rules take strings, and the session is an object path.
    fn subscribe(
        &self,
        member: &str,
        session: &str,
        callback: impl Fn(&Variant) + 'static,
    ) -> gio::SignalSubscriptionId {
        let session = session.to_owned();
        self.bus.signal_subscribe(
            None,
            Some(SHORTCUTS),
            Some(member),
            Some(PATH),
            None,
            DBusSignalFlags::NONE,
            move |_, _, _, _, _, parameters| {
                if parameters.n_children() > 1
                    && parameters.child_value(0).str() == Some(session.as_str())
                {
                    callback(parameters);
                }
            },
        )
    }
}

fn run(
    bus: Option<String>,
    main_loop: &glib::MainLoop,
    stop: &AtomicBool,
    trigger: Option<&'static str>,
    report: &Report,
) -> Result<(), String> {
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
    let portal = Portal {
        bus,
        main_loop,
        stop,
        requests: Cell::new(0),
    };
    // Must be the connection's first portal call; older portals do not know
    // it and name the app from its process, if at all.
    let _ = portal.call(
        PATH,
        REGISTRY,
        "Register",
        &(APP_ID, Options::new()).to_variant(),
    );
    let version = portal.call(
        PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(SHORTCUTS, "version").to_variant(),
    );
    if version.is_err() {
        return Err(
            "Der Desktop bietet keine globalen Tasten an (GlobalShortcuts-Portal fehlt).".into(),
        );
    }
    let created = portal.request("CreateSession", |token| {
        (options([
            ("handle_token", token.to_variant()),
            ("session_handle_token", "gelabber_ptt".to_variant()),
        ]),)
            .to_variant()
    })?;
    let session = created
        .get("session_handle")
        .and_then(|handle| handle.str().map(str::to_owned))
        .ok_or("Das Portal nennt keine Sitzung.")?;
    let session_path = ObjectPath::try_from(session.clone()).map_err(|e| e.to_string())?;

    let held = Rc::new(Cell::new(false));
    let pressed = {
        let (held, report) = (held.clone(), report.clone());
        portal.subscribe("Activated", &session, move |parameters| {
            if parameters.child_value(1).str() == Some(SHORTCUT) && !held.replace(true) {
                report(Event::Press);
            }
        })
    };
    let released = {
        let (held, report) = (held.clone(), report.clone());
        portal.subscribe("Deactivated", &session, move |parameters| {
            if parameters.child_value(1).str() == Some(SHORTCUT) && held.replace(false) {
                report(Event::Release);
            }
        })
    };
    // The user bound another key in the desktop's settings.
    let changed = {
        let report = report.clone();
        portal.subscribe("ShortcutsChanged", &session, move |parameters| {
            if let Some(shortcuts) = parameters.child_value(1).get::<Vec<(String, Options)>>() {
                report(Event::Bound {
                    trigger: described(&shortcuts),
                });
            }
        })
    };

    let mut about = options([("description", "Push-to-Talk".to_variant())]);
    if let Some(trigger) = trigger {
        about.insert("preferred_trigger".into(), trigger.to_variant());
    }
    let bound = portal.request("BindShortcuts", |token| {
        (
            session_path.clone(),
            vec![(SHORTCUT.to_owned(), about)],
            // No parent window: a portal dialog cannot be tied to a window
            // of another toolkit's process here.
            String::new(),
            options([("handle_token", token.to_variant())]),
        )
            .to_variant()
    });
    let outcome = bound.map(|results| {
        let shortcuts = results
            .get("shortcuts")
            .and_then(|list| list.get::<Vec<(String, Options)>>())
            .unwrap_or_default();
        report(Event::Bound {
            trigger: described(&shortcuts),
        });
        portal.wait(|| false);
    });

    for subscription in [pressed, released, changed] {
        portal.bus.signal_unsubscribe(subscription);
    }
    if held.get() {
        report(Event::Release);
    }
    let _ = portal.call(&session, SESSION, "Close", &().to_variant());
    let _ = portal.bus.close_sync(None::<&Cancellable>);
    outcome
}

#[cfg(test)]
mod tests {
    use super::{
        DESKTOP, Event, Options, PATH, REQUEST, SHORTCUT, SHORTCUTS, described, options, start_on,
    };
    use gio::{
        Cancellable, DBusConnection, DBusConnectionFlags, DBusNodeInfo,
        glib::{self, ToVariant, Variant},
    };
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn the_trigger_is_the_description_of_our_shortcut() {
        let list = vec![
            (
                "other".to_owned(),
                options([("trigger_description", "Ctrl+X".to_variant())]),
            ),
            (
                SHORTCUT.to_owned(),
                options([("trigger_description", "F13".to_variant())]),
            ),
        ];
        assert_eq!(described(&list).as_deref(), Some("F13"));
        let unbound = vec![(
            SHORTCUT.to_owned(),
            options([("trigger_description", "".to_variant())]),
        )];
        assert_eq!(described(&unbound), None);
        assert_eq!(described(&[(SHORTCUT.to_owned(), Options::new())]), None);
    }

    /// The shortcut list as it crosses D-Bus, `a(sa{sv})`, reads back.
    #[test]
    fn shortcut_lists_read_back_from_their_variant() {
        let list = vec![(
            SHORTCUT.to_owned(),
            options([("trigger_description", "Pause".to_variant())]),
        )];
        let wire = list.to_variant();
        assert_eq!(wire.type_().as_str(), "a(sa{sv})");
        let back = wire.get::<Vec<(String, Options)>>().unwrap();
        assert_eq!(described(&back).as_deref(), Some("Pause"));
    }

    const PORTAL_XML: &str = r#"<node>
      <interface name="org.freedesktop.portal.GlobalShortcuts">
        <method name="CreateSession">
          <arg type="a{sv}" direction="in"/><arg type="o" direction="out"/>
        </method>
        <method name="BindShortcuts">
          <arg type="o" direction="in"/><arg type="a(sa{sv})" direction="in"/>
          <arg type="s" direction="in"/><arg type="a{sv}" direction="in"/>
          <arg type="o" direction="out"/>
        </method>
        <property name="version" type="u" access="read"/>
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

    /// A session bus without service files: no real portal starts on it,
    /// even where one is installed.
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
    fn private_bus() -> Option<(Daemon, String)> {
        static BUSES: AtomicUsize = AtomicUsize::new(0);
        let config = std::env::temp_dir().join(format!(
            "gelabber-ptt-bus-{}-{}.conf",
            std::process::id(),
            BUSES.fetch_add(1, Ordering::Relaxed)
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

    /// The portal's side, as far as the binding uses it: a session, the
    /// shortcut bound to the suggested key, then the key pressed and
    /// released once.
    fn fake_portal(address: String, named: mpsc::Sender<()>, bound: mpsc::Sender<Option<String>>) {
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
                    let info = DBusNodeInfo::for_xml(PORTAL_XML).unwrap();
                    let interface = info.lookup_interface(SHORTCUTS).unwrap();
                    let bound = Mutex::new(bound);
                    bus.register_object(
                        PATH,
                        &interface,
                        move |bus, sender, _, _, method, parameters, invocation| {
                            let caller = sender.trim_start_matches(':').replace('.', "_");
                            let options_at = if method == "CreateSession" { 0 } else { 3 };
                            let token = parameters
                                .child_value(options_at)
                                .get::<Options>()
                                .and_then(|o| o.get("handle_token").and_then(|t| t.get::<String>()))
                                .unwrap();
                            let request = format!("{PATH}/request/{caller}/{token}");
                            let session = format!("{PATH}/session/{caller}/gelabber_ptt");
                            let results = if method == "CreateSession" {
                                options([("session_handle", session.to_variant())])
                            } else {
                                let shortcuts = parameters
                                    .child_value(1)
                                    .get::<Vec<(String, Options)>>()
                                    .unwrap();
                                let suggested = shortcuts[0]
                                    .1
                                    .get("preferred_trigger")
                                    .and_then(|t| t.get::<String>());
                                bound.lock().unwrap().send(suggested.clone()).unwrap();
                                let about = options([(
                                    "trigger_description",
                                    suggested.unwrap_or_default().to_variant(),
                                )]);
                                options([(
                                    "shortcuts",
                                    vec![(SHORTCUT.to_owned(), about)].to_variant(),
                                )])
                            };
                            let path =
                                glib::variant::ObjectPath::try_from(request.clone()).unwrap();
                            invocation.return_value(Some(&(path,).to_variant()));
                            let emit = |path: &str, interface: &str, name: &str, args: Variant| {
                                bus.emit_signal(Some(sender), path, interface, name, Some(&args))
                                    .unwrap();
                            };
                            emit(&request, REQUEST, "Response", (0u32, results).to_variant());
                            if method == "BindShortcuts" {
                                let session = glib::variant::ObjectPath::try_from(session).unwrap();
                                for name in ["Activated", "Deactivated"] {
                                    let args = (session.clone(), SHORTCUT, 7u64, Options::new());
                                    emit(PATH, SHORTCUTS, name, args.to_variant());
                                }
                            }
                        },
                        |_, _, _, _, _| 1u32.to_variant(),
                        |_, _, _, _, _, _| false,
                    )
                    .unwrap();
                    bus.call_sync(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "RequestName",
                        Some(&(DESKTOP, 0u32).to_variant()),
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

    #[test]
    fn the_portal_binds_the_suggested_key_and_reports_press_and_release() {
        let Some((_daemon, address)) = private_bus() else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let (bound, suggested) = mpsc::channel();
        let (named, ready) = mpsc::channel();
        fake_portal(address.clone(), named, bound);
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        let (events, heard) = mpsc::channel();
        let events = Mutex::new(events);
        let session = start_on(
            Some(address),
            Some("F13"),
            Arc::new(move |event| {
                let _ = events.lock().unwrap().send(event);
            }),
        );
        let wait = Duration::from_secs(10);
        assert_eq!(
            suggested.recv_timeout(wait).unwrap().as_deref(),
            Some("F13")
        );
        assert_eq!(
            heard.recv_timeout(wait).unwrap(),
            Event::Bound {
                trigger: Some("F13".into())
            }
        );
        assert_eq!(heard.recv_timeout(wait).unwrap(), Event::Press);
        assert_eq!(heard.recv_timeout(wait).unwrap(), Event::Release);
        drop(session);
        // Ended: neither a failure nor anything else follows.
        assert!(heard.recv_timeout(Duration::from_millis(500)).is_err());
    }

    #[test]
    fn without_a_portal_the_binding_fails_with_a_reason() {
        let Some((_daemon, address)) = private_bus() else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let (events, heard) = mpsc::channel();
        let events = Mutex::new(events);
        let _session = start_on(
            Some(address),
            None,
            Arc::new(move |event| {
                let _ = events.lock().unwrap().send(event);
            }),
        );
        let event = heard.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            matches!(&event, Event::Failed { reason } if reason.contains("GlobalShortcuts")),
            "{event:?}"
        );
    }
}
