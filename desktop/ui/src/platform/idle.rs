//! The system's idle time for presence (#566): input in any app keeps the
//! user online, as in Discord, not only input in Gelabber's window. One
//! thread reads it every [`POLL`] (Wayland: follows the compositor's idle
//! events) and hands the client the time of the last input; the client
//! decides when that means away (`gelabber_client::presence`).
//!
//! Sources, the first that works:
//! - Windows: `GetLastInputInfo`.
//! - Linux on Wayland: `ext-idle-notify-v1` (Hyprland, Sway, KWin, Mutter
//!   48+), else GNOME's `org.gnome.Mutter.IdleMonitor` on the session bus.
//!   `org.freedesktop.ScreenSaver.GetSessionIdleTime` is left out: KWin on
//!   Wayland answers 0 there, which would keep everyone online forever.
//! - Linux on X11: the MIT-SCREEN-SAVER extension, else Mutter's monitor.
//!
//! Without a source only the window's input counts, as before.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gelabber_client::presence::{SystemIdle, set_system_idle};
use parking_lot::Mutex;

/// How often the idle time is read.
const POLL: Duration = Duration::from_secs(30);

/// Time since the last keyboard or mouse input anywhere in the session.
/// May block for a moment; runs on the idle thread only.
pub trait Probe: Send {
    /// `None` once the source stopped answering.
    fn idle(&mut self) -> Option<Duration>;
}

/// The last input as the thread last read it; what the client asks.
#[derive(Default)]
struct LastInput(Mutex<Option<Instant>>);

impl SystemIdle for LastInput {
    fn last_input(&self) -> Option<Instant> {
        *self.0.lock()
    }
}

impl LastInput {
    /// One reading: `None` falls back to the window's input alone.
    fn read(&self, probe: &mut dyn Probe, now: Instant) {
        *self.0.lock() = probe.idle().and_then(|idle| now.checked_sub(idle));
    }
}

/// Starts the idle thread once per process and tells the client about it.
pub fn start() {
    let last = Arc::new(LastInput::default());
    set_system_idle(last.clone());
    let spawned = std::thread::Builder::new()
        .name("gelabber-idle".into())
        .spawn(move || {
            let Some(mut probe) = system_probe() else {
                tracing::debug!("system idle: no source, only window input counts");
                return;
            };
            loop {
                last.read(probe.as_mut(), Instant::now());
                std::thread::sleep(POLL);
            }
        });
    if let Err(e) = spawned {
        tracing::debug!("system idle: {e}");
    }
}

#[cfg(windows)]
fn system_probe() -> Option<Box<dyn Probe>> {
    let mut probe = windows::LastInputInfo;
    probe.idle()?;
    tracing::debug!("system idle: GetLastInputInfo");
    Some(Box::new(probe))
}

