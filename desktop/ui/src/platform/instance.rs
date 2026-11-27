//! One app per user: a second start tells the running one to show its
//! window (it may sit hidden in the tray) and ends before it opens one, as
//! tauri-plugin-single-instance does for the Tauri app. A Unix socket in the
//! runtime directory on Unix, a loopback port with a secret on Windows
//! (where a named mutex tells that an app runs).

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

const SHOW: &str = "show";
const ANSWER: &str = "ok";
const WAIT: Duration = Duration::from_secs(2);
/// Longer than any line of ours (`show` and a 32-digit secret).
const MAX_LINE: usize = 64;

type Wake = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

/// The running app's side.
pub struct Instance {
    show: Arc<AtomicBool>,
    wake: Wake,
    #[cfg(unix)]
    path: std::path::PathBuf,
}

pub enum Claim {
    /// This is the only app; keep the instance for its lifetime.
    First(Instance),
    /// Another one runs and was told to show itself.
    Running,
}

impl Instance {
    /// Called when a second start asks; from then on wakes the UI.
    pub fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake.lock() = Some(wake);
    }

    /// True once after a second start asked to be shown.
    pub fn take_show(&self) -> bool {
        self.show.swap(false, Ordering::AcqRel)
    }

    fn new() -> (Self, Arc<AtomicBool>, Wake) {
        let show = Arc::new(AtomicBool::new(false));
        let wake: Wake = Arc::default();
        (
            Self {
                show: show.clone(),
                wake: wake.clone(),
                #[cfg(unix)]
                path: Default::default(),
            },
            show,
            wake,
        )
    }
}

#[cfg(unix)]
impl Drop for Instance {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A stream whose reads can be given a timeout.
trait Timed: Read + Write {
    fn set_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
}

impl Timed for std::net::TcpStream {
    fn set_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)?;
        self.set_write_timeout(timeout)
    }
}

#[cfg(unix)]
impl Timed for std::os::unix::net::UnixStream {
    fn set_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)?;
        self.set_write_timeout(timeout)
    }
}

/// One line of at most [`MAX_LINE`] bytes, whole before `deadline`: the
/// timeout covers the line, not each read, so trickled bytes cannot hold it.
fn read_line<S: Timed>(stream: &mut S, deadline: Instant) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())?;
        stream.set_timeout(Some(left)).ok()?;
        match stream.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) if byte[0] == b'\n' => return String::from_utf8(line).ok(),
            Ok(_) if line.len() >= MAX_LINE => return None,
            Ok(_) => line.push(byte[0]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// A line from a second start; answers and wakes on `expect`.
fn serve<S: Timed>(mut stream: S, expect: &str, show: &AtomicBool, wake: &Wake) {
    let line = read_line(&mut stream, Instant::now() + WAIT);
    if line.as_deref().map(str::trim_end) == Some(expect) {
        show.store(true, Ordering::Release);
        let _ = writeln!(stream, "{ANSWER}");
        if let Some(wake) = wake.lock().clone() {
            wake();
        }
    }
}

/// Each connection on a thread of its own: a slow one keeps no other out.
fn serve_all<S: Timed + Send + 'static>(
    incoming: impl Iterator<Item = std::io::Result<S>>,
    expect: String,
    show: Arc<AtomicBool>,
    wake: Wake,
) {
    let expect: Arc<str> = expect.into();
    for stream in incoming.flatten() {
        let (expect, show, wake) = (expect.clone(), show.clone(), wake.clone());
        let spawned = std::thread::Builder::new()
            .name("gelabber-instance-ask".into())
            .spawn(move || serve(stream, &expect, &show, &wake));
        if let Err(e) = spawned {
            tracing::warn!("single instance: {e}");
        }
    }
}

/// Sends the request; true when a running app answered.
fn ask<S: Timed>(mut stream: S, line: &str) -> bool {
    let deadline = Instant::now() + WAIT;
    if stream.set_timeout(Some(WAIT)).is_err() || writeln!(stream, "{line}").is_err() {
        return false;
    }
    read_line(&mut stream, deadline)
        .as_deref()
        .map(str::trim_end)
        == Some(ANSWER)
}

/// The socket's name: per user, and per config directory so separate
/// profiles (`GELABBER_CONFIG_DIR`) can run side by side.
fn name() -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    super::paths::config_dir().hash(&mut hasher);
    format!("gelabber-{:016x}", hasher.finish())
}

