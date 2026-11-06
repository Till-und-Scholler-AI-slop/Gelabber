//! Chromium's localStorage (`Local Storage/leveldb`, as WebView2 keeps it),
//! read in place and never written: no LOCK is taken and nothing is
//! compacted, so the Tauri app's profile stays as it was.
//!
//! LevelDB keeps the store in tables (`.ldb`, `.sst`) and write-ahead logs
//! (`.log`); the MANIFEST that `CURRENT` names lists the live tables and the
//! oldest live log. Every entry carries a sequence number, so the newest
//! entry of a key wins whatever file it is in, and a deletion hides the key.
//! Formats: LevelDB's doc/log_format.md and doc/table_format.md; Chromium's
//! `leveldb_local_storage_leveldb.cc` (`"_" <origin> "\0" <key>`) and
//! `cached_storage_area.cc` (a format byte, then UTF-16 or Latin-1).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use super::{Areas, Failure};

const BLOCK: usize = 32 * 1024;
const HEADER: usize = 7;
const FOOTER: usize = 48;
const TABLE_MAGIC: u64 = 0xdb47_7524_8b80_fb57;
/// What `DomStorageDatabaseLevelDB` writes under `VERSION` and reads back.
const VERSION: &[u8] = b"1";
/// More than one value of a localStorage area can be (its quota is 10 MiB
/// of UTF-16); a block that claims more is damaged.
const MAX_BLOCK: usize = 64 << 20;

/// Per key the newest entry seen: its sequence number and value (`None`:
/// deleted).
type Merge = BTreeMap<Vec<u8>, (u64, Option<Vec<u8>>)>;
/// Live keys and values.
type Entries = BTreeMap<Vec<u8>, Vec<u8>>;

/// What a read went through, for the tests that keep the fixture honest.
#[derive(Debug, Default)]
pub(super) struct Stats {
    pub tables: usize,
    pub logs: usize,
    pub snappy_blocks: usize,
}

/// The localStorage areas of the store in `dir`: origin to key to value.
pub(super) fn areas(dir: &Path) -> Result<Areas, Failure> {
    read(dir).map(|(areas, _)| areas)
}

pub(super) fn read(dir: &Path) -> Result<(Areas, Stats), Failure> {
    let (entries, stats) = entries(dir)?;
    let mut areas = Areas::new();
    if entries.is_empty() {
        return Ok((areas, stats));
    }
    match entries.get(b"VERSION".as_slice()) {
        Some(version) if version.as_slice() == VERSION => {}
        version => {
            return Err(Failure::Permanent(format!(
                "{}: localStorage schema {:?}, not 1",
                dir.display(),
                version.map(|v| String::from_utf8_lossy(v).into_owned())
            )));
        }
    }
    for (key, value) in entries {
        let Some(rest) = key.strip_prefix(b"_") else {
            continue;
        };
        let Some(nul) = rest.iter().position(|&b| b == 0) else {
            continue;
        };
        let (Ok(origin), Some(key), Some(value)) = (
            std::str::from_utf8(&rest[..nul]),
            decode_dom(&rest[nul + 1..]),
            decode_dom(&value),
        ) else {
            continue;
        };
        areas
            .entry(origin.to_owned())
            .or_default()
            .insert(key, value);
    }
    Ok((areas, stats))
}

/// A string as Chromium's `CachedStorageArea` stores it: format byte 0,
/// then UTF-16 in the machine's order (x86: little-endian); 1, Latin-1.
pub(super) fn decode_dom(bytes: &[u8]) -> Option<String> {
    match bytes.split_first()? {
        (0, utf16) if utf16.len() % 2 == 0 => String::from_utf16(
            &utf16
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .collect::<Vec<_>>(),
        )
        .ok(),
        (1, latin1) => Some(latin1.iter().map(|&b| char::from(b)).collect()),
        _ => None,
    }
}

