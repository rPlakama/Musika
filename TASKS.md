# Musika — Task list (one entry per critic)

Legend: `[ ]` todo · `[x]` done.
Priority: **P0** robustness/correctness · **P1** Nvim fidelity · **P2**
player/UX features · **P3** nice-to-have.
Source sections refer to `CRITIQUE.md`.

## Nvim logic (CRITIQUE §2)

- [ ] **N1 (P1)** Rework "visual mode" into a real pane/window mode (or implement
  `Ctrl-w h/j/k/l`); reserve "visual" for content selection.
- [ ] **N2a (P1)** Motion counts: `3j`, `5l` (= +5 s seek), etc.
- [ ] **N2b (P2)** Operator + motion grammar and text objects (`d`, `y`, `cw`, `i(`).
- [ ] **N3 (P1)** Single `ESC` / `Ctrl-[` returns to normal; keep `ESC ESC` as an alias.
- [ ] **N4 (P1)** Explicit mode state machine; show current mode in the status bar.
- [ ] **N5a (P1)** Command line (`:`) with a few commands (`:q`, `:sync`, `:set`).
- [ ] **N5b (P1)** `/` search-in-list with `n`/`N`.
- [ ] **N6 (P1)** Resolve `Space`: leader (vi convention) vs play/pause.
- [ ] **N7 (P1)** Drop `Ctrl-N`/`Ctrl-P` scroll duplication (keep `j/k`).
- [ ] **N8a (P1)** Implement `gg`/`G` (Top/Bottom) — currently dead bindings.
- [ ] **N8b (P1)** which-key / showcmd for pending sequences.
- [ ] **N8c (P2)** `?` / `:help` cheatsheet overlay.
- [ ] **N9 (P2)** Disambiguate overloaded keys (`Enter`, `q`) and document them.

## Music-player features (CRITIQUE §3)

- [ ] **M1 (P2)** Queue view: reorder, remove, "play next", "add to queue".
- [x] **M2 (P0)** Playback context: any "play" builds a proper queue (album/list),
  not a 1-track queue.
- [ ] **M3 (P2)** Shuffle / repeat (one / all); wire the inert `⤨` button.
- [ ] **M4 (P2)** Volume + mute in the UI (and mpv volume binding).
- [ ] **M5 (P2)** Scrobble playback back to Navidrome (`scrobble`); report play counts.
- [ ] **M6 (P2)** Lyrics, Playlists, Radio, Download caching (currently placeholders).
- [x] **M7 (P0)** MPRIS/D-Bus now-playing: metadata forwarded to mpv (via IPC);
  enable mpv's MPRIS plugin (`mpv-mpris`, set `MUSIKA_MPRIS_SCRIPT` or drop it in
  `~/.config/mpv/scripts`). Media keys then work through mpv/MPRIS.
- [ ] **M8 (P1)** Gapless playback and stream-error retry / auto-advance handling.
- [ ] **M9 (P3)** A-B repeat, playback rate, crossfade, seek-to-percent.

## UIX / interaction (CRITIQUE §4)

- [ ] **U1 (P1)** Discoverability: keyboard hints + which-key/help (overlaps N8b/N8c).
- [ ] **U2 (P1)** Visible keyboard-focus state on individual controls (Back, Sync, segments).
- [x] **U3 (P2)** Convey play/pause with icons, not colour alone.
- [x] **U4 (P2)** Remove the duplicate "Sync library" button (sidebar vs status bar).
- [ ] **U5 (P2)** Re-evaluate the rotated "LIBRARY" title (scanability).
- [ ] **U6 (P2)** Add controls to the fullscreen player (seek/volume/transport).
- [x] **U7 (P1)** Wire the search field to real results (overlaps N5b).
- [ ] **U8 (P0)** Empty/error states for album-with-no-songs, failed sync, failed playback.
- [ ] **U9 (P2)** Retune transition timing (100 ms is abrupt; ~150–200 ms).
- [ ] **U10 (P2)** Guard fractional sizing so labels never get tiny on large displays.
- [ ] **U11 (P2)** Close mouse/keyboard parity gaps.

## Architecture / robustness (CRITIQUE §5)

- [x] **A1 (P0)** Remove `block_on` from the UI thread for lists/covers; use async + model.
- [x] **A2 (P0)** Scope the cover cache to album/artist art (stop per-song covers).
- [ ] **A3 (P1)** Handle cross-DB coherence (search/FTS rebuild repair path).
- [ ] **A4 (P1)** Make cover revalidation cancellable and cheaper.
- [ ] **A5 (P1)** Cancellable sync + HTTP retry/backoff/rate-limit.
- [ ] **A6 (P1)** Harden server assumptions (file-structure vs ID3 ids); handle `getIndexes.child`.
- [x] **A7 (P0)** mpv cleanup on abnormal exit (watchdog) + no orphaned processes.
- [ ] **A8 (P0)** Replace pervasive `eprintln!` with surfaced errors (overlaps U8).
- [x] **A9 (P2)** Avoid leaking the auth token in process argv / mirror file.
- [ ] **A10 (P2)** Fix docs drift (roadmap: "right bar", "Space+F", stale durations).

