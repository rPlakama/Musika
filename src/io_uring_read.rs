// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! Reusable io_uring file reader.
//!
//! A [`Reader`] owns one ring for its lifetime. [`Reader::read_many`] opens a
//! batch, submits all reads before waiting, and returns one result per path.
//! No ring is created per file.

#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::fs::File;
use std::io;
use std::mem;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use io_uring::{IoUring, opcode, types};

/// Submission-queue depth. A batch is split into chunks of this size so that
/// every read in a chunk can be enqueued before the ring is submitted.
const ENTRIES: u32 = 64;

struct Slot {
    /// Keeps the descriptor open until the chunk's completions are reaped.
    file: File,
    buf: Vec<u8>,
    /// Bytes already read into `buf` across short reads in this chunk.
    read_so_far: usize,
}

struct Ring {
    uring: IoUring,
    /// Set when a submission fails part-way. The submission queue may still
    /// hold entries that point at buffers which are no longer alive, so the
    /// ring must never be submitted again.
    poisoned: bool,
    #[cfg(test)]
    fail_submit_errno: Cell<Option<i32>>,
    #[cfg(test)]
    eintr_submit_count: Cell<usize>,
    #[cfg(test)]
    short_read_cap: Option<usize>,
}

impl Ring {
    fn sqe_len_cap(&self) -> Option<usize> {
        #[cfg(test)]
        {
            self.short_read_cap
        }
        #[cfg(not(test))]
        {
            None
        }
    }
}

pub(crate) struct Reader {
    ring: RefCell<Ring>,
}

impl Reader {
    pub(crate) fn new() -> io::Result<Self> {
        // 64 slots is enough for a cover thumbnail batch and keeps the ring
        // small enough to live for a worker's lifetime.
        let uring = IoUring::new(ENTRIES)?;
        Ok(Self {
            ring: RefCell::new(Ring {
                uring,
                poisoned: false,
                #[cfg(test)]
                fail_submit_errno: Cell::new(None),
                #[cfg(test)]
                eintr_submit_count: Cell::new(0),
                #[cfg(test)]
                short_read_cap: None,
            }),
        })
    }

    pub(crate) fn is_poisoned(&self) -> bool {
        self.ring.borrow().poisoned
    }

