# Musika

<p align="center">
  <img src="src/icons/musika.svg" alt="Musika" width="144">
</p>

Musika is a Keyboard driven Navidrome/Subsonic Client made with AI assistance. Written in Rust and GPUI. It contains many functionalities, such as _vi motions_, _cache & download system_, an indexed db based search, caching in covers, and other performance extras. Everything is thoroughly tested by me. Mostly coding is made by AI and me.

# Features

1 - Alt-Menu -- A multipoint(Alt) call for Tracks, Albums. Which is capable of transporting through other sections, or executing context-driven actions (like bulk-download, favoriting, etc;). Four quadrants: Add to Queue (N), Favorite (S), Go to Artist (W), Go to Album (E — on an album row the east slot carries the download toggle instead), aimed with `h/j/k/l` or the arrows (mouse works too), fired on Alt release, cancelled by Esc or a neutral aim. With a visual selection up (`v`/`V`) the verbs apply to the whole range, including a bulk **Download** that toggles: a selection already cached is removed instead.

2 - Download System -- Local-first, two ledgers in `media.db` and two tiers: the artifact tables own the blobs + the container *rules*, the download ledger owns the intent and progress (queued → active → done/error). An explicit song download is `permanent`; selecting an album or an artist writes a rule, downloads every existing track and keeps catching new ones on sync — toggling it off removes the rule and its files. Permanent/rule blobs live under `~/.config/musika/media`, the purgeable playback tier under `~/.cache/musika/media`. Row glyphs show the state (outline → spinning loop → filled). Every scope has a **Downloaded** ordering that shows only local downloads, and a 5 s server health probe switches the app to that downloaded catalog automatically when the server stops answering — restoring the full library on the first successful ping, without a toast. When the app opens, the first successful ping runs one incremental library sync so the local catalog starts current; a sync interrupted by the server going away pauses and replays when the connection returns.

3 - Search -- `/` searches the surface in front of you (literal, smartcase, viewport-scoped) with in-place match blocks and `[i/N]` counts; Enter jumps, Esc abandons. The global overlay uses a hybrid search (ARC-SEARCH): the in-process Nucleo fuzzy index answers first, and SQLite FTS5 fills any column that comes back short. `MUSIKA_SEARCH_ENGINE=nucleo` or `fts` pins one engine. The fuzzy catalog is held to a memory budget (`MUSIKA_SEARCH_INDEX_BUDGET_MIB`, 64 MiB by default); a library past it is searched by FTS instead, and `0` restores the old unlimited catalog.

4 - Playback -- a lightweight, crash-isolated MPV audio engine drives playback through a single persistent headless `mpv` subprocess over a private Unix-domain JSON IPC socket (`--no-video --no-config --gapless-audio=yes`): queue order, persistent shuffle, history-aware previous, repeat, sleep timer, seek/volume, MPRIS, automatic crash/stream-error recovery, native gapless playback, ReplayGain loudness normalization, Equalizer APO-style EQ profiles compiled to MPV `lavfi` filters, and synced lyrics (OpenSubsonic) that ride the shared scroll body. Playback runs in the headless core as an event-driven thread (`poll(2)` on the command eventfd + MPV IPC socket), so the window only paints the state the core publishes. A transient stream failure reopens the same row at the same position with bounded backoff; only a genuinely unreachable server stops playback with **“Unable to play track, server unavailable”**, and the next successful health probe resumes it once. 404, auth and decoder failures stay online and keep their own error. Output devices are selectable (`output`), follow the default (`auto`), and support both PipeWire (`output backend pipewire`) and direct ALSA (`output backend alsa` / `output alsa hw:0,0`) with optional `audio-exclusive` and DSP bypass (`output bitperfect on`). Streams can be bitrate/format-capped with a background playback cache; completed plays scrobble to the server, with offline retries.

5 - Covers, lyrics and palettes are each cached in their own DB and purged on sync — nothing derived is fetched twice. Album art, artist portraits and playlist artwork are cached locally; artist portraits and playlist-specific covers are warmed by a detached background pass after sync. Playlist pages use a fixed collage of album originals (the provider mosaic is only a fallback), so artwork is sharp and does not flicker as the cache warms.

