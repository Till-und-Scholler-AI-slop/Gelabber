//! Gives freed heap memory back to the operating system (#614).
//!
//! glibc's allocator keeps memory that was freed in its arenas instead of
//! returning it: a burst of WebSocket connections or sign-ins (Argon2 holds
//! 19 MiB per hash) leaves the process at its peak size for good. The memory
//! is reused, not leaked, but on a small server one spike would otherwise
//! hold it until the next restart. `malloc_trim(0)` walks every arena and
//! releases the free pages, including those in the middle of a heap. It also
//! covers memory that C and C++ code allocates (mediasoup's worker, aws-lc),
//! which a Rust global allocator would not.
//!
//! A trim cannot shrink the top of a thread arena, though, and glibc raises
//! its mmap threshold to the size of every large block freed (up to 32 MiB),
//! so Argon2's 19 MiB blocks soon come from those arenas and stay. A fixed
//! threshold keeps large blocks in their own mappings, which `free` unmaps.
//!
//! Elsewhere (musl, macOS, Windows) [`release_after_peaks`] does nothing.

use std::time::Duration;

/// How often the trimmer runs. In the #614 load test a trim took about 1 ms
/// (with 2000 open WebSockets too), and 24 ms for the one that handed back
/// 440 MB at once.
pub const TRIM_INTERVAL: Duration = Duration::from_secs(30);

/// Blocks of at least this size get their own mapping and go back to the OS
/// as soon as they are freed. glibc starts at 128 KiB; 1 MiB keeps buffers
/// that grow and shrink often in the arenas.
pub const MMAP_THRESHOLD: usize = 1024 * 1024;

/// Fixes the mmap threshold at [`MMAP_THRESHOLD`] and starts a background
/// thread that calls `malloc_trim(0)` every `interval`.
///
/// A plain thread, not a Tokio task: a trim holds each arena's lock while
/// it walks it, and must not block a runtime worker meanwhile.
pub fn release_after_peaks(interval: Duration) -> std::io::Result<()> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fix_mmap_threshold();
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    std::thread::Builder::new()
        .name("malloc-trim".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(interval);
                trim();
            }
        })?;
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    let _ = interval;
    Ok(())
}

/// Stops glibc from raising the mmap threshold after large frees.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn fix_mmap_threshold() {
    let threshold = libc::c_int::try_from(MMAP_THRESHOLD).unwrap_or(libc::c_int::MAX);
    // SAFETY: mallopt only changes the allocator's tuning parameters. It
    // returns 0 when it rejects the value, which leaves glibc's defaults in
    // place: worse at returning memory, nothing else.
    unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, threshold) };
}

/// Returns free heap pages to the OS now. `true` if any were released.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn trim() -> bool {
    // SAFETY: malloc_trim takes no pointers and only touches the
    // allocator's own state under its locks; any thread may call it.
    unsafe { libc::malloc_trim(0) == 1 }
}

#[cfg(all(test, target_os = "linux", target_env = "gnu"))]
mod tests {
    use super::*;

    /// Both tests read the RSS of the whole process, so they take turns.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RSS of this process in KiB, from /proc/self/status.
    fn rss_kib() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").expect("procfs");
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
            .expect("VmRSS line")
    }

    #[test]
    fn large_blocks_go_back_on_free() {
        let _turn = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        fix_mmap_threshold();
        // Argon2's block size. Without the fixed threshold the first free
        // raises it above 19 MiB and the second block stays in an arena.
        for round in 0..3 {
            let block = vec![1u8; 19 * 1024 * 1024];
            let held = rss_kib();
            drop(block);
            let freed = rss_kib();
            assert!(
                freed + 16 * 1024 < held,
                "round {round}: RSS {held} KiB with the block, {freed} KiB after free"
            );
        }
    }

    #[test]
    fn trim_returns_freed_small_allocations() {
        let _turn = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Many small blocks stay below the mmap threshold, so they live in
        // the heap. One long-lived block at the end pins the top, which is
        // exactly when free() alone cannot shrink it.
        let blocks: Vec<Vec<u8>> = (0..64 * 1024).map(|_| vec![1u8; 1024]).collect();
        let pin = vec![1u8; 1024];
        let before_free = rss_kib();
        drop(blocks);
        let after_free = rss_kib();
        assert!(trim(), "malloc_trim found nothing to release");
        let after_trim = rss_kib();
        drop(pin);
        assert!(
            after_trim + 32 * 1024 < after_free,
            "RSS did not fall by 32 MiB: allocated {before_free} KiB, freed {after_free} KiB, trimmed {after_trim} KiB"
        );
    }
}
