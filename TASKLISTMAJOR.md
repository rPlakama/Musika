# Musika — TaskListMajor

Master task list merging `CRITIQUE.md`, `TASKS.md`, `MICRODESIGNS.md`,
`roadmap.md` and `materialyou3idea.md`, deduplicated and audited against the
current working tree (commit `43e0f72` + uncommitted MY3/search/login-loading
work).

Legend: `[x]` solved · `[ ]` open. Priorities: **P0** correctness/robustness ·
**P1** vi fidelity · **P2** player/UX features · **P3** nice-to-have.
Each task states **Why** (rationale, source doc) and **Details** (what the
work is); solved/partial tasks add **Now** with the current evidence.

---

## 1. Vi modal engine (`VI`)

- [x] **VI-1 · Mode indicator in the status bar** — P1 · (N4 partly, MY3 QW10)
  - **Why:** Vi users must always know the mode; silent mode changes break trust.
  - **Details:** colored `NORMAL`/`INSERT`/`VISUAL`/`COMMAND` chip in the bar.
  - **Now:** `NORMAL` chip replaced the "LIBRARY" title (RightBar.qml); the
    search overlay shows an `INSERT`/`NORMAL` chip too.
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
- [x] **VI-5 · Single `Esc` dismisses overlays** — P1 · (N3, CRITIQUE Ph1.3)
  - **Why:** Esc is the invariant cancel key; unbound Esc feels broken.
  - **Details:** close fullscreen → pop album page → close search, in order.
  - **Now:** `escape` maps to `Action::Back` (keybind.rs); Home closes fullscreen
    first (Home.qml); search overlay Esc steps typing → normal → close.
- [ ] **VI-6 · `Ctrl-[` alias + pending-sequence cancel** — P1 · (N3 residual)
  - **Why:** `Ctrl-[` is muscle memory for Esc; Esc should also clear a half-typed
    sequence/count.
  - **Details:** add `ctrl-[` token; feed Esc through `Keymap.reset()`.
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
- [ ] **VI-10 · Drop `Ctrl-N`/`Ctrl-P` duplication** — P1 · (N7)
  - **Why:** `j/k` already scroll; duplicate bindings muddy the keymap.
- [ ] **VI-11 · Wire `gg`/`G`** — P1 · (N8a)
  - **Why:** bound in the keymap but no handler, so they dead-end silently.
  - **Details:** Top/Bottom in every list + queue.
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
- [ ] **VI-16 · Key overload disambiguation (`Enter`, `q`)** — P2 · (N9)
  - **Why:** `Enter` means play/open/confirm depending on context; `q` and Esc
    both Back; document or split.
- [ ] **VI-17 · Status bar focus behaviour** — P2 · (CRITIQUE ghost focus)
  - **Why:** focusing a non-interactive pane used to make keys inert.
  - **Details:** either strip region 2 from the focus ring or keep it and give it
    real actions.
  - **Now:** `Enter` triggers sync (RightBar.handleAction); region remains in the
    ring.
- [ ] **VI-18 · Space leaks through the modal search** — P2 · (MD32)
  - **Why:** the overlay is documented as modal, but `play_pause` is dispatched
    before Home sees the overlay (Main.qml:24-29).
  - **Details:** route `play_pause` through Home/overlay first.
- [ ] **VI-19 · `i` is dead outside search** — P3 · (MD33)
  - **Why:** `InsertMode` is bound globally but only the overlay consumes it.
  - **Details:** scope the binding or consume it (mode indicator). 
- [ ] **VI-20 · keyTimer comment/mechanism drift** — P3 · (MD34)
  - **Why:** the comment claims bare Space is deferred; it resolves immediately —
    only `g g`/`shift-space …` are.
  - **Details:** fix the comment or make deferral per-binding (ties to VI-9).

## 2. Queue & playback grammar (`Q`)

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
- [ ] **Q-3 · Shuffle / repeat** — P2 · (M3)
  - **Why:** the `⤨` button exists but is inert; repeat-one/all is expected.
  - **Details:** wire the button + keys, show active state via state layer.
- [ ] **Q-4 · Volume + mute in UI and keys** — P2 · (M4, CRITIQUE Ph3.1)
  - **Why:** mpv has volume, the backend keeps it, but the bar is a static bar.
  - **Details:** draggable slider + percentage, `m` mute, `+/-`/`]v [v` steps,
    focus state layers; expose slot in `qml.rs`.