/// Every live key and value of the LevelDB in `dir`; empty when there is
/// none.
fn entries(dir: &Path) -> Result<(Entries, Stats), Failure> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(transient(dir, e)),
    };
    let mut files = BTreeMap::new();
    for entry in listing {
        let path = entry.map_err(|e| transient(dir, e))?.path();
        if let Some(file) = numbered(&path) {
            files.insert(file, path);
        }
    }
    let (tables, logs): (Vec<_>, Vec<_>) = match live_files(dir)? {
        Some(live) => {
            let tables = live.tables.iter().map(|number| {
                [Kind::Table, Kind::OldTable]
                    .iter()
                    .find_map(|&kind| files.get(&(*number, kind)))
                    .cloned()
                    // Compaction replaced it after CURRENT was read: next start.
                    .ok_or_else(|| {
                        Failure::Transient(format!("{}: table {number} is gone", dir.display()))
                    })
            });
            let tables = tables.collect::<Result<_, _>>()?;
            let logs = files
                .iter()
                .filter(|((number, kind), _)| {
                    *kind == Kind::Log && (*number >= live.log || *number == live.prev_log)
                })
                .map(|(_, path)| path.clone())
                .collect();
            (tables, logs)
        }
        // Without a MANIFEST every file counts; an obsolete table could
        // bring back a key deleted since.
        None => {
            let (tables, logs): (Vec<_>, Vec<_>) =
                files.iter().partition(|((_, kind), _)| *kind != Kind::Log);
            let paths = |files: Vec<(&(u64, Kind), &PathBuf)>| {
                files.into_iter().map(|(_, path)| path.clone()).collect()
            };
            (paths(tables), paths(logs))
        }
    };
    let mut merge = Merge::new();
    let mut stats = Stats::default();
    for table in &tables {
        stats.snappy_blocks += table_entries(table, &read_file(table)?, &mut merge)?;
        stats.tables += 1;
    }
    for log in &logs {
        for record in log_records(&read_file(log)?, Damage::SkipBlock) {
            write_batch(&record, &mut merge);
        }
        stats.logs += 1;
    }
    let entries = merge
        .into_iter()
        .filter_map(|(key, (_, value))| Some((key, value?)))
        .collect();
    Ok((entries, stats))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Table,
    OldTable,
    Log,
}

/// `000005.ldb` and the like: LevelDB's own files with their number.
fn numbered(path: &Path) -> Option<(u64, Kind)> {
    let name = path.file_name()?.to_str()?;
    let (number, extension) = name.split_once('.')?;
    let kind = match extension {
        "ldb" => Kind::Table,
        "sst" => Kind::OldTable,
        "log" => Kind::Log,
        _ => return None,
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((number.parse().ok()?, kind))
}

fn transient(path: &Path, e: io::Error) -> Failure {
    Failure::Transient(format!("{}: {e}", path.display()))
}

/// The whole file. std opens it sharing read, write and delete (Windows),
/// so Chromium, which shares read and write, keeps working on it.
fn read_file(path: &Path) -> Result<Vec<u8>, Failure> {
    std::fs::read(path).map_err(|e| transient(path, e))
}

struct Live {
    tables: BTreeSet<u64>,
    log: u64,
    prev_log: u64,
}

/// The MANIFEST's view: live tables and the oldest live log. `None` when
/// CURRENT or the MANIFEST is missing or does not read.
fn live_files(dir: &Path) -> Result<Option<Live>, Failure> {
    let missing_or = |path: &Path, e: io::Error| match e.kind() {
        io::ErrorKind::NotFound => Ok(None),
        _ => Err(transient(path, e)),
    };
    let current_path = dir.join("CURRENT");
    let current = match std::fs::read(&current_path) {
        Ok(current) => current,
        Err(e) => return missing_or(&current_path, e),
    };
    let Some(name) = std::str::from_utf8(&current)
        .ok()
        .map(str::trim_end)
        .filter(|n| n.starts_with("MANIFEST-") && !n.contains(['/', '\\']))
    else {
        return Ok(None);
    };
    let manifest_path: PathBuf = dir.join(name);
    let manifest = match std::fs::read(&manifest_path) {
        Ok(manifest) => manifest,
        Err(e) => return missing_or(&manifest_path, e),
    };
    let mut live = Live {
        tables: BTreeSet::new(),
        log: 0,
        prev_log: 0,
    };
    for edit in log_records(&manifest, Damage::Stop) {
        if version_edit(&edit, &mut live).is_none() {
            return Ok(None);
        }
    }
    Ok(Some(live))
}

/// One VersionEdit of the MANIFEST (LevelDB's version_edit.cc) applied to
/// `live`; `None` for a tag it does not know.
fn version_edit(edit: &[u8], live: &mut Live) -> Option<()> {
    let mut pos = 0;
    while pos < edit.len() {
        match varint(edit, &mut pos)? {
            // Comparator name.
            1 => {
                slice(edit, &mut pos)?;
            }
            2 => live.log = varint(edit, &mut pos)?,
            // Next file number, last sequence.
            3 | 4 => {
                varint(edit, &mut pos)?;
            }
            // Compaction pointer: level, key.
            5 => {
                varint(edit, &mut pos)?;
                slice(edit, &mut pos)?;
            }
            6 => {
                varint(edit, &mut pos)?;
                live.tables.remove(&varint(edit, &mut pos)?);
            }
            7 => {
                varint(edit, &mut pos)?;
                live.tables.insert(varint(edit, &mut pos)?);
                varint(edit, &mut pos)?;
                slice(edit, &mut pos)?;
                slice(edit, &mut pos)?;
            }
            9 => live.prev_log = varint(edit, &mut pos)?,
            _ => return None,
        }
    }
    Some(())
}

/// What a damaged record of a log costs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Damage {
    /// The rest of its block, as LevelDB's log reader recovers a write-ahead
    /// log: a log reused after a kill holds the next session's writes
    /// behind a torn record.
    SkipBlock,
    /// The rest of the file: LevelDB does not open a store whose MANIFEST
    /// is damaged, and a torn tail of it is the edit in flight.
    Stop,
}

