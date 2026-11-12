//! SPSC PCM ring shared by the decoder worker and the PipeWire RT callback.
//!
//! `musika` presents interleaved S16 stereo at the current track's sample rate
//! (see [`super::decode`]). The producer (decoder) writes frames ahead; the
//! consumer (PipeWire process callback) reads them. Both positions are absolute
//! frame counters; the ring index is masked because the allocated capacity is a
//! power of two.
//!
//! The storage is allocated once for the highest supported pipeline rate
//! ([`MAX_RATE`]) so it is never reallocated or moved while the RT callback may
//! be reading it: the 524,288-frame reference ring (2 MiB, ~10.9 s at 48 kHz)
//! scaled to [`MAX_RATE`] is 2,097,152 frames, i.e. 8 MiB. The *usable* limit
//! is atomic and set by [`Ring::set_rate`], so a lower rate keeps roughly the
//! same 10.9 s of queued audio in less of the buffer, and a 192 kHz track can
//! use all of it. The 8 MiB is allocated for the whole sink lifetime; that is
//! the deliberate memory trade-off for a reallocation-free ring.
//!
//! The producer blocks on an eventfd when the ring is full instead of polling
//! every millisecond. The consumer signals the eventfd after draining frames
//! and after reset/flush, so a paused stream costs no wakeups.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub(crate) const CHANNELS: usize = 2;
/// Default pipeline rate (Hz): used until a decoder reports the source rate and
/// as the decoder fallback when FFmpeg cannot report one.
pub(crate) const DEFAULT_RATE: u32 = 48_000;
/// Highest pipeline rate (Hz). Sources above it are resampled down by the
/// decoder so the stream format and this ring stay within a bounded range.
pub(crate) const MAX_RATE: u32 = 192_000;
/// Ring length at [`DEFAULT_RATE`]: 524,288 frames = 2 MiB = ~10.9 s.
const REFERENCE_FRAMES: usize = 524_288;
/// Allocated capacity in frames: the reference ring scaled to [`MAX_RATE`].
///
/// `MAX_RATE` is an exact multiple of [`DEFAULT_RATE`], so this is exactly
/// `REFERENCE_FRAMES * (MAX_RATE / DEFAULT_RATE)` = 2,097,152 frames.
pub(crate) const RING_FRAMES: usize =
    REFERENCE_FRAMES * (MAX_RATE as usize / DEFAULT_RATE as usize);
pub(crate) const RING_BYTES: usize = RING_FRAMES * CHANNELS * std::mem::size_of::<i16>();
const RING_MASK: u64 = (RING_FRAMES as u64) - 1;

pub(crate) struct Ring {
    samples: Box<[UnsafeCell<i16>]>,
    /// Frames handed to the consumer (absolute, monotonically increasing).
    write: AtomicU64,
    /// Frames consumed by the consumer (absolute, monotonically increasing).
    read: AtomicU64,
    /// Frames actually played by the RT callback (absolute).
    played: AtomicU64,
    /// Incremented whenever stale producer writes must be rejected.
    epoch: AtomicU64,
    /// Usable frame limit for the current pipeline rate (<= [`RING_FRAMES`]).
    limit: AtomicU64,
    /// Current pipeline rate (Hz).
    rate: AtomicU32,
    closed: AtomicBool,
    failed: AtomicBool,
    reset: AtomicBool,
    flush: AtomicBool,
    paused: AtomicBool,
    muted: AtomicBool,
    /// Volume in 1/1000 units (0..=1000).
    volume: AtomicU32,
    wake_fd: libc::c_int,
}

// SAFETY: `Ring` is a true SPSC buffer shared across threads: the decode
// worker is the only writer and the PipeWire RT callback is the only reader.
// Positions are absolute frame counters published through acquire/release
// atomics; the sample storage is only accessed through its `UnsafeCell`s and
// only inside the range owned by the respective side, so concurrent shared
// access through `&Ring` is sound.
unsafe impl Sync for Ring {}

