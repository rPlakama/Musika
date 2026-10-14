# Musika — TaskListMajor

Master task list merging `CRITIQUE.md`, `TASKS.md`, `MICRODESIGNS.md`,
`roadmap.md` and `materialyou3idea.md`, re-audited against the current working
tree (commit `434896e`, clean) and organised by delivery bucket.

Legend: `[x]` solved · `[ ]` open. Priorities: **P0** correctness/robustness ·
**P1** vi fidelity · **P2** player/UX features · **P3** nice-to-have.
Each task keeps its area id (`VI`, `Q`, `P`, `LYR`, `S`, `UI`, `THM`, `ARC`,
`CACHE`, `AUTH`, `DOC`), its **Why** and **Details**; solved and partial tasks
add **Now** with the current evidence. Solved entries live in the reference
section at the bottom.

### Buckets

- **Must have** — the app is broken, unsafe or misleading without it: data
  loss, panics, silent failures, wrong library data, stream errors, and
  explicitly prioritised work.
- **Should have** — the product is incomplete without it: vi fidelity, player
  features, search/UX completeness, cache and integration work.
- **Quick wins** — small, bounded, low-risk changes with outsized value
  (polish, cleanup, constants, docs, validation).

### Start here (ranked)

Order = trust/correctness > unblocking > user value > effort.

1. **S-8** — Songs column follows a single album match (top priority:
   album-scoped search reads in track order).
2. **ARC-25** — partial-fetch guard before the destructive purge (data loss).
3. **ARC-5** — errors to toasts instead of `eprintln!`.
4. **VI-11** — `gg`/`G` in every list (high priority: already-bound motions that
   dead-end).
5. **UI-17 + Q-3** — shuffle is the last inert control; wire it + repeat.
6. **UI-22** — empty/error states (P0, half-landed).
7. **VI-2** — mode machine; unblocks VI-3/4/7/12/13/14/15.
8. **S-4 + Q-10** — search scope order/priority (Songs first) and song
   activation opening fullscreen.
9. **LYR-8** — smooth centre-follow + edge fades.
10. **ARC-24 + ARC-33** — `catalog_epoch` + `schema_generation` in every meta.
11. **ARC-29 + ARC-30** — content-addressed covers + hash-keyed colors.
12. **VI-4** — visual selection (rows + count badge).

## Must have

The app is unsafe or misleading without these.

- [x] **VI-11 · `gg`/`G` (Top/Bottom) in every list** — P1 · **high priority** · (N8a, MY3 §4.2)
  - **Why:** `g g` and `G` are already bound (`keybind.rs:122-123` →
    `top`/`bottom`) but no QML handler consumes them, so the motions dead-end
    silently — the most common vi jump is missing while `j/k` alone cannot
    cross a long list.
  - **Details:**
    - consume `top`/`bottom` in every cursor list: `AlbumList`, `SongList`,
      `ArtistList`, `AlbumEntry` songs, `QueueDrawer`, and the three
      `SearchColumn`s (active column only, D-1);
    - semantics: move the cursor to the first/last row and keep it visible
      (`positionViewAtIndex(…, Contain)`); empty list = no-op; already at the
      target = no-op; cursor movement stays `instant` (R5) and does not bloom;
    - `AlbumEntry` special case: `gg` lands on the first song, `G` on the last
      (the toolbar stays a `k`-from-row-0 affordance);
    - pending-`g` feedback stays the keymap timeout until showcmd lands
      (VI-12); a dead-end `g <other>` already resets and retries the token.
  - **Plan:**
    1. per-list `handleAction` cases for `top`/`bottom` (the six surfaces
       above);
    2. one shared two-liner (`currentIndex` = `0` / `count - 1` plus
       `positionViewAtIndex`) instead of six copies;
    3. verification: keyboard pass in each surface, including empty lists
       (Playlists/Radio placeholders, empty queue) and the search overlay's
       active column;
    4. counts (`5G`) and `gg` in visual mode stay with VI-7/VI-4.
  - **Now:** one shared `Theme.cursorEdge` (first/last row plus
    `positionViewAtIndex(…, Contain)`; empty and already-at-the-edge are
    no-ops) replaces the per-list copies; consumed by the shared `MediaList`
    (Albums and Songs now share one list component, wrappers map the payload),
    `ArtistList`, `AlbumEntry` (leaving the toolbar for the songs),
    `QueueDrawer`, and the active `SearchColumn` (`SearchOverlay.moveEdge`);
    the helper is runtime-tested against a real `ListView` (QtQuickTest) and
    the tree is qmllint clean.

- [ ] **Q-5 · Gapless + stream-error retry/auto-advance** — P1 · (M8)
  - **Why:** end-of-track errors currently stop the session silently.

- [ ] **UI-22 · Empty/error states polish** — P0 · (U8)
  - **Why:** album-with-no-songs, failed sync, failed playback need real states.
  - **Now:** error toasts landed; loading screen landed for startup ping.
    Loading skeletons landed as the shared `SkeletonList` (extracted from the
    search columns) and wired into `SearchColumn`, `MediaList` (Albums/Songs/
    Artists via a `loading` flag the wrappers toggle) and the album page's
    songs; the album page also gets a real "No songs in this album" state.
    Open: failed sync state (status/toast wording), failed playback state,
    login Enter-submit + busy fields, playlist/radio placeholders.

- [ ] **ARC-3 · Cancellable sync + HTTP retry/backoff/timeouts** — P1 · (A5, O5)
  - **Why:** a hung request blocks the sync pipeline; `reqwest` has no timeout.

- [ ] **ARC-4 · ID-namespace correctness** — P1 · (A6, O4)
  - **Why:** `getIndexes` returns file-structure IDs passed to ID3 endpoints;
    handle `getMusicDirectory`/`indexes.child`.

- [ ] **ARC-5 · Surface errors instead of `eprintln!`** — P0 · (A8)
  - **Why:** users never see console output.
  - **Now:** toast channel exists for sync/playback; many call sites still log.

- [ ] **ARC-12 · Poison-safe player lock** — P1 · (O10)
  - **Why:** `lock().unwrap()` on the reader thread can panic the UI tick.

- [x] **ARC-25 · Refuse destructive replace/purge on partial fetches** — P1 · (DB analysis)
  - **Why:** a failed artist/album fetch is logged and skipped, then
    `library::replace` wipes the tables and covers/lyrics purges evict the
    missing ids — one HTTP hiccup deletes cached content.
  - **Details:** fail the sync or carry failed ids forward before `replace`;
    audit `getIndexes(ifModifiedSince)` partial-index semantics; run cover/
    lyric purges only when the fetch was complete.
  - **Now:** `fetch_artists`/`fetch_albums` collect every failed id (and task
    panics) and `ensure_complete` bails with "N of M fetches failed … nothing
    was changed" before `replace` or any purge runs; unit test covers it.
    Navidrome audit (server/subsonic/browsing.go): `getIndexes` returns the
    *complete* index when the last scan is newer than `ifModifiedSince`,
    otherwise an empty index — no partial replace hazard on the target server.

- [ ] **ARC-34 · `.unwrap()` audit — 8 production sites; recover or surface, never abort** — P1 · (ARC-12, ARC-5)
  - **Why:** a poisoned player `Mutex` or a failed runtime/DB init panics the
    whole app; production unwraps hide failure context instead of collecting
    it. The 82 test unwraps are fixtures and should stay.
  - **Inventory (90 lines / 94 occurrences, `rg '\\.unwrap\\(\\)' src`):**
    - production, 8: `player.rs:124,133,166,216,303,322` and the reader thread
      `player.rs:465` (all `shared.lock().unwrap()`); `main.rs:37`
      `Runtime::new().unwrap()`; `main.rs:45` is an `expect` on `db::init()`;
    - tests, 82 lines: `colors.rs` 35, `lyrics.rs` 20, `api.rs` 8,
      `http_client.rs` 8, `search.rs` 7, `crypto.rs` 4 (leave as-is, excluded
      from the lint).
  - **Details:**
    - `player.rs`: one `fn shared(&self) -> MutexGuard<Shared>` helper (and a
      local wrapper for the reader's `Arc<Mutex<_>>`) using
      `unwrap_or_else(PoisonError::into_inner)` — `Shared` is a plain snapshot,
      poisoning carries no invariant, so recover instead of aborting; closes
      ARC-12 and guarantees a reader panic can't kill the UI tick.
    - `main.rs`: initialize the runtime and DBs in `main` with `anyhow`
      context and exit(1) after printing the error chain (`main() -> Result`
      or explicit match); remove `Runtime::new().unwrap()` from `runtime()`
      (the `OnceLock` is filled once in main).
    - error collection: background paths use the existing report pattern
      (`SyncReport`/`CoverReport`) or `errorRaised` toasts; no
      `eprintln!`-only paths (ties ARC-5).
    - enforcement: `#![cfg_attr(not(test), deny(clippy::unwrap_used,
      clippy::expect_used))]` in `main.rs` plus a `cargo clippy` gate; a
      justified site needs an inline `#[allow]` with a reason, so new raw
      unwraps are deliberate.
    - policy: tests keep their unwraps; prefer `expect("<what>")` when a
      failure message would save debugging time.

## Should have

The product is incomplete without these.

- [ ] **VI-2 · Global mode state machine** — P1 · (N4, CRITIQUE Ph1.1, MY3 §4.1)
  - **Why:** modes exist only locally in the search overlay; the rest of the app
    has no explicit fallback state, so future features would guess.
  - **Details:** `Mode` enum (Normal/Visual/Command/Search/Insert) in Rust,
    reflected in QML; Esc invariant targets Normal; mode colors per MY3 §3.3.

- [ ] **VI-3 · `Ctrl-W h/j/k/l` pane navigation** — P1 · (N1, CRITIQUE §3.1, MY3 §4.4)
  - **Why:** vi window motions are instant and directional; the current Ctrl-W
    modal lock with Enter-commit is not vi and blocks input.
  - **Details:** directional focus jumps across sidebar/main/player; strip the
    status bar from the ring; keep the region border as the only feedback.
  - **Now:** blur overlay retired (scrim + border, Home.qml); Ctrl-W still
    toggles the region overlay instead of jumping.

- [ ] **VI-4 · `v`/`V` content selection** — P1 · (CRITIQUE Ph1.2/§2.5, N1)
  - **Why:** visual mode must mean "select rows", not "pick a pane"; batch
    queueing/deleting depends on it.
  - **Details:** row selection with `secondaryContainer` + count badge; operators
    (VI-8) act on the selection.

- [ ] **VI-7 · Count register (`3j`, `10l`, `5G`)** — P1 · (N2a, CRITIQUE Ph2.2, MY3 §4.2)
  - **Why:** counts turn motions into a grammar; today digits are dropped.
  - **Details:** numeric prefixes in `keybind.rs`; consume in lists (rows) and in
    the player (seek seconds); count chip appears `micro`, clears instantly.

- [ ] **VI-8 · Operator grammar (`d`, `y`, `p`, `P`, `.`)** — P2 · (N2b, CRITIQUE Ph2)
  - **Why:** queue editing is the musical equivalent of buffer editing.
  - **Details:** `dd` dequeues, `yy` yanks a track/album, `p`/`P` append/play-next,
    `.` repeats; requires the queue view (Q-1).

- [ ] **VI-9 · Space latency / leader decision** — P1 · (N6, CRITIQUE §3.1, MY3 QW9)
  - **Why:** whether Space is play/pause or leader, it must not be ambiguous or
    delayed; the 800 ms deferred resolver is a smell.
  - **Details:** pick one convention (zero-latency play/pause recommended),
    per-binding deferral, drop the global timeout where possible.

- [ ] **VI-12 · which-key / showcmd** — P1 · (N8b, CRITIQUE Ph1.4, MY3 §4.3)
  - **Why:** pending sequences are invisible; undocumented binds require reading
    the source.
  - **Details:** showcmd in the status bar, which-key card above the player bar
    (`micro` in / `instant` out, MY3 §3.1).

- [ ] **VI-13 · `:` command line** — P1 · (N5a, CRITIQUE §2.7, MY3 step 9)
  - **Why:** the power-hatch for commands without UI: `:sync`, `:q`, `:set volume`,
    `:sort`, `:help`.
  - **Details:** bottom prompt, `zCommand` layer, `quick` rise; command registry.

- [ ] **VI-14 · In-list `/` and `?` with `n`/`N`** — P1 · (N5b, CRITIQUE §2.6)
  - **Why:** vi search is an in-buffer motion, not only a global palette.
  - **Details:** `/` filters/highlights the active list; `n`/`N` cycle matches.

- [ ] **VI-15 · `?` / `:help` cheatsheet** — P2 · (N8c, MY3 step 10)
  - **Why:** discoverability of the whole grammar.
  - **Details:** square card in the which-key style, grouped by mode.

- [ ] **Q-3 · Shuffle / repeat** — P2 · (M3)
  - **Why:** the shuffle button exists but is inert; repeat-one/all is expected.
  - **Details:** wire the shuffle icon + repeat mode, show the active state via
    a state layer (stop is already wired; see UI-17).
  - **Now:** the transport uses `random-svgrepo-com.svg` with no `onClicked`;
    `player.rs` has no shuffle/repeat state.

- [ ] **Q-6 · A-B repeat, playback rate, crossfade** — P3 · (M9)

- [x] **Q-8 · Native MPRIS service (`org.mpris.MediaPlayer2.musika`)** — P2 · (roadmap "MPRIS node" design)
  - **Why:** the mpv-mpris script is an extra install; a stock desktop should
    see the node, and media keys/panels/lock screens must work everywhere.
  - **Details:** `mpris.rs` bridge over zbus, no-op without a session bus,
    Metadata/Position/Can* mapping, rate-limited signals, `playerctl` acceptance
    test across GNOME/KDE/sway/XFCE; milestones M0–M5 in the roadmap. Keep the
    mpv script path (Q-7) as the opt-in fallback.
  - **Now:** `src/mpris.rs` owns `org.mpris.MediaPlayer2.musika` (zbus 5,
    tokio runtime; root + player interfaces as separate types). The Qt thread
    publishes a `Snapshot` from `sync_player`/`tick` (volume 0..1, µs
    positions) and drains `mpris_*` commands through the normal dispatcher
    (`Main.qml` maps them globally). Transport Play/Pause/PlayPause/Stop/Next/
    Previous/Seek/SetPosition + Get/Set Volume, `Seeked`, and change-only
    PropertiesChanged rate-limited to <4/s (Position excluded). Metadata:
    `mpris:trackid` `/org/musika/track/<id>` (sanitized; spec NoTrack when
    idle), `mpris:length` µs only when known, `mpris:artUrl`,
    `xesam:title/artist/album`. Without `DBUS_SESSION_BUS_ADDRESS` or with
    `MUSIKA_MPRIS_SCRIPT` set it logs once and no-ops (no double node).
    Verified under a private `dbus-run-session`: name owned, Identity/
    PlaybackStatus/Metadata reads, Set Volume 0.42 round-tripped through QML
    back to the bus, idle metadata = NoTrack. Not runnable in this env:
    `playerctl` and the per-DE panel matrix; roadmap M5 extras (queue
    playlist, OpenUri) deliberately out of scope for now.

- [ ] **P-1 · Scrobble back to Navidrome** — P2 · (M5, CRITIQUE Ph3.2)
  - **Why:** play counts / recently played never update server-side.
  - **Details:** `scrobble` at 50%/4 min and on completion; `api.rs:1318` ready.

- [ ] **P-2 · Starred/favorites view + toggle** — P2 · (CRITIQUE Ph3.2)
  - **Why:** no way to value tracks; `getStarred`/`star` endpoints unused.
  - **Details:** `s`/`*` toggle, favorites filter.

- [ ] **P-8 · ArtistEntry page (Navidrome-style)** — P2 · (ARTISTENTRYSTUDY.md; P-2, CACHE-5)
  - **Why:** artists are a dead end today (activation filters `AlbumList`); the
    media cache also needs a per-artist chooser ("cache the whole artist"), and
    Navidrome's artist entry is the reference: image, favorite, stars,
    biography, actions, download size.
  - **Details:**
    - entry: round avatar, name+server tag, action row, collapsible biography,
      album list reusing the `AlbumList` row; keybar `Enter` album · `Space`
      play all · `s` favorite · `1..5` rate · `c` cache · `r` refresh · `Esc`;
    - favorite → `star`/`unstar` with fetch-on-open + optimistic + revert;
    - stars → `set_rating`, whole row hidden when the server has no artist
      ratings (capability flag);
    - description → `getArtistInfo2`/`getArtistInfo`, cached in library.db
      `artist_info(artist_id, biography, url, fetched_at)`, `r` refreshes;
    - In-Cache → DB4 `media_rule(kind='artist')`; label Off / queued `n of N` /
      `Cached · bytes` (bytes = `SUM(songs.size)`); OFF prunes rule-owned files
      unless pinned;
    - staleness: removed albums/artists and changed fingerprints delete the
      cached blobs (at least the file) + rows after a successful sync; all
      under `~/.config/musika/`;
    - backend: `requestArtist`, `requestArtistInfo`, `starArtist`,
      `rateArtist`, `setArtistCache` + `artist`/`artist_info`/`artist_cache`
      channels; `artist_info` migration.
  - **Milestones (ARTISTENTRYSTUDY.md §6):** A shell/navigation · B favorite +
    stars · C biography · D cache rule + purge · E polish/docs.
  - **Now (partial):** A landed — `ArtistEntry.qml` (only circle = round
    avatar; name + server tag; four-column **cover grid**, titles + year/count
    under the covers, `h/j/k/l` + Enter with a highlighted cell) is pushed from
    `ArtistList` Enter and from artist hits in search (both signals now carry
    artist id + cover, so the header paints instantly); Enter opens
    `AlbumEntry`, `Esc`/back always returns to Main unfiltered (`showSegment(0)`,
    out of the Artists segment or a search result). Backend `requestArtist(id)`
    + `artist` channel returns the artist plus its albums by `albums.artist_id`
    (static SQL; id-based, so same-named artists never mix — unit-tested).
    B favorite/stars, C biography, D cache rule + staleness purge,
    E keybar/mouse polish open. The header's identity strip (biography slot +
    Favorite/Stars/In-cache/Shuffle chips) is carved out as UI-52 with ARC-42
    as its backend; B–D stay behavioural and land on top of both.