/// The records of a log file (32 KiB blocks of fragments); a torn tail
/// loses the write in flight when the app was killed, as in LevelDB.
fn log_records(bytes: &[u8], damage: Damage) -> Vec<Vec<u8>> {
    let mut records = Vec::new();
    let mut partial: Option<Vec<u8>> = None;
    for block in bytes.chunks(BLOCK) {
        let mut pos = 0;
        while pos + HEADER <= block.len() {
            let header = &block[pos..pos + HEADER];
            let checksum = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let length = usize::from(u16::from_le_bytes([header[4], header[5]]));
            let kind = header[6];
            // Zeroes: the rest of the block is padding (preallocated space).
            if kind == 0 && length == 0 {
                partial = None;
                break;
            }
            let data = block
                .get(pos + HEADER..pos + HEADER + length)
                .filter(|data| unmask(checksum) == crc32c(&[&[kind], data]));
            let Some(data) = data else {
                if damage == Damage::Stop {
                    return records;
                }
                partial = None;
                break;
            };
            match kind {
                1 => {
                    partial = None;
                    records.push(data.to_vec());
                }
                2 => partial = Some(data.to_vec()),
                3 => {
                    if let Some(partial) = &mut partial {
                        partial.extend_from_slice(data);
                    }
                }
                4 => {
                    if let Some(mut whole) = partial.take() {
                        whole.extend_from_slice(data);
                        records.push(whole);
                    }
                }
                // An unknown type costs that fragment and its record.
                _ if damage == Damage::Stop => return records,
                _ => partial = None,
            }
            pos += HEADER + length;
        }
    }
    records
}

/// One WriteBatch of a log: a sequence number, a count, then puts and
/// deletions, one sequence number each. Whole or not at all, as LevelDB
/// applies it.
fn write_batch(record: &[u8], merge: &mut Merge) {
    let parsed = (|| {
        let first = u64::from_le_bytes(record.get(..8)?.try_into().ok()?);
        let count = u32::from_le_bytes(record.get(8..12)?.try_into().ok()?);
        let mut pos = 12;
        let mut ops = Vec::new();
        for seq in (first..).take(usize::try_from(count).ok()?) {
            let tag = *record.get(pos)?;
            pos += 1;
            let key = slice(record, &mut pos)?;
            let value = match tag {
                1 => Some(slice(record, &mut pos)?),
                0 => None,
                _ => return None,
            };
            ops.push((key, seq, value));
        }
        (pos == record.len()).then_some(ops)
    })();
    for (key, seq, value) in parsed.into_iter().flatten() {
        put(merge, key, seq, value.map(<[u8]>::to_vec));
    }
}

fn put(merge: &mut Merge, key: &[u8], seq: u64, value: Option<Vec<u8>>) {
    match merge.get(key) {
        Some((newest, _)) if *newest > seq => {}
        _ => {
            merge.insert(key.to_vec(), (seq, value));
        }
    }
}