#[cfg(target_os = "linux")]
fn system_probe() -> Option<Box<dyn Probe>> {
    /// Which source answered, for the log.
    #[derive(Debug)]
    enum Source {
        ExtIdleNotify,
        X11ScreenSaver,
        MutterIdleMonitor,
    }
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let mutter =
        || linux::mutter().map(|p| (Source::MutterIdleMonitor, Box::new(p) as Box<dyn Probe>));
    let found = if wayland {
        linux::wayland()
            .map(|p| (Source::ExtIdleNotify, Box::new(p) as Box<dyn Probe>))
            .or_else(mutter)
    } else {
        linux::x11()
            .map(|p| (Source::X11ScreenSaver, Box::new(p) as Box<dyn Probe>))
            .or_else(mutter)
    };
    let (source, probe) = found?;
    tracing::debug!("system idle: {source:?}");
    Some(probe)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_probe() -> Option<Box<dyn Probe>> {
    None
}

#[cfg(windows)]
mod windows {
    use std::time::Duration;

    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    pub struct LastInputInfo;

    impl super::Probe for LastInputInfo {
        fn idle(&mut self) -> Option<Duration> {
            let mut info = LASTINPUTINFO {
                cbSize: size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            // SAFETY: `info` is a valid LASTINPUTINFO with its size set.
            if unsafe { GetLastInputInfo(&mut info) } == 0 {
                return None;
            }
            // Both tick counts wrap after 49.7 days; the difference does not.
            // SAFETY: no arguments.
            let now = unsafe { GetTickCount() };
            Some(Duration::from_millis(u64::from(
                now.wrapping_sub(info.dwTime),
            )))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use parking_lot::Mutex;
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry, wl_seat};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
        self, ExtIdleNotificationV1,
    };
    use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;
    use x11rb::connection::Connection as _;

    use super::Probe;

    /// The compositor says "idle" after this long without input; until it
    /// does, the input counts as now (off by at most this much).
    const WAYLAND_THRESHOLD: Duration = Duration::from_secs(30);

    #[derive(Default)]
    struct WaylandIdle {
        /// Since when the compositor reports idle, `None` while active.
        idled: Option<Instant>,
        /// The connection ended.
        gone: bool,
    }

    /// `ext-idle-notify-v1`: the compositor only tells when input stops and
    /// starts again, so the idle time is the threshold plus the time since
    /// it said idle.
    pub struct Wayland(Arc<Mutex<WaylandIdle>>);

    impl Probe for Wayland {
        fn idle(&mut self) -> Option<Duration> {
            let state = self.0.lock();
            if state.gone {
                return None;
            }
            Some(
                state
                    .idled
                    .map_or(Duration::ZERO, |since| WAYLAND_THRESHOLD + since.elapsed()),
            )
        }
    }

    struct Events(Arc<Mutex<WaylandIdle>>);

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Events {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for Events {
        fn event(
            _: &mut Self,
            _: &wl_seat::WlSeat,
            _: wl_seat::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ExtIdleNotifierV1, ()> for Events {
        fn event(
            _: &mut Self,
            _: &ExtIdleNotifierV1,
            _: <ExtIdleNotifierV1 as Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ExtIdleNotificationV1, ()> for Events {
        fn event(
            events: &mut Self,
            _: &ExtIdleNotificationV1,
            event: ext_idle_notification_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                ext_idle_notification_v1::Event::Idled => {
                    events.0.lock().idled = Some(Instant::now());
                }
                ext_idle_notification_v1::Event::Resumed => events.0.lock().idled = None,
                _ => {}
            }
        }
    }

    /// Its own connection to the compositor and a thread for its events.
    pub fn wayland() -> Option<Wayland> {
        let conn = Connection::connect_to_env().ok()?;
        let (globals, mut queue) = registry_queue_init::<Events>(&conn).ok()?;
        let qh = queue.handle();
        let notifier: ExtIdleNotifierV1 = globals.bind(&qh, 1..=2, ()).ok()?;
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).ok()?;
        let threshold = WAYLAND_THRESHOLD.as_millis() as u32;
        // Version 2 counts input only: a playing video's idle inhibitor
        // does not keep an empty chair online.
        let notification = if notifier.version() >= 2 {
            notifier.get_input_idle_notification(threshold, &seat, &qh, ())
        } else {
            notifier.get_idle_notification(threshold, &seat, &qh, ())
        };
        let shared = Arc::new(Mutex::new(WaylandIdle::default()));
        let mut events = Events(shared.clone());
        std::thread::Builder::new()
            .name("gelabber-idle-wayland".into())
            .spawn(move || {
                // Owned here: dropping them would end the notification.
                let _keep = (conn, notifier, seat, notification);
                while queue.blocking_dispatch(&mut events).is_ok() {}
                events.0.lock().gone = true;
                tracing::debug!("system idle: Wayland connection ended");
            })
            .ok()?;
        Some(Wayland(shared))
    }

    /// The MIT-SCREEN-SAVER extension of the X server.
    pub struct X11 {
        conn: x11rb::rust_connection::RustConnection,
        root: u32,
    }

    impl Probe for X11 {
        fn idle(&mut self) -> Option<Duration> {
            let reply = x11rb::protocol::screensaver::query_info(&self.conn, self.root)
                .ok()?
                .reply()
                .ok()?;
            Some(Duration::from_millis(reply.ms_since_user_input.into()))
        }
    }

    pub fn x11() -> Option<X11> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let mut probe = X11 { conn, root };
        probe.idle()?;
        Some(probe)
    }

    /// GNOME's idle monitor on the session bus (milliseconds).
    pub struct Mutter(zbus::blocking::Connection);

    impl Probe for Mutter {
        fn idle(&mut self) -> Option<Duration> {
            self.0
                .call_method(
                    Some("org.gnome.Mutter.IdleMonitor"),
                    "/org/gnome/Mutter/IdleMonitor/Core",
                    Some("org.gnome.Mutter.IdleMonitor"),
                    "GetIdletime",
                    &(),
                )
                .ok()?
                .body()
                .deserialize::<u64>()
                .ok()
                .map(Duration::from_millis)
        }
    }

    pub fn mutter() -> Option<Mutter> {
        let conn = zbus::blocking::connection::Builder::session()
            .ok()?
            .build()
            .ok()?;
        let mut probe = Mutter(conn);
        probe.idle()?;
        Some(probe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Option<Duration>);

    impl Probe for Fixed {
        fn idle(&mut self) -> Option<Duration> {
            self.0
        }
    }

    #[test]
    fn a_reading_becomes_the_time_of_the_last_input() {
        let last = LastInput::default();
        let now = Instant::now() + Duration::from_secs(600);
        last.read(&mut Fixed(Some(Duration::from_secs(42))), now);
        assert_eq!(last.last_input(), Some(now - Duration::from_secs(42)));
    }

    #[test]
    fn a_source_that_stops_answering_falls_back_to_the_window() {
        let last = LastInput::default();
        let now = Instant::now() + Duration::from_secs(600);
        last.read(&mut Fixed(Some(Duration::ZERO)), now);
        last.read(&mut Fixed(None), now);
        assert_eq!(last.last_input(), None);
    }
}