- [ ] **P-4 · Playlists (index, view, play, create/edit)** — P2 · (M6, roadmap; `api.rs` already wraps all five endpoints)
  - **Why:** the Playlists sidebar segment is a disabled placeholder, the queue
    cannot be saved, and CACHE-5's playlist rule plus Q-8's optional MPRIS
    playlist interface both wait on an index. The whole Subsonic playlist
    surface is already typed in `api.rs:1211-1277`, so this is wiring + a
    disposable cache, not new protocol.
  - **Research — Subsonic 1.16.1 / OpenSubsonic (Navidrome parity):**
    - `getPlaylists` (1.0.0): playlists the user may play; `username` filter
      (1.8.0) is **not implemented by Navidrome** — never rely on it; fields
      id, name, comment?, owner?, public?, songCount, duration, created/
      changed (ISO-8601), coverArt? (not guaranteed), allowedUser;
    - `getPlaylist(id)` (1.0.0): the playlist + ordered `entry[]` with full
      Child metadata per song; the same song may appear more than once and the
      order is playlist order — store positions, never dedupe by song id;
    - `createPlaylist` (1.2.0; returns the playlist since 1.14.0): without
      `playlistId` it creates (name required); with `playlistId` it updates the
      name and **replaces** the contents with the given `songId` list ("one
      songId parameter for each song in the playlist") — this is the only
      replace op, and the basis for reorder;
    - `updatePlaylist` (1.8.0, owner only): `songIdToAdd` appends (repeatable),
      `songIndexToRemove` removes by position (repeatable, 0-based), optional
      name/comment/public; empty response, no move;
    - `deletePlaylist` (1.2.0): id only;
    - Navidrome: all five implemented (`username` ignored, public/private +
      owner enforced); `download` accepts playlist ids (future CACHE-5 rule).
      OpenSubsonic adds no playlist-specific extension, so keep the core shape.
      Cover art is not guaranteed: build a local mosaic from the first
      entries' `coverArt`, reusing `CoverImage`;
    - edits are owner-only and server-authoritative: optimistic UI, refetch the
      playlist after every mutation, revert + toast on failure.
  - **Details:**
    - new DB8 `playlists.db` like lyrics/colors (`migrations_playlists/`,
      `src/playlists.rs`, `Dbs.playlists`, `MUSIKA_PLAYLISTS_FILE`); purely a
      disposable cache — the server stays the source of truth:
      - `playlist(id PK, name, comment, owner, public, song_count, duration,
        created, changed, cover_art, fetched_at)`;
      - `playlist_song(playlist_id, position, song_id, title, artist, album,
        duration, cover_art, PRIMARY KEY(playlist_id, position))`, FK cascade;
        entries are denormalized so a playlist renders even when a song left
        library.db (row marked unavailable, skipped on play with a summary);
      - `playlists_meta(key, value)` mirrors `library_meta` for the last full
        refresh;
    - refresh policy: one `getPlaylists` call on login, after a successful
      library sync, on `r`/the sync button, and when the segment opens with a
      stale/empty index; `getPlaylist` contents are lazy — fetched when a
      playlist opens stale (`changed` newer than the cached row) and after
      every edit; purge rows for playlists that left the server, never inside
      the library `replace` transaction (ARC-25 rule);
    - backend (`qml.rs`): `requestPlaylists`, `requestPlaylist(id)`,
      `refreshPlaylists`, `createPlaylist(name, songIds)`,
      `renamePlaylist(id, name)`, `deletePlaylist(id)`,
      `addSongToPlaylist(id, songId)`, `removeSongFromPlaylist(id, index)`,
      `playPlaylist(id, start)`; channels `playlists`, `playlist`; edits
      optimistic + revert-on-error, refetch after apply; playback reuses the
      `playerLoad` queue path (`playlist_song` positions → `Track`s, missing
      songs skipped);
    - QML: `PlaylistList.qml` (segment 3; `Enter` open · `Space` play all ·
      `a` create · `r` rename · `d d` delete) and `PlaylistEntry.qml`
      (round? no — square header mosaic + name/owner/public/comment/count/
      duration, ordered song rows in the SongList row style with position as
      the track column; `Enter` plays from that position · `Space` plays all ·
      `d d` removes the cursor entry); a small `Prompt.qml` text overlay for
      create/rename; `PlaylistPicker.qml` (searchable add-to-playlist chooser)
      opened from song/album/queue contexts; `SideBar.qml` enables
      `segments[3]`;
    - keys (`keybind.rs` + dispatch): `a p` add the current song/selection to
      a playlist, `a n` new playlist from the current row (both sequences keep
      the `g g` pending pattern); playlist list keys above; every mutation is
      also an IPC command (`playlists_refresh`, `playlist_open <id>`,
      `playlist_play <id>`, `playlist_add <id> <song-id>`,
      `playlist_remove <id> <index>`, `playlist_create <name>`), so niri/
      Noctalia scripts and the README grammar stay single-sourced.
  - **Modules/modifications:** `api.rs` (endpoints exist — add fixture tests;
    cover-mosaic input), `src/playlists.rs` + `migrations_playlists/` (new),
    `db.rs` (DB8 pool/path; update the module doc pool list), `index.rs` (refresh +
    purge hooks, `SyncReport.playlists`), `qml.rs` (slots/channels/queue
    builder), `PlaylistList/PlaylistEntry/PlaylistPicker/Prompt.qml` (new),
    `SideBar.qml` (enable), `Home.qml` (routing), `keybind.rs`, `ipc.rs`/
    README (grammar).
  - **Incrementals (each shippable):**
    - **I0 read-only segment** — live `getPlaylists` (no DB yet) into
      `PlaylistList`, SideBar enabled; verify the segment lists real names.
    - **I1 DB8 index** — migrations, `playlists.rs` refresh/read helpers,
      sync/login/segment hooks, purge; verify the list survives offline.
    - **I2 view + play** — lazy `getPlaylist` contents into `PlaylistEntry`;
      ordered rows, `Enter` play-from, `Space` play-all; verify duplicate
      songs keep their positions and missing songs are skipped with a toast.
    - **I3 create/rename/delete** — `Prompt.qml` + the three round-trips,
      optimistic + revert; verify from the UI and via IPC.
    - **I4 entry edits** — `PlaylistPicker` (`a p`) appends via `songIdToAdd`,
      `d d` removes positionally via `songIndexToRemove`, refetch after apply;
      verify an external edit appears after `r`.
    - **I5 polish/integration** — cover mosaic, reorder (remove+append or
      `createPlaylist` replace), CACHE-5 playlist rule hook, MPRIS playlist
      interface (Q-8 M5), unit tests (`playlists.rs` SQL + API fixtures).

- [ ] **P-5 · Radio** — P2 · (M6, roadmap)
  - **Why:** sidebar segment is a placeholder; Subsonic radio endpoints exist.

- [x] **P-6 · Resume-on-launch** — P2 · (roadmap DB7 "Session status")
  - **Why:** power users expect the session to survive restarts; today the
    player resets and `Home` always opens on the album list.
  - **Details:** DB7 `statussave.db` (single row: server/username, album_id,
    song_id, queue + queue_index + position, volume, muted, updated_at);
    dirty-flag writes behind a dedicated 500 ms debounce (not the player
    tick) plus a flush on window close; read in `login_succeeded` and applied
    (volume/mute to the player, album page reopened, queue restored paused at
    the saved position -- no autoplay); unknown ids / account mismatch are
    ignored silently. Milestones S0-S3 in the roadmap's "Session status"
    design. DB7 stays separate: credentials is login material only.
  - **Now:** S0–S3 landed — DB7 `statussave.db` (`migrations_status/`,
    `src/status.rs`: single-row upsert + load with tests, `MUSIKA_STATUS_FILE`;
    `Dbs.status`). Writes are account-scoped and coalesced by a generation
    debounce (500 ms task, deliberately not the player tick, so stopped-state
    changes persist); volume/mute restore in `login_succeeded` through
    `applyStatus`, with auto-login now calling `rememberAccount` (it never
    touched the Login fields, so the row could not be matched before).
    Verified live: MPRIS set 0.42 → stored row → restored 0.42 on relaunch.
    S2: `rememberAlbum` on every album-page push (grid/list/search/song-album
    flows) and `rememberSong` on `current_song_id` changes feed the same row;
    restore validates the ids against library.db (`resume_payload`, unknown
    ids dropped like a first launch) and emits `statusRestored` → `Home`
    reopens the album page and loads the queue **paused**
    (`Player::load_paused`, no autoplay/fullscreen; a missing-credentials
    queue degrades to just the album page instead of failing the login).
    S3: the row also carries the full queue snapshot (`queue` JSON ids +
    `queue_index`) and the playhead (`position`); `resume_payload` prefers the
    snapshot (stale ids dropped, index follows the surviving current song;
    pre-S3 rows fall back to the S2 album rebuild), and `playerLoadPaused`
    applies the saved position once. Position rides the tick every 10 s while
    playing; `flushStatus` on window close cancels the pending debounce and
    does one synchronous upsert. Tests: storage round trip + id validation +
    snapshot index (81 pass). E2E restart test still deferred: a live instance
    shares `~/.config/musika` and writes the same row.

- [ ] **LYR-6 · Optional: karaoke v2, lyric FTS, `:lyrics`** — P3 · (roadmap L5)
  - **Why:** word-level timing and lyric search are stretch goals; server
    support for `enhanced` varies.
  - **Details:** `enhanced=true` `cueLine`/`agents`/translations; index lyric
    text in search.db FTS5; `:lyrics sync` backfill per album/artist.

- [ ] **LYR-10 · Word-level karaoke (`cueLine`)** — P3 · (roadmap "Lyrics presentation" P3)
  - **Why:** optional stretch; the OpenSubsonic `enhanced=true` payload offers
    word/syllable timing.
  - **Details:** accent fill/underline per word; only when `kind="main"` and
    every line is timed.


- [ ] **S-6 · Noctalia-style search chrome (scope chips + sizing)** — P2 · (roadmap mixed design)
  - **Why:** identity and counts live in the column headers, and the panel uses
    a raw 70% × 65%; the mixed design gives one chip row and a measured range.
  - **Details:** toolbar + `Songs 12 · Albums 4 · Artists 3` chip row (active
    chip `primaryContainer`, others muted, only one accented — D-1), panel
    `clamp(win*0.70, 560, 1100)` × `clamp(win*0.65, 420, 760)`, context
    keybar; column headers drop the counts.

- [ ] **S-8 · Songs column follows a single album match (track order)** — P1 · **top priority** · (search follow-up)
  - **Why:** when a query resolves to exactly one album, the Songs column still
    ranks hits by FTS score, scattering the album's tracks; it should read (and
    play) in track order so search doubles as album navigation.
  - **Details:** in `search_json`: if `albums.len() == 1`, replace the ranked
    song list with that album's tracks ordered by `(disc_number, track)` from
    `library.db` (no server call); 0 or >1 album matches keep today's
    `search::search` rank. Payload gains `album_context` so the overlay can
    label the scope while activation keeps the existing album-queue behavior;
    cursor, counts and keyboard flow unchanged.
  - **Verify:** "kid a" (one album) → Songs lists that album in track order and
    Enter plays from the selection; "radiohead" (many) → rank order; unit test
    for the ordering/selection helper.


- [ ] **UI-8 · Rows sized as viewport fractions** — P1 · (MD16)
  - **Why:** row height grows with the window; use a fixed unit per view.

- [ ] **UI-9 · Extract a shared `ListRow`** — P2 · (MD17)
  - **Why:** three near-identical delegates drift (zebra/cursor/cover/text).

- [ ] **UI-19 · Type-role sweep + clamps everywhere** — P2 · (U10, MY3 step 7)
  - **Why:** inline `font.pixelSize` fractions drift per view.
  - **Now:** clamps/roles applied in RightBar, NowPlaying, Toast; lists, sidebar
    and album pages still inline.

- [ ] **UI-20 · Focus states on every control** — P1 · (U2)
  - **Why:** keyboard focus must be visible; only IconButton has a focus
    hairline; sync button/segments/rows don't.

- [ ] **UI-21 · Mouse/keyboard parity gaps** — P2 · (U11)
  - **Why:** some affordances are mouse-only and vice versa.

- [ ] **UI-32 · Unlimited horizontal collapse (responsive plan)** — P2 · (window policy)
  - **Why:** `Main.qml:27` clamps the window at `Screen.width * 0.4`; Qt itself
    permits any width (`minimumWidth` can be 0), so the floor is self-imposed.
    Tiling users (niri/Hyprland side slots) should be able to squeeze the
    window and still reach every action, instead of the window refusing to
    shrink.
  - **Plan:**
    1. **R0 window policy:** drop the width clamp (`minimumWidth: 0`, keep
       `minimumHeight` for now); initial size/ratio unchanged. The
       compositor's own floor (decorations) is then the only limit.
    2. **R1 breakpoints:** add `Theme.breakpoints = { mini: 420, narrow: 640,
       compact: 900 }` + `Theme.isNarrow(width)` so every component shares one
       vocabulary (AlbumEntry's ad-hoc `width >= 900` becomes `compact`;
       logical-px clamps are sanctioned here like `fontSize`).
    3. **R2 shell (`Home.qml`):** the 15% sidebar collapses to a segment strip
       below `narrow` (or hides; keys/segments still switch views) and the
       main area takes the rest; focus-border contract unchanged.
    4. **R3 bars:** `RightBar` elides the sync message, hides the mode chip
       and indexed counts below `mini`, keeps state + sync button;
       `InferiorBar` drops the volume slider then the time labels below
       `narrow` (seek stays), tightens transport spacing, and below `mini`
       keeps only prev/play/next.
    5. **R4 content:** `AlbumEntry` cover caps at `min(coverSide, width - 2m)`
       and the header stacks below `compact`; list rows hide the right-hand
       meta (tracks/duration) below `mini` and elide to title+artist;
       `SearchOverlay` switches to a single active column below `narrow`
       (Tab still cycles) instead of squeezing 28/32/40; fullscreen
       `NowPlaying` stacks or shows one pane at a time below `compact`;
       `QueueDrawer`/`Toast` width clamp to `width - 2*margin`.
    6. **R5 text audit:** every collidable label gets `elide:
       Text.ElideRight`; audit `implicitWidth` bindings that could resist the
       collapse.
    7. **R6 verification:** drag to 700/500/320/160 px with shell, album,
       fullscreen, search, queue and toast open; keyboard-only pass at each
       width; no clipped hairlines/overlaps; confirm the Wayland compositor
       clamps only to its own minimum.
  - **Acceptance:** continuous resize down to ~160 px with no layout errors
    or warnings and every primary action still keyboard-reachable; no
    component overflows its pane.
  - **Now (partial):** R0 + R1 + R2 landed. `Main.qml` drops the width clamp
    (`minimumWidth: 0`, `minimumHeight` kept); `Theme.breakpoints = { mini:
    420, narrow: 640, compact: 900 }` + `Theme.isNarrow(width)`; `AlbumEntry`
    uses `compact`. `Home.narrow` drives the shell: `sideW` becomes an ~8%
    strip (min 40 px) and `SideBar.narrow` swaps each segment label for an
    icon (svgrepo set `album-2` / `music-1005` / `artist` /
    `playlist-minimalistic-2` / `radio`, re-baked white like every glyph) and
    hides the stats; cursor/focus contract unchanged. Open: R3 bars, R4
    content, R5 text audit, R6 resize pass.