- [ ] **Q-5 · Gapless + stream-error retry/auto-advance** — P1 · (M8)
  - **Why:** end-of-track errors currently stop the session silently.
- [ ] **Q-6 · A-B repeat, playback rate, crossfade** — P3 · (M9)
- [ ] **Q-7 · MPRIS/D-Bus metadata** — P2 · (M7) — solved
  - **Now:** mpv MPRIS script + metadata via IPC (`mpris:artUrl`).
- [ ] **Q-8 · Native MPRIS service (`org.mpris.MediaPlayer2.musika`)** — P2 · (roadmap "MPRIS node" design)
  - **Why:** the mpv-mpris script is an extra install; a stock desktop should
    see the node, and media keys/panels/lock screens must work everywhere.
  - **Details:** `mpris.rs` bridge over zbus, no-op without a session bus,
    Metadata/Position/Can* mapping, rate-limited signals, `playerctl` acceptance
    test across GNOME/KDE/sway/XFCE; milestones M0–M5 in the roadmap. Keep the
    mpv script path (Q-7) as the opt-in fallback.

## 3. Player & media features (`P`)

- [ ] **P-1 · Scrobble back to Navidrome** — P2 · (M5, CRITIQUE Ph3.2)
  - **Why:** play counts / recently played never update server-side.
  - **Details:** `scrobble` at 50%/4 min and on completion; `api.rs:1318` ready.
- [ ] **P-2 · Starred/favorites view + toggle** — P2 · (CRITIQUE Ph3.2)
  - **Why:** no way to value tracks; `getStarred`/`star` endpoints unused.
  - **Details:** `s`/`*` toggle, favorites filter.
- [ ] **P-4 · Playlists** — P2 · (M6, roadmap)
  - **Why:** sidebar segment is a placeholder.
  - **Details:** index + view + play; create/edit later.
- [ ] **P-5 · Radio** — P2 · (M6, roadmap)
  - **Why:** sidebar segment is a placeholder; Subsonic radio endpoints exist.
- [ ] **P-6 · Resume-on-launch** — P2 · (roadmap P2)
  - **Why:** power users expect the queue/session to survive restarts.
- [ ] **P-7 · Fullscreen controls completeness** — P2 · (U6)
  - **Why:** transport + seek landed, but volume/mute are absent in fullscreen.
  - **Details:** reuse IconButton/state layers; keep play/pause centered.

## 4. Lyrics pipeline (`LYR`) — roadmap "Lyrics" design

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
- [ ] **LYR-5 · Prefetch, miss TTL, local `.lrc` override** — P2 · (roadmap L4)
  - **Why:** avoids per-open latency and request storms, and lets users supply
    their own synced lyrics.
  - **Details:** prefetch lyrics for the next N queue tracks (reuse
    `FETCH_CONCURRENCY=8`); cache misses as `source='none'` with TTL (~7 days)
    plus manual refresh; `~/.config/musika/lyrics/<song_id>.lrc` wins over the
    DB and is never purged.
  - **Now:** `.lrc`/`.txt` override and the 7-day miss TTL landed with LYR-3;
    prefetch-next-N and manual refresh are still open.
- [ ] **LYR-6 · Optional: karaoke v2, lyric FTS, `:lyrics`** — P3 · (roadmap L5)
  - **Why:** word-level timing and lyric search are stretch goals; server
    support for `enhanced` varies.
  - **Details:** `enhanced=true` `cueLine`/`agents`/translations; index lyric
    text in search.db FTS5; `:lyrics sync` backfill per album/artist.

## 5. Search (`S`)

- [x] **S-1 · Floating search (three columns, keyboard, states)** — P1 · (roadmap design)
  - **Now:** `SearchOverlay.qml` + `SearchColumn.qml`: Artists|Albums|Songs,
    debounce, skeleton, no-matches, Enter open/play, Esc modes, hover.
- [x] **S-2 · Search appearance pass** — P2 · (MD13/MD14/MD9/MD12)
  - **Now:** two-line title/subtitle cells, mode chip, mode-aware footer,
    "N albums" wording, clamped type, `quick` easing.
- [x] **S-3 · Query `search.db` (FTS5), retire `fd`/tmpfs** — P0 · (O1, CRITIQUE §3.4)
  - **Now:** palette and overlay query FTS5 through `Backend.requestSearch`.