    pub(crate) fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut results = self.read_many(std::slice::from_ref(&path.to_path_buf()));
        results
            .pop()
            .unwrap_or_else(|| Err(io::Error::from(io::ErrorKind::InvalidInput)))
    }

    pub(crate) fn read_many(&self, paths: &[PathBuf]) -> Vec<io::Result<Vec<u8>>> {
        if paths.is_empty() {
            return Vec::new();
        }

        let mut ring = self.ring.borrow_mut();
        if ring.poisoned {
            return paths.iter().map(|_| Err(poisoned_error())).collect();
        }

        let mut results: Vec<Option<io::Result<Vec<u8>>>> = paths.iter().map(|_| None).collect();
        let sqe_cap = ring.sqe_len_cap();

        for chunk_start in (0..paths.len()).step_by(ENTRIES as usize) {
            if ring.poisoned {
                break;
            }
            let chunk_count = (paths.len() - chunk_start).min(ENTRIES as usize);
            let mut slots: Vec<Option<Slot>> = (0..chunk_count).map(|_| None).collect();
            let mut pending = 0usize;

            // Phase 1: open, size, allocate, and enqueue every read in the
            // chunk before waiting on any completion.
            {
                let mut queue = ring.uring.submission();
                for (offset, slot_cell) in slots.iter_mut().enumerate() {
                    let index = chunk_start + offset;
                    // `File::open` uses O_RDONLY | O_CLOEXEC on Linux.
                    let file = match File::open(&paths[index]) {
                        Ok(file) => file,
                        Err(error) => {
                            results[index] = Some(Err(error));
                            continue;
                        }
                    };
                    let len = match file.metadata() {
                        Ok(metadata) => metadata.len(),
                        Err(error) => {
                            results[index] = Some(Err(error));
                            continue;
                        }
                    };
                    // Match the C reader: empty (and unsized) files read as
                    // empty without submitting anything.
                    if len == 0 {
                        results[index] = Some(Ok(Vec::new()));
                        continue;
                    }
                    // io_uring read lengths are 32-bit.
                    if len > u64::from(u32::MAX) {
                        results[index] = Some(Err(errno(libc::EOVERFLOW)));
                        continue;
                    }

                    let mut buf = Vec::new();
                    if buf.try_reserve_exact(len as usize).is_err() {
                        results[index] = Some(Err(errno(libc::ENOMEM)));
                        continue;
                    }
                    buf.resize(len as usize, 0);

                    let mut slot = Slot {
                        file,
                        buf,
                        read_so_far: 0,
                    };
                    let entry = read_sqe(&mut slot, index, sqe_cap);

                    // SAFETY: the descriptor and exact-size buffer are moved
                    // into `slots` below. `slots` cannot reallocate, and it
                    // stays alive until every submitted read in this chunk has
                    // completed (or the ring is poisoned and never reused).
                    if unsafe { queue.push(&entry) }.is_err() {
                        results[index] = Some(Err(errno(libc::EAGAIN)));
                        continue;
                    }
                    pending += 1;
                    *slot_cell = Some(slot);
                }
            }

            // Phase 2: submit the chunk and reap completions, resubmitting any
            // short reads until every slot reaches EOF or its expected length.
            while pending > 0 {
                if let Err(error) = submit_and_wait_with_retry(&ring, pending) {
                    let code = error.raw_os_error().unwrap_or(libc::EIO);
                    ring.poisoned = true;
                    leak_pending(&mut slots);
                    for (offset, slot) in slots.iter().enumerate() {
                        if slot.is_some() {
                            results[chunk_start + offset] = Some(Err(errno(code)));
                        }
                    }
                    break;
                }

                let mut completed = 0usize;
                let mut resubmit = Vec::new();
                {
                    let mut completion = ring.uring.completion();
                    while completed < pending {
                        let Some(cqe) = completion.next() else {
                            break;
                        };
                        completed += 1;
                        let Some(offset) = (cqe.user_data() as usize).checked_sub(chunk_start)
                        else {
                            continue;
                        };
                        let Some(slot_cell) = slots.get_mut(offset) else {
                            continue;
                        };
                        let Some(slot) = slot_cell.as_mut() else {
                            continue;
                        };
                        let result = cqe.result();
                        if result < 0 {
                            results[chunk_start + offset] =
                                Some(Err(io::Error::from_raw_os_error(-result)));
                            *slot_cell = None;
                        } else if result == 0 {
                            let mut buf = mem::take(&mut slot.buf);
                            buf.truncate(slot.read_so_far);
                            results[chunk_start + offset] = Some(Ok(buf));
                            *slot_cell = None;
                        } else {
                            let remaining = slot.buf.len().saturating_sub(slot.read_so_far);
                            let bytes = (result as usize).min(remaining);
                            slot.read_so_far += bytes;
                            if slot.read_so_far >= slot.buf.len() || bytes == 0 {
                                let mut buf = mem::take(&mut slot.buf);
                                buf.truncate(slot.read_so_far);
                                results[chunk_start + offset] = Some(Ok(buf));
                                *slot_cell = None;
                            } else {
                                resubmit.push(offset);
                            }
                        }
                    }
                }

                if completed < pending {
                    // The wait ended before every completion was reaped. Keep
                    // the remaining buffers alive rather than freeing memory
                    // that may still be in flight.
                    ring.poisoned = true;
                    leak_pending(&mut slots);
                    break;
                }

                if resubmit.is_empty() {
                    break;
                }

                let mut next_pending = 0usize;
                {
                    let mut queue = ring.uring.submission();
                    for offset in resubmit {
                        let index = chunk_start + offset;
                        let Some(slot_cell) = slots.get_mut(offset) else {
                            continue;
                        };
                        let Some(slot) = slot_cell.as_mut() else {
                            continue;
                        };
                        let entry = read_sqe(slot, index, sqe_cap);
                        // SAFETY: `slot.file` and `slot.buf` remain owned by
                        // `slots[offset]` until this resubmitted read completes
                        // or the ring is poisoned.
                        if unsafe { queue.push(&entry) }.is_err() {
                            results[index] = Some(Err(errno(libc::EAGAIN)));
                            *slot_cell = None;
                            continue;
                        }
                        next_pending += 1;
                    }
                }
                pending = next_pending;
            }

            if ring.poisoned {
                break;
            }

            // Dropping `slots` closes the descriptors and releases the buffers
            // whose reads failed or finished; completed buffers were moved into
            // `results` above.
        }

        results
            .into_iter()
            .map(|result| result.unwrap_or_else(|| Err(poisoned_error())))
            .collect()
    }
}

fn read_sqe(slot: &mut Slot, index: usize, max_len: Option<usize>) -> io_uring::squeue::Entry {
    let remaining = slot.buf.len().saturating_sub(slot.read_so_far);
    let request_len = match max_len {
        Some(cap) => remaining.min(cap).max(1),
        None => remaining,
    };
    opcode::Read::new(
        types::Fd(slot.file.as_raw_fd()),
        slot.buf[slot.read_so_far..].as_mut_ptr(),
        request_len as u32,
    )
    .offset(slot.read_so_far as u64)
    .build()
    .user_data(index as u64)
}

