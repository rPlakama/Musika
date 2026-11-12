# Playback engine

Musika's playback path is Rust-owned. The only C code involved is the external
system `libpipewire` and `libav*` implementations reached through the
`pipewire` and `ffmpeg-next` bindings; there are no `native/*.c` files and no
`cc::Build` audio step.

## Modules

| Module | Responsibility |
| --- | --- |
| `src/player.rs` | queue model and shared types (`Track`, `Repeat`, `State`) |
| `src/player/core.rs` | `PlaybackCore`: pure, deterministic queue/transport policy (`CoreCommand` → `Outcome`, `EngineEvent` → update) |
| `src/player/engine.rs` | `Engine`: std-thread actor over the PipeWire sink and FFmpeg workers; emits `EngineEvent`s |
| `src/player/pipewire.rs` | module root / `Native` and `SinkState` re-export |
| `src/player/pipewire/native.rs` | blocking transport façade over sink + worker + sink-driven clock, event accessors and wake descriptor |
| `src/player/pipewire/decode.rs` | FFmpeg demux/decode/resample to interleaved S16 stereo at the track's native rate (capped at 192 kHz), seek/interrupt/stream-size |
| `src/player/pipewire/worker.rs` | long-lived decode worker and preload worker; format adoption, pause parking, rate-aware chunking, rate-change gapless handoff, generation-based load superseding |
| `src/player/pipewire/sink.rs` | PipeWire main-loop thread, per-track-rate stream (re)negotiation, RT process callback, gapless wake, stream-state reporting, mixer volume/mute |
| `src/player/pipewire/ring.rs` | lock-free SPSC PCM ring with rate-scaled usable capacity, absolute frame counters, eventfd wakeup |
| `src/player/pipewire/status.rs` | shared status/clock/boundary state and the engine eventfd |
| `src/io_uring_read.rs` | io_uring batch reader (feature `io-uring`, benchmark/tests) |

## Service shape

The core owns playback in one `musika-playback` thread holding a `PlaybackCore`
(queue and transport policy, no threads/audio/tokio) and an `Engine` (the
transport actor). It sleeps on its channel and wakes on commands or engine
events: a 25 ms tick while playing/loading, 250 ms idle.

The old `Player::pump()`/10 Hz poll is gone. Gapless boundary crossings, EOF,
decoder failures and PipeWire stream errors arrive as engine events; the engine
thread blocks on an eventfd written by the sink's RT callback, the sink loop and
the decode worker, with a 10 ms (playing/loading) or 250 ms (idle) poll timeout
only as a safety net. The daemon consumes `PlaybackOutput`s and publishes a
snapshot, so MPRIS, the status flush and a new UI never touch the playback
thread.

## Data flow

```
PlaybackCore ─► AudioCommand ─► Engine ─► Native (sink + decode/preload workers)
     ▲                                                │
     └────── EngineEvent (boundary, EOF, failure) ◄───┘
                                                      │
                ring (8 MiB S16 capacity) ─► PipeWire RT callback ─► DAC
```

The sink keeps all PipeWire objects on its loop thread. Commands (pause,
volume, format change, shutdown) travel through a PipeWire channel; volume/mute
are atomics read by the process callback. The decoder writes PCM with an epoch;
reset/flush increments the epoch and the RT callback discards queued audio
before the producer resumes, so stale writes can never leak across a load or
seek.

## Format-aware PCM pipeline

The transport is no longer fixed to S16 stereo 48 kHz. The decoder reports the
track's sample rate from the FFmpeg codec parameters (falling back to the first
decoded frame and then 48 kHz), and swresample only converts the sample
format/channel layout to packed S16 stereo. No sample-rate conversion happens
unless the source is above **192 kHz**: those are resampled down to 192 kHz,
the highest rate the pipeline presents. Everything that derives from the rate
scales with it:

- The worker reads in ~20 ms chunks (`rate / 50`, clamped), so seek/stop
  latency stays constant in time rather than in frames; its PCM scratch buffer
  is sized for the largest chunk (20 ms at 192 kHz).
- The parking low-water mark is 2 s of frames at the current rate and the
  parked/full threshold is 7/8 of the ring's rate-scaled usable capacity.
- The sink rebuilds its PipeWire stream (S16LE, stereo) to the track's rate
  before the worker writes any PCM for it. A rebuild is used because changing
  the format of a connected `pw_stream` in place is not supported by the
  binding; the old stream is destroyed only after the replacement connected.
  The decode worker blocks in `Sink::set_format` until the loop thread applied
  the change, so the ring rate and the graph format can never disagree when
  PCM starts.
- The paused-position math in `Native::set_paused` divides the played-frame
  delta by the current pipeline rate.

A rate change flushes the queued PCM (it was produced at the previous rate).

## Buffer / latency notes

- FFmpeg URL AVIO read-ahead: **32 MiB** (`buffer_size`), probe limited to
  1 MiB/1 s with a defaults retry for exotic files.
- HTTP I/O uses persistent connections for repeated requests and a **120 s**
  read/write timeout. Rapid skips discard superseded queued loads (a newer load
  generation is dropped before it reaches the decoder).
- Sink PCM ring: reference budget of 524,288 frames = 2 MiB = 10.9 s at
  48 kHz stereo S16 (**8 MiB** = 2,097,152 frames allocated up front so a
  192 kHz track keeps the same ~10.9 s without a reallocation under the RT
  callback). The atomic usable limit is `RING_FRAMES * rate / 192_000` frames,
  so the queue length is constant in time at every rate; the memory trade-off
  is deliberate.
