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

## Gap audit and roadmap (2026-11)

Scope: an audit of `src/player/**`, `src/daemon/playback.rs`, `src/queue.rs`
and this document, plus the questionnaire answers recorded for the playback
roadmap. The audit describes the tree as of 2026-11. The transition/loudness
items from the questionnaire landed in the same month (see "Implemented after
the audit"); the remaining roadmap phases are still plans.

### What the engine has

- PipeWire output as packed S16 stereo at the track's native rate. Sources
  above **192 kHz** are resampled down to `MAX_PIPELINE_RATE`; the sink rebuilds
  its stream per track rate before PCM for that track is written.
- A lock-free SPSC ring allocated with **8 MiB** (2,097,152 frames) up front,
  so the rate-scaled usable limit stays a constant ~10.9 s at any rate.
- An event-driven RT engine: the engine thread blocks on an eventfd written by
  the RT callback, the sink loop and the decode worker, with only a poll
  timeout as a safety net. There is no `pump()`/10 Hz poll.
- FFmpeg demux/decode/resample with a **32 MiB** AVIO read-ahead, a 1 MiB/1 s
  probe plus a defaults retry for exotic files, `+fastseek`, and a **120 s**
  read/write timeout.
- Local-first URLs: a cached media blob is handed to the decoder as a
  `file://` URL and only a cold row uses the provider stream URL.
- Same-rate gapless handoff with a preloaded decoder; at a rate change the
  worker drains the ring and renegotiates the stream, which leaves the
  documented small gap.
- Rate-scaled seek trimming; pause parking that keeps a seekable decoder open,
  and reopen on resume for non-seekable streams or pauses over 30 s.
- A 20 ms output envelope fades pause/resume, seeks and fresh track starts
  (RT callback, no extra thread or allocation) so transitions do not click.
- Optional 5 s equal-length crossfade: the next decoder is preloaded at the
  current pipeline rate, mixed with the outgoing track's tail, and the
  boundary carries the next track's already-played offset; off by default.
- ReplayGain loudness normalization (off/track/album, off by default) reads
  the `replaygain_track_gain` / `replaygain_album_gain` tags captured by the
  decoder and applies the linear gain in the decode worker with clipping.
- Sink failure drops and rebuilds the engine, then replays the current row at
  the same playhead (`CoreCommand::Recover`); ordinary decode/track failures
  are reported and the queue advances.
- MPRIS 2.2 plus the IPC `status`/`queue` snapshots; a pure, deterministic
  `PlaybackCore` policy model covered by 49 tests in `src/test/playback.rs`.
- Live PipeWire tests exist but are `#[ignore]`d, and CI runs
  `cargo test --lib` without an audio device.

### Gaps

**Network resilience.** A transient HTTP open/read failure is not retried:
`open_input` returns the error, `EngineEvent::Failed` stores `last_error` and
the queue advances. There is no bounded retry/backoff, no
reconnect-at-position after a network hiccup, and no offline/latch handling
inside playback. Long tracks are protected by the 7/8-to-2 s ring
drain/reopen policy and the 120 s rw timeout, not by retrying a failed request.

**Buffering visibility.** The published snapshot exposes `loading` and
`last_error` only (IPC `status` adds state/position/duration/volume/muted/
engine_available/current). There is no buffered span, ring fill, decode lead,
underrun/dropout count or first-PCM/seek latency exposed. `Status` carries
`level`/`peak` internally for metering, but they do not reach the snapshot, UI
or IPC.

**Output/format.** The pipeline presents packed S16 stereo only. There is no
F32 path, no S24, no configurable resampler and no dither; swresample converts
to S16 and the sink has no other sample format to negotiate.

**OS/device integration.** Output is PipeWire-only. There is no output-device
enumeration or selection (streams target the default node), no suspend/resume
or hotplug policy beyond the stream-error rebuild, and no wake-lock/inhibit
handling. MPRIS is the only OS-facing surface; no other platform backend
exists.

**Transitions/loudness.** Anti-click fades, optional crossfade and
tag-driven ReplayGain normalization now ship (see below). Still missing: EBU
R128 analysis/scan for untagged files, a user-configurable crossfade length,
F32 processing headroom, and gapless closure of the rate-change gap when
crossfade is off (the normal handoff still drains/renegotiates). Volume stays
node-level with a fixed-point RT fallback. The animated lyric fades in the UI
remain visuals, not audio.

**Safety net/CI.** Errors are strings (`PlaybackOutput::Error(String)`,
`last_error: Option<String>`) rather than structured codes, and there are no
playback counters for underruns, decode lead or transition gaps. Media demux
paths have no fuzz harness. Audio cannot be exercised in CI: live PipeWire
tests are ignored and `ci.yml` runs `cargo test --lib` on runners without an
audio session, so graph-negotiation or RT regressions are only caught by
manual runs.

**Fixed latency-config.** Ring capacity (8 MiB / 2,097,152 frames), the ~20 ms
worker chunk and the parking thresholds are compiled in. There is no
configurable buffer size, target latency or clock policy for different
machines and sinks.

### Recorded decisions (2026-11 questionnaire)

These are the answers recorded for the roadmap. The transitions/loudness and
UI-control answers landed in the 2026-11 pass; the reliability, F32 and CI
items remain roadmap work.

- Reliability: the whole reliability list is in scope (network resilience,
  buffering visibility, structured errors + counters, safety-net tests and CI).
- Format: **F32 internal and output only**. Rejected: S24, a configurable
  resampler, and dither.
- Transitions/loudness: close the rate-change gap, add fade-in/out, optional
  crossfade, and loudness normalization.
- Crossfade UI: a control beside repeat, using the provided
  `src/icons/transition-fade-sharp.svg`, bound to `Ctrl+F`.
- Normalization UI: a bold `N` on the top bar and in fullscreen to the right of
  repeat, bound to `Ctrl+Shift+N`. `Ctrl+N` stays next-track.
- Engineering: playback counters plus structured errors; extend the
  `PlaybackCore` model tests and add media fuzz tests.
- Rejected/deferred: no configurable buffer/latency settings; no deeper
  prefetch beyond the next track.

### Implemented after the audit (2026-11)

- **Fades:** 20 ms linear output envelopes in the PipeWire RT callback for
  pause, resume, seek, manual track changes/stop and the start of a fresh
  track/load. The stream is still deactivated on pause, but only after the
  fade-out has been consumed; gapless and crossfaded handoffs deliberately do
  not double-fade.
- **Crossfade:** `Ctrl+F` toggles an optional 5 s equal-length crossfade. The
  preload worker decodes the next track at the current pipeline rate
  (`Decoder::open_with_output_rate`), the worker mixes the tail/head with
  complementary gains, and the boundary event now reports the next track's
  start offset so the core, MPRIS and UI positions stay correct. With
  crossfade on, a rate change no longer drains/renegotiates the stream; with
  it off the old handoff behavior is unchanged.
- **Normalization:** `Ctrl+Shift+N` cycles off → track → album. The decoder
  captures ReplayGain tags at open (format metadata first, stream metadata
  fallback) and the worker applies the linear gain before the ring, clipping
  at S16. Default off; no EBU R128 scan yet.
- **UI:** a bold `N` chip on the top bar and in fullscreen right of the repeat
  insight, plus `transition-fade-sharp.svg` beside the repeat insight when
  crossfade is on. Mode changes travel through `PlayerOp` and the existing
  `PlaybackState.reserved` bits (no frame-size change). All of it is
  session-only, like volume.
- **Tests:** unit tests for the fade envelope, ReplayGain mode selection and
  clipping, crossfade-aware boundary offset, protocol round-trips and core
  state; the live ignored crossfade test asserts the boundary arrives with a
  multi-second next-track offset.

### Roadmap

The phase order below is the planned implementation order. Each phase is
expected to land with tests and documentation; nothing is committed on a date.

- **Phase 0 — safety net.** Structured playback errors and counters
  (underruns, decode lead, first-PCM/seek latency, transition gaps), media fuzz
  tests, and the test/CI hooks needed to catch regressions before behaviour
  changes.
- **Phase 1 — network resilience.** Bounded retry/backoff and
  reconnect-at-position for transient HTTP failures without skipping the track;
  surface the buffering fields added in Phase 0.
- **Phase 2 — device/system.** Output-device enumeration and selection,
  follow-default handling, suspend/resume and hotplug recovery, and an explicit
  wake/inhibit policy on top of the existing stream-error rebuild.
- **Phase 3 — F32.** Move the decode -> ring -> PipeWire path to F32 (internal
  and output only), keeping S16 only as the fallback where a sink cannot accept
  F32; no S24, resampler configuration or dither.
- **Phase 4 — transitions/loudness/UI binds (landed 2026-11).** Shipped:
  fades, optional crossfade, tag-driven normalization and both UI controls.
  Remaining follow-ups here: configurable crossfade length, EBU R128 fallback
  analysis, and F32 headroom from Phase 3.
- **Phase 5 — docs/measurements.** Update this document and
  `docs/Keybinds.md`, extend the measurement tables with transition, latency
  and RSS numbers for the new paths, and state what remains unimplemented.

### Open notes

- `src/icons/transition-fade-sharp.svg` is now referenced by the fullscreen
  crossfade control and is tracked with that change.
- Loudness normalization ships **default off**, and ReplayGain tags are read
  first: tag-driven gain comes before any analysis or fallback policy.
