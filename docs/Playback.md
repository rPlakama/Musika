# Playback engine

Musika's playback path is Rust-owned. The only C code involved is the external
system `libpipewire` and `libav*` implementations reached through the
`pipewire` and `ffmpeg-next` bindings; there are no `native/*.c` files and no
`cc::Build` audio step.

## Modules

| Module | Responsibility |
| --- | --- |
| `src/player.rs` | queue/state owner and public `Player` API |
| `src/player/pipewire.rs` | module root / `Native` re-export |
| `src/player/pipewire/native.rs` | transport façade, `Native::poll`, gapless boundary and `Player` integration |
| `src/player/pipewire/decode.rs` | FFmpeg demux/decode/resample to interleaved S16 stereo 48 kHz, seek/interrupt/stream-size |
| `src/player/pipewire/worker.rs` | long-lived decode worker and preload worker |
| `src/player/pipewire/sink.rs` | PipeWire main-loop thread, stream, RT process callback and controls |
| `src/player/pipewire/ring.rs` | lock-free SPSC PCM ring, absolute frame counters, eventfd wakeup |
| `src/player/pipewire/status.rs` | shared status/clock/boundary state |
| `src/io_uring_read.rs` | io_uring batch reader (feature `io-uring`, benchmark/tests) |

## Data flow

```
Player command ─► Worker (long-lived decode thread)
                    │  Decoder (FFmpeg, Rust bindings)
                    │  Preload worker opens the next track early
                    ▼
                  Ring (2 MiB S16 stereo ring; producer waits on eventfd)
                    │
PipeWire loop ──► process callback (RT-safe) ─► sink buffer ─► DAC
                    │
                    └─ played-frame counter drives the player clock
```

The sink keeps all PipeWire objects on its loop thread. Commands (pause,
shutdown) travel through a PipeWire channel; mute/volume are atomics read by
the process callback. The decoder thread writes PCM with an epoch; reset/flush
increments the epoch and the RT callback discards queued audio before the
producer resumes, so stale writes can never leak across a load or seek.

## Buffer / latency notes

- FFmpeg URL AVIO read-ahead: **32 MiB** (`buffer_size`), probe limited to
  1 MiB/1 s with a defaults retry for exotic files.
- Sink PCM ring: **2 MiB** = 524,288 frames = 10.9 s at 48 kHz stereo S16.
  This is intentionally unchanged; it decouples the network/decode jitter from
  the graph without adding seek latency.
- Loading clears when enough decoded audio is buffered (a twentieth of the
  stream size, capped to the ring) or playback starts.
- Paused playback blocks the decoder on the eventfd and does not wake it.
- Pausing interrupts and drops the active decoder/HTTP response after capturing
  the played position. Resume opens a fresh media request and seeks the new
  decoder to that position, avoiding reuse of an idle or expired HTTP socket.

## Gapless and clock

When the current decoder reaches EOF and a preloaded decoder is ready, the
worker swaps decoders without flushing the ring and records the absolute ring
frame where the next track begins. The player/UI index and position only
advance when the sink's played-frame counter reaches that boundary, so short
tracks and gapless transitions stay aligned with audible output. Without a
preloaded decoder the worker marks decode completion; the final index advance
waits until the sink drains the ring too.

Predecode starts when the remaining known duration is at most 30 s, or after
the first PCM for an unknown-duration stream. A preloaded decoder stays in one
slot; load/seek/stop cancels it through its interrupt flag.

## Measurements

The verification pass compares the previous C engine with this engine on the
same isolated library/device/config. Both were packaged release builds; the
sample sums voluntary + nonvoluntary context switches over 3 s.

| Engine | Playing switches/3 s | Paused switches/3 s | Threads |
| --- | ---: | ---: | ---: |
| previous C sink/decoder | 3,120 | 2,991 | 20 |
| Rust sink/decoder | 281 | 426 | 20 |

That is an ~11× reduction while playing and a ~7× reduction while paused. The
remaining paused switches are mostly the PipeWire loop/tokio/tray threads, not
the decode worker. Track changes keep the thread count stable (20 tasks).

Functional checks:
- generated WAV open/decode/seek tests (decoder) pass;
- live PipeWire WAV playback, predecoded gapless handoff (`gapless.next` appears
  between the first and second decoders' PCM), pause/resume and seek pass;
- `pw-record` monitor capture during streamed playback is non-silent
  (peak 14974/32767 on one run), confirming real output;
- `cargo test` 397 passed / 16 ignored; `cargo test --features io-uring`
  401 passed / 16 ignored; final `nix build` succeeds.

Local WAV fixtures reached `first.pcm.written` in 9–27 ms; a streamed seek
landed at the requested position (20 s request while paused, 30.08 s after a
seek during playback) and pause froze MPRIS position. The old per-track
`malloc_trim(0)` calls and thread creation are gone by construction. A full
first-PCM/seek/RSS p50/p95 comparison against the retired C engine was not
captured in this pass; the context-switch table and the functional/audio-capture
checks above are the measured evidence. Targets retained for a follow-up
measurement run: no >5 % first-PCM/seek regression, stable thread count, no
underruns in the skip/seek/gapless loop.

## Verification commands

```sh
# full tests (includes ignored PipeWire tests only when explicitly requested)
cargo test --quiet
cargo test --features io-uring --quiet
# live PipeWire transport tests (require a running PipeWire session)
cargo test --quiet player::pipewire::native::tests -- --ignored
nix build . --print-out-paths
```

`MUSIKA_STARTUP_TRACE=1` still prints `play.requested`, `first.pcm.written`,
`fetch.done` and `gapless.next` markers. `Player::debug_report` includes the
sink played/buffered counters for diagnostics.
