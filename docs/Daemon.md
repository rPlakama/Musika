# Musika daemon and UI architecture

Musika runs as two processes:

```
musika        core daemon: databases, playback, library/sync, MPRIS, tray, both sockets
  └─ musika-ui   GPUI window child: presentation only, speaks the framed core protocol
```

The core is the application. `musika-ui` is disposable: closing the window
stops the child, while the core keeps playing and stays in the tray. Launching
`musika` again (or left-clicking the tray icon) opens a child window when none
is open, or raises the existing one, against the still-running core.

## Lifecycle

- `musika` with no mode:
  1. probes `ipc.sock`; if a core is alive it asks it to `show` the window;
  2. otherwise starts a detached `musika --daemon` (setsid, logs in the config
     dir), waits for the sockets and asks the core to spawn `musika-ui`; then
     the CLI process exits.
- `musika --daemon`: runs the core in the foreground (systemd/debug use). It
  never exits when the UI window closes.
- `musika-ui`: the GPUI child. It connects to `daemon.sock`, sends `Hello`, and
  renders the state/queue/event stream. It exits when the window closes, the
  core goes away, or the core sends `Bye`.
- `qq` (or `quit` over IPC, or the tray Quit item): the UI asks the core to
  quit; the core answers `Bye { quit }`, flushes the session, shuts down
  tray/MPRIS/audio, closes the child, removes both sockets, and exits.
- Closing the window with the compositor close action is **not** a quit: it
  only ends `musika-ui`.

## Sockets

Both sockets live in the config dir and can be overridden per process. Both are
user-private: parent dir `0700`, socket `0600`, and every connection is checked
with `SO_PEERCRED` (same uid only).

| Socket | Env override | Direction | Purpose |
| --- | --- | --- | --- |
| `ipc.sock` | `MUSIKA_IPC_SOCKET` | external clients → core | line-based command registry (`play_pause`, `show`, `quit`, `status`, ...) |
| `daemon.sock` | `MUSIKA_DAEMON_SOCKET` | UI ↔ core | framed core/UI protocol v2: handshake, player requests, state, queue deltas, bridge requests/events, heartbeat |

`ipc.sock` keeps the historical text contract: existing commands stay
fire-and-forget, and the read-only `status`/`queue` commands answer with one
JSON line. Lines are capped at 64 KiB, a client gets 30 s to send one, and each
connection has its own reader thread.

## Core/UI protocol

`daemon.sock` is framed, not JSON-lines, and both binaries must be the same
build. Every message is:

```
frame := u32 payload_len | u8 kind | u8 flags | u16 reserved | payload
```

The header is 8 bytes, integers are little-endian, a payload may not exceed
4 MiB. Payloads are fixed-layout binary on the hot path (`playback_state`,
`player_request`/`player_reply` (about 14 bytes), `queue_delta`, `ping`/`pong`,
`activate`) and JSON for the cold control path (`hello`/`welcome`/`reject`/
`bye`/`protocol_error`, the method-shaped `request_json`/`reply_json` bridge
requests and the 38 `event_json` bridge events). Version comes first: the client
sends `Hello { proto, role: ui|control, client, features }` and the core answers
`Welcome { proto, core_version, session, features }`. A different `proto` gets
`Reject { code: version_mismatch }` and the connection closes, so mismatched
builds fail loudly. Bridge requests stay method-shaped (about 58 methods) but
are now correlated by id and acked.

## Playback state and queue

`PlaybackState` is a fixed 48-byte anchor, not a tick: `seq`, `queue_rev`,
`anchor_monotonic_ns`, `index`, `duration_ms`, `position_ms`, `flags` (muted,
loading, gapless-armed, engine-unavailable, seeking, ended), `volume_milli`,
`state`, `repeat` and `rate_milli`. The core publishes it on change plus a ~5 s
drift correction while playing; `musika-ui` extrapolates the playhead locally
from the anchor against `CLOCK_MONOTONIC`. Paused/stopped playback is silent
apart from the 5 s heartbeat.

The queue is sent as deltas, never re-sent whole on a row edit: `Reset { count }`
followed by 256-row `QueueRows` chunks for a first load or resync, then `Insert`,
`Remove`, `Move` and `Patch` (favourite) as rows change. `rev` is monotonic; a
gap tells the client to resync. Row bodies omit the resolved stream URL (it
carries provider credentials): id, title, artist, album, album id, favourite,
duration and the cover fields. Player requests are correlated by `req` and
typed (play/pause, next/previous, volume, mute, seek, jump, queue edits, repeat,
shuffle, favourite, `resync`). Relative controls (`seek_back`/`seek_forward` =
±5 s, `volume_up`/`volume_down`) are resolved by the core against its
authoritative clock, never against a UI copy.

## Reliability

Each client has a writer task with a bounded ordered queue plus a single
"latest state" slot: playback states coalesce, while queue deltas, events and
replies are never silently dropped. A client that stops draining is closed and
resyncs on reconnect. A 5 s heartbeat detects a dead peer (core sends `ping`,
client answers `pong`); four silent intervals close the connection. Malformed,
oversized or unknown frames get a typed `ProtocolError` then `Bye { protocol }`.
`musika-ui` reconnects with a 50 ms → 1 s backoff and re-handshakes/resyncs, and
only quits on `Bye { quit|shutdown }`; a protocol mismatch shows a
"rebuild/restart" error instead of a half-broken session.

## Ownership

The core owns SQLite databases, the Subsonic session, playback and native
PipeWire/FFmpeg audio, covers cache, downloads, sync, session status, MPRIS and
the StatusNotifierItem tray. The UI never opens those resources; it reads
already-cached cover files for painting, keeps cursor/scroll/overlay state
locally, and sends every data/player/session action over `daemon.sock`. It keeps
no optimistic player mirror: transport and queue state are whatever the core
last published. External `ipc.sock` commands always work in the core (playback,
volume, seek, shuffle, radio, random, sync, status/queue, show, quit);
presentation/navigation commands (`home`, `move_down`, `search`, ...) are
forwarded to an attached UI and ignored without one.

## Tray

The core registers a StatusNotifierItem (`musika`) with the standard KDE/
freedesktop watcher:

- left click → `show` (spawn/raise `musika-ui`);
- menu: **Shuffle** = `shuffle queue`, **Radio** = `radio current`,
  **Random** = play a random non-empty album from `library.db`,
  **Quit** = `quit` (flush status, close the UI child and stop the core).

Without a tray host the core still runs; the next `musika` launch opens the
window. Under niri the UI also asks niri to focus its own window on an explicit
`show` request, because Wayland/GPUI cannot activate applications itself.

## systemd (optional)

```ini
# ~/.config/systemd/user/musika.service
[Unit]
Description=Musika core daemon

[Service]
ExecStart=%h/.nix-profile/bin/musika --daemon
Restart=on-failure
Environment=MUSIKA_IPC_SOCKET=%t/musika/ipc.sock
Environment=MUSIKA_DAEMON_SOCKET=%t/musika/daemon.sock

[Install]
WantedBy=default.target
```

Start the UI with `musika` (or the desktop entry) after the service is up.

## Logs

- daemon: `$XDG_CONFIG_HOME/musika/musika-daemon.log` when started by the
  `musika` launcher (foreground `--daemon` logs to the terminal);
- UI child: `$XDG_CONFIG_HOME/musika/musika-ui.log`;
- frame/startup diagnostics keep their existing `MUSIKA_FRAME_STATS` /
  `MUSIKA_STARTUP_TRACE` / `MUSIKA_DEBUG` switches.