// SAFETY: moving a `Ring` between threads is sound because it holds no
// thread-affine state: the samples are a plain heap allocation and the wake fd
// is a kernel eventfd usable from any thread. Producer/consumer roles are
// enforced by which side calls `write`/`read_into`, and once the ring is shared
// all access goes through `&Ring`, which is covered by the `Sync` impl above.
unsafe impl Send for Ring {}

impl Ring {
    pub(crate) fn new() -> Result<Self, String> {
        // SAFETY: `eventfd` has no Rust-side preconditions; it creates a new
        // kernel eventfd and returns a descriptor (or -1). A successful
        // descriptor is owned by `Ring` and closed exactly once in `Drop`; a
        // failed one is rejected below and never stored.
        let wake_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if wake_fd < 0 {
            return Err(format!("eventfd: {}", std::io::Error::last_os_error()));
        }
        let samples = (0..RING_FRAMES * CHANNELS)
            .map(|_| UnsafeCell::new(0i16))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            samples,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            played: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            limit: AtomicU64::new(REFERENCE_FRAMES as u64),
            rate: AtomicU32::new(DEFAULT_RATE),
            closed: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            reset: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            volume: AtomicU32::new(1000),
            wake_fd,
        })
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub(crate) fn played_frames(&self) -> u64 {
        self.played.load(Ordering::Acquire)
    }

    pub(crate) fn buffered_frames(&self) -> u64 {
        self.write
            .load(Ordering::Acquire)
            .saturating_sub(self.read.load(Ordering::Acquire))
    }

    /// Absolute frame number of the next producer write. Used as the gapless
    /// boundary marker: the sink's played counter shares this frame space.
    pub(crate) fn written_frames(&self) -> u64 {
        self.write.load(Ordering::Acquire)
    }

    /// Current pipeline rate (Hz).
    pub(crate) fn rate(&self) -> u32 {
        self.rate.load(Ordering::Acquire)
    }

    /// Frames of the allocated ring that the producer may use at the current
    /// pipeline rate (the rest keeps the same temporal headroom at lower rates).
    pub(crate) fn usable_frames(&self) -> u64 {
        self.limit.load(Ordering::Acquire)
    }

    /// Switches the pipeline rate and returns the previous rate (Hz).
    ///
    /// The usable limit preserves the reference budget of ~10.9 s:
    /// `RING_FRAMES * rate / MAX_RATE` frames. A rate change is expected to come
    /// with a flush (the sink does it) because frames already queued were
    /// produced at the previous rate; without a flush the producer simply waits
    /// for the consumer to drain below the new limit.
    pub(crate) fn set_rate(&self, rate: u32) -> u32 {
        let rate = rate.clamp(1, MAX_RATE);
        let usable = (RING_FRAMES as u64 * u64::from(rate) / u64::from(MAX_RATE)).max(1);
        let previous = self.rate.swap(rate, Ordering::AcqRel);
        self.limit.store(usable, Ordering::Release);
        previous
    }

    pub(crate) fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub(crate) fn set_failed(&self, failed: bool) {
        self.failed.store(failed, Ordering::Relaxed);
        self.wake();
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
        self.wake();
    }

    pub(crate) fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub(crate) fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub(crate) fn volume_milli(&self) -> u32 {
        self.volume.load(Ordering::Relaxed)
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub(crate) fn set_volume(&self, volume: f64) {
        let milli = (volume.clamp(0.0, 1.0) * 1000.0).round() as u32;
        self.volume.store(milli, Ordering::Relaxed);
    }

    /// Discards queued audio and rejects all writes from older epochs.
    pub(crate) fn reset(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.closed.store(false, Ordering::Relaxed);
        self.reset.store(true, Ordering::Release);
        self.flush.store(true, Ordering::Release);
        self.failed.store(false, Ordering::Relaxed);
        self.wake();
    }

    /// Discards queued audio but keeps the epoch (seek or format change).
    pub(crate) fn flush(&self) {
        self.flush.store(true, Ordering::Release);
        self.wake();
    }

    pub(crate) fn close_writes(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.wake();
    }

    pub(crate) fn wake(&self) {
        let one = 1u64;
        // SAFETY: `wake_fd` is the eventfd owned by this ring (valid for the
        // ring's lifetime, closed once in `Drop`) and `one` is a live 8-byte
        // `u64` matching the requested size. `EFD_NONBLOCK` keeps the write
        // non-blocking; the return value is ignored because eventfd writes
        // coalesce and a full counter is not an error for a wakeup.
        unsafe {
            _ = libc::write(
                self.wake_fd,
                &one as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            );
        }
    }

    fn drain_wake(&self) {
        let mut value = 0u64;
        // SAFETY: `wake_fd` is the ring's owned eventfd and is still open
        // here; `value` is a live 8-byte `u64` used as eventfd's read buffer.
        // The read is non-blocking and an empty counter is intentionally
        // ignored.
        unsafe {
            _ = libc::read(
                self.wake_fd,
                &mut value as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            );
        }
    }

    fn wait_wake(&self) {
        // The process callback wakes us after consuming frames or on reset,
        // pause/resume and shutdown. The timeout is a safety net for a missed
        // wake (and for a deactivated stream that will not service a pending
        // reset until it is resumed): it guarantees the producer re-checks the
        // epoch/closed/paused flags instead of parking forever.
        let mut pollfd = libc::pollfd {
            fd: self.wake_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` is a single fully initialised `pollfd` whose `fd` is
        // the ring's own eventfd (valid because the ring outlives this call)
        // and whose `revents` field is writable for the duration of the call;
        // `poll` is passed the matching array length of one. A negative `fd`
        // cannot occur here (`Ring::new` rejects it) and poll ignores it
        // anyway.
        unsafe {
            _ = libc::poll(&mut pollfd, 1, 250);
        }
        self.drain_wake();
    }

    /// Writes interleaved stereo frames from the decoder thread.
    ///
    /// Returns early when the sink was reset to a newer epoch or writes were
    /// closed. The caller owns `pcm` and may call again after a retry.
    pub(crate) fn write(&self, pcm: &[i16], epoch: u64) {
        let mut offset = 0usize;
        let total = pcm.len() / CHANNELS;
        while offset < total {
            if self.closed() || self.epoch() != epoch {
                return;
            }
            // The RT callback owns reset/flush: it discards everything written
            // before the reset. Do not race new PCM into that discard window.
            if self.reset.load(Ordering::Acquire) || self.flush.load(Ordering::Acquire) {
                self.wait_wake();
                continue;
            }
            let write = self.write.load(Ordering::Relaxed);
            let read = self.read.load(Ordering::Acquire);
            let used = write.saturating_sub(read);
            let limit = self.limit.load(Ordering::Acquire);
            let room = limit.saturating_sub(used);
            if room == 0 {
                self.wait_wake();
                continue;
            }
            let take = (total - offset).min(room as usize);
            // The mask/bounds use the allocated capacity; `limit` only decides
            // how much of it the current rate may fill.
            let start = (write & RING_MASK) as usize;
            let first = take.min(RING_FRAMES - start);
            let source = &pcm[offset * CHANNELS..(offset + take) * CHANNELS];
            let destination = self.samples[start * CHANNELS..(start + first) * CHANNELS]
                .as_ptr()
                .cast::<i16>() as *mut i16;
            // SAFETY: `start < RING_FRAMES` (masked) and `first <= RING_FRAMES -
            // start`, so the destination range lies inside the allocated
            // `samples` buffer; `source` lies inside the caller's `pcm` slice.
            // The ranges belong to different allocations and the producer owns
            // the absolute range `[write, write + take)`, so no reader touches
            // them concurrently.
            unsafe {
                std::ptr::copy_nonoverlapping(source.as_ptr(), destination, first * CHANNELS);
            }
            if take > first {
                let tail = &pcm[(offset + first) * CHANNELS..(offset + take) * CHANNELS];
                let wrap_destination = self.samples[..(take - first) * CHANNELS]
                    .as_ptr()
                    .cast::<i16>() as *mut i16;
                // SAFETY: the wrapped write starts at the beginning of the
                // buffer and `take - first < take <= RING_FRAMES`, so
                // `[0, take - first)` is inside the allocation; `tail` lies in
                // the caller's slice. The ranges are in different allocations
                // and the producer owns `[write, write + take)`, so no consumer
                // reads them concurrently.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        tail.as_ptr(),
                        wrap_destination,
                        (take - first) * CHANNELS,
                    );
                }
            }
            self.write.store(write + take as u64, Ordering::Release);
            offset += take;
        }
    }

    /// RT callback side: fills `out` with at most `out.len()/2` frames.
    /// Returns the number of frames written (the rest are silence).
    pub(crate) fn read_into(&self, out: &mut [i16], max_frames: usize) -> usize {
        let reset = self.reset.swap(false, Ordering::AcqRel);
        let flush = self.flush.swap(false, Ordering::AcqRel);
        if reset || flush {
            let write = self.write.load(Ordering::Acquire);
            self.read.store(write, Ordering::Release);
            if reset {
                self.failed.store(false, Ordering::Relaxed);
            }
            self.wake();
        }

        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        let available = write.saturating_sub(read) as usize;
        let frames = available.min(max_frames).min(out.len() / CHANNELS);
        if frames > 0 {
            let start = (read & RING_MASK) as usize;
            let first = frames.min(RING_FRAMES - start);
            let source = self.samples[start * CHANNELS..(start + first) * CHANNELS]
                .as_ptr()
                .cast::<i16>();
            // SAFETY: `start < RING_FRAMES` (masked) and `first <= RING_FRAMES -
            // start`, so the source range lies inside the allocated `samples`
            // buffer; `out` has room for `frames * CHANNELS` samples and
            // `first <= frames`. Source and destination are different
            // allocations and the consumer owns `[read, read + frames)`.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    source,
                    out[..first * CHANNELS].as_mut_ptr(),
                    first * CHANNELS,
                );
            }
            if frames > first {
                let wrap_source = self.samples[..(frames - first) * CHANNELS]
                    .as_ptr()
                    .cast::<i16>();
                // SAFETY: the wrapped read starts at the beginning of the
                // buffer and `frames - first < frames <= RING_FRAMES`, so
                // `[0, frames - first)` is inside the allocation; `out` has
                // room for the remaining `frames - first` frames. The ranges
                // are in different allocations and the consumer owns
                // `[read, read + frames)`.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        wrap_source,
                        out[first * CHANNELS..frames * CHANNELS].as_mut_ptr(),
                        (frames - first) * CHANNELS,
                    );
                }
            }
            self.read.store(read + frames as u64, Ordering::Release);
            self.played.fetch_add(frames as u64, Ordering::Relaxed);
            self.wake();
        }
        frames
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        // SAFETY: `wake_fd` was created by `eventfd` in `Ring::new` and is
        // owned by this ring; `Drop` runs exactly once, so the descriptor is
        // closed once and no reference to it can outlive the ring (producer
        // and consumer only hold `Arc<Ring>` clones of this same allocation).
        unsafe {
            _ = libc::close(self.wake_fd);
        }
    }
}