- [ ] **S-4 · Decide/align search column order** — P3 · (MD10, roadmap §Floating search)
  - **Why:** roadmap says Songs|Albums|Artists; impl/docstring use
    Artists|Albums|Songs. Pick one and sync the docs.
- [ ] **S-5 · Hover steals the search cursor** — P3 · (MD15)
  - **Why:** passing the mouse over a column re-targets `activeColumn`; only
    click should change it.

## 6. UI microdesigns (`UI`)

- [ ] **UI-1 · Region frame colours** — P2 · (MD1)
  - **Why:** PLAYER uses `Theme.error` (red) and four accents read as identity,
    not state; unify or only show in visual mode (Home.qml).
- [ ] **UI-2 · Remove always-on search hint strip** — P2 · (MD2)
  - **Why:** it owns a full row forever and is noise after the first day.
- [x] **UI-3 · De-duplicate sync state** — P2 · (MD3)
  - **Now:** the button no longer relabels to "Syncing…" and uses the disabled
    content token; residual: the progress track is always visible when idle.
- [ ] **UI-4 · Counts live in three places** — P2 · (MD4)
  - **Why:** sidebar stats + per-list "N indexed" + content counts duplicate.
- [x] **UI-5 · Two stacked titles removed** — P2 · (MD5)
  - **Now:** "LIBRARY" title replaced by the mode chip.
- [ ] **UI-6 · Placeholder segments look active** — P3 · (MD6)
  - **Why:** Playlists/Radio look selectable but only open an empty state.
- [ ] **UI-7 · Rotated "LIBRARY" visual-mode label** — P3 · (MD7/U5)
  - **Why:** rotated text is unreadable at a glance.
- [ ] **UI-8 · Rows sized as viewport fractions** — P1 · (MD16)
  - **Why:** row height grows with the window; use a fixed unit per view.
- [ ] **UI-9 · Extract a shared `ListRow`** — P2 · (MD17)
  - **Why:** three near-identical delegates drift (zebra/cursor/cover/text).
- [ ] **UI-10 · Hover parity + initial cursor** — P2 · (MD18)
  - **Why:** AlbumList tints the cover on hover, others do nothing; no row is
    highlighted until the first key.
- [x] **UI-11 · `FadeText` opt-out** — P2 · (MD19)
  - **Now:** `animated: false` on list/search/album/toast delegates.
- [ ] **UI-12 · Metadata line position consistency** — P3 · (MD20)
  - **Why:** "N tracks" is right-aligned; "N albums" sits under the name.
- [ ] **UI-13 · Scroll position hint** — P3 · (MD21)
  - **Why:** long lists have no scrollbar/position feedback for mouse users.
- [ ] **UI-14 · Expandable album notes** — P2 · (MD22)
  - **Why:** notes clamp to 3 lines with no way to read the rest.
- [ ] **UI-15 · Album "‹ Back" chrome** — P3 · (MD23)
  - **Why:** duplicates `q`/Esc; keep only if the toolbar gains more.
- [ ] **UI-16 · "COVER" fallback text** — P3 · (MD24)
  - **Why:** placeholder word in the art slot; use a glyph/blank surface.
- [ ] **UI-17 · Inert controls: shuffle/stop/volume** — P1 · (MD25)
  - **Why:** a control that does nothing breaks trust; the volume bar is a
    static 60% fill and the speaker ignores the player volume.
  - **Details:** wire (Q-3/Q-4) or remove until implemented.
- [ ] **UI-18 · Icon language consistency** — P2 · (MD26/MD35)
  - **Why:** emoji speaker breaks the monochrome look; glyphs vary in weight.
- [ ] **UI-19 · Type-role sweep + clamps everywhere** — P2 · (U10, MY3 step 7)
  - **Why:** inline `font.pixelSize` fractions drift per view.
  - **Now:** clamps/roles applied in RightBar, NowPlaying, Toast; lists, sidebar
    and album pages still inline.
- [ ] **UI-20 · Focus states on every control** — P1 · (U2)
  - **Why:** keyboard focus must be visible; only IconButton has a focus
    hairline; sync button/segments/rows don't.
- [ ] **UI-21 · Mouse/keyboard parity gaps** — P2 · (U11)
  - **Why:** some affordances are mouse-only and vice versa.
- [ ] **UI-22 · Empty/error states polish** — P0 · (U8)
  - **Why:** album-with-no-songs, failed sync, failed playback need real states.
  - **Now:** error toasts landed; loading screen landed for startup ping;
    empty states still mostly PlaceholderList.
