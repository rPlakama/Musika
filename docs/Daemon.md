# Musika daemon and UI architecture

Musika runs as two processes:

```
musika        core daemon: databases, native audio, library/sync, MPRIS, tray
  └─ musika-ui   GPUI window child: presentation only, talks IPC to the core
```

The core is the application. `musika-ui` is disposable: closing the window
stops the child, while the core keeps playing and stays in the tray. Launching
`musika` again (or left-clicking the tray icon) opens a child window when none
is open, or raises the existing one, against the still-running core.

## Lifecycle

- `musika` with no mode:
  1. probes `ipc.sock`; if a core is alive it asks it to `show` the window;
  2. otherwise starts a detached `musika --daemon` (setsid, logs in the config
     dir) and waits for the sockets;
  3. asks the core to spawn `musika-ui` and exits.
- `musika --daemon`: runs the core in the foreground (systemd/debug use). It
  never exits when the UI window closes.
- `musika-ui`: the GPUI child. It connects to `daemon.sock`, sends `Hello`, and
  renders the event/player stream. It exits when the window closes or the core
  goes away.
- `qq` (or `quit` over IPC, or a tray-less system shutdown): the UI sends
  `Shutdown` to the core, which flushes the session, shuts down tray/MPRIS,
  closes the child, removes both sockets, and exits.
- Closing the window with the compositor close action is **not** a quit: it
  only ends `musika-ui`.

## Sockets

Both sockets live in the config dir and can be overridden per process:

| Socket | Env override | Direction | Purpose |
| --- | --- | --- | --- |
| `ipc.sock` | `MUSIKA_IPC_SOCKET` | external clients → core | line-based command registry (`play_pause`, `show`, `quit`, ...) |
| `daemon.sock` | `MUSIKA_DAEMON_SOCKET` | UI ↔ core | JSON-lines protocol: `Hello`, bridge requests, `PlayerCommand`, bridge events, player snapshots/ticks, `Activate`, `Shutdown` |

The JSON protocol is versioned by the `Hello { version }` field. Bridge
requests are method-shaped and go to the core's local bridge implementation;
non-player bridge events stream back to the UI. Player state travels as a full
snapshot on connect/queue revision changes and a small 10 Hz tick otherwise.

## Ownership

The core owns SQLite databases, the Subsonic session, playback and native
PipeWire/FFmpeg audio, covers cache, downloads, sync, session status, MPRIS and
the StatusNotifierItem tray. The UI never opens those resources; it reads
already-cached cover files for painting, keeps cursor/scroll/overlay state
locally, and sends every data/player/session action over `daemon.sock`.

External `ipc.sock` commands always work in the core (playback, volume, seek,
shuffle, radio, random, sync, show, quit). Presentation/navigation commands
(`home`, `move_down`, `search`, ...) are forwarded to an attached UI; when no
UI is attached they are ignored except for the core-safe subset.

## Tray

The core registers a StatusNotifierItem (`musika`) with the standard KDE/
freedesktop watcher:

- left click → `show` (spawn/raise `musika-ui`);
- menu: **Shuffle** = `shuffle queue`, **Radio** = `radio current`,
  **Random** = play a random non-empty album from `library.db`.

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
