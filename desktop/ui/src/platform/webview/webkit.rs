//! WebKitGTK's localStorage: one SQLite database per origin in
//! `localstorage/`, named `<scheme>_<host>_<port or 0>.localstorage`, with
//! `ItemTable(key TEXT, value BLOB)` and the values in UTF-16 (WebKit's
//! `SQLiteStorageArea.cpp`). The databases run in WAL mode; each is copied
//! with its `-wal` and the copy opened, so the WAL is applied without
//! touching the Tauri app's profile.

use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, OpenFlags, types::ValueRef};

use super::{Areas, Failure};

/// The areas of the http(s) origins in `dir` (`localstorage/`).
pub(super) fn areas(dir: &Path) -> Result<Areas, Failure> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Areas::new()),
        Err(e) => return Err(transient(dir, e)),
    };
    let mut areas = Areas::new();
    for entry in listing {
        let path = entry.map_err(|e| transient(dir, e))?.path();
        let Some(origin) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".localstorage"))
            .and_then(origin)
        else {
            continue;
        };
        let entries = read_copy(&path)?;
        if !entries.is_empty() {
            areas.insert(origin, entries);
        }
    }
    Ok(areas)
}

/// `https_chat.example.org_0` → `https://chat.example.org`, as WebKit's
/// `SecurityOriginData::fromDatabaseIdentifier` splits it (first and last
/// `_`); hosts it escaped for the file name (`%xx`) come back unescaped.
fn origin(identifier: &str) -> Option<String> {
    let (scheme, rest) = identifier.split_once('_')?;
    let (host, port) = rest.rsplit_once('_')?;
    if !matches!(scheme, "http" | "https") || host.is_empty() {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    let host = unescape(host)?;
    Some(match port {
        0 => format!("{scheme}://{host}"),
        port => format!("{scheme}://{host}:{port}"),
    })
}

fn unescape(host: &str) -> Option<String> {
    let bytes = host.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn transient(path: &Path, e: io::Error) -> Failure {
    Failure::Transient(format!("{}: {e}", path.display()))
}

/// The database at `path` read through a copy of it and its WAL (not the
/// `-shm` index, which SQLite rebuilds).
fn read_copy(path: &Path) -> Result<std::collections::BTreeMap<String, String>, Failure> {
    static COPIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("gelabber-webview-{}-{n}", std::process::id()));
    let copied = (|| {
        std::fs::create_dir_all(&dir)?;
        let copy = dir.join("copy.localstorage");
        std::fs::copy(path, &copy)?;
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        match std::fs::copy(PathBuf::from(wal), dir.join("copy.localstorage-wal")) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        Ok(copy)
    })();
    let read = copied
        .map_err(|e| transient(path, e))
        .and_then(|copy| query(&copy).map_err(|e| sqlite(path, e)));
    let _ = std::fs::remove_dir_all(&dir);
    read
}

fn query(copy: &Path) -> rusqlite::Result<std::collections::BTreeMap<String, String>> {
    let db = Connection::open_with_flags(
        copy,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut statement = db.prepare("SELECT key, value FROM ItemTable")?;
    let mut rows = statement.query([])?;
    let mut entries = std::collections::BTreeMap::new();
    while let Some(row) = rows.next()? {
        let (ValueRef::Text(key), value) = (row.get_ref(0)?, row.get_ref(1)?) else {
            continue;
        };
        let value = match value {
            ValueRef::Blob(utf16) if utf16.len() % 2 == 0 => String::from_utf16(
                &utf16
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&c| u16::from_ne_bytes(c))
                    .collect::<Vec<_>>(),
            )
            .ok(),
            ValueRef::Text(text) => std::str::from_utf8(text).ok().map(str::to_owned),
            _ => None,
        };
        if let (Ok(key), Some(value)) = (std::str::from_utf8(key), value) {
            entries.insert(key.to_owned(), value);
        }
    }
    Ok(entries)
}

/// Busy, locked and I/O errors may pass by the next start; a file that is
/// no database or no WebKit store will not.
fn sqlite(path: &Path, e: rusqlite::Error) -> Failure {
    let message = format!("{}: {e}", path.display());
    match e.sqlite_error_code() {
        Some(
            ErrorCode::DatabaseBusy
            | ErrorCode::DatabaseLocked
            | ErrorCode::SystemIoFailure
            | ErrorCode::CannotOpen
            | ErrorCode::PermissionDenied
            | ErrorCode::OutOfMemory
            | ErrorCode::DiskFull,
        ) => Failure::Transient(message),
        _ => Failure::Permanent(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/webview/webkitgtk")
    }

    const ORIGIN: &str = "http://127.0.0.1:8123";
    const DRAFT: &str = "gelabber:chat-draft:0f9a4c1e-8f1b-4f3e-9d55-2b7a1c3e5d01:c2d3e4f5-a6b7-4c8d-9e0f-1a2b3c4d5e6f";

    fn copy(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gelabber-webkit-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for entry in std::fs::read_dir(fixture()).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
        }
        dir
    }

    #[test]
    fn the_webkitgtk_store_of_0_8_9_reads_through_its_wal() {
        // The fixture's data lives in the WAL: the main files are one page.
        let main = std::fs::metadata(fixture().join("http_127.0.0.1_8123.localstorage")).unwrap();
        assert_eq!(main.len(), 4096);
        let areas = areas(&fixture()).unwrap();
        assert_eq!(
            areas.keys().collect::<Vec<_>>(),
            ["http://127.0.0.1:8123", "http://127.0.0.1:8124"]
        );
        let area = &areas[ORIGIN];
        let media: serde_json::Value = serde_json::from_str(&area["gelabber.media"]).unwrap();
        assert_eq!(media["state"]["inputGain"], 1.5);
        assert!(area["gelabber.interface"].contains("\"greeting\":\"Grüße\""));
        assert!(area[DRAFT].contains("bis gleich 👋"));
        let gone = "gelabber:chat-draft:0f9a4c1e-8f1b-4f3e-9d55-2b7a1c3e5d01:d4e5f6a7-b8c9-4dae-8f01-2b3c4d5e6f70";
        assert!(!area.contains_key(gone));
        assert_eq!(area.len(), 7, "{:?}", area.keys());
    }

    #[test]
    fn the_webkitgtk_store_stays_as_it_was() {
        let dir = copy("unchanged");
        let listing = || {
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| {
                    let path = e.unwrap().path();
                    (path.clone(), std::fs::read(&path).unwrap())
                })
                .collect();
            files.sort();
            files
        };
        let before = listing();
        areas(&dir).unwrap();
        assert_eq!(listing(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_names_give_http_origins_only() {
        assert_eq!(
            origin("https_chat.example.org_0").as_deref(),
            Some("https://chat.example.org")
        );
        assert_eq!(
            origin("http_127.0.0.1_8123").as_deref(),
            Some("http://127.0.0.1:8123")
        );
        assert_eq!(
            origin("http_%5B%3A%3A1%5D_8123").as_deref(),
            Some("http://[::1]:8123")
        );
        assert_eq!(origin("tauri_localhost_0"), None);
        assert_eq!(origin("http_host"), None);
    }

    #[test]
    fn a_file_that_is_no_database_is_a_permanent_failure() {
        let dir = copy("damaged");
        std::fs::write(
            dir.join("http_127.0.0.1_8123.localstorage"),
            vec![7u8; 8192],
        )
        .unwrap();
        std::fs::remove_file(dir.join("http_127.0.0.1_8123.localstorage-wal")).unwrap();
        assert!(matches!(areas(&dir), Err(Failure::Permanent(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