- Loading clears when enough decoded audio is buffered (a twentieth of the
  stream size, capped to the ring) or the first PCM is written. Only a load that
  is going to start audio sets the loading flag: the startup restore loads the
  saved queue paused, so the transport reads "Paused" from the first frame
  instead of spinning until a decoder that may never come reports ready.
- For seekable streams, the decoder closes its HTTP response when the PCM ring
  reaches roughly 7/8 of its rate-scaled usable capacity (about 9.5 s of queued
  audio). It opens a fresh response and seeks to the decoded boundary when
  playback drains the ring to roughly 2 s of frames at the current rate,
  keeping long tracks from depending on one idle HTTP connection.
- Pause keeps a seekable decoder open and stops reading it; the ring keeps its
  buffered audio, so resume continues from the same sample without reopening or
  seeking. Non-seekable streams are interrupted on pause and reopened at the
  paused position on resume, and a pause longer than 30 s also falls back to
  reopen (an idle HTTP connection is not reliable).

## Gapless and clock

When the current decoder reaches EOF and a preloaded decoder is ready, the
worker swaps decoders without flushing the ring and records the absolute ring
frame where the next track begins. The sink's RT callback writes the shared
engine eventfd when the played-frame counter reaches that boundary, and the
playback core advances the index and position only then, so short tracks and
gapless transitions stay aligned with audible output. Without a preloaded
decoder the worker marks decode completion; the final advance waits until the
sink drains the ring too.

A gapless handoff requires both tracks to share the pipeline rate, which is the
common case. When the preloaded track has a different rate, swapping immediately
would replay the queued old-rate frames at the new speed, so the worker instead
keeps playing the old track's queued PCM, drains the ring to zero and only then
renegotiates the stream, swaps the decoder and arms the boundary. That leaves a
small audible gap at the rate change (a few graph periods plus the local stream
rebuild), which is accepted; same-rate handoffs remain fully gapless.

Predecode starts when the remaining known duration is at most 30 s, or after
the first PCM for an unknown-duration stream. A preloaded decoder stays in one
slot; load/seek/stop cancels it through its interrupt flag.

## Volume and mute

Volume and mute are pushed to the PipeWire stream node on the loop thread
(`SPA_PROP_channelVolumes` and `SPA_PROP_softMute`), so system mixers and
`wpctl`/`pavucontrol` show the app volume. The RT callback keeps the
fixed-point `volume_milli` gain as its fallback path and applies mute by
zeroing the buffer; after the node control is accepted the sample path is left
at unity, so the signal is not attenuated twice. A refused stream control is
logged once.

## Stream-error recovery

A PipeWire `StreamState::Error` becomes `EngineEvent::SinkFailed`. The playback
service then drops the engine, builds a fresh one and sends `Recover`, which
reloads the current row at the same playhead instead of skipping the track. A
stream that comes back live reports `SinkRecovered`; ordinary decode/track
failures still report `Failed` and advance the queue as before.

## Measurements

The verification pass compares the previous C engine with this engine on the
same isolated library/device/config: packaged release builds, voluntary +
nonvoluntary context switches summed over 3 s.

| Engine | Playing switches/3 s | Paused switches/3 s | Threads |
| --- | ---: | ---: | ---: |
| previous C sink/decoder | 3,120 | 2,991 | 20 |
| Rust sink/decoder | 281 | 426 | 20 |

That is ~11× fewer switches while playing and ~7× while paused; the remaining
paused switches are the engine/sink/tokio/tray threads, not the decode worker.
Track changes keep the thread count stable. Functional checks: generated WAV
open/decode/seek tests pass; live PipeWire WAV playback, predecoded gapless
handoff (`gapless.next` between the first and second decoders' PCM),
pause/resume and seek pass; `pw-record` monitor capture during streamed playback
is non-silent (peak 14974/32767 on one run); `cargo test` and
`cargo test --features io-uring` pass, and `nix build` succeeds. Local WAV
fixtures reached `first.pcm.written` in 9–27 ms and a streamed seek landed at
the requested position; a full first-PCM/seek/RSS p50/p95 comparison against the
retired C engine was not captured in this pass.

## IPC telemetry / querying

The core exposes the playback snapshot without touching the playback thread:

```sh
printf 'status\n' | nc -U ~/.config/musika/ipc.sock
```

`status` answers with one JSON line: `state`, `index`, `queue_rev`, `duration`,
`position`, `volume`, `muted`, `loading`, `engine_available`, `current`
(`{id,title,artist,album}` or `null`) and `last_error`; `queue` answers with
`rev`, `index`, `len` and `ids`. See `docs/Commands.md` and `docs/Daemon.md`.

## Verification commands

```sh
# full tests (includes ignored PipeWire tests only when explicitly requested)
cargo test --quiet
cargo test --features io-uring --quiet
# live PipeWire transport tests (require a running PipeWire session)
cargo test --quiet player::pipewire::sink::tests -- --ignored
cargo test --quiet player::pipewire::worker::tests -- --ignored
nix build . --print-out-paths
```

`MUSIKA_STARTUP_TRACE=1` still prints `play.requested`, `first.pcm.written`,
`fetch.done` and `gapless.next` markers. `player::pipewire::decode` tests cover
native-rate decoding (44.1 kHz), the 192 kHz cap, rate-scaled seek trimming and
the ring's rate-scaled usable limit; the ignored sink test covers a live stream
rebuild to 44.1/96 kHz, and the ignored worker tests cover a live same-rate
gapless handoff plus the drain/renegotiate/swap path at a rate change.