## Prioritised rollup (CRITIQUE §6)

- **P0**: M2, M7, U8, A1, A2, A7, A8
- **P1**: N1, N2a, N3, N4, N5a, N5b, N6, N7, N8a, N8b, M8, U1, U2, U7, A3, A4, A5, A6
- **P2**: N2b, N8c, N9, M1, M3, M4, M5, M6, U3, U4, U5, U6, U9, U10, U11, A9, A10
- **P3**: M9

## Optimization / code-health (module review)

Dead & duplicated subsystems first (see the systems review).

- [x] **O1 (P0)** Pick one search engine: query `search.db` (FTS5) from the
  palette and delete the `/dev/shm` + `fd` pipeline, **or** drop the FTS5 path.
  Today `search::rebuild` writes a DB nobody reads and `fd` does the real work
  (`search.rs`, `index.rs:139`, `db.rs:35`).
- [ ] **O2 (P0)** Trim `api.rs` to the endpoints actually used (`getIndexes`,
  `getArtist`, `getAlbum`, `getCoverArt`, `ping`); move the rest behind a
  feature flag or delete. ~1,600 unused lines behind `#![allow(dead_code)]`.
- [ ] **O3 (P1)** Merge the duplicate sync pipelines: `qml.rs::sync_library`
  (691) vs `spawn_index_sync` (1044), and the `login` (312) / `startup` (355)
  authenticate-then-sync copies.
- [ ] **O4 (P1)** Fix the ID-namespace assumption in `index.rs`: `getIndexes`
  returns file-structure IDs but they are passed to the ID3 `getArtist`/
  `getAlbum`; also handle `indexes.child` / `getMusicDirectory` (A6).
- [ ] **O5 (P1)** Add HTTP timeouts to the `reqwest` client and retry/backoff
  for index + cover fetches (`http_client.rs:28`) (A5).
- [ ] **O6 (P1)** An unchanged sync still revalidates and `UPDATE`s every cover
  (`index.rs:69`, `covers.rs:183`). Skip cover refresh when up to date; batch
  the `touch()` writes (A4).
- [ ] **O7 (P1)** Cache `SecretKey` in a `OnceLock` (stop re-reading the key
  file per call) and route `crypto::key_path` through `db::config_dir` so
  `XDG_CONFIG_HOME` is honored (`crypto.rs:102`, `crypto.rs:110`).
- [ ] **O8 (P1)** `sync_player` emits ~13 signals and clones 5 strings every
  500 ms tick; emit only on change (`qml.rs:999`).
- [ ] **O9 (P1)** Collapse structural duplication: `fetch_artists`/
  `fetch_albums` (generic helper), `load_albums_json`/`load_artists_json`/
  `song_rows_json`, and the `all_songs`/`song`/`album_songs` projections.
- [ ] **O10 (P1)** Make the player lock poison-safe (no `lock().unwrap()` on
  the reader thread) so a reader panic cannot panic the UI tick (`player.rs:438`).
- [ ] **O11 (P2)** Delete dead code: `covers::cached_path`, the `index.txt`
  mirror (or document it as intentional), `SearchBar.qml` stub, and the dead
  `escape`/`ctrl-[` rows in `keybind.rs` (superseded by the early intercept).
- [ ] **O12 (P2)** Deduplicate constants: volume `70.0` (player + backend),
  `COVER_SIZE`/`FETCH_CONCURRENCY` (covers + index).
- [ ] **O13 (P2)** Stop masking errors as "Wrong username or password"; only
  fall back to that for real auth failures (`http_client.rs:211`).
- [x] **O14 (P2)** Fix the `palette_ready` race: mark ready only after a
  successful rebuild, not before (`qml.rs:868`).
- [x] **O15 (P2)** Address palette corpus churn/collisions: rebuilding the
  whole tmpfs tree and truncating names to 240 bytes can drop songs
  (`search.rs:119`).
- [ ] **O16 (P2)** Harden the mpv supervisor: avoid `sh -c` string building for
  the MPRIS path and avoid spawning `sh` just to `kill` (`player.rs:316`,
  `player.rs:419`).
- [ ] **O17 (P2)** Slim the `Backend` god-object surface and make
  `playback_state` an enum instead of a `String` (`qml.rs:10`, `qml.rs:72`).
- [ ] **O18 (P3)** Remove the two empty no-op user migrations (keep history
  tidy).

### Optimization rollup

- **P0**: O1, O2
- **P1**: O3, O4, O5, O6, O7, O8, O9, O10
- **P2**: O11, O12, O13, O14, O15, O16, O17
- **P3**: O18
