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
| `src/player/pipewire/decode.rs` | FFmpeg demux/decode/resample to interleaved S16 stereo 48 kHz, seek/interrupt/stream-size |
| `src/player/pipewire/worker.rs` | long-lived decode worker and preload worker; pause parking, generation-based load superseding |
| `src/player/pipewire/sink.rs` | PipeWire main-loop thread, stream, RT process callback, gapless wake, stream-state reporting, mixer volume/mute |
| `src/player/pipewire/ring.rs` | lock-free SPSC PCM ring, absolute frame counters, eventfd wakeup |
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
                ring (2 MiB S16) ─► PipeWire RT callback ─► DAC
```

The sink keeps all PipeWire objects on its loop thread. Commands (pause,
volume, shutdown) travel through a PipeWire channel; volume/mute are atomics
read by the process callback. The decoder writes PCM with an epoch; reset/flush
increments the epoch and the RT callback discards queued audio before the
producer resumes, so stale writes can never leak across a load or seek.

## Buffer / latency notes

- FFmpeg URL AVIO read-ahead: **32 MiB** (`buffer_size`), probe limited to
  1 MiB/1 s with a defaults retry for exotic files.
- HTTP I/O uses persistent connections for repeated requests and a **120 s**
  read/write timeout. Rapid skips discard superseded queued loads (a newer load
  generation is dropped before it reaches the decoder).
- Sink PCM ring: **2 MiB** = 524,288 frames = 10.9 s at 48 kHz stereo S16.
  This is intentionally unchanged; it decouples the network/decode jitter from
  the graph without adding seek latency.
- Loading clears when enough decoded audio is buffered (a twentieth of the
  stream size, capped to the ring) or the first PCM is written. Only a load that
  is going to start audio sets the loading flag: the startup restore loads the
  saved queue paused, so the transport reads "Paused" from the first frame
  instead of spinning until a decoder that may never come reports ready.
- For seekable streams, the decoder closes its HTTP response when the PCM ring
  reaches roughly 7/8 (about 9.5 s of queued audio). It opens a fresh response
  and seeks to the decoded boundary when playback drains the ring to roughly
  2 s, keeping long tracks from depending on one idle HTTP connection.
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
cargo test --quiet player::pipewire::native::tests -- --ignored
nix build . --print-out-paths
```

`MUSIKA_STARTUP_TRACE=1` still prints `play.requested`, `first.pcm.written`,
`fetch.done` and `gapless.next` markers.