fn submit_and_wait_once(ring: &Ring, want: usize) -> io::Result<usize> {
    #[cfg(test)]
    {
        let remaining = ring.eintr_submit_count.get();
        if remaining > 0 {
            ring.eintr_submit_count.set(remaining - 1);
            return Err(errno(libc::EINTR));
        }
        if let Some(code) = ring.fail_submit_errno.take() {
            return Err(errno(code));
        }
    }
    ring.uring.submit_and_wait(want)
}

fn submit_and_wait_with_retry(ring: &Ring, want: usize) -> io::Result<()> {
    loop {
        match submit_and_wait_once(ring, want) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

thread_local! {
    static WORKER_READER: RefCell<Option<Reader>> = const { RefCell::new(None) };
}

fn with_worker_reader<T>(f: impl FnOnce(&Reader) -> T) -> io::Result<T> {
    WORKER_READER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(Reader::is_poisoned) {
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(Reader::new()?);
        }
        let reader = slot
            .as_ref()
            .ok_or_else(|| io::Error::other("io_uring reader unavailable"))?;
        let out = f(reader);
        if reader.is_poisoned() {
            *slot = None;
        }
        Ok(out)
    })
}

/// Reads one file through the calling thread's persistent reader.
pub(crate) fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    with_worker_reader(|reader| reader.read(path))?
}

/// Batch read for a worker pool: one ring, reusing the thread-local reader.
pub(crate) fn read_many(paths: &[PathBuf]) -> io::Result<Vec<io::Result<Vec<u8>>>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    with_worker_reader(|reader| reader.read_many(paths))
}

fn errno(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}

fn poisoned_error() -> io::Error {
    io::Error::other("io_uring reader is unusable after a failed submission")
}