/// A table's entries into `merge`; the number of Snappy blocks it had. A
/// block with a bad checksum is skipped; a file that is no table is an
/// error.
fn table_entries(path: &Path, bytes: &[u8], merge: &mut Merge) -> Result<usize, Failure> {
    let damaged = |what: &str| Failure::Permanent(format!("{}: {what}", path.display()));
    let footer = bytes
        .len()
        .checked_sub(FOOTER)
        .map(|at| &bytes[at..])
        .ok_or_else(|| damaged("shorter than a table's footer"))?;
    if u64::from_le_bytes(footer[40..].try_into().unwrap_or_default()) != TABLE_MAGIC {
        return Err(damaged("no table magic"));
    }
    let mut pos = 0;
    let handles = (|| {
        let _metaindex = (varint(footer, &mut pos)?, varint(footer, &mut pos)?);
        Some((varint(footer, &mut pos)?, varint(footer, &mut pos)?))
    })();
    let index = handles.ok_or_else(|| damaged("footer"))?;
    let (index, _) = block(bytes, index)
        .map_err(|kind| damaged(&format!("compression type {kind}")))?
        .ok_or_else(|| damaged("index block"))?;
    let mut snappy_blocks = 0;
    for (_, handle) in block_entries(&index).ok_or_else(|| damaged("index block"))? {
        let mut pos = 0;
        let Some(handle) = varint(&handle, &mut pos).zip(varint(&handle, &mut pos)) else {
            return Err(damaged("index entry"));
        };
        let Some((data, snappy)) =
            block(bytes, handle).map_err(|kind| damaged(&format!("compression type {kind}")))?
        else {
            continue;
        };
        snappy_blocks += usize::from(snappy);
        for (internal, value) in block_entries(&data).into_iter().flatten() {
            let Some(split) = internal.len().checked_sub(8) else {
                continue;
            };
            let tag = u64::from_le_bytes(internal[split..].try_into().unwrap_or_default());
            let value = match tag & 0xff {
                1 => Some(value),
                0 => None,
                _ => continue,
            };
            put(merge, &internal[..split], tag >> 8, value);
        }
    }
    Ok(snappy_blocks)
}

/// The block at `(offset, size)` with its trailer checked and unpacked, and
/// whether it was Snappy; `Ok(None)` for a damaged block, `Err` with the
/// compression type when it is neither none (0) nor Snappy (1).
fn block(bytes: &[u8], (offset, size): (u64, u64)) -> Result<Option<(Vec<u8>, bool)>, u8> {
    let range = usize::try_from(offset).ok().and_then(|start| {
        let end = start.checked_add(usize::try_from(size).ok()?)?;
        Some((start, end))
    });
    let Some((start, end)) =
        range.filter(|&(_, end)| end.checked_add(5).is_some_and(|t| t <= bytes.len()))
    else {
        return Ok(None);
    };
    let data = &bytes[start..end];
    let kind = bytes[end];
    let checksum = u32::from_le_bytes(bytes[end + 1..end + 5].try_into().unwrap_or_default());
    if unmask(checksum) != crc32c(&[data, &[kind]]) {
        return Ok(None);
    }
    match kind {
        0 => Ok(Some((data.to_vec(), false))),
        1 => Ok(snappy(data).map(|data| (data, true))),
        other => Err(other),
    }
}

/// A block's entries: keys stored as a shared prefix with the previous key
/// plus the rest, then the restart array that ends the block.
fn block_entries(block: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    let count = u32::from_le_bytes(block.get(block.len().checked_sub(4)?..)?.try_into().ok()?);
    let end = block
        .len()
        .checked_sub(4)?
        .checked_sub(usize::try_from(count).ok()?.checked_mul(4)?)?;
    let entries = &block[..end];
    let mut out = Vec::new();
    let mut key: Vec<u8> = Vec::new();
    let mut pos = 0;
    while pos < entries.len() {
        let shared = usize::try_from(varint(entries, &mut pos)?).ok()?;
        let own = usize::try_from(varint(entries, &mut pos)?).ok()?;
        let value = usize::try_from(varint(entries, &mut pos)?).ok()?;
        if shared > key.len() {
            return None;
        }
        key.truncate(shared);
        key.extend_from_slice(entries.get(pos..pos.checked_add(own)?)?);
        pos += own;
        let value_bytes = entries.get(pos..pos.checked_add(value)?)?;
        pos += value;
        out.push((key.clone(), value_bytes.to_vec()));
    }
    Some(out)
}

