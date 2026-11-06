//! One app per user: a second start tells the running one to show its
//! window (it may sit hidden in the tray) and ends before it opens one, as
//! tauri-plugin-single-instance does for the Tauri app. A Unix socket in the
//! runtime directory on Unix, a loopback port with a secret on Windows.

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

const SHOW: &str = "show";
const ANSWER: &str = "ok";
const WAIT: Duration = Duration::from_secs(2);

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

/// A line from a second start; answers and wakes on `expect`.
fn serve<S: std::io::Read + Write>(stream: S, expect: &str, show: &AtomicBool, wake: &Wake) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_ok() && line.trim_end() == expect {
        show.store(true, Ordering::Release);
        let _ = writeln!(reader.get_mut(), "{ANSWER}");
        if let Some(wake) = wake.lock().clone() {
            wake();
        }
    }
}

/// Sends the request; true when a running app answered.
fn ask<S: std::io::Read + Write>(stream: S, line: &str) -> bool {
    let mut reader = BufReader::new(stream);
    if writeln!(reader.get_mut(), "{line}").is_err() {
        return false;
    }
    let mut answer = String::new();
    reader.read_line(&mut answer).is_ok() && answer.trim_end() == ANSWER
}

/// The socket's name: per user, and per config directory so separate
/// profiles (`GELABBER_CONFIG_DIR`) can run side by side.
fn name() -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    super::paths::config_dir().hash(&mut hasher);
    format!("gelabber-{:016x}", hasher.finish())
}

#[cfg(unix)]
pub fn claim() -> Claim {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    claim_at(&dir.join(format!("{}.sock", name())))
}

#[cfg(unix)]
fn claim_at(path: &std::path::Path) -> Claim {
    use std::os::unix::net::{UnixListener, UnixStream};
    let listen = |path: &std::path::Path| UnixListener::bind(path);
    let listener = match listen(path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if let Ok(stream) = UnixStream::connect(path) {
                let _ = stream.set_read_timeout(Some(WAIT));
                let _ = stream.set_write_timeout(Some(WAIT));
                if ask(stream, SHOW) {
                    return Claim::Running;
                }
            }
            // Left by an app that ended without cleaning up.
            let _ = std::fs::remove_file(path);
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
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(WAIT));
                serve(stream, SHOW, &show, &wake);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("single instance: {e}");
    }
    Claim::First(instance)
}

/// The secret a second start sends: kept in the config directory.
#[cfg(any(windows, test))]
fn claim_tcp(file: &std::path::Path) -> Claim {
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    if let Ok(text) = std::fs::read_to_string(file)
        && let Some((port, secret)) = text.trim().split_once(' ')
        && let Ok(port) = port.parse::<u16>()
        && let Ok(stream) = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), WAIT)
    {
        let _ = stream.set_read_timeout(Some(WAIT));
        if ask(stream, &format!("{SHOW} {secret}")) {
            return Claim::Running;
        }
    }
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(l) => l,
        Err(e) => return unguarded(e),
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else {
        return unguarded(std::io::Error::other("no port"));
    };
    let secret = uuid::Uuid::new_v4().simple().to_string();
    if let Err(e) = super::paths::write_atomic(file, format!("{port} {secret}").as_bytes(), true) {
        return unguarded(e);
    }
    let (instance, show, wake) = Instance::new();
    let expect = format!("{SHOW} {secret}");
    let spawned = std::thread::Builder::new()
        .name("gelabber-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(WAIT));
                serve(stream, &expect, &show, &wake);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("single instance: {e}");
    }
    Claim::First(instance)
}

#[cfg(windows)]
pub fn claim() -> Claim {
    match super::paths::file(&format!("{}.instance", name())) {
        Some(file) => claim_tcp(&file),
        None => unguarded(std::io::Error::other("no config directory")),
    }
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

    fn wait_for(instance: &Instance) -> bool {
        for _ in 0..200 {
            if instance.take_show() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
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
        let woken = Arc::new(AtomicBool::new(false));
        let flag = woken.clone();
        first.set_wake(Arc::new(move || flag.store(true, Ordering::Release)));
        assert!(matches!(claim_at(&path), Claim::Running));
        assert!(wait_for(&first));
        assert!(woken.load(Ordering::Acquire));
        assert!(!first.take_show());
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
        assert!(matches!(claim_tcp(&file), Claim::Running));
        assert!(wait_for(&first));
        // A wrong secret is not obeyed.
        let text = std::fs::read_to_string(&file).unwrap();
        let port: u16 = text.split(' ').next().unwrap().parse().unwrap();
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        assert!(!ask(stream, "show nope"));
        assert!(!first.take_show());
        let _ = std::fs::remove_file(&file);
    }
}