/// Applies mute/gain to an interleaved stereo buffer.
pub(crate) fn apply_gain(out: &mut [i16], muted: bool, volume_milli: u32, frames: usize) {
    if muted {
        out[..frames * CHANNELS].fill(0);
        return;
    }
    if volume_milli == 1000 {
        return;
    }
    let gain = volume_milli.min(1000) as i32;
    for sample in &mut out[..frames * CHANNELS] {
        *sample = ((i32::from(*sample) * gain) / 1000) as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_wraps_and_preserves_frames() {
        let ring = Ring::new().unwrap();
        // Use the full allocated capacity: at the default 48 kHz rate only a
        // quarter of it is usable.
        assert_eq!(ring.set_rate(MAX_RATE), DEFAULT_RATE);
        let epoch = ring.epoch();
        // Fill more than one ring to force a wrap; consume as we go.
        let mut input = vec![0i16; RING_FRAMES * 2];
        for (index, sample) in input.iter_mut().enumerate() {
            *sample = index as i16;
        }
        ring.write(&input, epoch);
        let mut out = vec![0i16; RING_FRAMES * 2];
        assert_eq!(ring.read_into(&mut out, RING_FRAMES), RING_FRAMES);
        assert_eq!(&out[..16], &input[..16]);
        ring.write(&input, epoch);
        assert_eq!(ring.read_into(&mut out, RING_FRAMES), RING_FRAMES);
        assert_eq!(&out[..16], &input[..16]);
    }

    #[test]
    fn usable_limit_scales_with_the_pipeline_rate() {
        let ring = Ring::new().unwrap();
        assert_eq!(ring.rate(), DEFAULT_RATE);
        assert_eq!(ring.usable_frames(), REFERENCE_FRAMES as u64);
        assert_eq!(RING_FRAMES, REFERENCE_FRAMES * 4);
        assert_eq!(RING_BYTES, 8 * 1024 * 1024);

        assert_eq!(ring.set_rate(96_000), DEFAULT_RATE);
        assert_eq!(ring.rate(), 96_000);
        assert_eq!(
            ring.usable_frames(),
            (RING_FRAMES as u64 * 96_000) / u64::from(MAX_RATE)
        );
        assert_eq!(ring.usable_frames(), 1_048_576);

        assert_eq!(ring.set_rate(MAX_RATE), 96_000);
        assert_eq!(ring.usable_frames(), RING_FRAMES as u64);

        assert_eq!(ring.set_rate(44_100), MAX_RATE);
        assert_eq!(
            ring.usable_frames(),
            (RING_FRAMES as u64 * 44_100) / u64::from(MAX_RATE)
        );
    }

    #[test]
    fn usable_limit_keeps_wrap_and_masking_correct() {
        let ring = Ring::new().unwrap();
        assert_eq!(ring.set_rate(DEFAULT_RATE), DEFAULT_RATE);
        let usable = ring.usable_frames() as usize;
        let epoch = ring.epoch();
        let mut input = vec![0i16; usable * CHANNELS];
        for (index, sample) in input.iter_mut().enumerate() {
            *sample = (index % 4096) as i16;
        }
        // Fill exactly the usable share, consume it, then fill again: the
        // second write starts past the old limit and wraps through the mask.
        ring.write(&input, epoch);
        assert_eq!(ring.buffered_frames(), usable as u64);
        let mut out = vec![0i16; usable * CHANNELS];
        assert_eq!(ring.read_into(&mut out, usable), usable);
        assert_eq!(&out[..16], &input[..16]);
        ring.write(&input, epoch);
        assert_eq!(ring.buffered_frames(), usable as u64);
        assert_eq!(ring.read_into(&mut out, usable), usable);
        assert_eq!(&out[..16], &input[..16]);
    }

    #[test]
    fn gain_is_fixed_point() {
        let mut out = [1000i16, -1000, 32767, -32768];
        apply_gain(&mut out, false, 500, 2);
        assert_eq!(out, [500, -500, 16383, -16384]);
        apply_gain(&mut out, true, 1000, 2);
        assert_eq!(out, [0, 0, 0, 0]);
    }

    #[test]
    fn reset_rejects_stale_epoch_writes() {
        let ring = Ring::new().unwrap();
        let old = ring.epoch();
        ring.reset();
        assert_ne!(ring.epoch(), old);
        let pcm = vec![1i16; 2 * RING_FRAMES];
        ring.write(&pcm, old);
        assert_eq!(ring.buffered_frames(), 0);
    }
}
