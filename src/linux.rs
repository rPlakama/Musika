// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Linux-only filesystem hints.
//!
//! These are advisory only: a failed hint must never affect correctness, so
//! callers treat every result as best-effort.

#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::path::Path;

/// Ask the kernel to start reading a file into the page cache.
///
/// This is the same intent as a buffered read, without copying the bytes into
/// userspace. The existing warm/read paths still run as a fallback.
#[cfg(target_os = "linux")]
pub(crate) fn advise_willneed(path: &Path) -> std::io::Result<()> {
    let file = std::fs::File::open(path)?;
    // SAFETY: `file` keeps the descriptor open for the duration of the call;
    // `posix_fadvise` only inspects the open file description and reports
    // invalid/unsupported advice through its return value, never as memory
    // unsafety.
    let _ = unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_WILLNEED) };
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn advise_willneed(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// `CLOCK_MONOTONIC` in nanoseconds, shared by the core and the UI process.
///
/// The core publishes playback clock anchors on this clock and `musika-ui`
/// extrapolates between updates, so the two processes need the same time base.
/// `CLOCK_MONOTONIC` is system-wide on Linux. On other targets fall back to the
/// wall clock (the UI only uses it for short extrapolation windows).
#[cfg(target_os = "linux")]
pub(crate) fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid `timespec` and CLOCK_MONOTONIC is always supported.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if result != 0 {
        return 0;
    }
    (ts.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as u64)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn monotonic_ns() -> u64 {
    crate::consts::now_nanos() as u64
}

/// Ask the kernel to pull a file into the page cache without copying it into
/// userspace. Used by the database warm-up: the old loop read the whole file
/// through a 1 MiB buffer, duplicating the kernel's readahead.
#[cfg(target_os = "linux")]
pub(crate) fn prefetch_file(path: &Path) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let fd = file.as_raw_fd();
    // SAFETY: `fd` is open for the duration of both calls; `readahead` and
    // `posix_fadvise` only inspect the open file description and report
    // unsupported advice through their return value, never through memory.
    // SAFETY: `fd` is open, and `readahead` only queues kernel readahead for
    // the open file description; failure is reported by the return value.
    let _ = unsafe { libc::readahead(fd, 0, metadata.len() as usize) };
    // SAFETY: `fd` is still open; `posix_fadvise` only inspects the open file
    // description and reports invalid advice through its return value.
    let _ = unsafe { libc::posix_fadvise(fd, 0, 0, POSIX_FADV_WILLNEED) };
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn prefetch_file(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
const POSIX_FADV_WILLNEED: std::ffi::c_int = 3;

/// Demote the calling thread to `SCHED_IDLE` + `IOPRIO_CLASS_IDLE`.
///
/// Best-effort only: a permission or kernel refusal must never affect
/// correctness, so callers ignore the result. Used for background I/O such as
/// the database warm-up and the bulk-sync SQLite worker.
#[cfg(target_os = "linux")]
pub(crate) fn demote_current() {
    // SAFETY: `SYS_gettid` takes no pointers and returns the calling thread id.
    let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
    let _ = set_idle_priority(tid);
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn demote_current() {}

/// Demote one task: `SCHED_IDLE` for CPU and `IOPRIO_CLASS_IDLE` for block I/O.
/// Returns true only when both calls were accepted.
#[cfg(target_os = "linux")]
pub(crate) fn set_idle_priority(tid: u32) -> bool {
    const SCHED_IDLE: std::ffi::c_int = 5;
    const IOPRIO_WHO_PROCESS: std::ffi::c_int = 1;
    const IOPRIO_CLASS_IDLE: std::ffi::c_int = 3;

    let param = libc::sched_param { sched_priority: 0 };
    // SAFETY: `tid` is a task id supplied by the caller (0 means the calling
    // thread for `sched_setscheduler`), and `param` is a fully initialised
    // `sched_param`; the call only reads those values and reports failure via
    // its return value.
    let sched = unsafe { libc::sched_setscheduler(tid as libc::pid_t, SCHED_IDLE, &param) };
    // SAFETY: raw `ioprio_set(2)` with integer-only arguments; the kernel
    // validates the target task/class and reports errors through the return
    // value, so no memory-safety precondition can be violated.
    let io = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            tid as libc::c_int,
            IOPRIO_CLASS_IDLE << 13,
        ) as std::ffi::c_int
    };
    sched == 0 && io == 0
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn set_idle_priority(_tid: u32) -> bool {
    false
}

/// glibc allocator policy for a long-lived daemon.
///
/// `M_ARENA_MAX=1` is already the memory/CPU trade-off this app wants; the
/// thresholds make large one-shot buffers (database/sync JSON, cover decode,
/// the PCM ring) use mmap and return to the kernel on free instead of growing
/// the heap high-water mark. `malloc_trim` is called only at burst boundaries.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn tune_allocator() {
    // SAFETY: every `mallopt` call only reads integer arguments and reports
    // failure through its return value; no pointer crosses this boundary.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 1);
    }
    // SAFETY: as above; integer-only allocator tuning.
    unsafe {
        libc::mallopt(libc::M_TRIM_THRESHOLD, 128 * 1024);
    }
    // SAFETY: as above; integer-only allocator tuning.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 512 * 1024);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn tune_allocator() {}

/// Return free heap pages to the kernel (glibc only; no-op elsewhere).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn malloc_trim() {
    // SAFETY: `malloc_trim` takes a padding size and only releases free
    // allocator memory; it has no pointer preconditions.
    unsafe {
        _ = libc::malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn malloc_trim() {}

/// Current process resident set size (`Rss` from `/proc/self/smaps_rollup`,
/// falling back to `VmRSS` in `/proc/self/status`) in KiB.
#[cfg(target_os = "linux")]
pub(crate) fn rss_kib() -> Option<u64> {
    if let Ok(rollup) = std::fs::read_to_string("/proc/self/smaps_rollup")
        && let Some(kib) = parse_proc_kib(&rollup, "Rss:")
    {
        return Some(kib);
    }
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_proc_kib(&status, "VmRSS:")
}

#[cfg(target_os = "linux")]
fn parse_proc_kib(text: &str, prefix: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.split_whitespace().next()?.parse::<u64>().ok();
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn rss_kib() -> Option<u64> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn prefetch_file_rejects_missing_and_non_file_targets() {
        let missing = std::env::temp_dir().join("musika-missing-prefetch");
        _ = std::fs::remove_file(&missing);
        assert!(prefetch_file(&missing).is_err());
        let dir = std::env::temp_dir().join(format!("musika-prefetch-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        assert!(prefetch_file(&dir).is_err());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rss_kib_reports_nonzero_on_linux() {
        let kib = rss_kib().expect("/proc/self/smaps_rollup or status must report RSS");
        assert!(kib > 0);
    }
}

/// Mark every inherited descriptor above stdio close-on-exec.
///
/// Used from `pre_exec` before spawning the UI child or the detached daemon:
/// SQLite, PipeWire, eventfd and socket descriptors must not leak into a
/// sibling process. Best-effort; the call is async-signal-safe and allocates
/// nothing, which is what the fork/exec window requires.
#[cfg(target_os = "linux")]
pub(crate) fn close_inherited_fds() -> std::io::Result<()> {
    // SAFETY: `close_range` only marks descriptor flags in the calling
    // process; the range starts at fd 3 so stdio is untouched, and failure
    // (old kernel/filesystem) is reported through the return value.
    let _ = unsafe { libc::close_range(3, u32::MAX, libc::CLOSE_RANGE_CLOEXEC as std::ffi::c_int) };
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn close_inherited_fds() -> std::io::Result<()> {
    Ok(())
}
