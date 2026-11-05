//! The app's config directory: the one the Tauri app uses
//! (`app_config_dir()` of the bundle identifier), so both share
//! `desktop.json`. `GELABBER_CONFIG_DIR` replaces it (tests, parity runs).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// desktop/app/tauri.conf.json `identifier`.
pub const IDENTIFIER: &str = "io.github.till-und-scholler-ai-slop.gelabber";

/// `<config>/io.github.till-und-scholler-ai-slop.gelabber`, per platform as
/// Tauri's `app_config_dir` (dirs::config_dir): %APPDATA% on Windows,
/// Application Support on macOS, `$XDG_CONFIG_HOME` or `~/.config` else.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GELABBER_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    // Tests that change a setting must not write into the user's own app.
    if cfg!(test) {
        let dir = format!("gelabber-ui-tests-{}", std::process::id());
        return Some(std::env::temp_dir().join(dir));
    }
    base_dir().map(|base| base.join(IDENTIFIER))
}

/// The directory the per-app directories are in (`%APPDATA%`, `~/.config`).
pub(super) fn base_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if cfg!(windows) {
        var("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
}

/// A file in the config directory.
pub fn file(name: &str) -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(name))
}

/// Writes whole or not at all (a half-written file would lose everything
/// in it). `private` files are readable by their owner only (0600).
pub fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // One partial file per write: two writers never mix their bytes.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{}-{n}.partial", std::process::id()));
    let partial = path.with_file_name(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let written = options.open(&partial).and_then(|mut out| {
        // A partial file left by an older run may have wider rights.
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::PermissionsExt;
            out.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        out.write_all(bytes)?;
        out.sync_all()
    });
    let done = written.and_then(|()| std::fs::rename(&partial, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    done
}

/// A JSON file of the config directory; `Null` when missing or unreadable.
pub fn read_json(name: &str) -> serde_json::Value {
    file(name)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null)
}

pub fn write_json(name: &str, value: &serde_json::Value, private: bool) -> io::Result<()> {
    let path = file(name).ok_or_else(|| io::Error::other("no config directory"))?;
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(&path, &bytes, private)
}

#[cfg(test)]
pub(crate) mod testing {
    use parking_lot::{Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());

    /// Points `GELABBER_CONFIG_DIR` at a fresh directory for one test; the
    /// lock keeps tests that read the variable apart.
    pub struct TempConfig {
        pub dir: std::path::PathBuf,
        _guard: MutexGuard<'static, ()>,
    }

    impl TempConfig {
        pub fn new(name: &str) -> Self {
            let guard = LOCK.lock();
            let dir = std::env::temp_dir().join(format!(
                "gelabber-platform-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            // SAFETY: tests that touch the environment hold `LOCK`.
            unsafe { std::env::set_var("GELABBER_CONFIG_DIR", &dir) };
            Self { dir, _guard: guard }
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            // SAFETY: as in `new`.
            unsafe { std::env::remove_var("GELABBER_CONFIG_DIR") };
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_are_the_owners_only_and_replace_whole() {
        let config = testing::TempConfig::new("paths");
        assert_eq!(config_dir().as_deref(), Some(config.dir.as_path()));
        let path = file("secret").unwrap();
        write_atomic(&path, b"one", true).unwrap();
        write_atomic(&path, b"two", true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let left = std::fs::read_dir(&config.dir).unwrap().flatten();
        assert!(
            left.into_iter()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".partial"))
        );
    }
}