/// Snappy's raw format (format_description.txt): the length, then literals
/// and copies of earlier output. `None` for anything out of bounds.
pub(super) fn snappy(input: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0;
    let length = usize::try_from(varint(input, &mut pos)?).ok()?;
    if length > MAX_BLOCK {
        return None;
    }
    let mut out = Vec::with_capacity(length);
    while pos < input.len() {
        let tag = input[pos];
        pos += 1;
        let (len, offset) = match tag & 3 {
            0 => {
                let mut len = usize::from(tag >> 2);
                if len >= 60 {
                    let bytes = len - 59;
                    let raw = input.get(pos..pos + bytes)?;
                    len = raw.iter().rev().fold(0, |n, &b| n << 8 | usize::from(b));
                    pos += bytes;
                }
                let literal = input.get(pos..pos.checked_add(len + 1)?)?;
                pos += len + 1;
                if out.len() + literal.len() > length {
                    return None;
                }
                out.extend_from_slice(literal);
                continue;
            }
            1 => {
                let low = *input.get(pos)?;
                pos += 1;
                (
                    4 + usize::from((tag >> 2) & 7),
                    usize::from(tag >> 5) << 8 | usize::from(low),
                )
            }
            2 => {
                let raw = input.get(pos..pos + 2)?;
                pos += 2;
                (
                    1 + usize::from(tag >> 2),
                    usize::from(u16::from_le_bytes([raw[0], raw[1]])),
                )
            }
            _ => {
                let raw = input.get(pos..pos + 4)?;
                pos += 4;
                (
                    1 + usize::from(tag >> 2),
                    usize::try_from(u32::from_le_bytes(raw.try_into().ok()?)).ok()?,
                )
            }
        };
        if offset == 0 || offset > out.len() || out.len() + len > length {
            return None;
        }
        // Byte by byte: a copy may overlap what it writes (runs).
        let start = out.len() - offset;
        for i in 0..len {
            out.push(out[start + i]);
        }
    }
    (out.len() == length).then_some(out)
}

fn varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

/// A varint32 length, then that many bytes.
fn slice<'a>(bytes: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let len = usize::try_from(varint(bytes, pos)?).ok()?;
    let out = bytes.get(*pos..pos.checked_add(len)?)?;
    *pos += len;
    Some(out)
}