- [ ] **UI-35 · Queue drawer modal parity: slide + scrim + blur** — P2 · (POLISHAUDIT §1.2, D-2)
  - **Why:** the drawer is modal for keys but has no scrim and the shell blur
    ignores it (`Home.qml:254`), so cursors behind it stay visible — D-2 says
    an open overlay owns the only visible selection.
  - **Details:** translate the panel in/out (`standard`), add the scrim at
    `Theme.scrimOpacity`, include `queueDrawer.opened` in the shell layer
    condition.

- [ ] **UI-36 · Click affordances everywhere: cursor + hover/press/focus** — P1 · (POLISHAUDIT §2, UI-10/20/21)
  - **Why:** `Qt.PointingHandCursor` is set in only two places; most rows,
    segments, buttons and mouse areas never signal clickability, and rows
    have no hover/press state at all.
  - **Details:** apply the documented pattern (`HoverHandler.cursorShape` +
    tap) to AlbumList/SongList/ArtistList/AlbumEntry/QueueDrawer/SearchColumn
    rows, SideBar segments, Toast Retry, Login, and the seek/volume/backdrop
    `MouseArea`s; add hover tint + press layer + focus hairline where marked.
  - **Now (partial):** the cursor pass landed — every row, segment, button
    and mouse area now sets `Qt.PointingHandCursor` via its `HoverHandler`
    (or the `MouseArea`); hover tint/press/focus parity still open.

- [x] **UI-37 · Shared fading `CoverImage`** — P2 · (POLISHAUDIT §3)
  - **Why:** every cover pops in (`visible: status === Ready`) and
    `cache: false` re-decodes on scroll; `status === Error` shows a blank
    colour.
  - **Details:** one `CoverImage.qml` (async, fade `quick` on Ready,
    `sourceSize` per context, error glyph) replacing the seven inline
    `Image`s; revisit the 512 px cache ceiling for hero sizes.
  - **Now:** `CoverImage.qml` (async + `quick` fade on Ready) replaced the
    inline blocks in AlbumList/SongList/ArtistList/SearchColumn/QueueDrawer/
    AlbumEntry/NowPlaying/PlaybackNode; `sourceSize` and the error glyph are
    still open.

- [ ] **UI-38 · Loading/empty states for the three main lists** — P2 · (POLISHAUDIT §5, UI-22)
  - **Why:** Album/Song/Artist lists render nothing until the first payload
    and have no empty state.
  - **Details:** reuse the SearchColumn skeleton pattern and add centred
    empty messages ("Nothing indexed yet — run a sync").
  - **Now (partial):** the empty state landed on Album/Song/Artist lists, and
    the loading skeletons landed with the shared `SkeletonList` (UI-22): the
    wrappers raise `MediaList.loading` while a payload is in flight and the
    skeleton covers the list area below the header.

- [x] **UI-39 · Motion token sweep + reduced motion** — P2 · (POLISHAUDIT §6, UI-19)
  - **Why:** legacy `Theme.duration`/`Theme.easeOut` (100 ms) still drive
    eight files, spin/bloom speeds are inline, and nothing can opt out.
  - **Details:** replace legacy uses with `Theme.motion.*`/`Theme.ease.*`;
    add `Theme.motion.spin`/`spinSlow`; add a `Theme.reduceMotion` gate for
    `FadeText`/bloom.
  - **Now:** `Theme.motion.spin`/`spinSlow` tokens (used by the sync spin);
    `Theme.reduceMotion` gates `FadeText` and the skip bloom; the seven
    legacy `Theme.duration`/`Theme.easeOut` files now use motion tiers.

- [ ] **UI-42 · SuperiorBar: API-backed server + user identity** — P2 · (RightBar audit)
  - **Why:** the top bar (today `RightBar.qml`) only shows a capitalized
    `Backend.serverType`; it never says who is connected. The Subsonic/
    Navidrome API already answers that, so the bar should own the identity.
  - **Details:**
    - rename/extract `RightBar.qml` → `SuperiorBar.qml` with explicit zones:
      brand/mode | spacer | identity | sync cluster;
    - identity from the API: `username` (login/stored) + server type and
      `serverVersion`/`openSubsonic` from the ping envelope, rendered as
      `user@server` (or a `getUser` avatar via `getAvatar_url` once available);
    - plumb the missing fields: `ping()` currently exposes only `type` —
      capture `serverVersion`/`openSubsonic` in the backend and notify, plus
      `Backend.username` display;
    - keep the "Last Sync" text, progress tick and spinning sync button;
      the mode chip stays out of the bar (the search overlay owns
      INSERT/NORMAL); identity is a FadeText with `elide`, no dead space;
    - offline/logged-out states: show "offline"/user only when known.
  - **Verify:** `getUser`/ping fields surfaced on login + reconnect; bar reads
    `user@Navidrome`; no overlap at minimum window width (pairs with UI-43).
  - **Now (partial):** renamed to `SuperiorBar.qml`; identity landed as
    "Welcome, <username>" on the left (body font, welcome scale, body 45%
    elide cap) and "Last Sync" scaled to match; the old NORMAL chip is gone
    and the sync glyph trimmed 10%. Open: `serverVersion`/`openSubsonic` from
    ping, `user@server`/avatar wording, offline state.

- [ ] **UI-43 · Text bounds sweep: a rectangle around every label** — P2 · (QML audit)
  - **Why:** several FadeText/Text items rely on implicit widths in Row/Column
    layouts, so long titles/subtitles can overlap neighbours or clip boxes
    (the SuperiorBar was fixed in `3221461`; the rest of the tree was not).
  - **Details:**
    - audit every `FadeText`/`Text` and give it a bounded slot: a Rectangle/
      Item container with both anchors set, or `Layout.fillWidth` +
      `elide`/`wrapMode`, so geometry never depends on text length;
    - prefer a small reusable `LabelBox` (or `TextSlot`) component with
      `text`/`color`/`elide` props and a sized parent; use it in lists,
      headers, bars, overlays;
    - grep patterns for the sweep: `FadeText` without `width|anchors.right|
      Layout.fillWidth`, `Column {` rows holding two unbounded texts, fixed
      `x:` positioning of labels;
    - keep the elide/wrap rule per context (rows elide, lyrics/notes wrap) and
      don't regress the MY3 type roles.
  - **Verify:** resize a window down to the minimum in every view (albums,
    songs, artists, ArtistEntry, album page, search, queue, fullscreen) with
    absurdly long metadata; no overlaps/clipped labels; `qmllint` clean.

- [ ] **UI-44 · Keybind ↔ insight parity ("if there's a keybind, there's an insight")** — P1 · (U1, VI-12/VI-15, order-cue case)
  - **Why:** a binding nobody can discover is dead weight, and insights must
    not drift from the engine: the order cue (`Shift-H`/`Shift-L`) was deleted
    with its hint strip in `3385091` and had to be restored in the AlbumList
    header, proving the rule has to be explicit and enforced.
  - **Details:**
    - every binding has at least one visible insight: a cue at its point of
      use (e.g. the order cue), the showcmd/which-key card while a sequence is
      pending (VI-12), or a `?`/`:help` row (VI-15);
    - one source of truth: `keybind.rs` gains a short description per binding
      (`("shift-h", OrderPrev, "previous album order")`), consumed by the
      cheatsheet and the README keybind table so hints cannot drift;
    - audit gate: a test/script diffs the binding table against the insight
      registry and fails on a binding without an insight (and vice versa);
    - keys with a local surface document themselves where they act (search
      footer, fullscreen keybar), not in the global registry.
  - **Now:** principle stated; the order cue is the current example; no
    registry or audit exists yet.