/// In the runtime directory, else the config directory: both the user's
/// own, unlike a fixed name in /tmp that another user could take first.
#[cfg(unix)]
pub fn claim() -> Claim {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|d| d.is_absolute() && d.is_dir())
        .or_else(super::paths::config_dir);
    let Some(dir) = dir else {
        return unguarded(std::io::Error::other("no runtime or config directory")); // i18n-ok
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return unguarded(e);
    }
    claim_at(&dir.join(format!("{}.sock", name())))
}

#[cfg(unix)]
fn claim_at(path: &std::path::Path) -> Claim {
    use std::os::unix::net::{UnixListener, UnixStream};
    let listen = |path: &std::path::Path| UnixListener::bind(path);
    let listener = match listen(path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            match UnixStream::connect(path) {
                Ok(stream) => {
                    if ask(stream, SHOW) {
                        return Claim::Running;
                    }
                    // An app listens but does not answer: its socket stays.
                    return unguarded(std::io::Error::other("the running app does not answer")); // i18n-ok
                }
                // Left by an app that ended without cleaning up.
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                    let _ = std::fs::remove_file(path);
                }
                Err(e) => return unguarded(e),
            }
            match listen(path) {
                Ok(l) => l,
                Err(e) => return unguarded(e),
            }
        }
        Err(e) => return unguarded(e),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    let (mut instance, show, wake) = Instance::new();
    instance.path = path.to_owned();
    let spawned = std::thread::Builder::new()
        .name("gelabber-instance".into())
        .spawn(move || serve_all(listener.incoming(), SHOW.into(), show, wake));
    if let Err(e) = spawned {
        tracing::warn!("single instance: {e}");
    }
    Claim::First(instance)
}

/// The secret a second start sends: kept in the config directory.
#[cfg(any(windows, test))]
fn claim_tcp(file: &std::path::Path) -> Claim {
    if ask_tcp(file) {
        return Claim::Running;
    }
    listen_tcp(file)
}

/// Asks the app whose port and secret `file` holds to show itself.
#[cfg(any(windows, test))]
fn ask_tcp(file: &std::path::Path) -> bool {
    use std::net::{Ipv4Addr, TcpStream};
    if let Ok(text) = std::fs::read_to_string(file)
        && let Some((port, secret)) = text.trim().split_once(' ')
        && let Ok(port) = port.parse::<u16>()
        && let Ok(stream) = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), WAIT)
    {
        return ask(stream, &format!("{SHOW} {secret}"));
    }
    false
}

#[cfg(any(windows, test))]
fn listen_tcp(file: &std::path::Path) -> Claim {
    use std::net::{Ipv4Addr, TcpListener};
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(l) => l,
        Err(e) => return unguarded(e),
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else {
        return unguarded(std::io::Error::other("no port")); // i18n-ok
    };
    let secret = uuid::Uuid::new_v4().simple().to_string();
    if let Err(e) = super::paths::write_atomic(file, format!("{port} {secret}").as_bytes(), true) {
        return unguarded(e);
    }
    let (instance, show, wake) = Instance::new();
    let expect = format!("{SHOW} {secret}");
    let spawned = std::thread::Builder::new()
        .name("gelabber-instance".into())
        .spawn(move || serve_all(listener.incoming(), expect, show, wake));
    if let Err(e) = spawned {
        tracing::warn!("single instance: {e}");
    }
    Claim::First(instance)
}

/// How long a start keeps asking an app that holds the guard but does not
/// answer (one on its way out); each ask takes up to [`WAIT`].
#[cfg(windows)]
const WAIT_FOR_RUNNING: Duration = Duration::from_secs(6);

/// What a start found while another app held the guard.
#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
enum Waited {
    /// It answered and shows itself.
    Shown,
    /// It ended; the guard is this start's now.
    Free,
    /// Neither, until the deadline.
    Silent,
}

/// Asks with `ask` until `deadline`; between tries `free` looks whether the
/// other app ended and left the guard.
#[cfg(any(windows, test))]
fn wait_for_running(
    mut ask: impl FnMut() -> bool,
    mut free: impl FnMut() -> bool,
    deadline: Instant,
    pause: Duration,
) -> Waited {
    loop {
        if ask() {
            return Waited::Shown;
        }
        if free() {
            return Waited::Free;
        }
        if Instant::now() + pause >= deadline {
            return Waited::Silent;
        }
        std::thread::sleep(pause);
    }
}