/// Forget buffers that may still be referenced by in-flight requests. Their
/// descriptors can be closed safely; the kernel already holds its own file
/// reference for submitted requests, and the poisoned ring is never submitted
/// again.
fn leak_pending(slots: &mut [Option<Slot>]) {
    for slot in slots.iter_mut().flatten() {
        let _leaked = mem::ManuallyDrop::new(mem::take(&mut slot.buf));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "musika-uring-{tag}-{}-{}",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn read_many_preserves_order_and_maps_errors() {
        let dir = scratch_dir("order");
        let empty = dir.join("empty.bin");
        let first = dir.join("first.bin");
        let second = dir.join("second.bin");
        let missing = dir.join("missing.bin");
        fs::write(&empty, []).expect("empty");
        fs::write(&first, b"first").expect("first");
        fs::write(&second, b"second").expect("second");

        let paths = vec![first, missing, empty, second];
        let reader = Reader::new().expect("reader");
        let results = reader.read_many(&paths);
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].as_deref().expect("first"), &b"first"[..]);
        assert!(results[1].is_err(), "missing file must report an error");
        assert_eq!(results[2].as_deref().expect("empty"), &b""[..]);
        assert_eq!(results[3].as_deref().expect("second"), &b"second"[..]);
        fs::remove_dir_all(dir).expect("remove scratch");
    }

    #[test]
    fn read_many_spans_multiple_submission_chunks() {
        let dir = scratch_dir("chunks");
        let count = ENTRIES as usize + 3;
        let paths: Vec<PathBuf> = (0..count)
            .map(|index| dir.join(format!("chunk-{index}.bin")))
            .collect();
        for (index, path) in paths.iter().enumerate() {
            let bytes = vec![b'a' + (index % 26) as u8; index + 1];
            fs::write(path, &bytes).expect("write chunk file");
        }

        let reader = Reader::new().expect("reader");
        let results = reader.read_many(&paths);
        assert_eq!(results.len(), count);
        for (index, result) in results.into_iter().enumerate() {
            let expected = vec![b'a' + (index % 26) as u8; index + 1];
            assert_eq!(result.expect("chunk read"), expected, "file {index}");
        }
        fs::remove_dir_all(dir).expect("remove scratch");
    }

    #[test]
    fn poisoned_ring_stops_subsequent_chunks() {
        let dir = scratch_dir("poison-chunks");
        let chunk_one_len = ENTRIES as usize;
        let mut paths = Vec::with_capacity(chunk_one_len + 2);
        for index in 0..chunk_one_len {
            let path = dir.join(format!("first-{index}.bin"));
            fs::write(&path, format!("payload-{index}")).expect("write chunk 1 file");
            paths.push(path);
        }
        // Put a missing path and a valid path in chunk 2. If chunk 2 ran Phase 1
        // after chunk 1 poisoned the ring, `missing` would fail with ENOENT and
        // `valid` would succeed; breaking immediately yields `poisoned_error()`
        // (`ErrorKind::Other`) for every entry in chunk 2.
        let chunk_two_missing = dir.join("never-opened.bin");
        let chunk_two_valid = dir.join("never-submitted.bin");
        fs::write(&chunk_two_valid, b"should-not-be-read").expect("write chunk 2 file");
        paths.push(chunk_two_missing);
        paths.push(chunk_two_valid);

        let reader = Reader::new().expect("reader");
        reader.ring.borrow().fail_submit_errno.set(Some(libc::EIO));

        let results = reader.read_many(&paths);
        assert!(reader.is_poisoned());
        assert_eq!(results.len(), paths.len());

        for (index, result) in results[..chunk_one_len].iter().enumerate() {
            let err = result.as_ref().expect_err("chunk 1 should fail with EIO");
            assert_eq!(err.raw_os_error(), Some(libc::EIO), "chunk 1 slot {index}");
        }
        for (offset, result) in results[chunk_one_len..].iter().enumerate() {
            let err = result
                .as_ref()
                .expect_err("chunk 2 must not be opened or submitted after poison");
            assert_eq!(
                err.kind(),
                io::ErrorKind::Other,
                "chunk 2 slot {offset} should report poisoned error, got {err:?}"
            );
            assert!(
                err.raw_os_error().is_none(),
                "chunk 2 slot {offset} must not report ENOENT/OS error"
            );
        }

        fs::remove_dir_all(dir).expect("remove scratch");
    }

    #[test]
    fn retries_submit_and_wait_on_eintr() {
        let dir = scratch_dir("eintr");
        let path = dir.join("sample.bin");
        let expected = b"retried after eintr without poisoning".to_vec();
        fs::write(&path, &expected).expect("write sample");

        let reader = Reader::new().expect("reader");
        reader.ring.borrow().eintr_submit_count.set(3);

        let actual = reader.read(&path).expect("read should succeed after EINTR");
        assert_eq!(actual, expected);
        assert!(!reader.is_poisoned(), "EINTR must not poison the ring");

        fs::remove_dir_all(dir).expect("remove scratch");
    }

    #[test]
    fn resubmits_short_reads_from_read_so_far_offset() {
        let dir = scratch_dir("short-read");
        let first = dir.join("multi-wave.bin");
        let second = dir.join("exact.bin");
        let first_bytes: Vec<u8> = (0..103u8)
            .map(|i| i.wrapping_mul(7).wrapping_add(3))
            .collect();
        let second_bytes = b"abcdefg".to_vec();
        fs::write(&first, &first_bytes).expect("write first");
        fs::write(&second, &second_bytes).expect("write second");

        let reader = Reader::new().expect("reader");
        reader.ring.borrow_mut().short_read_cap = Some(13);

        let results = reader.read_many(&[first, second]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().expect("first"), &first_bytes);
        assert_eq!(results[1].as_ref().expect("second"), &second_bytes);
        assert!(!reader.is_poisoned());

        fs::remove_dir_all(dir).expect("remove scratch");
    }

    #[test]
    fn worker_reader_replaces_poisoned_ring_for_read_file_and_read_many() {
        let dir = scratch_dir("worker-reset");
        let one = dir.join("one.bin");
        let two = dir.join("two.bin");
        fs::write(&one, b"alpha").expect("write one");
        fs::write(&two, b"beta").expect("write two");

        // Prime WORKER_READER and mark its ring poisoned.
        assert_eq!(read_file(&one).expect("prime read_file"), b"alpha");
        WORKER_READER.with(|slot| {
            let guard = slot.borrow();
            let reader = guard.as_ref().expect("worker reader initialized");
            reader.ring.borrow_mut().poisoned = true;
        });

        // Both `read_many` and `read_file` must replace the poisoned reader.
        let batch = read_many(&[one, two.clone()]).expect("read_many recovers");
        assert_eq!(batch[0].as_ref().expect("batch 0"), b"alpha");
        assert_eq!(batch[1].as_ref().expect("batch 1"), b"beta");

        WORKER_READER.with(|slot| {
            let guard = slot.borrow();
            let reader = guard.as_ref().expect("worker reader present");
            reader.ring.borrow_mut().poisoned = true;
        });
        assert_eq!(read_file(&two).expect("read_file recovers"), b"beta");

        fs::remove_dir_all(dir).expect("remove scratch");
    }
}
