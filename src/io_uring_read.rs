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
}

struct Ring {
    uring: IoUring,
    /// Set when a submission fails part-way. The submission queue may still
    /// hold entries that point at buffers which are no longer alive, so the
    /// ring must never be submitted again.
    poisoned: bool,
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
            }),
        })
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

        for chunk_start in (0..paths.len()).step_by(ENTRIES as usize) {
            let chunk_count = (paths.len() - chunk_start).min(ENTRIES as usize);
            let mut slots: Vec<Option<Slot>> = (0..chunk_count).map(|_| None).collect();
            let mut submitted = 0usize;

            // Phase 1: open, size, allocate, and enqueue every read in the
            // chunk before waiting on any completion.
            {
                let mut queue = ring.uring.submission();
                for offset in 0..chunk_count {
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

                    let entry = opcode::Read::new(
                        types::Fd(file.as_raw_fd()),
                        buf.as_mut_ptr(),
                        len as u32,
                    )
                    .offset(0)
                    .build()
                    .user_data(index as u64);

                    // SAFETY: the descriptor and exact-size buffer are moved
                    // into `slots` below. `slots` cannot reallocate, and it
                    // stays alive until every submitted read in this chunk has
                    // completed (or the ring is poisoned and never reused).
                    if unsafe { queue.push(&entry) }.is_err() {
                        results[index] = Some(Err(errno(libc::EAGAIN)));
                        continue;
                    }
                    submitted += 1;
                    slots[offset] = Some(Slot { file, buf });
                }
            }

            // Phase 2: submit the whole chunk and wait for its completions.
            if submitted > 0 {
                match ring.uring.submit_and_wait(submitted) {
                    Ok(_) => {
                        let mut completed = 0usize;
                        {
                            let mut completion = ring.uring.completion();
                            while completed < submitted {
                                let Some(cqe) = completion.next() else {
                                    break;
                                };
                                completed += 1;
                                let Some(offset) =
                                    (cqe.user_data() as usize).checked_sub(chunk_start)
                                else {
                                    continue;
                                };
                                let Some(slot) = slots.get_mut(offset).and_then(Option::as_mut)
                                else {
                                    continue;
                                };
                                let result = cqe.result();
                                results[chunk_start + offset] = Some(if result < 0 {
                                    Err(io::Error::from_raw_os_error(-result))
                                } else if result == 0 {
                                    Ok(Vec::new())
                                } else {
                                    let mut buf = mem::take(&mut slot.buf);
                                    buf.truncate((result as usize).min(buf.len()));
                                    Ok(buf)
                                });
                            }
                        }
                        if completed < submitted {
                            // The wait ended before every completion was
                            // reaped. Keep the remaining buffers alive rather
                            // than freeing memory that may still be in flight.
                            ring.poisoned = true;
                            leak_pending(&mut slots);
                        }
                    }
                    Err(error) => {
                        let code = error.raw_os_error().unwrap_or(libc::EIO);
                        ring.poisoned = true;
                        leak_pending(&mut slots);
                        for offset in 0..chunk_count {
                            if slots[offset].is_some() {
                                results[chunk_start + offset] = Some(Err(errno(code)));
                            }
                        }
                    }
                }
            }

            // Dropping `slots` closes the descriptors and releases the buffers
            // whose reads failed or produced no bytes; successful buffers were
            // moved into `results` above.
        }

        results
            .into_iter()
            .map(|result| result.unwrap_or_else(|| Err(poisoned_error())))
            .collect()
    }
}

thread_local! {
    static WORKER_READER: RefCell<Option<Reader>> = const { RefCell::new(None) };
}

/// Reads one file through the calling thread's persistent reader.
pub(crate) fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    WORKER_READER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(Reader::new()?);
        }
        slot.as_ref()
            .ok_or_else(|| io::Error::other("io_uring reader unavailable"))?
            .read(path)
    })
}

/// Batch read for a worker pool: one ring, one submit wave.
pub(crate) fn read_many(paths: &[PathBuf]) -> io::Result<Vec<io::Result<Vec<u8>>>> {
    let reader = Reader::new()?;
    Ok(reader.read_many(paths))
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
        mem::forget(mem::take(&mut slot.buf));
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
}