/// The named mutex decides whether an app runs, the port only carries
/// "show": an app that is slow to answer does not get a second one beside
/// it. One that never answers is named in a message box, not passed over silently.
#[cfg(windows)]
pub fn claim() -> Claim {
    let Some(file) = super::paths::file(&format!("{}.instance", name())) else {
        return unguarded(std::io::Error::other("no config directory")); // i18n-ok
    };
    match first_by_mutex(&name()) {
        Ok(true) => listen_tcp(&file),
        Ok(false) => {
            let waited = wait_for_running(
                || ask_tcp(&file),
                || first_by_mutex(&name()).unwrap_or(false),
                Instant::now() + WAIT_FOR_RUNNING,
                Duration::from_millis(500),
            );
            match waited {
                Waited::Shown => Claim::Running,
                Waited::Free => listen_tcp(&file),
                Waited::Silent => {
                    tracing::warn!("single instance: the running app does not answer");
                    not_responding();
                    Claim::Running
                }
            }
        }
        Err(e) => {
            tracing::warn!("single instance: {e}");
            claim_tcp(&file)
        }
    }
}

/// Tells the user why nothing opens: a GUI process has no console to say it.
#[cfg(windows)]
fn not_responding() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MessageBoxW,
    };
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    // This start ends here, before main() chose the language: the running
    // app's, as it would start.
    crate::setup::start_locale();
    let text = wide(gelabber_i18n::app::instance_not_responding());
    let caption = wide("Gelabber");
    // No owner window: in front, or it may open behind other windows.
    // SAFETY: NUL-terminated UTF-16 strings that outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND,
        )
    };
}

/// Creates the session's named mutex; true when no app held it, and its
/// handle then stays open for the life of this process. When another app
/// holds it, this start's handle is closed again, so the mutex goes with
/// that app.
#[cfg(windows)]
fn first_by_mutex(name: &str) -> std::io::Result<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let wide: Vec<u16> = format!("Local\\{name}").encode_utf16().chain([0]).collect();
    // SAFETY: a NUL-terminated name that outlives the call; no attributes.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: reads this thread's last error, set by CreateMutexW.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: the handle CreateMutexW just returned, closed once.
        unsafe { CloseHandle(handle) };
        return Ok(false);
    }
    Ok(true)
}

#[cfg(not(any(unix, windows)))]
pub fn claim() -> Claim {
    unguarded(std::io::Error::other("unsupported"))
}