const CRC32C: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0x82f6_3b78
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC-32C (Castagnoli) over the parts in order.
fn crc32c(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for part in parts {
        for &byte in *part {
            crc = CRC32C[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
        }
    }
    !crc
}

/// LevelDB stores checksums masked (crc32c.h), so a CRC of data that holds
/// CRCs does not collide with them.
fn unmask(masked: u32) -> u32 {
    masked.wrapping_sub(0xa282_ead8).rotate_left(15)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/webview/chromium")
    }

    /// The fixture copied, so a test can change it.
    pub(crate) struct Copy(pub PathBuf);

    impl Copy {
        pub(crate) fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "gelabber-leveldb-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            copy_dir(&fixture(), &dir);
            Self(dir)
        }
    }

    impl Drop for Copy {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() && entry.file_name() != ".gitignore" {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }

    const ORIGIN: &str = "http://127.0.0.1:8123";
    const DRAFT: &str = "gelabber:chat-draft:0f9a4c1e-8f1b-4f3e-9d55-2b7a1c3e5d01:c2d3e4f5-a6b7-4c8d-9e0f-1a2b3c4d5e6f";
    const GONE: &str = "gelabber:chat-draft:0f9a4c1e-8f1b-4f3e-9d55-2b7a1c3e5d01:d4e5f6a7-b8c9-4dae-8f01-2b3c4d5e6f70";

    fn media_gain(areas: &Areas, origin: &str) -> f64 {
        let media: serde_json::Value =
            serde_json::from_str(&areas[origin]["gelabber.media"]).unwrap();
        media["state"]["inputGain"].as_f64().unwrap()
    }

    #[test]
    fn the_chromium_store_reads_as_the_page_left_it() {
        let (areas, stats) = read(&fixture()).unwrap();
        // What makes the fixture worth having (capture-chromium.sh): tables,
        // Snappy blocks and the log that came after them.
        assert!(stats.tables >= 1, "{stats:?}");
        assert!(stats.snappy_blocks >= 1, "{stats:?}");
        assert!(stats.logs >= 1, "{stats:?}");
        assert_eq!(
            areas.keys().collect::<Vec<_>>(),
            ["http://127.0.0.1:8123", "http://127.0.0.1:8124"]
        );
        let area = &areas[ORIGIN];
        // Phase 2's value in the log over phase 1's in the table.
        assert_eq!(media_gain(&areas, ORIGIN), 1.5);
        assert_eq!(media_gain(&areas, "http://127.0.0.1:8124"), 0.75);
        // Deleted in the log, written in the table.
        assert!(!area.contains_key("gelabber.fixture.filler"));
        assert!(!area.contains_key(GONE));
        // Latin-1 and UTF-16 strings.
        assert!(area["gelabber.interface"].contains("\"greeting\":\"Grüße\""));
        assert!(area[DRAFT].contains("bis gleich 👋"));
        assert_eq!(area["gelabber.media.source-audio-share"], "off");
        assert_eq!(area.len(), 7, "{:?}", area.keys());
    }

    #[test]
    fn a_torn_log_keeps_what_came_before() {
        let copy = Copy::new("torn");
        let live = copy.0.join("000005.log");
        let bytes = std::fs::read(&live).unwrap();
        // The last write batch (phase 2: overwrite and deletions) loses its
        // tail, as when the updater kills the app mid-write.
        std::fs::write(&live, &bytes[..bytes.len() - 3]).unwrap();
        let areas = areas(&copy.0).unwrap();
        assert_eq!(media_gain(&areas, ORIGIN), 0.8);
        assert!(areas[ORIGIN].contains_key(GONE));
        assert!(areas[ORIGIN].contains_key("gelabber.fixture.filler"));
        // A flipped bit ends the log there as well.
        let mut flipped = bytes.clone();
        flipped[HEADER + 2] ^= 1;
        std::fs::write(&live, &flipped).unwrap();
        assert_eq!(media_gain(&self::areas(&copy.0).unwrap(), ORIGIN), 0.8);
    }

    #[test]
    fn the_store_stays_as_it_was() {
        let listing = |dir: &Path| {
            let mut files: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| {
                    let path = e.unwrap().path();
                    (path.clone(), std::fs::read(&path).unwrap())
                })
                .collect();
            files.sort();
            files
        };
        let copy = Copy::new("unchanged");
        let before = listing(&copy.0);
        areas(&copy.0).unwrap();
        assert_eq!(listing(&copy.0), before);
    }

    #[test]
    fn no_manifest_reads_every_file() {
        let copy = Copy::new("no-manifest");
        std::fs::remove_file(copy.0.join("CURRENT")).unwrap();
        let (areas, stats) = read(&copy.0).unwrap();
        assert_eq!((stats.tables, stats.logs), (2, 2), "the unlisted table too");
        assert_eq!(media_gain(&areas, ORIGIN), 1.5);
        assert!(!areas[ORIGIN].contains_key(GONE));
    }

    #[test]
    fn only_what_the_manifest_lists_counts() {
        // Chromium ended during the last flush: 000006.ldb is written but not
        // in the MANIFEST, so LevelDB replays 000004.log and 000005.log instead.
        let (_, stats) = read(&fixture()).unwrap();
        assert_eq!((stats.tables, stats.logs), (1, 2));
        let copy = Copy::new("orphan");
        std::fs::write(copy.0.join("000006.ldb"), b"kein LevelDB").unwrap();
        assert_eq!(media_gain(&areas(&copy.0).unwrap(), ORIGIN), 1.5);
    }

    #[test]
    fn a_store_that_is_not_one_is_a_permanent_failure() {
        let copy = Copy::new("not-a-table");
        std::fs::write(copy.0.join("000003.ldb"), b"kein LevelDB").unwrap();
        assert!(matches!(areas(&copy.0), Err(Failure::Permanent(_))));
        // No store at all is an empty one.
        assert!(areas(&copy.0.join("fehlt")).unwrap().is_empty());
    }

    #[test]
    fn snappy_copies_may_overlap_and_stay_in_bounds() {
        // "ab" then a copy of length 6 from offset 2: "abababab".
        assert_eq!(
            snappy(&[8, 0b0000_0100, b'a', b'b', 0b0000_1001, 2]).as_deref(),
            Some(b"abababab".as_slice())
        );
        // An offset before the start, a length past the end.
        assert_eq!(snappy(&[8, 0b0000_0100, b'a', b'b', 0b0000_1001, 3]), None);
        assert_eq!(snappy(&[3, 0b0000_0100, b'a', b'b', 0b0000_1001, 2]), None);
    }

    #[test]
    fn dom_strings_decode_by_their_format_byte() {
        assert_eq!(decode_dom(&[1, 0xfc]).as_deref(), Some("ü"));
        assert_eq!(
            decode_dom(&[0, 0x3d, 0xd8, 0x4b, 0xdc]).as_deref(),
            Some("👋")
        );
        assert_eq!(decode_dom(&[0, 0x41]), None);
        assert_eq!(decode_dom(&[2, 0x41]), None);
    }

    /// What LevelDB writes, for stores no WebView would leave.
    mod write {
        use super::super::*;

        pub fn varint(mut n: u64) -> Vec<u8> {
            let mut out = Vec::new();
            while n >= 0x80 {
                out.push(n as u8 | 0x80);
                n >>= 7;
            }
            out.push(n as u8);
            out
        }

        fn mask(crc: u32) -> u32 {
            crc.rotate_right(15).wrapping_add(0xa282_ead8)
        }

        /// One fragment of a log with its header.
        pub fn record(kind: u8, data: &[u8]) -> Vec<u8> {
            let mut out = mask(crc32c(&[&[kind], data])).to_le_bytes().to_vec();
            out.extend_from_slice(&u16::try_from(data.len()).unwrap().to_le_bytes());
            out.push(kind);
            out.extend_from_slice(data);
            out
        }

        /// A WriteBatch of puts (`Some`) and deletions.
        pub fn batch(seq: u64, ops: &[(&[u8], Option<&[u8]>)]) -> Vec<u8> {
            let mut out = seq.to_le_bytes().to_vec();
            out.extend_from_slice(&u32::try_from(ops.len()).unwrap().to_le_bytes());
            for (key, value) in ops {
                out.push(u8::from(value.is_some()));
                out.extend(varint(key.len() as u64));
                out.extend_from_slice(key);
                if let Some(value) = value {
                    out.extend(varint(value.len() as u64));
                    out.extend_from_slice(value);
                }
            }
            out
        }

        /// `gelabber.media` of http://a set to `value` (Latin-1).
        pub fn media(seq: u64, value: &str) -> Vec<u8> {
            let value = [&[1], value.as_bytes()].concat();
            batch(
                seq,
                &[
                    (b"VERSION", Some(b"1")),
                    (b"_http://a\0\x01gelabber.media", Some(&value)),
                ],
            )
        }
    }

    fn media_of_a(dir: &Path) -> String {
        let areas = areas(dir).unwrap();
        areas["http://a"]["gelabber.media"].clone()
    }

    #[test]
    fn a_torn_record_costs_only_the_rest_of_its_block() {
        // A session killed after a record's header, then a reused log that
        // the next session appended to: its writes behind the torn record
        // are lost with that block, the next block's are not.
        let mut log = write::record(1, &write::media(1, "old"));
        log.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        log.extend_from_slice(&200u16.to_le_bytes());
        log.push(1);
        log.extend_from_slice(&[0; 20]);
        log.extend(write::record(1, &write::media(3, "mid")));
        log.resize(BLOCK, 0);
        log.extend(write::record(1, &write::media(4, "new")));
        let copy = Copy::new("torn-mid");
        for entry in std::fs::read_dir(&copy.0).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::write(copy.0.join("000003.log"), &log).unwrap();
        assert_eq!(media_of_a(&copy.0), "new");
        // A MANIFEST ends there.
        assert_eq!(log_records(&log, Damage::Stop).len(), 1);
    }

    #[test]
    fn an_unknown_record_type_costs_only_that_record() {
        let [old, unknown, new, first, last] = [
            write::media(1, "old"),
            b"neu".to_vec(),
            write::media(2, "new"),
            write::media(3, "half"),
            b"rest".to_vec(),
        ];
        let log = [
            write::record(1, &old),
            write::record(9, &unknown),
            write::record(1, &new),
            // A FIRST, then an unknown type: the record is lost whole.
            write::record(2, &first),
            write::record(9, &unknown),
            write::record(4, &last),
        ]
        .concat();
        assert_eq!(log_records(&log, Damage::SkipBlock), [old.clone(), new]);
        assert_eq!(log_records(&log, Damage::Stop), [old]);
    }
}