6 - GPUUI -- the interface runs on GPUI, Zed's GPU-accelerated hybrid immediate/retained UI framework. The Qt/QML front end is gone; the shell, library and entry pages, search, queue, fullscreen player, lyrics, overlays, Alt actions and session restore were all ported to the GPU renderer.

# Architecture

Musika is a headless core plus a disposable GPU window:

```
musika        core daemon: databases, native audio, library/network/sync,
              MPRIS, tray and the external IPC socket
  └─ musika-ui   GPUI child: renders state and sends IPC requests
```

Closing the window leaves `musika` playing in the tray; running `musika` again
(or left-clicking the tray icon) reopens a window against the live queue and
page. `qq` at Home is the true quit. See [docs/Daemon.md](docs/Daemon.md) and [docs/Playback.md](docs/Playback.md) for
the process lifecycle, sockets and systemd sample.

# Installation

Currently, it's a nix flake based installation; However, I plan in the future to provide another packaging schemes for MUSIKA. The flake installation follows as below:

in yer flake inputs.

```
musika.url = "github:rPlakama/musika";

```

then as a user/system package.

```
inputs.musika.packages.${pkgs.system}.default
```

# Sections

- Library scopes: Albums, Songs, Artists, Playlists. Shift-J/K cycles the scope; Shift-H/L cycles the ordering (Artist → Name → Year → Recent → **Downloaded** for albums and songs, Name → Albums → **Downloaded** for artists, Name → **Downloaded** for playlists). The Downloaded ordering filters the scope to local downloads. If the server stops answering, the top bar shows **Offline** and every surface serves that downloaded catalog until a ping succeeds. Radio opens from a row's Alt menu.
- Fullscreen player (Enter on a song): transport, seek, volume, and the lyrics pane.
- Queue drawer (`Ctrl-Q` slides it in from any view, library or fullscreen): `j/k`, Enter jumps, `Ctrl-Shift-J/K` reorder, `dd` removes, `q` or `Ctrl-Q` closes (Esc is a vi key and only ends a visual selection).
- Clipboard: `y`/`Y` yank the cursor row (or the visual range) from any list — Albums, Songs, Artists, album/artist pages, search and the queue — into the row clipboard; `p`/`P` paste it below/above the cursor in the open queue drawer. `Ctrl-N`/`Ctrl-P` skip tracks anywhere.
- Search overlay: hybrid Nucleo + FTS5 browser (`MUSIKA_SEARCH_ENGINE=nucleo` or `fts` pins one engine).

# Documentation

- [Keybinds](docs/Keybinds.md) — the full keyboard map for the library, queue,
  fullscreen player, search, line search, command line and Alt menu.
- [Commands and IPC](docs/Commands.md) — every `:` command, its arguments,
  examples, aliases and the Unix socket usage.

# Building

```
nix run .       # build + launch
nix develop     # dev shell (Rust + GPUI toolchain)
```

The core runs a stripped audio-only `mpv` runtime dependency (`mpvAudio` in `flake.nix`) over Unix JSON IPC; no PipeWire, ALSA, or FFmpeg Rust FFI bindings are linked into `musika`.

To benchmark startup, build a release binary and trace the phases:

```
nix develop -c cargo build --release
MUSIKA_STARTUP_TRACE=1 ./target/release/musika
```

`MUSIKA_STARTUP_TRACE=1` prints process-relative marks (`main`, `db.init`,
`app.gpuui`) plus the search-catalog and playback traces to stderr. It costs
nothing when unset.

# Contributions

Pull Requests are welcome if properly tested. I have no issues with AI -- since it would be hypocritical. Besides that, the development is nix-first, as my favorite development tool. Besides that, many core tools, which are present in other players, still missing from here, and bugs that I haven't caught yet. Opening an issue for any problem is also very welcome.

GPL-2.0-or-later.