/// Without the guard the app still starts; a second one may then run too.
fn unguarded(e: std::io::Error) -> Claim {
    tracing::warn!("single instance: {e}");
    Claim::First(Instance::new().0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// The wake-ups of `instance`, as the UI gets them.
    fn wake_ups(instance: &Instance) -> mpsc::Receiver<bool> {
        let (woke, wake_ups) = mpsc::channel();
        let show = instance.show.clone();
        instance.set_wake(Arc::new(move || {
            let _ = woke.send(show.load(Ordering::Acquire));
        }));
        wake_ups
    }

    /// Waits for the wake-up a second start causes; the frame it asks for
    /// must find `show` set, once (the UI shows the window then).
    fn shown(instance: &Instance, wake_ups: &mpsc::Receiver<bool>) {
        let set = wake_ups
            .recv_timeout(Duration::from_secs(30))
            .expect("no wake-up within 30 s");
        assert!(set, "woken before show was set");
        assert!(instance.take_show(), "woken, but nothing to show");
        assert!(!instance.take_show());
    }

    #[cfg(unix)]
    #[test]
    fn a_second_start_shows_the_first_over_the_socket() {
        let path = std::env::temp_dir().join(format!(
            "gelabber-test-{}.sock",
            uuid::Uuid::new_v4().simple()
        ));
        let Claim::First(first) = claim_at(&path) else {
            panic!("nothing runs yet");
        };
        let wake_ups = wake_ups(&first);
        assert!(matches!(claim_at(&path), Claim::Running));
        shown(&first, &wake_ups);
        drop(first);
        // A socket file left behind does not block the next start.
        std::fs::write(&path, b"").ok();
        let _ = std::fs::remove_file(&path);
        let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(stale);
        assert!(matches!(claim_at(&path), Claim::First(_)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_second_start_shows_the_first_over_loopback_with_the_secret() {
        let file = std::env::temp_dir().join(format!(
            "gelabber-test-{}.instance",
            uuid::Uuid::new_v4().simple()
        ));
        let Claim::First(first) = claim_tcp(&file) else {
            panic!("nothing runs yet");
        };
        let wake_ups = wake_ups(&first);
        assert!(matches!(claim_tcp(&file), Claim::Running));
        shown(&first, &wake_ups);
        // A wrong secret is not obeyed.
        let text = std::fs::read_to_string(&file).unwrap();
        let port: u16 = text.split(' ').next().unwrap().parse().unwrap();
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        // It ends once the app closed the connection, past its decision.
        assert!(!ask(stream, "show nope"));
        assert!(!first.take_show());
        assert!(wake_ups.try_recv().is_err(), "a wrong secret woke the UI");
        let _ = std::fs::remove_file(&file);
    }

    /// Anyone on the machine reaches the loopback port: a client that sends
    /// part of a line and then nothing must not keep a second start out.
    #[test]
    fn a_client_that_never_ends_its_line_keeps_nobody_out() {
        let file = std::env::temp_dir().join(format!(
            "gelabber-test-{}.instance",
            uuid::Uuid::new_v4().simple()
        ));
        let Claim::First(first) = claim_tcp(&file) else {
            panic!("nothing runs yet");
        };
        let wake_ups = wake_ups(&first);
        let text = std::fs::read_to_string(&file).unwrap();
        let port: u16 = text.split(' ').next().unwrap().parse().unwrap();
        // Connected first (accepted in order), part of a line, then silent.
        let mut held = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        held.write_all(b"x").unwrap();
        assert!(matches!(claim_tcp(&file), Claim::Running));
        shown(&first, &wake_ups);
        // Answered while the held client is still connected: an app that
        // served one client at a time would first have given up on it and
        // closed its connection (a read would then end, Ok(0)). Only if all
        // this took longer than the app's WAIT could it fail on sound code.
        held.set_nonblocking(true).unwrap();
        let read = held.read(&mut [0u8; 1]);
        assert!(
            matches!(&read, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "the held client was let go before the second start was answered: {read:?}"
        );
        let _ = std::fs::remove_file(&file);
    }

    /// A client that sends a byte every 25 ms would hold a reader whose
    /// timeout covered each read until the line limit ([`MAX_LINE`] bytes,
    /// over 1.6 s); the deadline ends it long before.
    #[test]
    fn a_line_is_read_whole_before_the_deadline_or_not_at_all() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let (count, stop) = (sent.clone(), done.clone());
        let writer = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            for _ in 0..2 * MAX_LINE {
                if stop.load(Ordering::Acquire) || stream.write_all(b"x").is_err() {
                    return;
                }
                count.fetch_add(1, Ordering::AcqRel);
                // A slow client, not a synchronisation.
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        let (mut stream, _) = listener.accept().unwrap();
        let deadline = Instant::now() + Duration::from_millis(300);
        assert_eq!(read_line(&mut stream, deadline), None);
        let sent_by_then = sent.load(Ordering::Acquire);
        done.store(true, Ordering::Release);
        writer.join().unwrap();
        assert!(
            sent_by_then < MAX_LINE,
            "the reader waited for {sent_by_then} bytes, past its deadline"
        );
    }

    /// Another app holds the guard: asked again for a while; it may answer,
    /// or end and leave the guard free, or neither.
    #[test]
    fn a_start_waits_for_the_running_app_before_giving_up() {
        let pause = Duration::from_millis(5);
        let window = Duration::from_millis(200);
        let soon = || Instant::now() + window;
        let mut tries = 0;
        let answers = wait_for_running(
            || {
                tries += 1;
                tries == 3
            },
            || false,
            soon(),
            pause,
        );
        assert_eq!(answers, Waited::Shown);
        let mut looks = 0;
        let ends = wait_for_running(
            || false,
            || {
                looks += 1;
                looks == 2
            },
            soon(),
            pause,
        );
        assert_eq!(ends, Waited::Free);
        // Silent only once the next pause would pass the deadline.
        let deadline = soon();
        let mut asked: u128 = 0;
        let silent = wait_for_running(
            || {
                asked += 1;
                false
            },
            || false,
            deadline,
            pause,
        );
        assert_eq!(silent, Waited::Silent);
        assert!(Instant::now() + pause >= deadline, "gave up early");
        assert!(asked > 1, "asked only once");
        // A pause between tries: no more of them fit before the deadline.
        let most = window.as_millis() / pause.as_millis() + 1;
        assert!(asked <= most, "asked {asked} times, past the deadline");
    }

    /// Only a socket nobody listens on is stale; one whose app is slow to
    /// answer stays, and this start runs without the guard.
    #[cfg(unix)]
    #[test]
    fn a_socket_that_does_not_answer_is_not_taken_over() {
        let path = std::env::temp_dir().join(format!(
            "gelabber-test-{}.sock",
            uuid::Uuid::new_v4().simple()
        ));
        let busy = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let Claim::First(second) = claim_at(&path) else {
            panic!("nobody answered");
        };
        assert!(second.path.as_os_str().is_empty(), "no socket of its own");
        drop(second);
        // The first app's socket is still its own.
        assert!(std::os::unix::net::UnixStream::connect(&path).is_ok());
        drop(busy);
        let _ = std::fs::remove_file(&path);
    }
}
