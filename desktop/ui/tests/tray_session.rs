//! The tray in a desktop session: a private session bus with a
//! StatusNotifierWatcher as KDE, waybar or the GNOME extension run one. The
//! item registers, shows its title, tooltip and icon, follows the unread
//! state, and its click and menu reach the app. Without a watcher there is
//! no tray (and so no close-to-tray). Its own binary: it points the session
//! bus of the whole process at the private one. Skipped without dbus-daemon.
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use gelabber_ui::platform::tray::{Event, Events, Tray, Unread};
use zbus::blocking::{Connection, Proxy, connection};
use zbus::zvariant::OwnedValue;

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn private_bus() -> Option<(Daemon, String)> {
    const CONFIG: &str = r#"<busconfig>
      <type>session</type>
      <listen>unix:tmpdir=/tmp</listen>
      <auth>EXTERNAL</auth>
      <policy context="default">
        <allow send_destination="*" eavesdrop="true"/>
        <allow eavesdrop="true"/>
        <allow own="*"/>
      </policy>
    </busconfig>"#;
    let config =
        std::env::temp_dir().join(format!("gelabber-tray-bus-{}.conf", std::process::id()));
    std::fs::write(&config, CONFIG).ok()?;
    let mut child = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", config.display()))
        .args(["--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let output = child.stdout.take();
    let daemon = Daemon(child);
    let mut address = String::new();
    BufReader::new(output?).read_line(&mut address).ok()?;
    let _ = std::fs::remove_file(config);
    Some((daemon, address.trim().to_owned()))
}

struct Watcher {
    items: Mutex<mpsc::Sender<String>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        let _ = self.items.lock().unwrap().send(service);
    }

    fn register_status_notifier_host(&self, _service: String) {}

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        Vec::new()
    }
}

type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
type ToolTip = (String, Pixmaps, String, String);

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_tray_registers_follows_the_unread_state_and_answers_clicks() {
    let Some((_daemon, address)) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    // SAFETY: the only test of this binary, before any thread reads it.
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &address) };
    let events = Events::default();
    let woken = Arc::new(Mutex::new(0u32));
    let wake = {
        let woken = woken.clone();
        Arc::new(move || *woken.lock().unwrap() += 1)
    };

    // No host on the bus: no tray, so the app offers no close-to-tray.
    assert!(Tray::start(events.clone(), wake.clone()).is_none());

    let (items, registered) = mpsc::channel();
    let _watcher = connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at(
            "/StatusNotifierWatcher",
            Watcher {
                items: Mutex::new(items),
            },
        )
        .unwrap()
        .build()
        .unwrap();
    let mut tray = Tray::start(events.clone(), wake).expect("a tray with a host");
    let service = registered.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(tray.visible());

    let bus = Connection::session().unwrap();
    // Read afresh each time, as a host does on NewToolTip and NewIcon.
    let item: Proxy = zbus::blocking::proxy::Builder::new(&bus)
        .destination(service.as_str())
        .unwrap()
        .path("/StatusNotifierItem")
        .unwrap()
        .interface("org.kde.StatusNotifierItem")
        .unwrap()
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .unwrap();
    let text = |name: &str| item.get_property::<String>(name).unwrap();
    assert_eq!(text("Id"), gelabber_ui::platform::APP_ID);
    assert_eq!(text("Title"), "Gelabber");
    assert_eq!(text("Category"), "Communications");
    // (icon name, icon pixmap, title, description)
    let tip = || item.get_property::<ToolTip>("ToolTip").unwrap().2;
    assert_eq!(tip(), "Gelabber");
    let icon = || item.get_property::<Pixmaps>("IconPixmap").unwrap();
    let quiet = icon();
    assert_eq!(quiet.len(), 1);
    let (width, height, data) = &quiet[0];
    assert_eq!(data.len(), (width * height * 4) as usize);

    // Three new messages: the tooltip says so and the icon gets its dot.
    tray.set_unread(Unread {
        count: 3,
        any: true,
    });
    wait_until("the tooltip follows", || {
        tip() == "Gelabber: 3 neue Nachrichten"
    });
    assert_ne!(icon(), quiet, "the unread dot");

    // A click on the icon shows the window; the menu's entries do their part.
    item.call_method("Activate", &(0i32, 0i32)).unwrap();
    wait_until("the click", || !events.lock().is_empty());
    assert_eq!(std::mem::take(&mut *events.lock()), vec![Event::Show]);

    let menu = Proxy::new(&bus, service.as_str(), "/MenuBar", "com.canonical.dbusmenu").unwrap();
    let entries: Vec<(i32, HashMap<String, OwnedValue>)> = menu
        .call("GetGroupProperties", &(vec![1i32, 2], vec!["label"]))
        .unwrap();
    let labels: Vec<(i32, String)> = entries
        .into_iter()
        .map(|(id, props)| (id, String::try_from(props["label"].clone()).unwrap()))
        .collect();
    assert_eq!(
        labels,
        vec![(1, "Gelabber öffnen".to_owned()), (2, "Beenden".to_owned())]
    );
    let click = |id: i32| {
        menu.call_method(
            "Event",
            &(id, "clicked", zbus::zvariant::Value::I32(0), 0u32),
        )
        .unwrap();
    };
    click(2);
    wait_until("Beenden", || !events.lock().is_empty());
    assert_eq!(std::mem::take(&mut *events.lock()), vec![Event::Quit]);
    click(1);
    wait_until("Gelabber öffnen", || !events.lock().is_empty());
    assert_eq!(std::mem::take(&mut *events.lock()), vec![Event::Show]);
    assert!(*woken.lock().unwrap() >= 3, "each click wakes the window");
}