- [ ] **UI-23 · Toast retry covers playback errors too** — P2 · (MD38)
  - **Why:** the Retry button only renders for `kind === "sync"`.
- [x] **UI-24 · Fullscreen 50/50 split + px sweep** — P2 · (MD27/MD29)
  - **Now:** cover/lyrics 50/50; NowPlaying px → fractions/theme radius.
- [x] **UI-25 · MY3 motion & state system** — P1 · (MY3 steps 1–4, 8; QW1–8, 10–12)
  - **Now:** motion tiers/easings, shape/state/z tokens, scrim, container roles,
    `EnterView`/`ExitView`, blur retired, IconButton state layers, toast
    quick-in/lazy-out, mode chip, linear progress, search weights documented.
- [ ] **UI-26 · Centralize `formatTime`** — P3 · (MD30)
  - **Why:** duplicated in six files (InferiorBar, NowPlaying, SongList,
    AlbumEntry, PlaybackNode, SearchOverlay).
- [ ] **UI-27 · Separators / font ratios** — P3 · (MD36/MD37)
  - **Why:** "  ·  ", "   " and "·" coexist; title fractions 0.24/0.26/0.26.

## 7. Architecture & robustness (`ARC`)

- [ ] **ARC-1 · Cross-DB coherence repair path** — P1 · (A3)
  - **Why:** library/search/covers writes can diverge on interruption.
- [ ] **ARC-2 · Cover revalidation cheaper/cancellable** — P1 · (A4, O6)
  - **Why:** an unchanged sync still revalidates and `UPDATE`s every cover.
- [ ] **ARC-3 · Cancellable sync + HTTP retry/backoff/timeouts** — P1 · (A5, O5)
  - **Why:** a hung request blocks the sync pipeline; `reqwest` has no timeout.
- [ ] **ARC-4 · ID-namespace correctness** — P1 · (A6, O4)
  - **Why:** `getIndexes` returns file-structure IDs passed to ID3 endpoints;
    handle `getMusicDirectory`/`indexes.child`.
- [ ] **ARC-5 · Surface errors instead of `eprintln!`** — P0 · (A8)
  - **Why:** users never see console output.
  - **Now:** toast channel exists for sync/playback; many call sites still log.
- [ ] **ARC-6 · Docs drift** — P2 · (A10)
  - **Why:** roadmap still says "Space+F", "right bar", stale durations.
- [ ] **ARC-7 · Trim `api.rs` to used endpoints** — P0 · (O2)
  - **Why:** ~1,600 lines behind `#![allow(dead_code)]`.
- [ ] **ARC-8 · Merge duplicate sync/login pipelines** — P1 · (O3)
  - **Why:** `qml.rs::sync_library` vs `spawn_index_sync`; login/startup copies.
- [ ] **ARC-9 · Cache `SecretKey`, honor `XDG_CONFIG_HOME`** — P1 · (O7)
  - **Why:** re-reads the key file per call; path bypasses `config_dir`.
- [ ] **ARC-10 · `sync_player` signal thrift** — P1 · (O8)
  - **Why:** ~13 signals + string clones every 500 ms tick.
- [ ] **ARC-11 · Collapse structural duplication** — P1 · (O9)
  - **Why:** fetch helpers, JSON loaders and song projections repeat.
- [ ] **ARC-12 · Poison-safe player lock** — P1 · (O10)
  - **Why:** `lock().unwrap()` on the reader thread can panic the UI tick.
- [ ] **ARC-13 · Dead code cleanup** — P2 · (O11, MD31)
  - **Why:** `PlaybackNode.qml` is uninstantiated, `covers::cached_path` unused
    until the media cache lands, `index.txt` mirror intentional?
- [ ] **ARC-14 · Deduplicate constants** — P2 · (O12)
  - **Why:** volume 70.0, `COVER_SIZE`, `FETCH_CONCURRENCY` exist twice.
- [ ] **ARC-15 · Stop masking errors as auth failures** — P2 · (O13)
  - **Why:** every envelope error becomes "Wrong username or password".
- [ ] **ARC-16 · mpv supervisor hardening** — P2 · (O16)
  - **Why:** `sh -c` string building for MPRIS/kill; avoid spawning shells.
