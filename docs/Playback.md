# Playback engine

Musika drives audio playback through a single persistent, crash-isolated,
headless **`mpv`** subprocess over a private Unix-domain JSON IPC socket.
There are no PipeWire, ALSA, or FFmpeg Rust FFI bindings in `Cargo.toml`;
the Nix flake packages a stripped audio-only `mpvAudio` derivation
(`mpv-unwrapped.override` inheriting `ffmpeg-headless` with PipeWire + ALSA
support and all video/GUI/scripting/disc features disabled) as a runtime
dependency of the headless `musika` core.

## Modules

| Module | Responsibility |
| --- | --- |
| `src/player.rs` | shared queue/transport/output vocabulary (`Track`, `Repeat`, `Normalization`, `State`, `OutputDevice`, `list_output_devices`, startup warmup) |
| `src/player/core.rs` | `PlaybackCore`: pure, deterministic queue/transport policy (`CoreCommand` → `Outcome`, `EngineEvent` → update) |
| `src/player/error.rs` | `PlaybackError` / `ErrorKind` / `ErrorStage`: structured error classification for retry/offline/advance decisions |
| `src/player/engine.rs` | `Engine`: event-driven std thread sleeping in `poll(2)` on the command `eventfd` and the MPV IPC socket |
| `src/player/mpv.rs` | MPV module root and live `audio-device-list` cache (`list_output_devices` fallback via `mpv --audio-device=help`) |
| `src/player/mpv/protocol.rs` | pure JSON IPC command encoders, event/reply parsers, argv builder, device normalizer, and error log correlator |
| `src/player/mpv/process.rs` | persistent `MpvProcess` lifecycle: private runtime socket, `PR_SET_PDEATHSIG(SIGTERM)`, inherited-fd closing, non-blocking IPC framing |
| `src/player/mpv/transport.rs` | `MpvTransport`: maps `AudioCommand`s to MPV IPC, tracks `playlist_entry_id` for gapless handoff, emits `EngineEvent`s and clock counters |
| `src/player/mpv/eq.rs` | Equalizer APO → MPV `lavfi` translation layer (`Preamp`, parametric filters, and piecewise-linear `GraphicEQ` via `firequalizer`), run once in the daemon/IPC layer |
| `src/eq.rs` | backend-agnostic Equalizer APO profile parser & directory loader (including AutoEq `Filter N:`, `LSQ`/`HSQ`, `BW Oct`, and up to 128 `GraphicEQ` bands) |

## Architecture & data flow

```text
PlaybackCore ─► AudioCommand ─► Engine (poll thread) ─► MpvTransport ─► Unix JSON IPC ─► mpv (--idle=yes --no-video)
     ▲                                                                                          │
     └─────────────────── EngineEvent (Loaded, FirstPcm, Boundary, Ended, ...) ◄────────────────┘
                                                                                                │
                                                                          --ao=pipewire / --ao=alsa ─► DAC
```

- **Single persistent process**: `mpv` starts once at daemon startup (`--idle=yes --no-video --no-config --load-scripts=no --gapless-audio=yes --audio-client-name=musika`). No process is spawned per track.
- **Zero orphan guarantee**: `pre_exec` sets `PR_SET_PDEATHSIG(SIGTERM)` and closes inherited file descriptors (`crate::linux::close_inherited_fds`), and `Drop` sends `quit` with a bounded 200 ms grace period before `SIGKILL` + socket unlink.
- **Event-driven engine**: `Engine` sleeps in `poll(2)` on both its command `eventfd` and `mpv`'s non-blocking Unix socket, waking immediately on commands or MPV events (`file-loaded`, `playback-restart`, `end-file`, `property-change`, `log-message`).
- **Crash isolation & recovery**: If `mpv` exits unexpectedly or the IPC socket drops, `MpvTransport` emits `EngineEvent::SinkFailed`. `PlaybackService` drops and rebuilds the engine, syncs volume/mute/EQ/normalization/output settings, and issues `CoreCommand::Recover { position }` to resume at the same playhead without skipping the track.

## Features mapped natively to MPV

- **Gapless playback**: `Load` queues the current URL (`loadfile <url> replace`) and the next URL (`loadfile <next> append`) with `--gapless-audio=yes`. `MpvTransport` tracks `playlist_entry_id` so stale `end-file` events from rapid skips are ignored; when MPV advances to `next_entry_id`, `MpvTransport` trims index `0` (`playlist-remove 0`) to keep the playlist bounded and emits `EngineEvent::Boundary`.
- **ReplayGain loudness normalization**: `Ctrl+Shift+N` / `:normalize <off|track|album>` maps to MPV's `replaygain` property (`no`/`track`/`album`) with `--replaygain-clip=no`.
- **Equalizer APO profiles**: `Ctrl+E` / `:eq <next|off|list|profile-name>` parses `~/.config/musika/eqs/*.{txt,eq}` via `src/eq.rs` (including AutoEq `Filter N:`, `LSQ`/`HSQ`, `BW Oct`, and up to 128 `GraphicEQ` bands) and translates them once in the daemon/IPC layer via `src/player/mpv/eq.rs` into an FFmpeg `lavfi=[...]` string (`volume`, parametric biquads, and `firequalizer` for exact piecewise-linear `GraphicEQ`) applied live via MPV's `af` property. `:eq list` returns `{active, mpv_af, profiles}` over IPC.
- **Output devices, backends, exclusive & bit-perfect**:
  - `:output list` reads MPV's live observed `audio-device-list` (falling back to `mpv --audio-device=help` only when no engine is running).
  - `:output <device>` / `:output default` sets `audio-device` live (automatically prefixing bare legacy PipeWire node names with `pipewire/`). If a pinned PipeWire device disappears from `audio-device-list`, the transport reverts to `auto` and emits `EngineEvent::OutputFallback`.
  - `:output backend pipewire|alsa` and `:output alsa <device>` switch `--ao` live by rebuilding the MPV instance cleanly at the current playhead.
  - `:output exclusive on|off` sets `audio-exclusive=yes|no`.
  - `:output bitperfect on|off` clears `af`, `replaygain`, and `audio-samplerate`.
  - `:stream fixed <Hz|off>` sets `audio-samplerate` (`0` = native track rate).
- **Network resilience & buffering**: MPV runs with `--cache=yes --demuxer-max-bytes=8MiB --demuxer-max-back-bytes=2MiB --demuxer-readahead-secs=20` (with `argv[0]` set to `musika-mpv`) and FFmpeg reconnect options (`reconnect=1,reconnect_streamed=1,reconnect_delay_max=4,timeout=120000000`). `paused-for-cache` drives `EngineEvent::Buffering(bool)` and the `underruns` counter. Recent `warn`/`error` `log-message` lines are correlated with `end-file` errors so HTTP `401`/`403`/`404`/`5xx`, DNS, and codec errors classify accurately through `PlaybackError::classify`.

## Verification commands

```sh
nix develop --command cargo clippy --all-targets -- -D warnings
nix develop --command cargo test --lib
nix develop --command cargo test --lib --no-default-features
nix develop --command cargo test --lib --features io-uring io_uring_read
nix build . --print-out-paths
```

`src/test/playback.rs` includes both deterministic mock-socket IPC integration
tests (no audio hardware required) and a real headless `mpv --ao=null`
integration test exercising `Loaded`, `FirstPcm`, `Pause`/`Resume`, gapless
`Boundary`, `Ended`, and clean shutdown against generated WAV fixtures.