- [x] **UI-45 · Selection stage: the cursor row expands like the lyrics stage** — P2 · (design task; LYR-7 reference)
  - **Why:** list rows are uniform and the cursor is only a colour fill (the
    `MediaList` delegate tints `ListView.isCurrentItem`/hover), so a list reads
    flat. The lyrics stage already proved the small/BIG/small rhythm — the
    current value should visually lead its neighbours the same way, making the
    selection readable at a glance.
  - **Details:**
    - current selection only: the cursor row's primary text grows (scale
      ~1.08-1.12, or `title`/`headline` role) and takes full contrast; every
      other row keeps the normal `body` size — no per-row size ramps;
    - hover behaves like the cursor: `rowHover.hovered` gets the same expanded
      state (mouse/keyboard parity), so browse-by-mouse feels identical;
    - **increase the lateral width margin** of the list box (both sides, e.g.
      `viewWidth * 0.04` vs today's 0.02) and keep titles elided inside that
      gutter, so the expanded row never touches the pane walls or clips;
    - transform-based (`scale`, transform origin left/centre) so neighbours
      never reflow or jump; `quick`/`micro` with `Theme.ease` curves, flat
      under `reduceMotion`; consider a shared `Theme.selectionEmphasis` beside
      `lyricsEmphasis`;
    - scope: `MediaList` rows (Albums/Songs/Artists), `SearchColumn` rows and
      the queue rows (drawer + fullscreen pane); the lyrics stage stays the
      reference implementation.
  - **Verify:** minimum window width with long titles: no clipping/overlap,
    elide still works, adjacent rows don't move, mouse hover and keyboard
    cursor expand identically, `reduceMotion` renders the plain list.
  - **Now:** `Theme.selectionEmphasis` (1.10, flat under `reduceMotion`) plus
    the shared `Theme.fitScale` cap; the primary text of the cursor/hover row
    scales about its left edge (transform only, no reflow) in `MediaList`,
    `SearchColumn`, `QueueDrawer` and the fullscreen queue (hover added to the
    fullscreen rows), and the cover/thumbnail grows with the same
    `quick`/Bezier animation wherever the row has art; `MediaList` gutters
    widened to `viewWidth * 0.04` with bounded, elided titles so the grown row
    never reaches the walls. Left for a desktop session: the neighbour
    colour/contrast polish and a real-pointer hover parity pass.


- [ ] **UI-46 · Smooth scroll engine (distance-aware cursor follow)** — P2 · (SCROLLSTUDY.md)
  - **Why:** every list follows the cursor with an instant
    `positionViewAtIndex(..., Contain)` (MediaList:71, AlbumEntry:124,
    ArtistList:72, SearchColumn:105, QueueDrawer:69/96, NowPlaying:99/120/148),
    so moves teleport; fixed durations are distance-blind and the lyrics
    follow fights manual scrolling. Reads as blocky.
  - **Details:**
    - shared `scrollIndex(list, index, mode)` helper (`Theme.js`/`ScrollAnim`):
      target from `itemAtIndex` (minimal move for Contain, centring for
      Center), clamped to `[0, contentHeight - height]`;
    - distance-aware duration — `clamp(px / ~2.5, 80, 240)` ms with
      `ease.decelerate`, retarget a running animation instead of restarting;
    - follow guard: pause while `dragging`/`movementState` is non-stop, resume
      on selection/track change or after idle (lyrics ~4 s);
    - instant `positionViewAtIndex` only for far jumps (`gg`/`G`);
    - `boundsBehavior: DragOverBounds` + `reuseItems`/`cacheBuffer: height` so
      delegates don't pop in blank; no effects during movement;
    - consider the ListView highlight machinery
      (`highlightMoveDuration`/`highlightRangeMode`) once UI-45 gives the
      cursor a visual highlight;
    - `Theme.reduceMotion` → instant.
  - **Consumers:** UI-41 (list easing), LYR-8 (lyrics follow), UI-45
    (selection stage), queue/search lists.
  - **Now (partial):** `ListScroll.qml` eases small `contentY` corrections
    (quick/Bezier, half-row context; instant fallback for unrealised rows and
    `reduceMotion`) and is wired into the search columns and both queues;
    payload swaps crossfade, reorders stay live. Open: the distance-aware
    `clamp(px / 2.5, 80, 240)` helper for MediaList/AlbumEntry/ArtistList,
    `DragOverBounds`, `reuseItems`/`cacheBuffer`, and the lyrics
    `SmoothedAnimation` follower.
  - **Verify:** frame graph (`QSG_RENDER_TIMING=1`) smooth for 1-row, 5-row
    and 20-row moves; wheel mid-follow doesn't stutter; long titles don't
    clip while expanding; `reduceMotion` flat.

- [ ] **THM-6 · Rewrite the color engine on DMS's pipeline** — P2 · (MATUGENSTUDY.md; roadmap T5+)
  - **Why:** DMS cleanly separates seed extraction, palette specs, caching,
    change detection and coalescing; Musika's single-shot engine re-quantizes
    for every change, cannot reach the 2025/DMS specs, re-applies identical
    palettes, and keeps its knobs in env vars.
  - **Details:**
    - split `colors.rs` into seed/scheme/engine (R0);
    - source modes: default `Score` + `colorful` (caelestia-style scoring port)
      + user seed override; seed cache in DB6 keyed `cover_hash|mode|engine`
      with a bounded LRU (R1);
    - spec versions 2021/2025/DMS in the generator tag; verify
      `material-colors 0.5` coverage or port the palette tables (R2);
    - role-map hash / applied key so identical palettes are a no-op (R3);
    - single coalescing worker with cancel-last on a low-priority task (R3);
    - options persist as settings (scheme/source/contrast/spec/seed) instead of
      `MUSIKA_COLOR_*` env vars; `requestColors` reads them (R4);
    - `colors_preview(seed)` API for a future theme tab (R4).
  - **Now (partial):** R0–R2 landed — `src/colors/{seed,scheme,engine}.rs`, with
    `colors.rs` as facade; DB6 `color_seed` LRU (32) + `cover_color.role_hash`/
    `colorfulness`; source modes `dominant|colorful|#rrggbb`
    (`MUSIKA_COLOR_SOURCE`, colorful = caelestia scoring port); specs
    `2021|2025|dms` (`MUSIKA_COLOR_SPEC`, default `dms`; DMS tables ported from
    `spec.go`); `role_hash` in the `colors` payload. Open: R3 coalescing worker
    + no-op apply in `Palette`, R4 persisted settings + `colors_preview`,
    light/smart mode.
  - **Study:** `MATUGENSTUDY.md` (DMS pipeline, comparison table, take/leave,
    milestones R0–R4, risks).


- [ ] **ARC-1 · Cross-DB coherence repair path** — P1 · (A3)
  - **Why:** library/search/covers writes can diverge on interruption.

- [ ] **ARC-2 · Cover revalidation cheaper/cancellable** — P1 · (A4, O6)
  - **Why:** an unchanged sync still revalidates and `UPDATE`s every cover.

- [ ] **ARC-7 · Trim `api.rs` to used endpoints** — P0 · (O2)
  - **Why:** ~1,600 lines behind `#![allow(dead_code)]`.

- [ ] **ARC-8 · Merge duplicate sync/login pipelines** — P1 · (O3)
  - **Why:** `qml.rs::sync_library` vs `spawn_index_sync`; login/startup copies.

- [ ] **ARC-11 · Collapse structural duplication** — P1 · (O9)
  - **Why:** fetch helpers, JSON loaders and song projections repeat.

- [ ] **ARC-36 · Module duplication sweep (optimization & maintenance)** — P2 · (module audit)
  - **Why:** the same scaffolds are copied instead of shared, so a fix in one
    module silently misses the others and every copy is a place for the next
    bug and for docs drift. Coordinates the duplication already tracked in
    ARC-5/7/8/10/11/14 and UI-26, and adds the cross-module items they miss.
  - **Inventory (`rg -c` at audit time):**
    - `fn now()` defined 4×: `covers.rs:298` (String), `lyrics.rs:723` and
      `colors/engine.rs:381` / `colors/seed.rs:370` (i64);
    - cache lifecycle copied 3×: covers.rs (`refresh`/`upsert`/purge),
      lyrics.rs (`fetch`/`load_cached`/`store`/`purge_with`),
      colors/engine.rs (`fetch`/`load`/`store`/`purge`/`purge_with`) — same
      validate -> generate -> upsert -> purge-missing shape with three
      hand-written `purge_with` seams;
    - qml.rs async scaffold: 22 `get_qml_method_invoker`, 17
      `runtime().spawn`, 15 `let Some(dbs) = crate::dbs().cloned() else`
      copies, all ending in one `emitData` call;
    - client construction copied 5× in qml.rs (`load_credentials` +
      `SubsonicClient::new`) across tracks / album-notes / lyrics / colors;
    - `eprintln!` 44× overall (qml 22, index 10, lyrics 5, rest 7) — a single
      log route instead of 44 call sites (ARC-5);
    - palette.rs MY3 defaults duplicate the Theme.js role table, held together
      by a "keep in sync" comment (`palette.rs:67`) — two sources for the same
      32 roles;
    - library.rs repeats full SELECT projections per order/filter arm
      (`albums()` match; `all_songs`/`song`/`album_songs`/`artists`);
    - QML: `formatTime` 16 matches across 6 files (UI-26);
      `Connections { target: Backend }` 14× is inherent and stays.
  - **Details:**
    - `consts.rs`: add `now_secs()`; the four local helpers disappear (covers
      converts to String at its single write site);
    - new `cache.rs`: `purge_missing(derived, table, key_col, source,
      source_table)` plus a small upsert/TTL helper where SQL is truly
      identical — per-domain SQL stays, the scaffold goes;
    - qml.rs: one `emit_async!(self, channel, future)` macro (clones the
      invoker + `dbs` once) and `async fn client(dbs) -> Result<SubsonicClient>`;
      loaders collapse to a line each (overlaps ARC-8/ARC-11);
    - errors: single `log_error!` that routes to `errorRaised` when a UI is
      up and to stderr otherwise (ARC-5);
    - palette: make palette.rs `DEFAULT_*` the only role table (delete or
      generate the Theme.js colour copies; shape/motion/type stay there);
    - library projections: shared `const` column fragments or one builder per
      entity (ARC-11).
  - **Now (partial):** `consts::now_secs()` replaces the four local `now()`
    helpers (covers binds `.to_string()` at its two write sites); new
    `cache::purge_missing` collapses the lyric and color purge seams (covers
    keeps its own loop because it also removes files), with a unit test.
    Open: the qml.rs `emit_async!` scaffold + shared client constructor,
    `log_error!` routing (ARC-5), palette/Theme role single source, and the
    library projection fragments.
  - **Verify:** audit counts drop (`now` 4→1, `spawn` 17→~4, `eprintln` 44→~5,
    `SubsonicClient::new` 7→2), `cargo test` stays green, no visual diff.

- [ ] **ARC-17 · Slim the `Backend` god-object** — P2 · (O17)
  - **Why:** huge slot surface; `playback_state` is a `String`, should be enum.
  - **Now (partial):** the Qt bridge's data layer left `qml.rs` — `browse.rs`
    (list/search JSON loaders + cover-URL resolution) and `queue.rs` (Track
    queue builders); `qml.rs` 1957 → 1793 lines, keeping the QObject,
    tick/sync glue and invoker-driven orchestration. Open: `playback_state`
    String → enum, slot-surface reduction. Next module candidate: `api.rs`
    (1894 lines: protocol types + client methods).

- [ ] **ARC-24 · Shared sync epoch across the five DBs** — P1 · (DB analysis)
  - **Why:** only library.db knows when it last synced; search/covers/lyrics
    can silently lag after a failed step and nothing can detect it.
  - **Details:** `meta(key, value)` in every DB (`search_meta`, `covers_meta`
    to match library/lyrics); publish `catalog_epoch` last in the sync
    pipeline; on a failed step publish no epoch and skip destructive purges;
    the UI can flag stale derived data.

- [ ] **ARC-26 · ATTACH-based purges/joins** — P2 · (DB analysis)
  - **Why:** `lyrics::purge_with` and `covers::refresh` materialize every id
    in Rust and cross-DB lookups are duplicated (`song_cover_ids`,
    `COALESCE(al.cover_art, s.cover_art)`).
  - **Details:** dedicated maintenance connection that ATTACHes the DBs;
    anti-join purge (`NOT IN (SELECT id FROM lib.songs)`) and cross-DB
    resolution in SQL; keep the per-DB pools for normal I/O.

- [ ] **ARC-27 · Snapshot credentials + cover URL map** — P2 · (DB analysis)
  - **Why:** `load_credentials` decrypts on every call and `covers::url_map`
    scans covers.db on seven request paths (list load, album open, play,
    search, queue).
  - **Details:** rebuild an in-memory `{credentials, cover_urls}` snapshot
    when the catalog epoch changes; check `is_file()` before publishing file
    URLs; share the stream client instead of re-decrypting.

- [ ] **ARC-28 · Disposable-DB repair policy** — P2 · (DB analysis)
  - **Why:** `db::init` failure panics even though four of five DBs are
    re-fetchable; `index.txt` is written non-atomically; `syncLibrary` has no
    backend re-entrancy guard.
  - **Details:** recreate derived DBs on corruption, mark them dirty for
    resync, keep boot alive when only derived stores fail; tmp+rename the
    mirror; reject a second concurrent sync.

- [ ] **ARC-29 · Content-addressed cover files** — P2 · (DBWIRING J1)
  - **Why:** blobs are named `md5(cover id)`, so byte-identical art under
    several ids is copied once per id even though the content hash is known.
  - **Details:** name files `hash.ext` (keep `hash_algo`), let several
    `covers` rows point at one file, keep purge = unreferenced files only;
    migrate existing files by relinking rows to their stored hash.

- [ ] **ARC-30 · Hash-keyed color schemes** — P2 · (DBWIRING J1, THM-2)
  - **Why:** `cover_color` keys on `cover_art`, so two ids sharing one image
    store the same scheme twice; the scheme is a pure function of (bytes,
    generator).
  - **Details:** `PRIMARY KEY(cover_hash, generator)`, dedupe/backfill the
    migration, purge via `cover_hash NOT IN (SELECT hash FROM covers)`,
    keep `cover_art` only as a pruning hint.

- [ ] **ARC-31 · Projection-trimmed search** — P2 · (DBWIRING J2/J3)
  - **Why:** `search.db` copies title/artist/album/duration and every hit is
    hydrated field-by-field; the library already owns those columns.
  - **Details:** index-only FTS5 (`content=''`) returning `song_id` + `rank`;
    hydrate hits from the epoch snapshot; rebuild handles the migration.

- [ ] **ARC-32 · Cross-DB verify/dedupe job** — P2 · (DBWIRING J3/J4)
  - **Why:** dangling ids (lyrics/colors/media for songs or covers that left)
    are only removed by the next purge, and duplicates accumulate silently.
  - **Details:** ATTACH/batched checks for search ids missing from library,
    color hashes missing from covers, cover rows with missing files,
    unreferenced files, duplicate colors; report counts at sync end, full
    repair on demand (`:db verify`).

- [ ] **ARC-33 · Provenance + `schema_generation` in every meta** — P2 · (DBWIRING J0, ARC-24)
  - **Why:** derived DBs do not record what they were built from (only colors
    store a hash), so staleness and layout drift are undetectable.
  - **Details:** `schema_generation` + `catalog_epoch` (+ `generator` where
    relevant) in library/search/covers/lyrics/colors meta; migrations seed
    them; verify/startup repair compare both values.

- [ ] **ARC-37 · Complete the IPC command surface (scriptable parity)** — P2 · (IPC audit)
  - **Why:** the socket only reaches commands that happen to flow through
    `keyHandler.dispatch`, so transport/queue/library slots the UI can call have
    no command spelling; scripts cannot do everything a keyboard can, unknown
    commands are dropped silently, and nothing ever replies.
  - **Inventory (Backend slots vs command spellings):**
    - reachable today: `play_pause`, `next_track`/`prev_track`, `mute`,
      `volume_up`/`volume_down`, `order_next`/`order_prev`, `yank`/`paste`,
      `toggle_fullscreen`, `search`, `visual_mode`, `queue_toggle`,
      `play_song_in_album <id>`, `lyrics_refresh`, `colors_refresh`, and the
      movement verbs (`move_*`, `enter`, `back`, `queue_up/down/remove`);
    - missing: `playerStop`, `playerSeek`, absolute `playerSetVolume`,
      `queueJump`/`queueMove` with arguments, `playAlbum <id> [start]`,
      `playSong <id>`, `syncLibrary`, `requestSearch <query>` (search opens
      blank), and a `status` query for the read-only properties.
  - **Plan:**
    1. Rust command registry (`commands.rs`): `name [args...]` -> handler on
       `Backend`; strict whitespace parsing, numeric validation (seek >= 0 and
       clamped to duration, volume 0-100, queue indices bounds-checked),
       aliases (`next`/`next_track`, `pause`, `toggle`).
    2. `handleCommand` parses in Rust first: a registry hit calls the slot
       directly (no QML round-trip); unknown strings fall through to the
       existing `command_issued` pump, so UI action strings and
       `play_song_in_album <id>` keep working (move that parse into Rust as an
       alias too).
    3. Replies: `ipc::serve` handlers return `Option<String>`; the socket
       writes `ok [detail]` / `err <message>` per line (fire-and-forget for UI
       action strings). `status` replies with playback state / title / artist /
       position / duration / queue index+len / volume / muted / syncing.
    4. `help` lists the registry (name, args, description); reuse for VI-15's
       `:help`.
    5. Safety: no shell, no interpolation into commands; unknown names get an
       error instead of silence.
  - **Draft surface:** `play`, `pause`, `play_pause`, `stop`, `next`,
    `previous`, `seek <sec|+N|-N>`, `volume <0-100>`, `volume_up/down [step]`,
    `mute [on|off|toggle]`, `queue_toggle`, `queue_jump <i>`,
    `queue_move <from> <to>`, `queue_remove <i>`, `play_song <id>`,
    `play_album <id> [start]`, `play_song_in_album <id>`, `search [query]`,
    `sync`, `lyrics_refresh`, `colors_refresh`, `order_next`/`order_prev`,
    `fullscreen`, `pane_cycle`, `visual_mode`, `status`, `help`.
  - **Milestones:** P0 registry + transport verbs with replies + parser tests;
    P1 queue/library commands; P2 `status`/`help` + README matrix; P3
    `search <query>` and `:command` reuse (VI-13).
  - **Verify:** socket integration test (temp `MUSIKA_IPC_SOCKET`) drives
    seek/volume/queue_move and reads replies; README IPC examples updated.

- [x] **ARC-38 · `Icon.qml` facade: source + role + badge + fallback** — P2 · (SVGSTUDY.md; CACHE-7)
  - **Why:** icon URLs and palette roles repeat at every call site (`SideBar`,
    `InferiorBar`, `NowPlaying`, `ExitChip`, `SearchOverlay`, `SuperiorBar`,
    `Theme.volumeIcon`), and there is no way to tag status on an icon
    (cache/offline) or recover from a vector parse error.
  - **Details:** one `Item` wrapping `SymbolIcon` plus a corner badge layer
    (`glyph`, colors, `Loader`-gated so it costs nothing when absent) and a
    text fallback when the vector status is Error; properties `name`/`source`
    and `role`/`symbolColor` with a `Palette` default; fixed frame and
    `iconRatio` semantics unchanged. Badges stay QML-side, never encoded in
    asset or desktop-entry names (DMS's `svg+corner:` pitfall). First consumer:
    cache/offline badge for CACHE-7.
  - **Verify:** qmllint clean; no pixel diff on existing surfaces; badge absent
    adds no items; a broken source paints the fallback.
  - **Now:** `Icon.qml` landed — `name` (short qrc name or URL) /
    `source`, `symbolColor` (palette role, `Palette.onSurface` default),
    `iconRatio` passthrough, `Loader`-gated corner badge (`badge`,
    `badgeGlyphColor`, `badgeBackground`) and a `fallbackText` shown when the
    vector cannot be parsed. `SymbolIcon` gained `ready`
    (`implicitWidth > 0`; a broken vector reports 0). Call sites move over as
    they are touched; the first planned consumer is CACHE-7's cache/offline
    badge.

- [x] **ARC-39 · Remote SVG artwork ingest policy** — P2 · (SVGSTUDY.md; DMS `svgsource.go`, ARC-29)
  - **Why:** `covers.rs::extension` maps jpeg/png/webp only (fallback `.img`),
    so `image/svg+xml` art is stored undecodable; `colors/seed.rs::sample`
    then fails palette extraction and `CoverImage` (no QtSvg) stays blank.
    DMS's lesson applies: rasterize before every non-SVG consumer.
  - **Details:** detect by content type and/or `<svg` sniff; rasterize at
    ingest with `resvg` to a bounded PNG (max edge, e.g. 1024, plus byte cap);
    store as a normal hash-addressed cover (ARC-29); reject malformed/oversize
    input to the placeholder path and never store an undecodable blob; palette
    decodes the stored PNG only; no shelling out (ARC-16), no QtSvg runtime
    dependency.
  - **Verify:** unit tests (small SVG blob → dimensions + sampled pixel;
    oversize/malformed rejected); cover fetch + seed extraction end to end; no
    new processes spawned.
  - **Now:** `resvg` dependency added; `covers.rs` detects SVG by
    content type or `<svg`/XML sniff, rasterizes at ingest to a bounded PNG
    (`rasterize_svg`: ≤1024 px longest edge, input/output byte caps) and
    either stores it as a normal cover (`store_art`, content type
    `image/png`, hash of the stored bytes) or drops it with a log line so the
    placeholder path owns it; the palette sampler only ever sees raster bytes.
    Unit tests cover sniffing, bounded raster with a sampled pixel, small-SVG
    passthrough size, malformed/oversize rejection and raster passthrough; the
    live fetch rides the existing cover sync path.

- [x] **ARC-40 · Bound cover/art decode size** — P2 · (SVGSTUDY.md; DMS `backer.sourceSize`, D-5)
  - **Why:** `CoverImage.qml` loads full server-resolution art (`cache: false`)
    in every delegate; a large library decodes far more pixels than any
    surface paints, spiking memory and scroll cost.
  - **Details:** DPR-aware `sourceSize` per surface (grid cell ×2, entry cover
    ×1, D-5 ambient backdrop = blur texture size); keep `cache: false` (disk
    blobs); no visible softening at the painted size.
  - **Verify:** memory profile with a 4000² cover before/after; screenshot diff
    per surface plus a ListScrollStudy scroll pass.
  - **Now:** `CoverImage` decodes through `sourceSize`: `sizeFactor` (list/grid
    thumbs ×2 for the hover scale, hero covers ×1), ambient mode decodes for
    the blur texture (×0.25), all in device pixels (×`Screen.devicePixelRatio`)
    rounded up to a 32 px grid so resizes do not reload per pixel; aspect ratio
    is preserved by `sourceSize`. The memory profile + per-surface screenshot
    pass is deferred to a desktop session.

- [x] **ARC-41 · Icon pipeline verification (bake + references + fallback)** — P3 · (SVGSTUDY.md; D-6, UI-31)
  - **Why:** `bake-icons-white.sh` is unchecked `sed` (misses `<style>` rules,
    `#fff`, `currentColor`, gradient stops), dangling `qrc:/app/icons/*.svg`
    references ship silently, and a vector parse error paints a blank frame.
  - **Details:** extend the bake to the missed forms; add
    `scripts/check-icons.sh` (every svgrepo asset contains a white fill/stroke,
    every referenced qrc icon exists, every asset is referenced or
    brand-listed) wired into the Nix build beside `qmllint`; the runtime
    fallback comes from ARC-38.
  - **Verify:** the check fails on a deliberately unbaked asset and on a
    dangling reference; `nix build` stays green; manual blank-icon recovery.
  - **Now:** the bake covers attribute and CSS fills/strokes, `stop-color`
    gradients and `currentColor`, and leaks the root `fill="none"` case into
    white. `scripts/check-icons.sh` enforces the three rules (white ink,
    resolving `qrc:/app/icons/*` references — QML and JS — and
    referenced-or-brand) with brand-listed multicolor assets; unreferenced
    spares warn by default and fail under `--strict`; wired into `flake.nix`
    `preBuild`. Both failure modes probed (unbaked asset, dangling reference).
    Untracked spare assets are outside the git source, so the package build
    sees only referenced/brand assets; prune or brand-list them if committed.
    The runtime fallback is ARC-38's `Icon.fallbackText`.

- [ ] **ARC-42 · Artist curation + biography + cache-state backend** — P2 · (ARTISTENTRYSTUDY.md §3/§5; UI-52, P-8 B–D, P-2, Q-3, CACHE-1/5)
  - **Why:** UI-52's header strip (biography slot + Favorite/Stars/In-cache/
    Shuffle) ships as placeholders because no channel provides artist star
    state, rating capability, biography, cache-rule status or an
    artist-scoped shuffle. P-8's milestones B–D would otherwise grow ad-hoc
    calls; this task is the single backend surface they (and UI-52) consume.
  - **Backend gaps to close (ARTISTENTRYSTUDY.md §5):**
    - `qml.rs` slots: `requestArtistInfo(id)` (`getArtistInfo2` →
      `getArtistInfo` fallback, `api.rs`), `starArtist(id, on)`
      (`star`/`unstar` with artist ids), `rateArtist(id, stars)`
      (`set_rating`, capability-guarded), `setArtistCache(id, on)` +
      `requestArtistCache(id)`, and `shuffleArtist(id)` (Q-3's engine; until
      then a shuffled artist queue built backend-side);
    - channels: `artist_info` (biography + external url + `fetched_at`),
      `artist_curation` (starred, rating, `canRate`), `artist_cache`
      (Off / queued `n of N` / `Cached · bytes`);
    - `library.db`: `artist_info(artist_id PK, biography, url, fetched_at)`
      migration + upsert/read/purge (disposable; purge when the artist left);
      `artists.starred` update on toggle;
    - cache state aggregates DB4 `media_rule(kind='artist', ref_id=<id>)` plus
      `SUM(songs.size)` from `library.db` (CACHE-1/CACHE-5);
    - optimistic toggle + revert + toast on failure; state fetch on open and
      refresh (`r`); offline serves the cached biography with its `fetched_at`.
  - **Capability rule:** a server without artist ratings (`ping` extensions)
    reports `canRate: false`; the Stars chip stays hidden — never dead stars.
  - **Consumers:** UI-52 (the strip), then P-8 B–D lay the flows on top
    (keybar `s`/`1..5`/`c`/`r`, optimistic UI, staleness purge on sync).
  - **Verify:** unit tests for `artist_info` upsert/purge and the cache
    aggregation; star toggle persists across restart; rating hidden on a
    capability-less server; shuffle builds an artist-only queue; offline
    biography + `fetched_at`; no round-trips while the entry is closed.
  - **Now (writing paths):** DB8 `artist_user.db` landed (`artist_user.rs`:
    `starred`/`rating` with CHECK 0..5, index on `starred`, cross-field-
    preserving upserts, `starred_ids`, QML payload; wired into `db.rs` as the
    eighth pool, `MUSIKA_ARTIST_USER_FILE` override). `library.db` gained
    `artist_info` (migration + `upsert_artist_info`/`artist_info`/
    `purge_stale_artist_info`, the last hooked into the sync's purge phase).
    `qml.rs` slots: `requestArtistUser`, `starArtist` (star/unstar → persist →
    `artist_user` emit; error toast), `rateArtist` (0..=5 validated →
    `setRating` → persist → emit), `requestArtistInfo`
    (`getArtistInfo2`→`getArtistInfo` → cache in library.db → `artist_info`
    emit; offline serves the cached row). Tests: 4 in `artist_user` (round
    trip/cross-field/index/bounds/payload) + 1 library round-trip/purge.
    Open: capability gate for ratings (needs UI-42's ping fields), QML wiring
    (UI-52), artist-scoped shuffle (Q-3), In-cache aggregation (CACHE-1/5).

- [ ] **ARC-43 · Alt-hold quick actions — radial quadrant overlay (experimental)** — P3 · (roadmap Alt-hold quick actions; UI-52, ARC-42)
  - **Why:** per-item actions are keyboard-only today (queue moves,
    activation, navigation) and there is no context menu; a mouse user hovers
    a row and has to leave it to act. The overlay is mouse parity for the same
    verbs.
  - **Details:** hold Alt over a track/album → four equal boxes N/S/E/W around
    a center anchor: N Add to Queue, S Favorite, one horizontal slot
    contextual Go to Album (track) / Go to Artist (album), the opposite slot
    open in v1 ("—"); release over a box fires, center dead zone / leaving the
    item / Esc cancels; TUI styling (monospace, box-drawing borders, single
    accent, `Palette` tokens, center crosshair); one component taking item +
    `{quadrant -> action, label}` config; flag off by default; suppress while
    the keymap has a pending token so `alt-j`/`alt-k` never arm it.
  - **Verify:** track row + album card repro; context swap correct in both
    directions; center dead zone never fires; overlay absent with Alt up and
    never steals hover/click; flag off = zero items/animations; the acceptance
    list in the roadmap design section passes.

- [ ] **CACHE-1 · DB4 `media.db` + `media/` blobs** — P2 · (roadmap Media cache; MEDIACACHESTUDY.md §3/§5)
  - **Why:** audio is never stored; playback dies without network. Research
    picked an independent reqwest fetch (`download` original / `stream` with
    transcode params) over mpv `--stream-record` (seek/switch breaks the file).
  - **Details:** schema `media(song_id PK, kind, file, size, quality,
    fingerprint, rule_id, last_used, created_at)` + `media_rule` + `media_meta`;
    blobs split permanent/rule (`~/.config/musika/media/`) vs playback/rolling
    (`~/.cache/musika/media/`); `.part` + atomic rename; `media::cached_path`
    and a batch resolver for `build_track`; purge only after a successful sync
    (covers.rs pattern).

- [ ] **CACHE-2 · Playback cache + prefetch next N** — P2 · (MEDIACACHESTUDY.md §4)
  - **Why:** stutter-free playback; Symfonium-style rolling buffer.
  - **Details:** reuse `Player::upcoming(n)` (lyrics already prefetches 3);
    trigger on load/next/prev/queueJump/track-change tick; bounded concurrency
    2-4 with an in-flight set; current uncached track fetched in parallel while
    it plays; prefetch never swaps the playing file, only future `loadfile`s.

- [ ] **CACHE-3 · Rolling cache (LRU, max size)** — P2
  - **Details:** cap configurable (default ~2 GiB), evict LRU by `last_used`;
    playback tier capped separately (~512 MiB); permanent/rule exempt.

- [ ] **CACHE-4 · Permanent cache + "Move to permanent" pin** — P2
  - **Details:** pin drops `kind='rule'`/`rule_id` and moves the blob to the
    config-root `media/` (never auto-removed; uncapped, warn threshold in UI).

- [ ] **CACHE-5 · Auto-download rules** — P2 · (MEDIACACHESTUDY.md §6)
  - **Why:** Symfonium parity: favorites, playlists, genres, and artists flagged
    "in download cache" auto-download existing + future content; rule removal
    prunes unless pinned.
  - **Details:** choose whole containers, Symfonium-style: artist
    (`ref_id = artist_id`; chooser = the ArtistEntry In-Cache button, P-8),
    album (`ref_id = album_id`), playlist once indexed (P-4), genre; favorites
    from `library.db.starred`; every rule downloads existing content and
    catches new content on each sync; re-evaluated after every successful
    library sync (queue missing, prune uncovered unless pinned).

- [ ] **CACHE-6 · Quality & network policy** — P2 · (MEDIACACHESTUDY.md §2/§8)
  - **Details:** transcode via `stream?maxBitRate=&format=` (fingerprint includes
    quality), `download` for original; prefer-server-online toggle; max parallel
    fetches; Wi-Fi-only has no desktop meaning until metering detection exists.

- [ ] **CACHE-7 · Cache status UI** — P2
  - **Details:** per-item state (cached/partial/queued) + used-bytes vs cap in
    the status bar; offline filter; permanent-tier warning threshold.


- [x] **PKG-1 · Desktop entry + hicolor app icons (NixOS parity)** — P2 · (DESKTOPICONSTUDY.md)
  - **Why:** the package ships a bare binary: no `.desktop`, no app icon, so
    launchers/docks show a generic blank entry and no window grouping metadata.
    The study covers the Desktop Entry spec, icon-theme lookup, Qt
    `desktopFileName`/WM_CLASS/app_id and NixOS's XDG wiring.
  - **Details:**
    - install `$out/share/applications/musika.desktop` (Type/Name/Exec/Icon/
      Categories/Keywords/StartupWMClass=musika/SingleMainWindow/StartupNotify),
      validated with `desktop-file-validate` (or `pkgs.makeDesktopItem`);
    - install `hicolor/scalable/apps/musika.svg` + 48/128/256 PNGs from a new
      brand asset (`src/icons/P.svg` can seed it);
    - `main.rs`: `QApp::application_name("musika")` so WM_CLASS/app_id match
      the desktop file (qtbridge has no `desktopFileName` yet);
    - rely on NixOS `xdg.menus/icons/mime` defaults (pathsToLink +
      `update-desktop-database`/`hicolor-icon-theme`) — nothing extra to wire;
    - optional upstream: qtbridge `desktop_file_name` + window icon.
  - **Verify:** `nix build`, store layout, `nix run .`, GNOME/KDE launcher and
    Wayland/X11 dock grouping.
  - **Now:** `musika.desktop` (validated in the build by
    `desktop-file-utils`), `src/icons/P.svg` installed as
    `hicolor/scalable/apps/musika.svg` + 48/128/256 PNGs rendered with
    `rsvg-convert`, and `QApp::application_name("musika")` in `main.rs`; all
    wired in `flake.nix` and verified with `nix build` (store layout). GNOME/KDE
    launcher + dock grouping still a manual pass on a desktop session.

## Quick wins

> **All quick wins are done** (2026-10-13): UI-2, UI-4, UI-6, UI-7, UI-10,
> UI-12, UI-14, UI-16, UI-23, UI-26, UI-27, UI-28, UI-29, ARC-6, ARC-9,
> ARC-10, ARC-16 and AUTH-2 landed; UI-15 was kept by decision (the chip now
> matches the fullscreen exit style, `q`/Esc remain the fast path); UI-17
> removed the inert shuffle until Q-3 brings real shuffle/repeat; UI-18 is
> satisfied — every transport/volume icon is SVG, remaining glyphs are
> monochrome text; ARC-18 removed the two empty migrations with
> `set_ignore_missing` for already-migrated databases.

Small, bounded, high-value changes.

> **0.0.8 batch** — moved from Should have on 2026-10-13 (small, bounded,
> low-risk): UI-13, UI-40, UI-41, LYR-8, S-9, AUTH-3, DOC-1.

- [ ] **UI-13 · Scroll position hint** — P3
  - **Why:** long lists have no scrollbar/position feedback for mouse users.
  - **Details:** one reusable `ScrollHint.qml` (opacity/transform only, never
    part of layout) attached to each long list: a slim bar pinned to the
    list's right edge, thumb height `max(h * h / contentHeight, h * 0.04)`,
    offset from `visibleArea.yPosition`, width
    `Math.max(2, h * Theme.hairlineRatio)`, `Palette.outlineVariant` at rest
    and `onSurfaceVariant` while scrolling or hovered; fade `quick`, hidden
    when `contentHeight <= height`.
  - **Targets:** `MediaList` (Albums/Songs/Artists), `SearchColumn`, the
    fullscreen queue in `NowPlaying.qml` and `QueueDrawer`; lyrics keep their
    own auto-follow and are out.
  - **Verify:** long lists show a proportional thumb that tracks wheel,
    trackpad and keyboard moves; short lists show nothing; no cursor/hover/
    click behaviour changes; `reduceMotion` shows/hides it without fading.

- [ ] **UI-40 · Palette transition wrapper (finish THM-4)** — P2 · (POLISHAUDIT §1.3)
  - **Why:** `Palette.apply` snaps every colour in one frame; THM-4 promised
    an animated swap.
  - **Details:** a QML `Palette` wrapper (or per-role Behaviors) animating
    role changes `micro`/`standard`, no animation on first apply/reset.

- [x] **UI-41 · List scroll easing + payload crossfades** — P3 · (POLISHAUDIT §4)
  - **Why:** keyboard moves teleport and model swaps flash.
  - **Details:** animate small `contentY` corrections for search/queue,
    crossfade lists on payload swap (`quick`), keep lyric follow in LYR-8.
  - **Now:** `ListScroll.qml` eases small `contentY` corrections
    (`quick`, Bezier standard, half-row context; instant `ListView.Contain`
    fallback for unrealised rows and under `reduceMotion`), wired into
    `SearchColumn`, the `QueueDrawer` (reorder-guarded) and the fullscreen
    queue; payload swaps crossfade `quick` in the search columns and both
    queues, with queue reorders staying live. The deeper distance-aware
    engine (frame-timing, follow guards, reuseItems) is UI-46.

- [ ] **LYR-8 · Smooth centre-follow + edge fades** — P2 · (roadmap "Lyrics presentation" P1)
  - **Why:** `positionViewAtIndex(Center)` teleports each line — once when a
    payload lands (NowPlaying.qml:156-157) and again on every active-line
    change (NowPlaying.qml:171-172), which fights manual scrolling whenever
    the 500 ms `Backend.position` tick moves `activeLyric` under the user's
    hand.
  - **Details:** animate `contentY` to the active delegate's centre
    (`standard`/`decelerate`, `quick` for tiny moves); pause follow on
    `onMovementStarted`, resume after ~4 s idle or on track change; top/bottom
    gradients from `surfaceContainerLow` to transparent.
  - **Verify:** the active line glides into centre; manual scroll is not
    yanked while playing; follow resumes after idle or on track change;
    unsynced lyrics stay static; gradients only over the scrollable range;
    `reduceMotion` keeps an instant `positionViewAtIndex`.

- [ ] **S-9 · Empty-query search surface + usage boost** — P2 · (roadmap mixed design, "Empty state" + "Selection, scope and ordering")
  - **Why:** an empty query leaves three empty columns; Noctalia leads with
    pinned/recent instead.
  - **Details:**
    - empty query: hide the results body (and the scope chips once S-6
      lands) and show one "Most played" surface instead — rows laid out like
      songs, play count as the right meta, ordered `play_count` desc, then
      title;
    - `song_search` carries no `play_count` and SQLite cannot join
      `search.db` to `library.db` (`search.rs:20-21`): copy the column into
      `song_search` (`migrations_search` + `rebuild` + `SongSearchRow`) and
      order typed hits by a bounded boost that keeps rank leading for strong
      matches, e.g. `ORDER BY rank - play_count * 0.1, rank` (tune the weight
      once against a real library);
    - scope switches keep each column's cursor and scroll position until the
      query changes (per-column cursors already exist; verify with S-6).
  - **Verify:** an empty field shows one Most-played list and no dead
    columns; typing restores the three columns with frequently played
    matches leading; clearing the field returns to Most played.

- [ ] **AUTH-3 · Login error/status roles** — P2
  - **Why:** `Login.qml` predates the MY3 roles: hand-written type fractions
    (`root.height * 0.0175`) and status as bare `Palette.error` text
    (Login.qml:127-135), so errors read as plain body copy.
  - **Details:** use `Theme.fontSize("label"|"body", root.height)` for the
    fields, button and status; render `Backend.statusMessage` on an
    `errorContainer`/`onErrorContainer` surface when not logged in and
    `primaryContainer`/`onPrimaryContainer` when logged in (hairline border,
    `Theme.radius`, word wrap, height reserved so the form does not jump).
  - **Verify:** a failed login shows a wrapping container message with AA
    contrast; success shows the primary container; no layout shift between
    states; no raw type fractions left in the file.

- [ ] **DOC-1 · Keep design docs in sync with the tree** — P2 · (A10)
  - **Why:** CRITIQUE/roadmap/materialyou3 describe pre-implementation states.
  - **Details:** a bounded pass, not a rewrite: add a one-line status header
    to `CRITIQUE.md`, `roadmap.md` and `materialyou3idea.md` ("study/
    proposal; `TASKLISTMAJOR.md` is the only status source; the tree wins")
    and fix the stale *current-state* claims found in this pass —
    `materialyou3idea.md:8` points at a non-existent `TASKS.md`;
    `materialyou3idea.md:44` ("one duration token") predates the
    `Theme.motion` tiers (Theme.js:43); `materialyou3idea.md:50` lists
    missing roles that `palette.rs` now ships (`errorContainer`,
    `tertiaryContainer`, `inverseSurface`, `scrim`, `surfaceDim`);
    `CRITIQUE.md:115` ("monolithic hardcoded actions") predates
    `keybind.rs`; tick the landed POLISHAUDIT quick-win items (§1.1 close
    fades, §1.4/AUTH-2 loading bar, §4 list easing) as consumed.
  - **Verify:** the three docs no longer assert anything the tree
    contradicts; dated diagnostics/history stay as written, only status
    claims change.

- [ ] **UI-47 · Microdesign: search column rule draws behind the next column's covers** — P3 · (design microtask)
  - **Why:** the structural rule (`|`) between search columns is a RowLayout
    sibling declared *before* the following column, so that column's covers
    paint over it — the divider is created behind the album cover.
  - **Details:** give the two divider Rectangles in `SearchOverlay.qml`
    (`columnsRow`, `:484-488` and `:508-512`) `z: 1` (or move them after the
    columns) so they paint above covers; keep them non-interactive and
    layout-neutral.
  - **Verify:** with results in every column the rules stay continuous over
    covers while scrolling; covers/rows stay clickable.

- [x] **UI-48 · Microdesign: search selection fills the whole row block** — P3 · (design microtask)
  - **Why:** lists (Albums/Songs/Artists) highlight the cursor/hover with a
    full-bleed row block (`secondaryContainer` / `surfaceContainerHigh` edge
    to edge). `SearchColumn` only fills the focused column's cursor row and
    gives hover no block fill at all, remembering inactive cursors with a dim
    left tick — partial marks where a block belongs, so search scans worse
    than the main lists.
  - **Details:** give every search row the list-style block highlight —
    cursor fill spanning the full row (`list.width`, thumb through meta), and
    the same `surfaceContainerHigh` hover fill as `MediaList`; keep the
    inactive-column cue as an accent *on* the block (or a border) instead of
    replacing it; the fill must not overflow the column (`clip: true` at the
    list, never across the UI-47 divider); keep the UI-45 scale emphasis and
    `Theme.selectionEmphasis` cap.
  - **Now:** `SearchColumn` rows are layout-only; a dedicated `highlight`
    layer sits over each row block, overflowing `width * 0.012` into the
    column gutters (clipped at the list), with cursor `secondaryContainer`,
    `MediaList`-style hover fill and the inactive cursor tick kept on the
    block. `qmllint` + offscreen run clean.
  - **Verify:** with results in all three columns, each column's cursor row
    and any hovered row show the identical full block fill as Albums/Songs/
    Artists; nothing spills over the dividers or panel edges.

- [x] **UI-49 · Microdesign: zero-result shake + icon flash in search** — P3 · (design microtask)
  - **Why:** a query with no hits only prints "no matches"; a physical
    "denied" cue makes the empty result register instantly, and a reusable FX
    covers every future rejected-input case.
  - **Details:**
    - trigger: on the transition to zero results across all three columns for
      a non-empty query (after the debounced response), and on Enter with a
      zero-result query; not on every keystroke while it stays zero;
    - FX: shake = horizontal keyframe oscillation of the search field/panel
      (a few ±px cycles) with a small bounce scale on the same target — one
      `ShakeFx`/extended `PressBounce` component so it is reusable, total
      **under 200 ms** (`quick`/`micro` tiers);
    - red flash: the search glyph flashes `Palette.error` and settles back
      inside the same 200 ms (the SVG is fixed white — flash a tinted layer
      behind/around it, or crossfade a red-tinted glyph copy instead of
      trying to tint the VectorImage);
    - `Theme.reduceMotion` skips both (or keeps only a static red tick);
      no layout shift, the field keeps focus and text.
  - **Verify:** gibberish query → one shake + red flash <200 ms; holding a
    zero-result query doesn't strobe; typing more re-triggers once per
    response; `reduceMotion` flat.
  - **Now:** `ShakeFx.qml` (reusable shake + bounce, transforms only, ~150 ms,
    flat under `reduceMotion`); the overlay fires it once per zero-hit
    response (guarded per distinct query via `_zeroQuery`) and on Enter with
    a zero-hit query, with a red-tinted glyph copy fading over the
    fixed-white SVG in the same window. Scripted harness: a repeated response
    for the same query stays silent, a changed query and Enter each fire
    once.

- [x] **UI-50 · Microdesign: ArtistEntry album selection expands like the lists** — P3 · (design microtask)
  - **Why:** lists and search columns now emphasize the current selection with
    the UI-45 expand effect (`Theme.selectionEmphasis` + `fitScale` cap, full
    block highlight), but the ArtistEntry album grid only tints the cell
    (`cell.current ? secondaryContainer`, hover fill) — a cover wall that
    reads flatter than every list it links to.
  - **Details:**
    - in `ArtistEntry.qml`'s grid delegate, apply the same emphasis to the
      current/hovered album: scale the cover + title block about a stable
      origin (top/center) at `Theme.selectionEmphasis`, title scale capped by
      `Theme.fitScale` so long titles stay elided inside the cell;
    - keep the block highlight + primary cover border, and hover expands too
      (mouse/keyboard parity, exactly like `MediaList`/`SearchColumn`);
    - transform-only (no `cellWidth`/layout change), `quick` with `Theme.ease`,
      flat under `Theme.reduceMotion`; leave headroom so the scaled cover
      doesn't clip at the cell edge.
  - **Verify:** cursor and hover over an album in ArtistEntry → cover/title
    expand like list rows, neighbours don't move, covers don't clip, long
    titles still elide, `reduceMotion` renders the plain grid.
  - **Now:** grid cells carry an `emphasized` state (cursor or hover, off
    under `reduceMotion`); the cover scales to `Theme.selectionEmphasis`
    about its centre and the title uses the `Theme.fitScale` cap — both
    transform-only with `quick`/`Theme.ease`; block highlight and primary
    border kept. `qmllint` + offscreen run clean.

- [x] **UI-51 · Remove the transport scopes: decoration, never a selection** — P3 · **top priority** · (code removal & cleaning)
  - **Why:** every transport verb is (or can be) a global binding, so letting
    the transport own a focus region/pane is dead weight: it dilutes the
    region model (`Home.activeRegion === 3`, the Ctrl-W candidate list, the
    PLAYER frame — all already gone, see Now), costs a fullscreen pane (Tab
    cycles transport → lyrics → queue and pane 0 eats `h/l/n/p/Enter`), and
    keeps a second keymap copy of the transport. The bar should stay visible
    and clickable, but never be a keyboard destination.
  - **Precondition:** bind seek globally first — new `SeekBack`/
    `SeekForward` actions on `,`/`.` (`h`/`l` stay `move_left`/`move_right`;
    `ctrl-h`/`ctrl-l` stay free), routed in `Home` to
    `Backend.playerSeek(±5)` like `next_track` (Home.qml:258-266). `Space`,
    `n/p`, `Ctrl-N/P`, `m`, `±` are already global; stop is MPRIS-only
    (`Main.qml:108-109`) and gets no key. Only then remove scopes.
  - **Details (removal):**
    - `Home.qml` (done — see Now): drop region 3 from the dispatch
      (`case 3`), the PLAYER region box/title, the `regionInDirection` edges
      to/from the player, and the `activeRegion === 3` condition on
      `toggle_fullscreen` (Shift+Enter becomes global); Ctrl-W candidates =
      sidebar/main/status.
    - `InferiorBar.qml` (done — see Now): delete `handleAction` — **keep the
      500 ms `Backend.tick()` timer**, playback/advance depends on it;
      buttons stay clickable/hoverable as decoration.
    - `NowPlaying.qml`: `pane` cycle becomes lyrics ↔ queue (drop pane 0 from
      `pane_cycle`, the `pane === 0` key block, the transport focus hairline
      and the keybar's transport hints); default pane = lyrics.
    - Sweep comments/docs/README (region lists, "Tab pane cycle", keymap
      table).
  - **Now:** the player bar is out of the region model — `Home` has no region 3
    (no Ctrl-W candidate, no PLAYER frame, no dispatch case, no `down` edge to
    it) and `InferiorBar.handleAction` is deleted (the 500 ms `Backend.tick()`
    timer stays). Seek moved out of the fullscreen transport pane: `SeekBack`/
    `SeekForward` on `,`/`.` (`token_from_qt` maps `0x2C`/`0x2E`) route in
    `Home` next to `next_track` (±5 s, works in every scope). Fullscreen `pane`
    is lyrics (1) ↔ queue (2) only: `Tab` toggles, `h`/`l` switch between the
    two, the `pane === 0` key block, transport focus hairline and
    click-to-focus are gone, and the keybar hints now show `,/.` seek and
    `Tab lyrics/queue`. README keymap swept (`Tab pane cycle` row removed,
    seek added to Playback).
  - **Verify:** Ctrl-W lists only sidebar/main/status; Shift+Enter toggles
    fullscreen from anywhere; Tab in fullscreen cycles lyrics/queue only;
    Space/`n/p`/`m`/`±`/seek work in every scope; mouse transport unchanged;
    tick/auto-advance intact.

- [ ] **UI-52 · ArtistEntry identity strip: biography slot + action chips (placeholders)** — P3 · (design microtask; ARTISTENTRYSTUDY.md §2, P-8, ARC-42)
  - **Why:** the artist header row is avatar + name + album count, and the
    whole right half of that parent rectangle sits empty while the album grid
    below owns the width. Navidrome fills exactly that space with the
    biography and the action cluster; landing the layout + visual shell now
    lets ARC-42 wire properties without moving a single rectangle, and keeps
    P-8's milestones B–D purely behavioural.
  - **Details:**
    - header becomes three zones: `[avatar] [name + album count] [identity
      strip]`; the strip is a `ColumnLayout` (`Layout.fillWidth: true`,
      `Layout.alignment: Qt.AlignVCenter`, spacing like the name column) so
      the two text zones share the free width and both elide;
    - strip top = **artist description** (~80 visible chars, two wrapped
      lines max, `Text.ElideRight`): a root `artistDescription` string renders
      when set (ARC-42's `artist_info` payload), otherwise the placeholder
      copy stands in ("Artist biography will appear here once the server
      metadata is wired."); no tap affordance yet (P-8 C owns expand), no
      rich text;
    - strip bottom = a `Row` of **icon-only placeholder actions**: one inline
      `ActionChip` component (square while it has no label; a label adds text
      + padding, kept for ARC-42's `Cached · <bytes>`) plus a five-glyph
      **Stars** cluster — `star.svg` filled / `star-outline.svg` empty
      (baked white, `SymbolIcon`), the first `artistRating` (0..5, Subsonic
      domain) filled at `Palette.primary`, the rest `onSurfaceVariant`, the
      cluster vertically centred against the chip row; chips use the
      `ExitChip` visual family (`surfaceContainerHigh` fill, `outlineVariant`
      hairline, `Theme.radius`), hover `secondaryContainer` with the shared
      `quick` colour behaviour; click and hover are inert (`cursorShape`
      only) with a `// ARC-42` comment — the controls are deliberately not in
      the region model until the backend defines focus behaviour; row order is
      stars · In-cache · Shuffle · Favorite; hovering a star previews the
      level incrementally up to the cursor (`hoveredStar`, -1 = none) and the
      hovered glyph fires the shared `PressBounce`; each star sits in a 0.05 h
      slot (the hitbox) around the 0.032 h glyph; glyphs:
      Favorite = filled `star.svg` gray while unfavorited (ARC-42 comment:
      yellow when favorited, Material palette), Shuffle =
      `outline-shuffle.svg`, In-cache = empty `outline-download.svg` (ARC-42
      comment: filled `baseline-download.svg`, green MY3 accent
      `Palette.tertiary`, when the rule reports cached);
    - the chips read the ARC-42 vocabulary already in their labels: Favorite
      (star toggle), Stars (rating, hidden when `canRate` is false), In-cache
      (rule state label Off / n of N / bytes), Shuffle (artist-scoped queue);
      placeholder text shows the "before" wording (`Favorite`, `Stars`,
      `In-cache`, `Shuffle`) so wiring only swaps text/colour/state;
    - UI-32 cross-ref: below `narrow` the strip hides (`Theme.isNarrow`) and
      the header falls back to today's two-zone layout; no new component file
      unless a second surface adopts the chip (then extract);
    - header backdrop: the artist art gets the same D-5 ambience as
      `AlbumEntry`/`NowPlaying` (`CoverImage.ambient` + the surface gradient
      wash behind the content), so the entry floats on the artist's colours.
  - **Verify:** at ≥narrow the header shows the biography placeholder over
    four chips on the right with no overlap with the avatar/name at compact
    widths; hovering a chip tints it and the cursor is a pointing hand;
    clicking does nothing (no backend, no toast, no focus grab); below narrow
    the strip is gone and the header reads exactly as today; biography text
    wraps to two lines and elides; `reduceMotion` still fades nothing.

- [x] **VI-6 · `Ctrl-[` alias + pending-sequence cancel** — P1 · (N3 residual)
  - **Why:** `Ctrl-[` is muscle memory for Esc; Esc should also clear a half-typed
    sequence/count.
  - **Details:** add `ctrl-[` token; feed Esc through `Keymap.reset()`.
  - **Now:** `token_from_qt` maps `[` (`0x5B`) so `Ctrl-[` becomes `ctrl-[`;
    bound to `Action::Back` next to `escape`, and a pending `g` is cleared when
    escape/`ctrl-[` arrives (unit-tested).

- [x] **VI-10 · Repurpose `Ctrl-N`/`Ctrl-P` as track skips** — P1 · (N7, Q-9)
  - **Why:** `Ctrl-N`/`Ctrl-P` only duplicated `j/k` scrolling; the better use
    is the one vi media users expect — next / previous track while browsing.
  - **Details:** `j/k` keeps the list cursor (and scrolling); `Ctrl-N`/`Ctrl-P`
    skip tracks wherever a list owns focus; fullscreen keeps the same keys
    (Q-9's transport rule) so the ergonomics match everywhere.
  - **Now:** `ctrl-n`/`ctrl-p` map to `NextTrack`/`PrevTrack`; `Home` handles
    both actions globally (after the modals and visual mode, before region
    routing), so main lists, sidebar and fullscreen all skip; `NowPlaying`
    hints say `Ctrl-N/P`; AlbumEntry's old `scroll_*` aliases are now inert.

- [x] **VI-16 · Key overload disambiguation (`Enter`, `q`)** — P2 · (N9)
  - **Why:** `Enter` means play/open/confirm depending on context; `q` and Esc
    both Back; document or split.
  - **Now:** documented in `keybind.rs` above `defaults()`: Enter is
    context-confirm (row open/play, queue jump, album back, status sync,
    visual commit); `Esc`/`q`/`Ctrl-[` are the same Back, Esc stepping out of
    typing/insert first; `Space` play/pause and `Shift-Enter` fullscreen.

- [x] **VI-17 · Status bar focus behaviour** — P2 · (CRITIQUE ghost focus)
  - **Why:** focusing a non-interactive pane used to make keys inert.
  - **Details:** either strip region 2 from the focus ring or keep it and give it
    real actions.
  - **Now:** kept with real actions: Enter syncs (`RightBar.handleAction`),
    `move_down`/`move_left`/`move_right` leave to the main list, and
    `RightBar.active` (from `Home.activeRegion === 2`) shows an "Enter: sync"
    hint while focused.

- [x] **VI-18 · Space leaks through the modal search** — P2 · (MD32)
  - **Why:** the overlay is documented as modal, but `play_pause` is dispatched
    before Home sees the overlay (Main.qml:24-29).
  - **Details:** route `play_pause` through Home/overlay first.
  - **Now:** `Main.dispatch` no longer handles `play_pause`; `Home` resolves it
    only after the search/queue modal captures, so Space does not leak while an
    overlay owns input (typing fields consume it as text).

- [x] **VI-19 · `i` is dead outside search** — P3 · (MD33)
  - **Why:** `InsertMode` is bound globally but only the overlay consumes it.
  - **Details:** scope the binding or consume it (mode indicator). 
  - **Now:** consumed — `i` opens the floating search in INSERT mode from any
    region (`Home`), while inside the overlay the modal routing re-enters
    typing; documented in the keymap semantics and the README.

- [x] **VI-20 · keyTimer comment/mechanism drift** — P3 · (MD34)
  - **Why:** the comment claims bare Space is deferred; it resolves immediately —
    only `g g`/`shift-space …` are.
  - **Details:** fix the comment or make deferral per-binding (ties to VI-9).
  - **Now:** the timer comment now names `g g`/`shift-space f` as the deferred
    sequences and states that bare Space fires immediately.

- [x] **Q-9 · Fullscreen queue & transport polish** — P2 · (fullscreen review)
  - **Why:** queue rows wear debug-ish `Prev`/`Next` tags, the order cannot be
    changed from the keyboard, and fullscreen transport uses bare `n`/`p`
    while the rest of the app standardises on `Ctrl-N`/`Ctrl-P`.
  - **Details:**
    - drop the `Prev`/`Next` row names and the expression that derives them
      (`index < queueIndex ? "Prev" : "Next"`, NowPlaying.qml queue delegate);
      keep only the active-row treatment;
    - `Alt-j` / `Alt-k` move the selected track down/up in the queue;
    - fullscreen transport keeps `Ctrl-N` / `Ctrl-P` for next/previous track
      (not bare `n`/`p`) so the ergonomics stay consistent everywhere.
  - **Now:** tags already gone; `alt-j`/`alt-k` bound to QueueDown/QueueUp
    (alongside Shift-J/K) and the fullscreen queue pane reorders, with a
    one-payload `_reordering` guard so the cursor does not snap back to the
    playing track; the pane hint shows `Alt-J/K move`. `Ctrl-N/P` are global.

- [x] **Q-10 · Song activation enters Fullscreen Playback** — P2 · (roadmap D-4)
  - **Why:** activating a song is the deliberate "listen now" verb; the user
    should land in the listening surface (cover + lyrics + queue) instead of
    back in the list they picked it from.
  - **Details:** Enter (and the equivalent click) on a song — floating search
    result or `AlbumEntry` song list — plays it and opens fullscreen; album and
    artist activation keep their navigation semantics; queue `Enter` (jump) is
    unchanged; the search overlay closes first, no wait on stream or palette.
   - **Now:** landed — `playerLoad` emits `playbackStarted`; `Home` flips
    `nowPlayingExpanded` on it (search songs + `AlbumEntry` rows both go
    through `playerLoad`), while the album-page IPC flow clears it again when
    it pushes the page. Refined: Songs-segment rows play through
    `playerLoadLibrary`, which queues the whole index in `all_songs` order
    starting on the clicked row (tracks before it included; no window and no
    lazy extension), so that tab never opens fullscreen. Queues come from the
    local index only (library.db rows, covers from covers.db; `stream_url` is
    offline URL building); search and `AlbumEntry` keep the fullscreen path.


- [x] **P-7 · Fullscreen controls completeness** — P2 · (U6)
  - **Why:** transport + seek landed, but volume/mute are absent in fullscreen.
  - **Details:** reuse IconButton/state layers; keep play/pause centered.
  - **Now:** right-anchored volume cluster in the fullscreen transport: tiered
    icon (click = mute) plus a compact draggable bar, sharing
    `Theme.volumeIcon` with the player bar, so play/pause stays centered;
    `m`/`-`/`+` remain global.


- [x] **LYR-5 · Prefetch, miss TTL, local `.lrc` override** — P2 · (roadmap L4)
  - **Why:** avoids per-open latency and request storms, and lets users supply
    their own synced lyrics.
  - **Details:** prefetch lyrics for the next N queue tracks (reuse
    `FETCH_CONCURRENCY=8`); cache misses as `source='none'` with TTL (~7 days)
    plus manual refresh; `~/.config/musika/lyrics/<song_id>.lrc` wins over the
    DB and is never purged.
  - **Now:** complete — override + 7-day miss TTL + 24h plain revalidation +
    legacy LRC parsing (earlier), sync-time bounded backfill (ARC-35), and now
    `Player::upcoming` prefetches the next 3 queue tracks on every request,
    with `lyrics::invalidate` + `Backend.refreshLyrics` (`lyrics_refresh`
    IPC) for a manual refresh.

- [x] **LYR-9 · Lyrics state polish** — P3 · (roadmap "Lyrics presentation" P2)
  - **Why:** "No lyrics" flashes before the first response, track changes swap
    text abruptly, and the tag leaks `subsonic · synced · xxx`.
  - **Details:** loading state before first payload; crossfade the list on
    track change (`quick`); suppress `xxx`/`und`; render `synced` as a small
    MY3 chip in `label` type.
  - **Now:** `lyricsLoading` shows a centered "Loading lyrics…" instead of the
    empty state; payload swaps fade the list (`quick`); `xxx`/`und` are
    suppressed and `synced` is a `secondaryContainer` chip in `label` type.

- [x] **S-4 · Align search order and default scope** — P2 · (MD10, roadmap mixed design)
  - **Decision (roadmap):** columns and priority are Songs > Albums > Artists;
    the cursor starts in the first non-empty scope and falls through in that
    order; counts move into the scope chips (S-6).
  - **Now:** landed — `SearchOverlay` columns are Songs(40)|Albums(32)|
    Artists(28) with `activeColumn` 0/1/2 and the start scope picked by
    Songs > Albums > Artists; headers still carry the counts until S-6.

- [x] **S-5 · Hover steals the search cursor** — P3 · (MD15)
  - **Why:** passing the mouse over a column re-targets `activeColumn`; only
    click should change it.
  - **Now:** the row `HoverHandler` only sets `PointingHandCursor`; the
    `hovered` signal and the overlay's `onHovered` handlers are gone, so hover
    never moves the cursor — click and keys only.
- [x] **S-7 · MainScope album ordering (`Shift-H`/`Shift-L`)** — P2 · (Subsonic order types)
  - **Why:** the album list was hardcoded to `ORDER BY artist, name`; the
    main scope should expose the Subsonic order families (by name, by year)
    without leaving the keyboard.
  - **Details:** `Backend.albumOrder` + `albumOrderNext/Prev`;
    `library::ALBUM_ORDERS = [Artist, Name, Year]` (alphabeticalByArtist /
    alphabeticalByName / byYear) cycled with wraparound; `library::albums`
    keeps static SQL per order/filter (no dynamic SQL); `AlbumList` reloads
    on `album_order_changed`; the hint strip shows `Order by <OrderType>`
    right of the search keybind, same type size.
  - **Now:** landed — `shift-l`/`shift-h` bindings + unit tests (65 pass);
    `AlbumList` re-requests through the backend order. Extended: the cycle is
    now shared (`library::ORDERS` + `cycle_order`, `stepped_order` in the
    backend) and drives the Songs segment too — `Backend.songOrder` +
    `songOrderNext/Prev`, `all_songs`/`songs_range`/`song_ordinal` take the
    same static-SQL order arms, `SongList` reloads on `song_order_changed`
    and shows `Order: …`; `Home` routes the keys by segment (Songs → song
    order), and the library queue is built in the order that was active when
    the row was played.

- [x] **UI-1 · Region frame colours** — P2 · (MD1)
  - **Why:** PLAYER uses `Theme.error` (red) and four accents read as identity,
    not state; unify or only show in visual mode (Home.qml).
  - **Now:** unified — the region delegate ignores the per-region colours and
    uses `Palette.primary` for both the border and the title, so the frame
    reads as focus state, not identity (the color array is gone).

- [x] **UI-2 · Remove always-on search hint strip** — P2 · (MD2)
  - **Why:** it owns a full row forever and is noise after the first day.

- [x] **UI-4 · Counts live in three places** — P2 · (MD4)
  - **Why:** sidebar stats + per-list "N indexed" + content counts duplicate.

- [x] **UI-6 · Placeholder segments look active** — P3 · (MD6)
  - **Why:** Playlists/Radio look selectable but only open an empty state.

- [x] **UI-7 · Rotated "LIBRARY" visual-mode label** — P3 · (MD7/U5)
  - **Why:** rotated text is unreadable at a glance.

- [x] **UI-10 · Hover parity + initial cursor** — P2 · (MD18)
  - **Why:** AlbumList tints the cover on hover, others do nothing; no row is
    highlighted until the first key.

- [x] **UI-12 · Metadata line position consistency** — P3 · (MD20)
  - **Why:** "N tracks" is right-aligned; "N albums" sits under the name.

- [x] **UI-14 · Expandable album notes** — P2 · (MD22)
  - **Why:** notes clamp to 3 lines with no way to read the rest.

- [x] **UI-15 · Album "‹ Back" chrome** — P3 · (MD23)
  - **Why:** duplicates `q`/Esc; keep only if the toolbar gains more.

- [x] **UI-16 · "COVER" fallback text** — P3 · (MD24)
  - **Why:** placeholder word in the art slot; use a glyph/blank surface.

- [x] **UI-17 · Inert controls (shuffle only now)** — P2 · (MD25)
  - **Why:** a control that does nothing breaks trust.
  - **Details:** wire (Q-3) or remove the shuffle button; volume and stop are
    done.
  - **Now:** volume (Q-4) and stop (`■` → `playerStop`) are wired; shuffle
    (`random-svgrepo-com.svg`) still has no `onClicked`.

- [x] **UI-18 · Icon language consistency** — P2 · (MD26/MD35)
  - **Why:** emoji speaker breaks the monochrome look; glyphs vary in weight.

- [x] **UI-23 · Toast retry covers playback errors too** — P2 · (MD38)
  - **Why:** the Retry button only renders for `kind === "sync"`.

- [x] **UI-26 · Centralize `formatTime`** — P3 · (MD30)
  - **Why:** duplicated in six files (InferiorBar, NowPlaying, SongList,
    AlbumEntry, PlaybackNode, SearchOverlay).

- [x] **UI-27 · Separators / font ratios** — P3 · (MD36/MD37)
  - **Why:** "  ·  ", "   " and "·" coexist; title fractions 0.24/0.26/0.26.

- [x] **UI-28 · Album page hero cover header** — P2 · (roadmap "Album page cover" D-3)
  - **Why:** the centered `0.32 × height` cover reads small on wide panes
    (≈ 16% of pane width) and leaves the description stacked under wide dead
    margins (`AlbumEntry.qml:156-188`).
  - **Details:** header row (square cover left, description right);
    `headerH = min(mainH*0.44, 512 + 2m)`, `cover = clamp(mainH*0.42, 160,
    512)`; no upscaling past the 512 px cache; the same grid drops to one
    column below ~900 px (cover centered over the description); songs keep
    ≥ 40% height; MY3 radius/hairline/hierarchy per D-3.
  - **Now:** `AlbumEntry.qml` header is a `GridLayout` — cover
    `clamp(0.42h, 160, 512)` left of the description (one column under
    900 px), fixed `headerHeight` when side by side; build + qmllint clean.
    Visual acceptance at 1366/1920/2560 still pending.

- [x] **UI-29 · Album page ambience backdrop** — P2 · (roadmap "Album page cover" D-5)
  - **Why:** the album page reads flat next to its own art; a blurred copy
    ties the hero to the theme without inventing a new color.
  - **Details:** full-pane blurred `albumCover` copy (`MultiEffect` blur
    1.0/64, saturation 0.6) behind the header, `Palette.surface` gradient
    α 0.82 → 0.30 → 1.0, `standard` fade-in, hidden without art; the one
    sanctioned non-modal blur (R2 amended).
  - **Now:** landed in `AlbumEntry.qml`; visual acceptance (bright/dark
    covers, songs-boundary seam) pending.

- [x] **UI-30 · Manual sync shows `Syncing…`, not `Up to date`** — P2 · (sync-state review)
  - **Why:** `RightBar.qml:90` renders `librarySyncing ? "Syncing…" :
    "Up to date"`, but the button/tap path (`RightBar.qml:45,139` →
    `Backend.syncLibrary`) never sets `library_syncing` — only the
    auto/login path (`qml.rs:1074`, invoking `librarySyncStarted`) does. A
    manual sync therefore reads "Up to date" for its whole duration, with the
    progress track parked at 0.
  - **Details:** enter the syncing state at the start of the manual
    `syncLibrary` slot (`qml.rs:434`), reusing `library_sync_started` (set
    `library_syncing`/`sync_progress`/`sync_message` before spawning);
    `librarySyncFinished` already clears it, including the failure path. Add
    the re-entrancy guard there too (a second click while syncing is ignored
    — ties ARC-28). Acceptance: the status flips to `Syncing…` within one
    frame of the click and returns to `Up to date`/the result message on
    completion or error.
  - **Now:** landed — `sync_library` returns early while `library_syncing`
    and calls `library_sync_started()` before spawning, so the bar flips to
    `Syncing…` immediately; the re-entrancy guard is in (ARC-28 partial).

- [x] **UI-34 · Overlay close fades actually play** — P1 · (POLISHAUDIT §1.1)
  - **Why:** `SearchOverlay.qml:40` and `QueueDrawer.qml:31` use
    `visible: opened`, so the opacity Behaviors are cut off the moment the
    overlay closes — the advertised fade-out never renders.
  - **Details:** follow the Toast/NowPlaying pattern
    (`visible: opened || panel.opacity > 0`) and gate the backdrop handlers
    with `enabled: root.opened`.
  - **Now:** both overlays stay rendered until the panel fade finishes; the
    search backdrop only dismisses while `opened`.

- [x] **ARC-6 · Docs drift** — P2 · (A10)
  - **Why:** roadmap still says "Space+F", "right bar", stale durations.

- [x] **ARC-9 · Cache `SecretKey`, honor `XDG_CONFIG_HOME`** — P1 · (O7)
  - **Why:** re-reads the key file per call; path bypasses `config_dir`.

- [x] **ARC-10 · `sync_player` signal thrift** — P1 · (O8)
  - **Why:** ~13 signals + string clones every 500 ms tick.

- [x] **ARC-13 · Dead code cleanup** — P2 · (O11, MD31)
  - **Why:** `PlaybackNode.qml` is uninstantiated, `covers::cached_path` unused
    until the media cache lands, `index.txt` mirror intentional?

- [x] **ARC-14 · Deduplicate constants** — P2 · (O12)
  - **Why:** volume 70.0, `COVER_SIZE`, `FETCH_CONCURRENCY` exist twice.
  - **Now:** `src/consts.rs` owns `FETCH_CONCURRENCY` (index, covers, lyrics)
    and `DEFAULT_VOLUME` (player); `COVER_SIZE` already had a single home in
    `covers.rs`.

- [x] **ARC-15 · Stop masking errors as auth failures** — P2 · (O13)
  - **Why:** every envelope error becomes "Wrong username or password".
  - **Now:** `SubsonicError` reads the code; only 40-44 fall back to the
    credentials message, everything else surfaces its own text or
    "Server rejected the request (code N)" (unit-tested).

- [x] **ARC-16 · mpv supervisor hardening** — P2 · (O16)
  - **Why:** `sh -c` string building for MPRIS/kill; avoid spawning shells.

- [x] **ARC-18 · Remove empty no-op migrations** — P3 · (O18)

- [x] **AUTH-2 · Loading screen polish** — P3
  - **Why:** countdown prints raw ms ("timeout 4800ms"), progress steps every
    100 ms without easing; no cancel.
  - **Details:** seconds or spinner, `linear` smoothing, Esc/click to login.

## Solved (reference)

Kept for rationale; the tree already carries these.

- [x] **VI-1 · Mode indicator in the status bar** — P1 · (N4 partly, MY3 QW10)
  - **Why:** Vi users must always know the mode; silent mode changes break trust.
  - **Details:** colored `NORMAL`/`INSERT`/`VISUAL`/`COMMAND` chip in the bar.
  - **Now:** `NORMAL` chip replaced the "LIBRARY" title (RightBar.qml); the
    search overlay shows an `INSERT`/`NORMAL` chip too.

- [x] **VI-5 · Single `Esc` dismisses overlays** — P1 · (N3, CRITIQUE Ph1.3)
  - **Why:** Esc is the invariant cancel key; unbound Esc feels broken.
  - **Details:** close fullscreen → pop album page → close search, in order.
  - **Now:** `escape` maps to `Action::Back` (keybind.rs); Home closes fullscreen
    first (Home.qml); search overlay Esc steps typing → normal → close.

- [x] **VI-21 · Global yank/paste (`y`/`P`) to the system clipboard** — P1 · **high priority** · (CRITIQUE Ph2 operators)
  - **Why:** copying a track out of the app and pasting a track reference back
    in are the two most basic Neovim-style verbs; today the system clipboard is
    unreachable from the keyboard.
  - **Details:** global (any region) `y` yanks the playing track as
    `Artist - Title` to the system clipboard; `P`/`Ctrl-V` paste the clipboard
    into the floating search field (`SearchOverlay.openWith`), so the text is
    visible and correctable before Enter plays the selection — raw clipboard
    text is never rejected in the background.
    `p` stays track-previous for now — the transport rebind and the full
    operator grammar (`yy`, `p`/`P` queue append/play-next) land with VI-8;
    yanking a cursor row / visual selection follows VI-4.
  - **Now:** v1 landed — `keybind.rs` `yank`/`paste` actions (`y`, `shift-p`,
    and system `ctrl-v` so clipboard managers like Noctalia can paste);
    `Main.qml` hidden `TextEdit` clipboard bridge + global dispatch (focused
    text fields still consume Ctrl-V themselves); `Home.pasteSearch` opens
    the search overlay with the text in the field via
    `SearchOverlay.openWith`; unit tests for the binding.


- [x] **Q-1 · Queue view (drawer) with reorder/remove** — P2 · (M1, CRITIQUE Ph2.1)
  - **Why:** the queue is the primary working document; today it is invisible.
  - **Details:** collapsible pane (`Ctrl-W q`/`:queue`), `j/k` navigation,
    `K/J` reorder, `dd` remove, canonical cursor token, `quick` collapse.
  - **Now (working tree):** `QueueDrawer.qml` (right drawer, toggled by `Ctrl-Q`
    / IPC `queue_toggle`); `j/k` + `Ctrl-N/P` move, `Enter` jumps, `Shift-J/K`
    reorder, `dd` removes; `player.rs` exposes queue/jump/remove/move_item and
    `emit_queue` publishes over the `queue` dataReady channel. (`Ctrl-W q`/`:queue`
    not used yet; `quick` easing pending.)

- [x] **Q-2 · Playback context builds a real queue** — P0 · (M2)
  - **Now:** `playAlbum`/`playSong` load album/list context instead of a single
    track (qml.rs playerLoad/playAlbum).

- [x] **Q-4 · Volume + mute in UI and keys** — P2 · (M4, CRITIQUE Ph3.1)
  - **Why:** mpv has volume, the backend keeps it, but the bar is a static bar.
  - **Details:** draggable slider + percentage, `m` mute, `+/-`/`]v [v` steps,
    focus state layers; expose slot in `qml.rs`.
  - **Now:** `Backend.volume`/`Backend.muted` properties + `playerSetVolume`/
    `playerAdjustVolume`/`playerToggleMute` slots; `player.rs` gained
    `muted`/`set_muted`/`toggle_mute` (mpv `mute` property); keys `m` mute,
    `-`/`+` (and `=`/shift-=`, keypad) step 5%; `InferiorBar` speaker icon by
    tier (mute/zero/low ≤33/high, white SVGs) + clickable/draggable bar and
    click-to-mute; stepping up unmutes. Icons sit in fixed invisible frames
    (`IconButton` layers) and the volume SVGs are translated to a shared
    speaker baseline, so tiers swap without moving. `]v`/`[v` left to VI-8.

- [x] **Q-7 · MPRIS/D-Bus metadata** — P2 · (M7)
  - **Now:** mpv MPRIS script + metadata via IPC (`mpris:artUrl`).

- [x] **LYR-1 · API: `getLyricsBySongId` + OpenSubsonic probe** — P2 · (roadmap L0)
  - **Why:** only the OpenSubsonic `songLyrics` endpoint returns per-song and
    synced lyrics; legacy `getLyrics` (artist/title, plain) is a fuzzy fallback.
  - **Details:** types for `lyricsList.structuredLyrics` (`lang`, `offset`,
    `synced`, `line[{start, value}]`); call `getOpenSubsonicExtensions`, cache the
    capability (`songLyrics` v1; ignore `enhanced` v2 for now).
  - **Now:** `api.rs` has `StructuredLyrics`/`LyricLine`/`LyricsList`/
    `OpenSubsonicExtension`, `get_lyrics_by_song_id`, `get_open_subsonic_extensions`.

- [x] **LYR-2 · DB5 cache (`lyrics.db`) + purge-on-sync** — P2 · (roadmap L1)
  - **Why:** multiDB style: re-fetchable content gets its own pool, keeps
    library.db lean and the purge domain independent, exactly like covers.
  - **Details:** `migrations_lyrics`, new pool in `Dbs`, schema
    `lyrics(song_id, source, synced, lang, offset_ms, text, fetched_at)` +
    `lyric_line(song_id, idx, start_ms, value)`; delete rows whose song id is
    missing from library.db after a successful sync; no blob directory.
  - **Now:** `migrations_lyrics/20261013000001_lyrics.sql` (`lyrics`,
    `lyric_line` FK-cascade, `lyrics_meta`); `Dbs.lyrics` + `lyrics_path()`;
    `lyrics.rs::purge_with` called from `index.rs` after a successful sync.

- [x] **LYR-3 · `requestLyrics` + plain rendering in NowPlaying** — P2 · (roadmap L2)
  - **Why:** the redesigned fullscreen pane still shows the hardcoded
    `lyricLines` placeholder (`TODO(M6)` in NowPlaying.qml).
  - **Details:** `Backend.requestLyrics(songId)` -> tokio ->
    `dataReady("lyrics")` JSON; render plain text; "No lyrics" empty state;
    fallback chain override -> bySongId -> legacy `getLyrics`.
  - **Now:** `Backend.currentSongId` + `requestLyrics()`; `lyrics.rs::fetch`
    (override → cache → FTS5-era server chain); NowPlaying renders lines or a
    centered "No lyrics"; source/lang tag in the header.

- [x] **LYR-4 · Synced rendering: highlight + auto-scroll + offset** — P2 · (roadmap L3)
  - **Why:** synced lyrics are the point of the feature; the pane currently
    fakes the active line (`currentLyric` stub).
  - **Details:** compare `Backend.position` (minus OpenSubsonic `offset`) against
    `start`; active line `primary`/bold (`micro`), scroll-to-center (`quick`);
    no animation for unsynced text.
  - **Now:** `activeLyric` binding applies `offset_ms`; `> … <` marker plus
    `micro` color; `positionViewAtIndex(…, Center)` on line change; plain
    lyrics stay static.

- [x] **LYR-7 · Lyrics stage states (kill the `> <` code-art)** — P2 · (roadmap "Lyrics presentation" P0)
  - **Why:** the active line is ASCII-marked at one uniform font size, so the
    pane reads like a text dump (NowPlaying.qml:456-473).
  - **Details:** active/near/far model: active `body` scaled ×1.12, full
    contrast; near ±1 opacity 0.8; far ±2+ opacity 0.5 and smaller; non-synced
    = uniform 0.8 column, no focus line; fixed-height delegates so state
    changes never reflow.
  - **Centred style:** lyrics are a centred column — `Text.AlignHCenter` and
    scale from the text centre (no left-anchored "Left → Right" growth); the
    optional left hairline is dropped in favour of scale + contrast.
  - **Pane-sized type:** the font is computed from the lyrics rectangle
    (`pane.height * 0.055`, clamped 10–18) with proportional left/right padding,
    so lines never collide with the pane walls at any window size.
  - **Now:** `NowPlaying.qml` — no `> <` marker; active/near/far opacity +
    scale about the text centre; the active emphasis is pre-calculated per
    line against the inset `lyricsStage` margin (long wrapped lines cap at the
    fit scale instead of being cut; unbreakable words can shrink below 1) and
    the grown slot adds symmetric padding so neighbours make room; uniform dim
    column when unsynced; pane-sized `lyricSize`/`lyricPad`.

- [x] **S-1 · Floating search (three columns, keyboard, states)** — P1 · (roadmap design)
  - **Now:** `SearchOverlay.qml` + `SearchColumn.qml`: Artists|Albums|Songs,
    debounce, skeleton, no-matches, Enter open/play, Esc modes, hover.

- [x] **S-2 · Search appearance pass** — P2 · (MD13/MD14/MD9/MD12)
  - **Now:** two-line title/subtitle cells, mode chip, mode-aware footer,
    "N albums" wording, clamped type, `quick` easing. Redesign pass: only the
    focused column carries the full highlight (inactive cursors keep a dim
    tick), column dividers + contained count pills, 38/40/22 weights with the
    artist count stacked as the subtitle, the glyph moved inside the input's
    padding, INSERT/NORMAL unified on the bar's primary accent, the viewport
    snapped to full rows with a footer rule, and the region frame hidden while
    the modal is up.

- [x] **S-3 · Query `search.db` (FTS5), retire `fd`/tmpfs** — P0 · (O1, CRITIQUE §3.4)
  - **Now:** palette and overlay query FTS5 through `Backend.requestSearch`.

- [x] **UI-3 · De-duplicate sync state** — P2 · (MD3)
  - **Now:** the button no longer relabels to "Syncing…" and uses the disabled
    content token; residual: the progress track is always visible when idle.

- [x] **UI-5 · Two stacked titles removed** — P2 · (MD5)
  - **Now:** "LIBRARY" title replaced by the mode chip.

- [x] **UI-11 · `FadeText` opt-out** — P2 · (MD19)
  - **Now:** `animated: false` on list/search/album/toast delegates.

- [x] **UI-24 · Fullscreen 50/50 split + px sweep** — P2 · (MD27/MD29)
  - **Now:** cover/lyrics 50/50; NowPlaying px → fractions/theme radius.

- [x] **UI-25 · MY3 motion & state system** — P1 · (MY3 steps 1–4, 8; QW1–8, 10–12)
  - **Now:** motion tiers/easings, shape/state/z tokens, scrim, container roles,
    `EnterView`/`ExitView`, niri-like backdrop blur on modal surfaces,
    IconButton state layers, toast
    quick-in/lazy-out, mode chip, linear progress, search weights documented.

- [x] **UI-31 · Play/pause loses its filled background (icon-only, all contexts)** — P2 · (icon-language review)
  - **Why:** play/pause is the only `IconButton` drawn with the `accent`
    container (`IconButton.qml:21,36,51`; used at `InferiorBar.qml:155` and
    `NowPlaying.qml:388`). The filled square competes with the transport's
    focus ring, breaks the monochrome glyph language (UI-18) and reads as a
    FAB rather than a transport control (R1/R3: hierarchy is tonal, not extra
    chrome).
  - **Details:** drop `accent: true` at both call sites; once no caller uses
    it, remove the `accent` property and its `onPrimaryContainer`/`onPrimary`
    and `stateFocus`-on-hover branches so the glyph uses the standard state
    layers (R6), keeping the focus hairline. Sizes, centring and
    `symbolRatio` stay. Decision: play/pause is always white
    (`Theme.iconStrong`), the one glyph that never follows the seeded palette
    or the theme roles.
  - **Now:** `IconButton` has no `accent` (transparent root, standard state
    layers + hairline) and draws either a text glyph or an SVG (`iconSource`,
    rendered with `VectorImage` so no QtSvg plugin is needed);
    `InferiorBar`/`NowPlaying` use the white transport SVGs from `src/icons/`
    (play/pause/next plus a mirrored copy of the skip icon for previous,
    embedded via a second `include_bytes_qml!` under `qrc:/app/icons/`;
    shuffle/stop stay glyphs for now), swapping with a `quick` (120 ms)
    crossfade. Icons render through `VectorImage.CurveRenderer` (built-in
    AA; the default GeometryRenderer aliases at icon sizes). Build + qmllint
    clean; visual check pending.

- [x] **UI-33 · Skip bloom feedback** — P2 · (transport polish)
  - **Why:** keyboard and mouse skips were easy to miss while browsing; the
    transport button should acknowledge the change.
  - **Details:** a small blurred bloom in the generated primary
    (`Palette.primary`), expanding 0.8→1.7 while fading 0.5→0
    (`standard`/`decelerate`); play/pause untouched.
  - **Now:** `IconButton.bloom()` + `Backend.trackSkipped(forward)` (emitted
    from `playerNext`/`playerPrevious`); `InferiorBar` and `NowPlaying` bloom
    the matching direction for keys, clicks and IPC alike.

- [x] **THM-1 · Engine spike: `material-colors` crate, cover → dark scheme** — P2 · (roadmap T0)
  - **Why:** matugen itself is GPL-2.0, but its engine (`material-colors`) is
    MIT/Apache-2.0; embedding it avoids a runtime CLI and keeps the licence
    clean.
  - **Details:** add `material-colors` (+ decode); replicate matugen's pipeline
    (resize 112, QuantizerCelebi @128, CAM16 chroma >= 5, `Score`, TonalSpot,
    dark scheme); fixture-image unit test asserts roles; no DB/QML yet.
  - **Now:** `colors.rs` embeds `material-colors 0.5` (`quantize`) + `image`
    (png/jpeg/webp/gif); `generate_from_path`/`from_seed` + 35-role map and
    JSON payload; 5 tests (role map, deterministic cover, grayscale fallback,
    transparent rejection, seed serialization).

- [x] **THM-2 · DB6 `colors.db` + hash invalidation + purge-on-sync** — P2 · (roadmap T1)
  - **Why:** generation is CPU work; the cache needs its own pool and must
    re-run when the cover bytes change.
  - **Details:** `migrations_colors` (`cover_color(cover_art PK, cover_hash,
    scheme, seed, dark JSON, generator, generated_at)` + `colors_meta`);
    `Dbs.colors` + `colors_path()`; read compares `covers.hash`; post-sync pass
    purges rows whose cover id vanished from covers.db.
  - **Now:** migrations_colors + `Dbs.colors`/`MUSIKA_COLORS_FILE`;
    `colors::fetch` (covers `cached_path` + `covers::hash` → cache hit or
    `spawn_blocking` generation), `fetch_into` test seam; purge hooked into
    `index.rs` after the cover pass; 3 cache tests (hit after source removal,
    hash-change regeneration, purge).

- [x] **THM-3 · `requestColors` play-scoped generation + `dataReady("colors")`** — P2 · (roadmap T2)
  - **Why:** only the playing cover should cost decode+quantize; a library-wide
    sweep is request amplification and must never happen.
  - **Details:** `Backend.requestColors()` mirrors `requestLyrics`; triggered on
    `currentCoverArt` change while playing (coalesced; one in-flight job per
    cover; stale-guard on cover art); payload
    `{cover_art, cover_hash, scheme, dark}`; static fallback on failure.
  - **Now:** `requestColors`/`colorsReady`/`refreshColors` in `qml.rs`; the
    cover_art/cover_url signals now fire only on change; payload carries
    `cover_art`, `cover_hash`, `scheme`, `seed`, `dark`; `dark: null` resets
    the palette (no art / failure); `Main.qml` receives `colors`, re-checks
    `cover_art === Backend.currentCoverArt` and applies.

- [x] **THM-4 · `Palette` singleton + QML role sweep + colour motion** — P2 · (roadmap T3)
  - **Why:** `Theme.js` is a static pragma-library, so dynamic roles need a
    notifying singleton; a cover swap should recolor, not repaint.
  - **Details:** `Palette.qml` (pragma Singleton) defaults from Theme.js,
    `apply(payload)` from the `colors` channel; migrate `Theme.<role>` to
    `Palette.<role>`; `micro`/`standard` colour behaviour; error/region/mode
    roles stay semantic, state layers stay fixed overlays.
  - **Now:** `palette.rs` is a Rust-backed qtbridge singleton (same mechanism
    as `Backend`) with 32 notifying roles, `apply`/`reset`, and MY3 defaults;
    227 `Theme.<role>` references swept to `Palette.<role>` across 19 QML
    files (adds `import musika` where missing, unifies the search scrim);
    shape/motion/type/z/state layers remain in Theme.js.

- [x] **THM-5 · Smart variant + contrast/lightness + manual refresh** — P3 · (roadmap T4)
  - **Why:** grayscale covers should not get a loud accent; power users want
    contrast control and a way to force regeneration.
  - **Details:** port `smart_scheme` colorfulness mapping (>70 vibrant, 20-70
    tonal spot, 6-20 neutral, <6 monochrome); settings for scheme/contrast;
    `:theme refresh` (or IPC) deletes the current row and re-requests.
  - **Now:** Hasler-Süsstrunk colorfulness picks the smart variant; knobs via
    `MUSIKA_COLOR_VARIANT` / `MUSIKA_COLOR_CONTRAST` (reflected in the cache
    generator tag, so a knob change invalidates); `colors::invalidate` +
    IPC `colors_refresh` (handled in Main.qml; `:theme refresh` waits on the
    command line, VI-13).

- [x] **ARC-19 · Async off the UI thread** — P0 · (A1)
  - **Now:** lists/covers load via Tokio + `dataReady`, no `block_on`.

- [x] **ARC-20 · Cover cache scoped to album/artist art** — P0 · (A2)

- [x] **ARC-21 · mpv cleanup on abnormal exit** — P0 · (A7)
  - **Now:** supervisor shell watches the parent PID.

- [x] **ARC-22 · Auth token not exposed in process argv/mirror** — P2 · (A9)
  - **Now:** mpv receives the stream URL over the IPC socket, not argv.

- [x] **ARC-23 · Palette race + corpus churn** — P2 · (O14/O15)
  - **Now:** FTS5 path replaced the tmpfs corpus entirely.

- [x] **ARC-35 · Sync button runs the complete fetch pass (index, covers, lyrics)** — P1 · **high priority**
  - **Why:** the Sync button is the user's "make everything current" verb, but
    lyrics were only ever fetched on demand, so a fully synced library could
    still have no lyrics; an up-to-date index also short-circuited most of the
    derived-cache work.
  - **Details:** `index::sync_library` must run the same passes in both
    branches (changed / up-to-date): index (`getIndexes`/`getArtist`/
    `getAlbum`), covers (`getCoverArt`), a bounded lyric backfill
    (`getLyricsBySongId` → legacy `getLyrics`), then the lyric/color purges;
    colors stay play-scoped and are never swept. The duplicate manual/login
    pipelines (ARC-8) must call this same core; cancellable sync (ARC-3)
    wraps it.
  - **Now:** landed — `lyrics::backfill` (cap `SYNC_BACKFILL_LIMIT = 64`,
    concurrency 8, misses re-checked after the 7-day TTL) runs from both
    branches via `index::refresh_lyrics`; `SyncReport.lyrics` shows the count
    in the finish message and the startup log. Verified live: an up-to-date
    startup sync fetched **53 lyrics**. Open: ARC-8 pipeline merge, ARC-3
    cancellation/timeouts, per-item progress for the backfill.


- [x] **AUTH-1 · Startup loading screen for cached login** — P2
  - **Now (working tree):** `loginPending`/`loginRequired` signals; 5 s ping
    timeout; `LoadingScreen.qml` countdown + progress.

- [x] **DOC-2 · Retire merged lists** — P3
  - **Why:** `TASKS.md` and `MICRODESIGNS.md` duplicated this file.
  - **Now:** both deleted; `CRITIQUE.md`, `roadmap.md` and `materialyou3idea.md`
    are kept as rationale/design docs.

- [x] **DOC-3 · GPLv2 licensing** — P2
  - **Why:** the project shipped without a license; contributions and
    redistribution needed explicit terms.
  - **Details:** canonical GNU GPL v2 text in `LICENSE`; SPDX
    `GPL-2.0-or-later` in `Cargo.toml`; FSF per-file notice (copyright +
    warranty) in every source file.
  - **Now:** landed — 339-line canonical `LICENSE`, manifest licence, and the
    notice in all 41 Rust/QML sources.

---


## Coverage

| Bucket | Open | Solved |
|--------|------|--------|
| Must have | 8 | 1 |
| Should have | 62 | 9 |
| Quick wins | 10 | 46 |
| Solved (reference) | — | 36 |
| **Total** | **79** | **92** |

### Coverage by area

| Area | Open | Solved |
|------|------|--------|
| Vi modal engine | 11 | 10 |
| Queue & playback grammar | 3 | 7 |
| Player & media features | 5 | 2 |
| Lyrics pipeline | 2 | 7 |
| Search | 2 | 6 |
| UI microdesigns | 18 | 30 |
| Dynamic color | 1 | 5 |
| Architecture & robustness | 23 | 19 |
| Media cache / offline | 7 | 0 |
| Auth / startup | 0 | 2 |
| Docs & process | 0 | 2 |
| Packaging | 0 | 1 |