- [ ] **ARC-17 · Slim the `Backend` god-object** — P2 · (O17)
  - **Why:** huge slot surface; `playback_state` is a `String`, should be enum.
- [ ] **ARC-18 · Remove empty no-op migrations** — P3 · (O18)
- [x] **ARC-19 · Async off the UI thread** — P0 · (A1)
  - **Now:** lists/covers load via Tokio + `dataReady`, no `block_on`.
- [x] **ARC-20 · Cover cache scoped to album/artist art** — P0 · (A2)
- [x] **ARC-21 · mpv cleanup on abnormal exit** — P0 · (A7)
  - **Now:** supervisor shell watches the parent PID.
- [x] **ARC-22 · Auth token not exposed in process argv/mirror** — P2 · (A9)
  - **Now:** mpv receives the stream URL over the IPC socket, not argv.
- [x] **ARC-23 · Palette race + corpus churn** — P2 · (O14/O15)
  - **Now:** FTS5 path replaced the tmpfs corpus entirely.

## 8. Media cache / offline (`CACHE`) — roadmap design

- [ ] **CACHE-1 · DB4 `media.db` + `media/` blobs** — P2 · (roadmap Media cache)
  - **Why:** audio is never stored; playback dies without network.
  - **Details:** row per song (`kind`, `rule_id`, validators, `last_used`); tmp +
    atomic rename; purge only after a successful sync (covers.rs pattern).
- [ ] **CACHE-2 · Playback cache + prefetch next N** — P2
  - **Why:** stutter-free playback; Symfonium-style rolling buffer.
  - **Details:** player owns the queue → prefetch next tracks; purgeable freely.
- [ ] **CACHE-3 · Rolling cache (LRU, max size)** — P2
- [ ] **CACHE-4 · Permanent cache + "Move to permanent" pin** — P2
- [ ] **CACHE-5 · Auto-download rules** — P2
  - **Why:** Symfonium parity: favorites, playlists, genres, and artists flagged
    "in download cache" auto-download existing + future content; rule removal
    prunes unless pinned.
- [ ] **CACHE-6 · Quality & network policy** — P2
  - **Details:** transcode bitrate on download, prefer-server online/Wi-Fi,
    Wi-Fi-only downloads, max simultaneous downloads.
- [ ] **CACHE-7 · Cache status UI** — P2
  - **Details:** per-item state (cached/partial/queued) + used-bytes vs cap in
    the status bar; offline filter.

## 9. Auth / startup (`AUTH`) — current WIP

- [x] **AUTH-1 · Startup loading screen for cached login** — P2
  - **Now (working tree):** `loginPending`/`loginRequired` signals; 5 s ping
    timeout; `LoadingScreen.qml` countdown + progress.
- [ ] **AUTH-2 · Loading screen polish** — P3
  - **Why:** countdown prints raw ms ("timeout 4800ms"), progress steps every
    100 ms without easing; no cancel.
  - **Details:** seconds or spinner, `linear` smoothing, Esc/click to login.
- [ ] **AUTH-3 · Login error/status roles** — P2
  - **Why:** login surface predates the MY3 roles (no container/error tokens,
    clamped type); align with Theme.

## 10. Docs & process (`DOC`)

- [ ] **DOC-1 · Keep design docs in sync with the tree** — P2 · (A10)
  - **Why:** CRITIQUE/roadmap/materialyou3 describe pre-implementation states.
- [x] **DOC-2 · Retire merged lists** — P3
  - **Why:** `TASKS.md` and `MICRODESIGNS.md` duplicated this file.
  - **Now:** both deleted; `CRITIQUE.md`, `roadmap.md` and `materialyou3idea.md`
    are kept as rationale/design docs.

---

## Snapshot

| Area | Open | Solved |
|------|------|--------|
| Vi modal engine | 18 | 2 |
| Queue & grammar | 7 | 1 |
| Player features | 6 | 0 |
| Lyrics pipeline | 2 | 4 |
| Search | 2 | 3 |
| UI microdesigns | 22 | 5 |
| Architecture | 18 | 5 |
| Media cache | 7 | 0 |
| Auth/startup | 2 | 1 |
| Docs | 2 | 0 |

Biggest wins to schedule next: **VI-3** (Ctrl-W panes) + **VI-2** (mode machine)
unlock most of the vi story; **UI-17/Q-4** (inert controls) and **ARC-5** remove
the remaining trust breaks; **CACHE-1/2** make the player offline-capable.
