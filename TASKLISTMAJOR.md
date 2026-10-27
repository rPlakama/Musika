# Musika — TaskListMajor

Master task list merging `CRITIQUE.md`, `TASKS.md`, `MICRODESIGNS.md`,
`roadmap.md`, `materialyou3idea.md` and `RECYCLECRITIQUE.md` (REC batch),
re-audited against the current working tree (0.4.7, commit `528a329`, clean)
and organised by delivery bucket.

Legend: `[x]` solved · `[ ]` open. Priorities: **P0** correctness/robustness ·
**P1** vi fidelity · **P2** player/UX features · **P3** nice-to-have.
Each task keeps its area id (`VI`, `Q`, `P`, `LYR`, `S`, `UI`, `THM`, `ARC`,
`ARC-UI`, `CACHE`, `AUTH`, `DOC`, `REC`), its **Why** and **Details**; solved
and partial tasks add **Now** with the current evidence. Solved entries live in
the reference section at the bottom.

### Recent landed (0.4.4–0.4.7, 2026-10-26)

Kept here so the open sections below do not claim work that already landed:

- **0.4.4** — contained page covers, vignette/blur experiments, album rows
  tied to the displayed cover, and 36 px/132 px row/cell floors.
- **0.4.5** — zero production unwraps/expects/panics (`6626ce1`,
  `d1d8341`), qml bridge extraction (`a96c551`, `82d3df8`, `24acf53`; now
  2,815 lines), test-module benchmarks/Vulkan probe (`c40ffb0`, `9f7b995`),
  debug startup/FPS/idle logs (`1eb1f15`, `1962abf`, `092f7ee`), numeric
  Ctrl-N/P skips (`ffa4874`), repeat modes (`aed6c1a`, `f7662c4`), action
  badges (`9b84550`), and the `qq` Bye overlay (`42fa90e`, `4db7c34`,
  `ac54f0a`).
- **0.4.6** — cover vignette/wash removal, blurred cover + shadow backdrop,
  restored smooth cover filtering, album tracks Tab-selectable, scroller
  bottom anchoring, transport core centred around the repeat accessory
  (`d3f4b46`), and shared thumbnail decode sizes (`b713298`–`d238008`).
- **0.4.7** — row text fit/wrap and vertical centering, selected rows above
  neighbours, short album lists bottom-aligned, and lyrics CurveRendering at
  device-pixel layer resolution (`06cef85`–`37294d3`).

Still open from this batch: **Q-3** shuffle half, **ARC-17/ARC-36** remaining
bridge/status extraction, **UI-63** Vulkan/RHI profile pass, a shared GPU
cover-texture cache beyond the decoded-pixmap sharing in `d238008`, and the
clippy deny gate listed under ARC-34.

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

2. **S-8** — Songs column follows a single album match (top priority:
   album-scoped search reads in track order).
3. **ARC-25** — partial-fetch guard before the destructive purge (data loss).
6. **UI-17 + Q-3** — shuffle is the last inert control; wire it + repeat.
7. **UI-22** — empty/error states (P0, half-landed).
8. **VI-2** — mode machine; unblocks VI-3/4/7/12/13/14/15.
9. **S-4 + Q-10** — search scope order/priority (Songs first) and song
   activation opening fullscreen.
10. **LYR-8** — smooth centre-follow + edge fades.
11. **ARC-24 + ARC-33** — `catalog_epoch` + `schema_generation` in every meta.
12. **ARC-29 + ARC-30** — content-addressed covers + hash-keyed colors.
13. **VI-4** — visual selection (rows + count badge).

## Must have

The app is unsafe or misleading without these.

### FIX-UPS — Rust audit (0.3.16)

- [x] **FIX-1 · Reap mpv when the IPC connect fails** — P0 · (process/thread leak)
  - **Why:** `Player::ensure_ready` stores the spawned `Child` before
    `connect_retry`. A failed connect returns `false` with the child still
    live; the next retry spawns another mpv and overwrites the old `Child`
    without TERM/wait, orphaning the process and its reader thread.
  - **Details:** store the child only after the socket connects; on failure
    TERM/wait it and clear `self.child`. Also clear a stale `self.socket`.
  - **Verify:** force a connect timeout (dead `mpv.sock`) and confirm no
    `mpv` process survives after the failed retry.
  - **Now:** `ensure_ready` is a `Ready` state machine (Connected /
    Connecting / Failed) and owns a `reap_child` (TERM + wait + stale-socket
    removal + `connect_deadline` clear) shared with `Drop`. The child is
    stored at spawn but is never orphaned — the connect deadline reaps it
    exactly once, and a later start re-spawns clean (covered by the
    `reap_child_takes_the_child_and_the_down` unit test).

- [x] **FIX-2 · Dedupe on-demand cover fetches and thumb jobs** — P1 · (task storm)
  - **Why:** every `covers::url_map_of` miss spawns a detached `fetch_ids`
    task, and every `cached_urls` call spawns detached thumbnail jobs. A
    page reload/scroll while the cover is still missing starts the same work
    again.
  - **Details:** keep an in-flight `HashSet<String>` per work kind (cover id
    for fetches, thumb file name for thumbs); skip ids already running and
    remove them when the task ends. Do not hold the set across await points.
  - **Verify:** request the same missing cover / thumb repeatedly; only one
    fetch/thumb job runs per id.
  - **Now:** process-global `LazyLock` claim registries (`ONGOING_FETCHES`
    keyed by cover id, `ONGOING_THUMBS` keyed by thumb file name) with
    short-lock claim/release (poison-safe, never held across an await).
    `url_map_of` claims ids before spawning the fetch task and releases
    when it ends (a failure is retryable by the next request);
    `cached_urls` filters its detached thumb jobs through the thumb
    registry, and `ensure_thumbs` returns the names it processed for the
    release. The sync pass (`covers::refresh`) is untouched. Pinned by
    `a_claimed_id_blocks_until_released`.

- [x] **FIX-3 · Do not leak the previous `Dbs` on wipe** — P2 · (memory/FD leak)
  - **Why:** `main::set_dbs` `Box::leak`s every new database set and drops
    only the pointer to the old one; `db::wipe_files` closes `library` and
    `media` but not the separate `search` pool. Repeated full wipes
    accumulate closed pools and stale connections.
  - **Details:** replace the leak with an owned swappable slot (e.g.
    `OnceLock<RwLock<Dbs>>` or `ArcSwap`), or explicitly close `search` and
    drop the old set when swapping.
  - **Verify:** wipe repeatedly; process FD/RSS count stays flat.
  - **Now:** `DBS` is a `OnceLock<RwLock<Arc<db::Dbs>>>`; `dbs()` returns an
    owned `Arc<db::Dbs>` and `pool()` an owned `SqlitePool` clone — no
    `Box::leak` anywhere. `set_dbs` swaps the slot and hands the previous
    set to `drop_dbs`, which closes the three distinct pools (`library`,
    `search`, `media`) on the runtime afterwards — the search pool wipe
    never closed is closed here. The 21 `crate::dbs().cloned()` call sites
    became `crate::dbs()` (the `spawn_task`/`spawn_data` bounds now take
    `FnOnce(Arc<db::Dbs>)`).

- [x] **FIX-4 · Generate/cancel library page loads** — P2 · (stale pumping)
  - **Why:** `qml::spawn_data` starts a task per albums/artists/songs/
    playlists request; old loads keep running and can emit after a newer
    order/filter/page request.
  - **Details:** mirror the `SEARCH_GEN` pattern: one generation per channel,
    increment on each request, drop the payload when the generation moved.
    Optionally coalesce identical in-flight requests.
  - **Verify:** hammer order keys/pages while a slow load is in flight; only
    the latest payload lands and old tasks are short-lived.
  - **Now:** `DATA_GEN` — one `u64` generation per channel in a
    poison-safe `LazyLock<Mutex<HashMap<&'static str, u64>>>`, bumped on
    every `spawn_data` request; the spawned task re-checks before starting
    its load and before emitting, so a superseded load is short-lived and
    never lands. Emit stays on the channel; coalescing intentionally not
    built (identical requests are rare and each misses are cheap).

- [x] **FIX-5 · Coalesce lyric requests** — P2 · (duplicate network work)
  - **Why:** `requestLyrics` starts a fetch plus three prefetches every
    call; there is no in-flight key, unlike `requestColors`.
  - **Details:** keep `lyrics_in_flight` (current song id) and skip repeated
    requests while it matches; clear it on delivery. Prefetch ids can use the
    same set.
  - **Verify:** repeated `requestLyrics` for one song starts one fetch.
  - **Now:** `lyrics_in_flight: Option<String>` mirrors the colors marker —
    `request_lyrics` skips while it matches, clearing happens on delivery
    (the fetch task invokes the new `lyricsReady` slot on the invoker after
    the emit), and `refresh_lyrics` drops the marker so a manual pass can
    re-request. The prefetches ride the same single task, so one request per
    beat owns them too.

- [x] **FIX-6 · Serialize the first search-index build** — P2 · (duplicate CPU)
  - **Why:** `search::index::ensure` allows concurrent first builds; each
    one reads the library and builds a full Nucleo catalog in
    `spawn_blocking`.
  - **Details:** add a build mutex/once-in-flight; the second caller waits
    for the slot instead of starting another build (the epoch guard already
    handles wipe races).
  - **Verify:** two simultaneous first searches produce one catalog build.
  - **Now:** a process-global `BUILD` `tokio::sync::Mutex` serialises `ensure`
    and `rebuild`; `ensure` double-checks the slot after acquiring the lock
    (the waiter may already have paid for the build), and both route through
    `rebuild_locked` — the last-swap-wins pathology is gone, the wipe-epoch
    guard is unchanged.

- [x] **FIX-7 · Guard overlapping sync cover passes** — P2 · (task overlap)
  - **Why:** the sync finishes before its detached cover task; a new sync
    can start a second cover pass while the first is still touching/writing
    rows.
  - **Details:** one in-flight cover-pass flag; a new pass either waits,
    merges its forced ids into the running pass, or skips (next TTL/sync
    covers it).
  - **Verify:** trigger two syncs back-to-back; only one cover pass runs.
  - **Now:** a process-global `COVER_PASS` `tokio::sync::Mutex` taken at the
    head of `index::cache_covers` (`cache_covers_locked` holds the body) —
    an overlapping pass waits instead of revalidating/rewriting rows
    concurrently; a skipping/merging policy was not needed since a queued
    pass is correct and self-expiring.

- [x] **FIX-8 · Cap/coalesce MPRIS commands** — P3 · (unbounded channel)
  - **Why:** `mpris::COMMAND_TX` is an unbounded `mpsc`; the 500 ms tick
    drains it, so a D-Bus burst between ticks grows memory without a cap.
  - **Details:** use a bounded channel (drop/coalesce when full) or a small
    pending-command set keyed by action.
  - **Verify:** flood MPRIS controls faster than the tick; queue length stays
    bounded.
  - **Now:** the channel is `mpsc::channel(COMMAND_CAP)` (64) and `submit`
    uses `try_send` through a small `queue_command` helper — a full queue
    drops the excess instead of growing or blocking the bus thread. Pinned
    by `a_full_command_queue_drops_instead_of_blocking`.

- [x] **FIX-9 · Serve IPC clients without blocking accept** — P3 · (liveness)
  - **Why:** `ipc::serve` calls `read_lines` on its accept thread; one client
    that keeps the socket open blocks every later client.
  - **Details:** hand each stream to `spawn_line_reader` (a short-lived
    thread/task) and keep the accept loop free, or use a single non-blocking
    reader with per-connection state.
  - **Verify:** open a silent IPC client, then send a real command from a
    second client; it still runs.
  - **Now:** the accept loop only accepts; each stream gets a
    `spawn_line_reader` thread that funnels lines into an `mpsc` channel,
    drained by one dispatcher thread that owns the handler (so the handler
    stays single-threaded and `Send`-only — `QmlMethodInvoker` is not
    `Sync`). Pinned by
    `a_silent_client_does_not_block_later_commands`.

- [x] **FIX-10 · Remove temp files after a failed atomic rename** — P3 · (temp litter)
  - **Why:** `cache::write_atomic` writes `*.tmp` then renames; if rename
    fails the temp file is left behind.
  - **Details:** on rename error remove the temp path before returning the
    error.
  - **Verify:** simulate a failed rename; no `*.tmp` remains.
  - **Now:** the rename error path removes the temp file before returning
    (context preserved). Pinned by `a_failed_replace_removes_the_temp_file`
    (rename onto a directory forces the failure).

- [x] **FIX-11 · Move `connect_retry` off the Qt thread** — P1 · (UI pumping)
  - **Why:** starting mpv sleeps 50 ms at a time for up to 3 s on the Qt
    thread (`player::connect_retry`), freezing the tick and input.
  - **Details:** poll the socket from the runtime (or a short-lived worker)
    and deliver readiness back to the player pump; or use an async connect.
  - **Verify:** start playback with a slow mpv; the UI stays responsive.
  - **Now:** `connect_retry` is gone. `ensure_ready` spawns on the first
    call and does exactly one per-call connect attempt — a Unix socket
    connect on a missing socket file returns immediately (ENOENT), nothing
    sleeps. `play_current` defers the rebuild (`deferred_start`, state
    Playing with a zeroed `Shared`), and `pump` resolves it on the next
    tick; `stop` during the deferral aborts it (and reuses the spawned
    child for the next start) — covered by
    `stop_during_a_deferred_start_aborts_it`. `finish_deferred_start`
    replays the intent issued while connecting (a `pause` from
    `load_paused` and the restored playhead `seek`), so a restored session
    still opens paused instead of autoplaying — pinned by
    `a_deferred_paused_start_stays_paused` and
    `a_deferred_start_keeps_the_pending_playhead`.

- [x] **FIX-12 · Remove the double `request_id` increment** — P3 · (cleanup)
  - **Why:** `Player::command` increments `request_id` twice per command;
    harmless but misleading for the mpv request/response mapping.
  - **Details:** keep one increment.
  - **Verify:** request ids increase by one per command in the player tests.
  - **Now:** the audit found a single increment (`player.rs` `command`);
    nothing to change. Pinned by `request_ids_increase_by_one_per_command`.

- [x] **#bug 2 · Order (type) swaps flicker the top rows** — P0 · (reported)
  - **Why:** changing the list order/type made the first rows (the top
    three) visibly flicker.
  - **Details:** traced to the instant-land settle added with the `G` bottom
    fix (`ScrollContent.land` schedules `_settleGlide` after releasing
    `_instantFollow`): the settle is a Qt `Contain`, and during an order
    swap's rebuild the count climbs row by row — every intermediate end is
    momentarily "the last row", so the settle ran mid-refill and nudged the
    viewport (Qt's `Contain` parks async offsets on a scrolled list — probed
    `-4`, `-96`, `-920`).
  - **Now:** rebuild churn lands are quiet (`_land(index, false)` — the
    `onCountChanged` branches), only host landings settle, and only for the
    true last row (`index === count - 1`; the only one that can be short
    under an estimated `contentHeight`). The cursor-position reset also
    rides the shared reload now (`MediaList.resetRowReload()` =
    `resetView` + `reload` + `TriggerRowReload`, used by the three
    `order_changed` handlers). The `G` case stays covered by
    `tst_bottomup`.

- [x] **#bug 3 · Favouriting in the queue steals the cursor** — P1 · (reported)
  - **Why:** favouriting a row in the fullscreen queue made the cursor jump
    to the currently playing track instead of staying on the highlighted
    row ("the favorite action is triggering UI-Reload Function, instead
    per-row reload").
  - **Details:** the verb's success path fired the full `TriggerRowReload`
    (`altFavorite` → `trigger_row_reload`): a queue-payload re-emit that
    `applyQueue` applies by re-following the playing index
    (`recallQueueCursor`) — and the `starred` signature change rebuilt both
    queue models besides. A heart write must never move the cursor.
  - **Now:** favourites land as the per-row signal `favoriteChanged(kind,
    id, on)`; `NowPlaying` patches every matching queue row in place
    (duplicates included) and the initiating surface keeps its optimistic
    patch. No payload, no rebuild, no cursor move; `album_state` still
    reconciles the keybar heart and the download-mark windows are no longer
    touched by a favourite.

- [x] **#bug 4 · Order-type swaps desync the cursor tracking near the top** — P1 · (reported)
  - **Why:** cycling the order (Shift-H/L) on the Albums/Songs tabs left the
    viewport and the cursor unsynchronised — flicker in the scrolloff range
    (the first rows, where the follow cannot centre the cursor), repaired
    only by forcing a fresh position (`G`). "Changing the order type should
    force a true UI reload of the ScrollModal".
  - **Details:** the swap raised `resetPending` but left the cursor live
    through the payload rebuild (`clear()` + N appends): every intermediate
    `onCountChanged` re-landed the cursor with `positionViewAtIndex(Contain)`
    against a half-filled model, and at the scrolloff edge the follow's
    centre target is clamped (`target == contentY` early-returns), so
    nothing repaired the realised window afterwards — the `G` settle did.
  - **Now:** `ScrollContent.resetView()` is a true pre-payload reset: the
    glide stops, `_pendingFollow` clears, the cursor parks at -1 (the churn
    runs as a fresh list, no per-append landings), the active view tops
    itself, and the memory zeroes; `resetViewNow()` still finishes the
    landing on the payload. Every order/filter swap (`resetRowReload`, the
    artist filter) rides it.

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

- [x] **UI-22 · Empty/error states polish** — P0 · (U8)
  - **Why:** album-with-no-songs, failed sync, failed playback need real states.
  - **Now:** error toasts landed; loading screen landed for startup ping.
    Loading skeletons landed as the shared `SkeletonList` (extracted from the
    search columns) and wired into `SearchColumn`, `MediaList` (Albums/Songs/
    Artists via a `loading` flag the wrappers toggle) and the album page's
    songs; the album page also gets a real "No songs in this album" state.
    The listed opens all landed since: sync failure surfaces as the status
    message + `Sync failed: …` toast, playback failure rides the Q-5 toast
    with the manual retry, Login is Enter-submit with busy fields
    ("Logging in…"), and the shared placeholder covers the unindexed
    segments.

- [ ] **ARC-3 · Cancellable sync + HTTP retry/backoff/timeouts** — P1 · (A5, O5)

  - **Why:** a hung request blocks the sync pipeline; `reqwest` has no timeout.

- [x] **ARC-memory** Leaving a Queque to start a new one, like initiating a new album, or whatever triggers a new queuque, should unload the memory of previous prefetch;
  - **Now:** a payload whose row signature differs clears the `fullscreen` scroll memory and drops `_recallQueue` before the model rebuild, so the new queue starts at its playing track instead of restoring the previous queue's position.

- [x] **ARC-IDK** Breaking down Row mechanism into smaller segments for UI Reload calls; Titles, Symbols, Covers -- all they have their own UI call instead of the whole UI recall; Benefits is many, such as lower CPU compromisse in Row reload/reset, and information update. As a example, the Action of Favorite/Removal of Favorite/Download or Removal of Download have their own Rectangle update call, which is not the whole Row; Another segmnet, is the 'expect factor row' which defines a row by a Rectangle without specifically it contents inside, reducing the amount of CPU cicles to render predicted rows (such as in Queuques) or render distance of rows;
  - **Now:** the shared `RowBody.qml` (cover, title/subtitle, meta, `RowMarks`) is consumed by `MediaList` rows and the fullscreen queue; each part is a named property, so a per-row change updates that part without recalling the list. Rows outside the near viewport band keep the cheap shell and load their content lazily through `RowBody.expected` (`ScrollContent.isExpected`), with the queue's scroll extent precalculated.

  - [x] **Sharing is Care ARC-IDK** Upon a queuque entering (FullPlayBackMode) screen, it should-precalculate the scroll range from cursor to latest Item based in the amoount of rows, const of height -- object if for a more stable scrolling (skipping without flicking) experience;
    - **Now:** `ScrollContent.contentHeightHint` lets fixed-height hosts supply the true extent before Qt's lazy delegate estimate settles; the fullscreen queue feeds `count * adaptiveRowHeight`, so its clamp/glide range is precalculated.

- [x] **ARC-IDK1** SearchUI design for Shift+Space++F search -- Sleek and less transparent, breath more -- proper details in init;
  - **Now:** the acrylic tint is 40% less transparent than the old 0.76 (`Theme.acrylic.tintAlpha`, clamped to 1.0), and the search panel gained wider inner margins/spacing plus padded field text so the toolbar and columns breathe without changing the result layout.

- [ ] **ARC-IDK2 · Row Predict parent pre-load** — P3 · (ARC-IDK follow-up)
  - **Why:** the predicted-row array should warm incrementally around the cursor instead of waiting for a far `G`/`gg` landing to realise its target.
  - **Details:** keep a small forward/backward prediction window in the parent, extend it in fixed batches as the cursor approaches, and drop rows far behind; the same seam serves Album/Songs and the queue so an edge jump never shows the load shell.

- [ ] **ARC-4 · ID-namespace correctness** — P1 · (A6, O4)
  - **Why:** `getIndexes` returns file-structure IDs passed to ID3 endpoints;
    handle `getMusicDirectory`/`indexes.child`.

- [ ] **ARC-5 · Surface errors instead of `eprintln!`** — P0 · (A8)
  - **Why:** users never see console output.
  - **Now:** toast channel exists for sync/playback; many call sites still log.
  - **Impl** Use notifications In-app (Top-right-corner) for logs and other information;

- [x] **ARC-12 · Poison-safe player lock** — P1 · (O10)
  - **Why:** `lock().unwrap()` on the reader thread can panic the UI tick.
  - **Now:** closed by ARC-34 — `Player::shared()` and the mpv reader recover
    poisoned guards with `PoisonError::into_inner`; a reader panic cannot
    abort the UI tick.

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

- [x] **ARC-34 · `.unwrap()` audit — 8 production sites; recover or surface, never abort** — P1 · (ARC-12, ARC-5)
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
  - **Now:** landed in `6626ce1` + `d1d8341` — zero production `.unwrap()`,
    `.expect()` or `panic!` sites. Player/mpv guards recover with
    `PoisonError::into_inner`, runtime and DB init print and exit(1), crypto
    returns an error instead of an `expect`, MPRIS uses an infallible
    `OwnedValue` fallback, and MD5/cover/color hashes return `Result` and
    propagate. Tests keep their fixtures. The clippy deny gate remains a
    nice-to-have, not implemented.

## Should have

The product is incomplete without these.

- [x] **ARC-UI-6 · One DownloadButton for every download surface** — P1 · (reported)
  - **Why:** download visuals are duplicated per surface (ArtistEntry's
    chip, the row marks, the fullscreen keybar chip), each with its own
    state wiring; and in both the chip and the rows the loading glyph,
    when it stops, keeps a half-spin angle — "returns it to a rolled
    animation (a few degrees) instead of the proper orientation".
  - **Details:** one reusable `DownloadButton` factoring the states —
    optional idle (outline), `fetching` (loading-loop, spinning),
    `done` (filled glyph) — with the spin **settling to the nearest full
    turn** on stop (the sync button's pattern) so the glyph lands upright.
    Reused wherever a download function exists: `RowMarks`, ArtistEntry's
    download chip, the fullscreen keybar's download chip.
  - **Now:** landed — `DownloadButton.qml` is the one visual (idle outline /
    fetching loop / filled done, the spin settling to the nearest full turn
    so the glyph lands upright; `c7549cc` fixes the glyph paint); consumers:
    `RowMarks`, ArtistEntry's chip, the fullscreen keybar chip and the Alt
    overlay tiles.

- [x] **REC-UI-1 · One row mechanism, everywhere** — P1 · (reported)
  - **Why:** "Fix Scroll/Row mechanism incosistencies; All rows present
    should use the same modal, to be reused, recicled, and highly
    consumable."
  - **Details:** rows grew per-surface grammars — state marks over the cover
    in one layout, on the right rail in another; per-list reload / cursor
    reset / trigger paths. Extract **one row contract** (state marks +
    `markVisible`/mark window + payload reload + cursor-position reset +
    `TriggerRowReload`) that every list consumes (MediaList, ArtistEntry,
    AlbumEntry, SearchColumn, the queue); grid cells keep the cover-corner
    marks as a layout *parameterisation* of the same contract, never a
    second grammar. Pairs with RECICLEISLOVE (RowSync) for the model half.
  - **Now:** landed — one `RowMarks` block backs every rail (MediaList rows
    and grid cells, SearchColumn, AlbumEntry, ArtistEntry cards, the
    fullscreen queue) drawing only inside the host cursor's ±15 window
    (`markVisible`), fed by the ledger sets through the shared
    `ResetRowMarks` / `rowReload` / `TriggerRowReload` seams; `RowState`
    owns selection + in-place writes, `SearchContract` the viewport
    snapshot. The model half (RowSync L1b) stays tracked under
    RECICLEISLOVE.

- [ ] **VI-2 · Global mode state machine** — P1 · (N4, CRITIQUE Ph1.1, MY3 §4.1)
  - **Why:** modes exist only locally in the search overlay; the rest of the app
    has no explicit fallback state, so future features would guess.
  - **Details:** `Mode` enum (Normal/Visual/Command/Search/Insert) in Rust,
    reflected in QML; Esc invariant targets Normal; mode colors per MY3 §3.3.

- [x] **VI-7 · Count register (`3j`, `10l`, `5G`)** — P1 · (N2a, CRITIQUE Ph2.2, MY3 §4.2)
  - **Why:** counts turn motions into a grammar; today digits are dropped.
  - **Details:** numeric prefixes in `keybind.rs`; consume in lists (rows) and in
    the player (seek seconds); count chip appears `micro`, clears instantly.
  - **Now:** the register lives in the `Keymap` (bare digits accumulate, cap
    999 999, a lone leading `0` stays unbound; the count rides the next fired
    action and `take_count()` is read once — 4 new keybind tests). Exposed as
    `Backend.keyCount` (mid-typing it feeds the chip; at fire time the
    consumers read it). Consumed by every list (`ScrollPanel` moves and
    `edgeJump` line jumps, the queue drawer, the fullscreen queue cursor and
    lyric scroll) and the player (`5,`/`5.` seek 5×5 s, `5=`/`5-` volume 25).
    The chip sits above the player bar, fading in over `micro`, clearing
    instantly with the register. Grid mode counts too (row-clamped).

- [ ] **VI-8 · Operator grammar (`d`, `y`, `p`, `P`, `.`)** — P2 · (N2b, CRITIQUE Ph2)
  - **Why:** queue editing is the musical equivalent of buffer editing.
  - **Details:** `dd` dequeues, `yy` yanks a track/album, `p`/`P` append/play-next,
    `.` repeats; requires the queue view (Q-1).
  - **Now:** the row half landed as VI-8a M0 below (yank/paste over the
    standalone `QueueDrawer`); `.` repeat and the generalized operator state
    machine stay here.

- [ ] **VI-8a · Row yank/paste (`y`/`Y`, `p`/`P`) — yank universal, paste queue-only** — P2 · (VI-8, VI-21, VI-4)
  - **Why:** "Rows can be 'Yanked' and 'Pasted' (Y/P) in queue and other
    selections besides just pushing it queue." Today the only row transfer
    verb is Alt "Add to Queue" (append-only), and VI-21's `y`/`P` only move
    the *playing track's text*. The queue drawer is the working document:
    yanking a row (or a visual range) and pasting it above/below the cursor
    makes duplication and queue editing positional instead of append-only.
  - **Gate:** paste is gated by `queueDrawerIsOpen` (`readonly property bool
    queueDrawerIsOpen: queueDrawer.open` in `Home.qml`): with the drawer
    closed `p`/`P` are inert ("nothing happens"). **Yank is universal** —
    every row surface exposes `yankRows`, and `Home.canYankRows` lets Main
    choose the row verb over the VI-21 text fallback. A yank made anywhere
    survives until the drawer opens; `QueueDrawer` exposes `yankRows()` /
    `pasteRows(above)` / `canPaste`.
  - **Semantics (paste positioning is the point):**
    - `y` yanks the cursor row, or the whole range while a visual selection
      is live (`v`/`V` on any list or in the drawer); `Y` is the cursor-row
      shorthand (vim line-yank).
    - `p` pastes **below** the cursor, `P` pastes **above** it (vim
      after/before), preserving clipboard order and duplicates.
    - The clipboard stores rows, not text: `{kind, ids[]}` in selection order
      (`song`/`album`/`artist`), duplicates preserved. Paste expands album/
      artist rows through `queue::quick_tracks_json_many` (online/offline
      aware), so a selection from Albums inserts its tracks — the same
      resolver as Alt "Add to Queue".
    - Paste never autoplays. An empty queue loads stopped (the append
      contract); the cursor lands on the first pasted row through the
      existing `_pendingQueueCursor` path.
    - Feedback is mandatory: every yank/paste/delete **raises the shared log
      banner** — `(Yanked!) N rows` / `(Pasted!) N rows` / `(Deleted!) N
      rows` — through the new `noticeRaised` channel into `ToastModal.qml`
      (the neutral sibling of the error `Toast`, fixed to the log style:
      secondary tint, `✓`, no retry). Empty targets stay silent; a failed
      insert reuses the error toast. `Backend.clipboardCount` feeds a future
      chip/hint.
  - **Key decisions (settled):**
    - Bare `n`/`p` track skips are **removed**; `Ctrl-N`/`Ctrl-P` are the
      standard (VI-10 already maps them). `p` becomes paste-below, `P`
      paste-above, and `n` is freed (reserved for VI-14's in-list
      next-match). Update `keybind.rs` `defaults()` + its doc comment,
      README, key hints and the VI-10 note/tests.
    - `y`/`Y` act on rows on every list and in the drawer (yank is
      universal); with nothing yankable, `y` keeps the VI-21 playing-track
      text copy as a fallback, and `Ctrl-V` stays the system **text** paste.
      `p`/`P` are inert outside the open queue drawer.
  - **Backend:**
    - `player.rs`: `insert_many(at, tracks)` — insert at a queue position;
      when the insert touches the mpv window, re-anchor like `move_item`
      (play_current + seek) so the playhead stays put; bump `revision`.
    - `qml.rs`: the universal clipboard (`clipboard_kind`/`clipboard_ids`,
      `clipboardCount`), `yankRows(kind, idsJson)` (logs `(Yanked!)`), and
      `pasteQueueRows(cursor, above)` → `queue::quick_tracks_json_many` →
      `pasteQueueTracks(...)` (logs `(Pasted!)`, re-emits the queue). Actions
      ride the key/IPC action stream; no extra IPC verb names needed.
    - `queue.rs`: reuse `quick_tracks_json_many` unchanged; duplicate ids are
      preserved by its id-ordered expansion.
  - **QML:** `y`/`Y` are handled by every row surface — the shared
    `ScrollPanel` (`yankKind` + `yankRows`), `MediaList` (binds the kind),
    `AlbumEntry`, `ArtistEntry` (wall), `SearchColumn`/`SearchOverlay`, and
    `QueueDrawer`; `Home.canYankRows` picks the row verb over the VI-21
    text fallback. `p`/`P` are consumed only by `QueueDrawer` through the
    `queueDrawerIsOpen` gate; `ToastModal.qml` is the log banner. Key hints
    + README.
  - **Milestones:**
    - **M0 queue-internal paste:** `insert_many` + drawer `p`/`P`
      (cursor and range) behind `queueDrawerIsOpen`, the
      `(Yanked!)`/`(Pasted!)` log, and tests for insert positioning,
      duplicate rows, empty queue, the window re-anchor, and the closed-gate
      no-op. *Landed.*
    - **M1 universal yank:** every list/search surface yanks by kind through
      the resolver; paste into the queue works from any yank (the drawer
      only supplies the cursor). *Landed with M0.*
    - **M2 polish/targets:** cut (`x`/`dd` yank+delete), `Alt` overlay
      "Paste here" verb, clipboard chip/hint, playlists (ARC-44 M3); `.`
      repeat stays VI-8.
  - **Open questions:** whether paste should ever target playlists directly
    (M2 target enum) and whether `Alt-y` should keep copying the playing
    track text while a list can yank rows instead.
  - **Now:** M0 + the universal yank half landed. The clipboard
    (`clipboard_kind`/`clipboard_ids` + `clipboardCount`) is Backend-wide:
    `yankRows(kind, idsJson)` accepts `song`/`album`/`artist` from any
    surface, preserves order and duplicates, and logs `(Yanked!)`. `p`/`P`
    (`paste_below`/`paste_above`) work only while the queue drawer is open:
    `pasteQueueRows` resolves the clipboard through
    `queue::quick_tracks_json_many`, then `pasteQueueTracks` inserts at the
    cursor (`Player::insert_many`: index shift for inserts at/above the
    current track, mpv-window re-anchor at the same playhead) and logs
    `(Pasted!)`. Yank surfaces: `ScrollPanel` (`yankKind`) / `MediaList`
    (Albums/Songs/Artists), `AlbumEntry`, `ArtistEntry`'s wall,
    `SearchColumn`/`SearchOverlay`, and `QueueDrawer`; `Home.canYankRows`
    gates the VI-21 text fallback. Logs ride `noticeRaised` →
    `ToastModal.qml`: yank/paste plus the queue deletions —
    `queueRemove`/`queueRemoveMany` log `(Deleted!) 1 row` / `(Deleted!) N
    rows` (only when a row actually left; `remove_many` returns its count).
    Keybind: bare `n`/`p` track aliases removed
    (`Ctrl-N`/`Ctrl-P` standard), `p`/`P`/`Y` bound; README + keybind docs
    updated. Remaining: M2 (cut, Alt "Paste here", clipboard chip/hint,
    playlists).

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
  - **Note:** generalised by ARC-UI-2 (SEARCHACTIONSTUDY.md); the in-list case
    is its M1–M3 acceptance subset.

- [ ] **VI-15 · `?` / `:help` cheatsheet** — P2 · (N8c, MY3 step 10)
  - **Why:** discoverability of the whole grammar.
  - **Details:** square card in the which-key style, grouped by mode.

- [ ] **Q-3 · Shuffle / repeat** — P2 · (M3)
  - **Why:** the shuffle button exists but is inert; repeat-one/all is expected.
  - **Details:** wire the shuffle icon + repeat mode, show the active state via
    a state layer (stop is already wired; see UI-17).
  - **Now:** repeat landed (`aed6c1a`, `f7662c4`, `d3f4b46`) — `repeat-rounded`
    is an accessory in the left gutter of the centred previous/play/next core
    (alpha 0 when off), Ctrl-R / click cycles off → one (`1` badge) → all →
    off; `Player::cycle_repeat` maps one to mpv `loop-file=inf`, all wraps the
    Rust queue at the end. Shuffle remains inert: the transport still shows
    `random-svgrepo-com.svg` with no `onClicked`.

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

- [ ] **P-4 · Playlists (index, view, play, create/edit)** — P2 · (M6, roadmap; **absorbed into ARC-44 — this entry keeps the roadmap framing; its Subsonic research feeds the ARC**)
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

- [ ] **ARC-44 · playlist.db — Subsonic playlists + local M3U, the Playlists segment end to end** — P2 · (absorbs P-4/M6; roadmap; planning)
  - **Why:** the Playlists sidebar segment is a disabled placeholder
    (`SideBar.qml` `segments[3].enabled = false` → `PlaceholderList` in
    `Home.qml`), the queue cannot be saved, and CACHE-5's playlist rule plus
    Q-8's optional MPRIS interface both wait on an index. P-4 collected the
    Subsonic research; this ARC is the build plan and adds the second source
    the app should own: **local M3U files**, indexed into the same disposable
    cache, so `/` and the Alt verbs work identically over both.
    *Correction to P-4:* `api.rs` no longer wraps the five playlist endpoints
    — ARC-7 trimmed the file to the 14 live calls, so the wrappers
    (`getPlaylists`, `getPlaylist`, `createPlaylist`, `updatePlaylist`,
    `deletePlaylist`) must be re-added (typed structs + fixture tests) rather
    than just consumed.
  - **Sources (one DB, `source` discriminator):**
    - **subsonic** — P-4's protocol research is canonical: `getPlaylists`
      (never the `username` filter — Navidrome ignores it), ordered
      `getPlaylist` entries (duplicates allowed, positions preserved, never
      dedupe), `createPlaylist` (replace semantics = reorder foundation),
      `updatePlaylist` (append/remove-by-index only), `deletePlaylist`;
      refresh on login, after a successful library sync, on `r`, and when the
      segment opens with a stale/empty index; contents fetch lazily when
      `changed` outruns the cached row and after every edit (optimistic +
      revert + refetch, server-authoritative); purge rows for playlists that
      left the server, never inside the library `replace` transaction
      (ARC-25).
    - **m3u** — the app owns a folder: `config_dir()/playlists/*.m3u`
      (`MUSIKA_PLAYLISTS_DIR` env override, matching the per-DB env pattern).
      Parser rules: `#EXTM3U` tolerated but not required; `#EXTINF:<secs>,
      <artist> - <title>` feeds the denormalized row; other `#` lines and
      blanks skipped; each entry either an absolute path or a path relative
      to the m3u's own directory; classic ANSI/UTF-8 with BOM tolerated.
      Matching to `library.db` by `songs.path` exact first, then by
      basename; unmatched entries are kept (rendered dimmed, skipped on play
      with a summary) so an m3u referencing a not-yet-synced file is honest.
      Re-scanned when the file's mtime/size changes (and on every playlists
      refresh); the DB rows are disposable — the files stay the source of
      truth. v1 is read-only in-app (edit the file, press `r`); exporting an
      app playlist to m3u is a later increment.
  - **DB (`playlist.db`, the DB8/lyrics pattern):**
    - `migrations_playlist/` + `build.rs` rerun list + `Dbs.playlists` +
      `migrate()` + a wipe scope;
    - `playlist(id PK, source, name, comment, owner, public, song_count,
      duration, created, changed, cover_art, path, mtime, fetched_at)`
      (`path`/`mtime` are the m3u file identity; server rows leave them
      NULL);
    - `playlist_song(playlist_id, position, song_id NULL, title, artist,
      album, duration, cover_art, PRIMARY KEY(playlist_id, position))`, FK
      cascade — denormalized so the page renders when a song left the
      library; `song_id` NULL = unmatched m3u entry;
    - `playlist_meta(key, value)` mirrors `library_meta`.
  - **Backend (`qml.rs`):** slots `requestPlaylists`, `requestPlaylist(id)`,
    `refreshPlaylists`, `playPlaylist(id, start)` (queue builder: ordered
    entries → the `playerLoad` path, missing songs skipped; covers ride the
    existing `refresh_cover_urls`/queue-cover pipeline); channels
    `playlists`, `playlist`. Mutations in the mutation milestone:
    `createPlaylist(name, songIds)`, `renamePlaylist`, `deletePlaylist`,
    `addSongToPlaylist`, `removeSongFromPlaylist` — optimistic + revert +
    refetch, and every one also an IPC command (`playlists_refresh`,
    `playlist_open <id>`, `playlist_play <id>`, `playlist_add <id>
    <song-id>`, `playlist_remove <id> <index>`, `playlist_create <name>`) so
    scripts and the README grammar stay single-sourced.
  - **Sync (`index.rs`):** a `playlists` stage in the full pass — m3u rescan
    + `getPlaylists` list refresh (+ staleness purge) — reported in
    `SyncReport.playlists`; the ARC-8 `spawn_sync` pipeline carries it
    unchanged.
  - **QML:** `SideBar.qml` enables `segments[3]`;
    `PlaylistList.qml` — the `MediaList` shape (title, `N songs · owner`
    subtitle, `duration · source` meta; `Enter` open, `Space` play all,
    adaptive rows and `/` paint come free through `ScrollContent` +
    `SearchText`); `PlaylistEntry.qml` — the album-page shape (square header
    with the first entry's art or a small local mosaic via `CoverImage`,
    owner/public/comment/count/duration; ordered rows with the position as
    the track column; `Enter` plays from that position, `Space` all, `d d`
    removes the cursor entry on server lists only); `QuickActions`
    `playlist(item)` verb set (Play all / Add to Queue); the generic `v`/`V`
    selection + batch queue/favorite already apply; `Home` routing swaps the
    `PlaceholderList` component for `PlaylistList`; a later `Prompt.qml`
    text overlay + `PlaylistPicker.qml` for create/rename/append.
  - **Milestones (each shippable):**
    - **M0 — DB + M3U + segment:** migration, `Dbs.playlists`, m3u parser +
      path matching (`src/playlists.rs`), scan on startup/sync,
      `requestPlaylists` → `PlaylistList`; the segment lists real m3u names
      offline. *Accept: restart keeps them; unmatched entries render dimmed;
      `cargo test` covers the parser (EXTINF, relative paths, BOM).*
    - **M1 — Subsonic list + contents:** re-add the five `api.rs` wrappers
      (+ fixture tests), refresh policy + purge, lazy `getPlaylist`;
      `PlaylistEntry` renders with positions; duplicate songs keep their
      slots. *Accept: offline restart shows cached contents; a server-side
      rename appears after `r`.*
    - **M2 — play + reach:** `playPlaylist` (missing songs skipped with a
      toast), `Space`/Enter semantics, `/` finds playlist rows and their
      entries (viewport-scoped), Alt verbs arming on playlist rows and song
      rows. *Accept: queue covers resolve; search jumps land; no dead verbs.*
    - **M3 — mutations:** create/rename/delete + add/remove with the
      optimistic/revert contract, `Prompt.qml` + `PlaylistPicker.qml`,
      IPC parity commands, `a p`/`a n` binds. *Accept: every mutation
      reverts visibly on a forced failure; IPC commands drive the same
      paths.*
    - **M4 — polish:** cover mosaic, reorder via `createPlaylist` replace,
      CACHE-5 playlist rule hook, MPRIS playlist interface (Q-8 M5),
      m3u export.
  - **Verify:** the M0–M4 acceptance lines plus a matrix: offline restart
    (both sources), m3u with relative + absolute paths and a missing file,
    duplicate songs, stale `changed`, m3u mtime rescan, wipe scope clears
    both sources, `SyncReport.playlists` counts, `/` viewport behavior over
    playlists, and playlist rows surviving a library wipe (denormalized).
  - **Modules:** `api.rs` (five wrappers + tests), `src/playlists.rs` +
    `migrations_playlist/` (new), `db.rs` (pool/path/migrate/wipe),
    `index.rs` (stage + purge hooks), `qml.rs` (slots/channels/queue),
    `PlaylistList/PlaylistEntry` (+ later `Prompt/PlaylistPicker`), `SideBar`,
    `Home`, `keybind.rs`, `ipc.rs`, `build.rs`, README grammar.
  - **Now:** M0 backend landed — `migrations_playlist/` (`playlist`,
    `playlist_song`, `playlist_meta`), `Dbs.playlist` + the `playlist.db`
    path env (`MUSIKA_PLAYLIST_FILE`) + migrate + `build.rs` rerun,
    `src/playlists.rs` (m3u discovery in `config_dir()/playlists` +
    `MUSIKA_PLAYLISTS_DIR`, EXTINF/BOM/CRLF/relative-path parser, path →
    basename → artist-title matcher, wholesale rewrite + vanished-file
    purge, segment payload; parser/matcher unit tests + a scan integration
    test), `Backend.requestPlaylists` → the `playlists` channel,
    `PlaylistList.qml`, the SideBar segment enabled, Home component swap.
    **The Subsonic list half landed too** — `getPlaylists`/`getPlaylist`
    wrappers restored to `api.rs` (ARC-7 had trimmed them) with fixture
    tests, `playlists::refresh_subsonic` (upsert + vanished purge, lazy
    contents untouched) runs on segment open and as stage 8 of the library
    sync (`SyncReport.playlists` in the log line). The entry page/playback/
    verbs are M2; `getPlaylist` contents arrive with it.

- [x] **ARC-45 · Previous/Next Place — a 32-step travel history (`[b`/`]b`, mouse Back/Forward)** — P2 · (reported; navigation review)
  - **Why:** navigation is only hierarchical today: `Back` pops one page
    (`Home.qml` `handleAction("back")`), a segment switch clears the page stack
    (`showSegment`), and nothing remembers the places left behind. After
    detouring through an artist, an album and a search hit, returning to the
    previous view means reconstructing it by hand. Vim users expect `[b`/`]b`
    (previous/next buffer) for exactly this, and the mouse's Back/Forward
    buttons should do the same; a bounded travel log gives both without a new
    persistence layer.
  - **Storage (`Home.qml`, UI-only, not persisted):**
    - a strict 2-D array, one row per visited place, `placeHistory[32][...]`,
      plus `placeIndex` (the row the user is on). 32 is the cap: when the
      33rd place lands the oldest row is dropped.
    - row schema (flat, so the history stays 2-D):
      `[segment, fullscreen, pageCount, kind, id, title, artist, meta, cover, starred, "", …]`
      — the first three cells are the shell (segment 0–3, fullscreen 0/1,
      page count), then one 8-cell block per pushed page, bottom→top. Album
      pages fill title/artist/meta/cover/starred (the meta string carries
      "year · tracks"); artist pages use `kind="artist"`, the id, name and
      cover (remaining cells empty). The displayed arguments are stored so
      restoring a place needs no new fetch endpoint.
    - cursor, scroll offsets, search text, the floating search, the line
      prompt, the queue drawer, the Alt overlay and the playhead are **not**
      places: history is a route log, not a full UI snapshot.
  - **Recording:**
    - `recordPlace()` snapshots the shell + page stack after a navigation
      mutation: `showSegment`, `openAlbum`, every
      `Back` branch that actually changed the view (fullscreen close, stack
      pop, artist detour; clearing a selection alone does not record), the
      `song_album`/`open_album`/`open_artist` pumps, `toggleFullscreen` and
      the `playbackStarted` auto-open.
    - adjacent duplicates collapse (same row = no push); when `placeIndex` is
      not at the end, a fresh navigation truncates the forward rows first
      (browser history), then appends and clamps to 32.
    - recording hangs off the state itself: `contentStack.onDepthChanged`
      catches every page push/pop (no push site can bypass the log),
      `onCurrentSegmentChanged` catches segment switches and
      `onNowPlayingExpandedChanged` catches fullscreen; `showSegment` raises
      the `_placing` guard around its multi-pop so it emits one row, not one
      per popped page.
    - seed with the initial segment at `Component.onCompleted`; the restored
      P-6 album (`statusRestored` → `openAlbum`) becomes the second row.
      `Home` is recreated on logout/wipe, so the log resets naturally.
  - **Restoring (`movePlace(delta)`):**
    - `prev_place`/`next_place` move `placeIndex` by one and replay that row;
      at either end they are a no-op (no toast).
    - close transient overlays first (search, line prompt, queue drawer, Alt
      overlay): with one open, the first invocation only dismisses it and a
      second moves. Then set `_placing` so `recordPlace()` is a no-op,
      restore the segment, pop the stack to the segment host with
      `StackView.Immediate`, re-create the stored album/artist pages from
      their blocks (also Immediate), set `nowPlayingExpanded` from the row,
      and clear the guard. A history jump is a cut, not a cascade of
      push/pop animations.
    - `Back`/Esc stays hierarchical and records its resulting place; `[b`
      therefore returns to the page the user escaped from (travel-log
      semantics, not an undo stack).
  - **Keybind (`keybind.rs`):** `Action::PrevPlace`/`NextPlace` with names
    `prev_place`/`next_place`; defaults `[ b` and `] b` (Vim buffer motions);
    `Ctrl-[` remains Back. `token_from_qt` currently maps `[` (0x5B) only for
    that chord — add `]` (0x5D). Counts are ignored (one step per press). Add
    the token + binding tests (`[` defers, `[ b` fires, `] b` fires).
  - **Mouse (`Main.qml`):** one window-level handler accepting only
    `Qt.BackButton`/`Qt.ForwardButton` (buttons 8/9; X11 and Wayland),
    dispatching `prev_place`/`next_place` through the same
    `keyHandler.dispatch` path, so keys, IPC and the mouse cannot drift. It
    must not steal left/right clicks or fight the per-row `MouseArea`s;
    verify over lists, covers, fullscreen and the overlays. If a compositor
    reports raw buttons instead of `Qt.BackButton`, fall back there and
    document it.
  - **IPC:** the existing command pump already forwards action names, so
    `prev_place`/`next_place` work over the socket with no new command; add
    them to the README grammar if it enumerates actions.
  - **Verify (matrix):**
    - Main → album A → artist B → album C, then `[b` ×3 walks back to Main
      and `]b` ×3 replays C; both ends no-op.
    - Esc from C lands on B and records it; `[b` returns to C (travel log).
    - navigate back twice, then open a new album: forward rows are gone,
      `]b` stays put; the 33rd navigation drops the oldest row and keeps the
      index at the end.
    - segment switch with pages open logs the segment root; restoring a row
      with two pages rebuilds both headers without duplicate recording.
    - a focused text field types `[b`; mouse Back/Forward still moves. With
      the search/drawer/line prompt open the first mouse press closes the
      overlay, the second moves.
    - fullscreen auto-open on play is one place; `[b` returns to the list it
      started from, `]b` reopens fullscreen.
    - `cargo test` (new keymap cases) and `qmllint` clean; manual mouse test
      on X11 and Wayland.
  - **Now:** landed — `Action::PrevPlace`/`NextPlace` (`prev_place`/
    `next_place`) with `[ b`/`] b` defaults and the `]` 0x5D token mapping;
    `Home.qml` keeps the bounded 2-D log (`placeHistory`, `placeIndex`,
    `placeMax: 32`), seeds on `Component.onCompleted`, records from
    `onDepthChanged`/`onCurrentSegmentChanged`/`onNowPlayingExpandedChanged`
    behind `_placing`, collapses adjacent duplicates, truncates the forward
    tail on a fresh move and drops the oldest row at 33. `movePlace` dismisses
    the open transient overlays first, then restores segment + page stack with
    `StackView.Immediate` from the stored blocks and replays the fullscreen
    flag; `Esc` still records its hierarchical result. `Main.qml` adds a
    `TapHandler` for `Qt.BackButton`/`Qt.ForwardButton` dispatching the same
    action names, so IPC/keyboard/mouse share one path. Keymap tests cover
    the tokens, the deferred `[` prefix and both bindings; `cargo test` 144
    pass; `scripts/check-qml.sh` clean. Open: the manual X11/Wayland mouse
    pass (compositor button mapping).

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

- [x] **UI-38 · Loading/empty states for the three main lists** — P2 · (POLISHAUDIT §5, UI-22)
  - **Why:** Album/Song/Artist lists render nothing until the first payload
    and have no empty state.
  - **Details:** reuse the SearchColumn skeleton pattern and add centred
    empty messages ("Nothing indexed yet — run a sync").
  - **Now:** both landed — the empty state covers Album/Song/Artist lists, and
    the loading skeletons ride the shared `SkeletonList` (UI-22): the
    wrappers raise `MediaList.loading` while a payload is in flight and the
    skeleton covers the list area below the header. No open items.

- [x] **UI-39 · Motion token sweep + reduced motion** — P2 · (POLISHAUDIT §6, UI-19)
  - **Why:** legacy `Theme.duration`/`Theme.easeOut` (100 ms) still drive
    eight files, spin/bloom speeds are inline, and nothing can opt out.
  - **Details:** replace legacy uses with `Theme.motion.*`/`Theme.ease.*`;
    add `Theme.motion.spin`/`spinSlow`; add a `Theme.reduceMotion` gate for
    `FadeText`/bloom.
  - **Now:** `Theme.motion.spin`/`spinSlow` tokens (used by the sync spin);
    `Theme.reduceMotion` gates `FadeText` and the skip bloom; the seven
    legacy `Theme.duration`/`Theme.easeOut` files now use motion tiers.

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
      on selection/track change or after idle (lyrics: 5 s, dropping to 2 s
      while the pane is unfocused — LYR-8);
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
    `DragOverBounds`, and the lyrics `SmoothedAnimation` follower.
    `reuseItems` is now enabled on both list/grid views (the delegates are
    property-driven); `cacheBuffer` was already the render overscan.
  - **Verify:** frame graph (`QSG_RENDER_TIMING=1`) smooth for 1-row, 5-row
    and 20-row moves; wheel mid-follow doesn't stutter; long titles don't
    clip while expanding; `reduceMotion` flat.

- [ ] **UI-55 · Design task — Acrylic panels: substitute the solid surfaces** — P2 · (MS Fluent Acrylic; R1/R2 amendment)
  - **Why:** every chrome surface paints an opaque role fill — `SuperiorBar:30`
    and `SideBar:44` (`surfaceContainerLow`), `InferiorBar:26`
    (`surfaceContainer`), `QueueDrawer:147` (`surfaceContainerHigh`),
    `SearchOverlay:351` (`surface`) and `Toast:38`
    (`errorContainer`/`tertiaryContainer`) — because R1/R2 reserve depth for
    the tonal ladder and allow blur only for the modal scrim. Microsoft's
    acrylic material (learn.microsoft.com/pt-br/windows/apps/design/style/acrylic)
    is the researched answer to that gap: translucency + blur + noise + tint
    gives surfaces depth without shadows, and it is the solid-panel
    substitution this task schedules.
  - **Details:**
    - **MS recipe:** translucency, blur and noise build the physical texture;
      an exclusion/blend layer guarantees UI contrast on top; a colour tint
      carries personalisation. Two blend types: *background acrylic* reveals
      the desktop/windows behind the app (transient surfaces: context menus,
      light-dismiss popups); *in-app acrylic* adds depth inside the frame
      (supporting UI that content scrolls under).
    - **MS rules:** never layer acrylic surfaces; never place two acrylic
      panes edge-to-edge (visible seam); no background acrylic on large
      background surfaces; no accent-coloured text or hyperlinks over acrylic
      (14 px body must keep its contrast ratio); vertical section-separating
      panes get an opaque fill. GPU-heavy — MS disables it in battery saver.
    - **MS fallbacks to solid:** transparency effects off, battery saver,
      low-cost hardware, high-contrast mode; background acrylic also goes
      solid on inactive windows and Xbox/HoloLens/tablet.
    - **Qt mapping (no native acrylic brush):** a shared `AcrylicPanel.qml`
      = `ShaderEffectSource` (`live: true`, `sourceRect` mapped to the panel)
      → `MultiEffect` (`blurEnabled`, `blur` ≈ 0.5–1.0, `blurMax` 48–64,
      `saturation` 0.6 — the existing niri-like pass, `Home.qml:362`) →
      translucent tint (`Palette.surfaceContainer*` at ~0.72–0.8 alpha) →
      tiled noise (`noise 0.03`; `Home.qml:359` already notes MultiEffect has
      no grain — a fragment-shader hash or a small PNG fixes it).
    - **No new dependency:** MultiEffect/ShaderEffectSource ship with the Qt
      already in-tree (`Home.qml`, `CoverImage.qml:56`); crop `sourceRect` to
      the panel and downscale `textureSize` to keep the pass cheap.
      Compositor blur (KWin blur-behind, `org_kde_kwin_blur_manager`,
      `ext-background-effect-v1`) is compositor-specific — niri/GNOME would
      not honour it, so it stays out of the critical path.
    - **Fallback token:** `Theme.acrylic` (tint alpha, blur, noise, `enabled`)
      mirroring `scrimOpacity`; `enabled: false` renders today's solid fill,
      so the accessibility and low-end paths are a one-flip change.
  - **Plan:**
    1. transient surfaces first, where MS wants background acrylic:
       `QueueDrawer` and `SearchOverlay` panels, then `Toast`;
    2. supporting chrome context: `SuperiorBar`/`InferiorBar` as in-app
       acrylic with content extending under; `SideBar` is a
       section-separating vertical pane, so it keeps the opaque fill unless
       it overlays content (narrow/compact modes) — MS's own rule;
    3. never stack the search overlay's shell-wide blur (`Home.qml:361`)
       with a panel acrylic — pick one per state;
    4. contrast pass (body text over the worst-case cover) and the
       `Theme.acrylic.enabled: false` solid path before wiring the next
       panel.
  - **Verify:** Wayland desktop pass (blur reads under each panel, no seam at
    bar/sidebar/drawer junctions, 14 px text keeps its ratio over artwork),
    fallback flips every panel back to solid, `QSG_RENDER_TIMING=1` frame
    cost while scrolling under a panel, qmllint clean.
  - **Now (partial):** the surfaces landed — `Theme.acrylic`
    (`enabled`/`tintAlpha`/`blur`/`blurMax`/`saturation`/`downscale`/`noise`;
    the solid role fill is the disabled/no-backdrop path) and the shared
    `AcrylicPanel.qml` (a `ShaderEffectSource` of `backdrop` mapped to the
    panel rect and downscaled, run through the existing MultiEffect recipe;
    translucent tint, tiled-PNG grain at ~0.03 — Qt 6 `ShaderEffect` takes
    `.qsb` URLs, so the inline hash shader became `noise.png` — and a border
    redraw above the material). Consumers: the `QueueDrawer` panel, the
    `SearchOverlay` panel, the `Toast` (`backdrop` switches between `shell`
    and `nowPlaying`) and the fullscreen lyrics/queue panes over their
    `ambience` item (cover + wash as one backdrop); `Home` no longer stacks
    its niri-like shell/fullscreen blur with the overlay acrylic. Open: the
    `SuperiorBar`/`InferiorBar` in-app panels and the desktop
    contrast/fallback pass.

- [ ] **UI-63 · Vulkan/RHI budget pass (notes)** — P2 · (GPU review)
  - **Why:** the RHI path is already Vulkan, but pipeline creation, MSAA and
    full-surface blur can still hitch the first interaction or waste GPU on
    static overlays. Capture a profile before changing anything; the frame
    graph is the only way to know whether lists, blur or overdraw dominate.
  - **Notes (operator-provided, not implemented):**
    - **Persistent pipeline cache:** `QQuickGraphicsConfiguration::
      setPipelineCacheSaveFile(<CacheLocation>/pipeline_cache.bin)` on the
      window/engine, so cold start does not re-JIT every effect pipeline.
    - **MSAA:** choose it explicitly (`QQuickWindow::setSampling` or the RHI
      equivalent); prefer no full-scene MSAA and per-shape antialiasing for
      rounded rects/text.
    - **MultiEffect/blur:** keep `blurMax` as low as acceptable,
      `autoPaddingEnabled: false` when no padding is needed, crop the
      `ShaderEffectSource`/`sourceRect` to the covered region, and use
      `live: false` + manual invalidation for static overlays instead of
      re-blurring every frame.
    - **Custom native work:** if particles/visualizers outgrow QML items,
      consider a `QSGRenderNode` with native `VkCommandBuffer` draws rather
      than N small `Rectangle`/`Image` nodes; use
      `QNativeInterface::QSGVulkanTexture::fromNative` when art comes from a
      non-Qt decoder, instead of a `QImage` CPU round-trip.
    - **Shader warm-up:** pre-render a 1×1 offscreen frame per effect variant
      before showing the window; keep `.qsb` permutations minimal.
    - **Profiling:** `QSG_RHI_PROFILE=1`, `QSG_RHI_SHADER_DEBUG=1`,
      `QSG_RENDERER_DEBUG=render`, and `QT_VK_DEBUG_LAYER=1` for validation.
    - **Cover uploads:** check whether album art needs RGBA8; opaque formats
      avoid blend state and batch better.
  - **Status:** notes only. Start with a profile of `NowPlaying`, the blurred
    overlays and grid scrolling; do not add a native render node without a
    measured bottleneck.

- [ ] **UI-64 · Nativemaxxing audit — 59 QML modules** — P2 · (native Qt review)
  - **Why:** hand-rolled items and event plumbing duplicate what Qt Quick
    already provides; the audit finds the cases where a native type keeps the
    contract with less custom state.
  - **Done this pass:** `ScrollPanel`'s A/B segments use `ColumnLayout`
    (`f38e3ac`); all app text goes through `AppText`; `RowBody` uses stable
    font buckets with wrap/elide; `AltActionsOverlay`'s dismiss scrim uses
    `TapHandler` instead of `MouseArea`; seek/volume are the native `ScrubBar`
    (Slider) in both the player bar and NowPlaying; `IconButton` is a native
    `AbstractButton` with the same Musika skin.
  - **Performance pass (native/frugal):** `AppText` defaults to
    `Text.NativeRendering` and opts into `QtRendering` only for transformed
    text (lyrics); `CoverBackdrop` uses the Rust 96 px blur cache and no
    full-page MultiEffect; `AcrylicPanel` skips the hidden blur pass when its
    tint is opaque; the lyric stage has native gradient edge fades instead of
    an offscreen mask layer; `ScrollContent` uses realised-delegate targets
    plus native `indexAt`/`displayMargin`, and only probes Qt for
    unrealised/far targets.
  - **Audit (native already):** `ScrollContent` is native `ListView`/
    `GridView` + `ScrollMemory`; rows use `HoverHandler`/`TapHandler`/
    `DragHandler`; `RatingBurst` is native `ParticleSystem`; badges, icons
    and covers use `Image`/`VectorImage`; animations are Qt
    `NumberAnimation`/`Behavior`/`ParticleSystem`; there are no remaining
    `MouseArea`s in `src/qml`.
  - **Further candidates:** `SymbolIcon` still pays a per-icon MultiEffect
    colorization pass; measure and cache tinted icons only if it remains hot.
  - **Open candidates:**
    - `Toast`/`ToastModal`, `QueueDrawer`, `SearchOverlay` → native
      `Popup`/`Drawer` where the custom acrylic backdrop and keyboard model
      allow;
    - `ActionChip`, `ShuffleChip` → `Button`/`ToolButton` styling instead of
      custom click state (IconButton is done);
    - `Home`'s `EnterView`/`ExitView` + travel log vs Qt `State`/
      `Transition`;
    - `ScrollContent`'s custom `contentY` glide stays because Qt has no
      animated `positionViewAtIndex`.
  - **Verify:** `scripts/check-qml.sh`, `cargo test` (195), `nix build`;
    no visual/behaviour diff per refactor.

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
  - **Now (post-0.4.2 modularization pass):** `consts::now_secs()` replaced the
    four local `now()` helpers; `cache::purge_missing` collapsed the lyric and
    color purge seams. The pass also added `scripts/modularity.sh`, the
    session/digest/offline primitives, `app` process state (root `dbs`/
    `runtime`/`pool` references 60→0), the library split (`library.rs` 1330→
    912 lines; `library/sync.rs` 293, `library/info.rs` 179), the browse split
    (`browse.rs` 941→176; `browse/rows.rs` 292, `browse/search.rs` 435), and
    the player split (`player.rs` 1099→676; `player/mpv.rs` 451). Duplicate
    `md5_hex` 2→1, lower-layer `crate::qml::{online,client}` 4→0, and
    `chunks(500)` 11→0 (one `cache::IN_BATCH`).
    Open: `qml.rs` logic extraction (2,815 lines, 115 qslots; the mark window
    is in `qml_backend/marks.rs` 257, queue publishing/player sync in
    `qml_backend/player.rs` 213, auth/startup in `qml_backend/auth.rs` 211,
    library state/info in `qml_backend/library.rs` 323, and download slots in
    `qml_backend/download.rs` 716), the remaining library query/state split,
    the optional covers split, and ARC-5.
  - **Verify:** `sh scripts/modularity.sh`, `cargo test` (195), `cargo fmt
    --check`, `nix build .#musika`; no visual diff.

- [ ] **ARC-17 · Slim the `Backend` god-object** — P2 · (O17)
  - **Why:** huge slot surface; `playback_state` is a `String`, should be enum.
  - **Now (post-0.4.7):** `qml.rs` is 2,815 lines with 115 qslots. The mark
    window (`marks.rs`), queue publishing/player sync (`player.rs`),
    auth/startup (`auth.rs`), library state/info (`library.rs`) and download
    bodies (`download.rs`) are out; the data layer (`browse`/`queue`) and
    process state (`app`) never lived here. Open: finish the status/search/
    transport wrappers, reduce the slot surface, and make `playback_state`
    an enum.

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

- [ ] **ARC-REV · IPC coverage review — does `ipc.rs` cover all the calls it needs** — P2 · (feeds ARC-37)
  - **Why:** the socket reaches whatever happens to flow through
    `keyHandler.dispatch` and misses the rest: `toggleShuffle` (the tab
    chips — probe-verified unreachable), `shuffleArtist`, `downloadAlbum`,
    `altFavorite`, `requestDownloads`, the row-reload triggers. Worse, a
    command can look right and be silently dropped: keymap *tokens* are not
    *action names* — Escape's token is `escape` but the action is `back`,
    `shift-tab` resolves to `pane_cycle_back`, and the socket takes action
    names only.
  - **Details:** one table over every `#[qslot]` on `Backend`, every `Action`
    name and every `command_issued` consumer; each row gets a verdict —
    command spelling (ARC-37 registry when it lands, action strings
    meanwhile), pump-reachable action string, or an explicit "not
    scriptable: reason"; token-vs-name traps and argument shapes
    (`shuffle_toggle <kind>`) are called out.
  - **Fix options:** land ARC-37's Rust registry and route every `#[qslot]`
    through it, or freeze the action-string pump and alias the differences
    (`escape` → `back`, add `shuffle_toggle`).
  - **Verify:** socket integration test (temp `MUSIKA_IPC_SOCKET`) walks the
    table and asserts every row runs; a new `#[qslot]` or `Action` without a
    table row fails the audit test.

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

- [x] **ARC-43 · Alt-hold quick actions — radial quadrant overlay** — P3 · (roadmap Alt-hold quick actions; UI-52, ARC-42)
  - **Why:** per-item actions are keyboard-only today (queue moves,
    activation, navigation) and there is no context menu; exposing them as a
    transient Alt-hold menu keeps the vi grammar readable instead of hiding
    it behind memorised chords.
  - **Details:** hold Alt on the current track/album → four equal boxes
    N/S/E/W around a center anchor: N Add to Queue, S Favorite, one
    horizontal slot contextual Go to Album (track) / Go to Artist (album),
    the opposite slot open in v1 ("—"); `h`/`j`/`k`/`l` or the arrow keys aim
    west/south/north/east while Alt is held and the center is the neutral
    aim — aiming toward the opposite quadrant steps back to neutral first,
    the next press continues (north + `j` = neutral, `j` again = south).
    Releasing Alt is Enter (fires the aim), releasing on neutral or Esc
    cancels. TUI styling: monospace labels, 1px hard edges, `Palette` roles,
    a neutral center dot, and acrylic option tiles over the shell (UI-55
    blur, like the search overlay), ~70% larger than the first pass; one
    component taking item + `{quadrant -> action, label}` config; suppress
    while the keymap has a pending token.
  - **Verify:** track row + album card repro; context swap correct in both
    directions; release on neutral and Esc never fire; overlay absent with
    Alt up and never steals hover/click; the acceptance list in the roadmap
    design section passes.
  - **Now (partial: X0 + X1 code paths):** `AltActionsOverlay.qml` — the
    self-contained arm/aim/confirm/cancel state machine with the roadmap hard
    rules (Alt release confirms, neutral aim fires nothing, Esc dismisses for
    the rest of the hold, aiming toward the opposite quadrant passes through
    neutral), acrylic option tiles over the blurred shell (AcrylicPanel,
    UI-55) at 1.7× and a neutral center dot, plus a config-driven
    `{north,south,east,west}` of `{label, invoke}` slots (open slots draw
    "—" and cannot be aimed). Alt press anchors on the active view's keyboard
    cursor: `MediaList` rows (Songs and Albums segments) and `ArtistEntry`
    album cards; `h/j/k/l` or the arrow keys reach the overlay as
    `quick_aim`/`quick_cancel` pumps without touching the keymap. Actions: N Add to Queue (new
    `playerAppend` → `player.append`, playback untouched), S Favorite
    (`star` on song/album ids), E the contextual swap — Go to Album on
    tracks (`altGoToAlbum`) / Go to Artist on albums (`altGoToArtist`),
    pumped as `open_album` / `open_artist` channels — W open. Disarmed while
    the keymap is pending (`Backend.keyPending`) or the search is open; side
    modals own the screen (ARC-UI-1 added the fullscreen queue). Alt is a
    default binding
    (no flag): `alt-j`/`alt-k` were retired from the keymap in its favour
    (queue reorder stays `Shift-J`/`Shift-K`) and the fullscreen pane hint
    was updated. Tests:
    `player::append_extends_the_queue_without_moving_the_cursor`,
    `queue::quick_actions_resolve_songs_albums_and_pages`, plus the keymap's
    `alt-j`/`alt-k`-unbound assertion (114 total). The follow-up ARC-UI-1
    filled the menu (four fixed verbs, recycled `QuickActions.qml`) and the
    overlay gained mouse taps (`TapHandler` on the tiles) plus arrow-key
    aiming beside `h/j/k/l`. Open: only the manual acceptance pass on
    hardware (the same caveat ARC-UI-1 carries).

### Alt action menu (ARC-UI-1)

- [x] **ARC-UI-1 · Four fixed Alt menu verbs** — P3 · (ARC-43 follow-up; roadmap Alt-hold quick actions)
  - **Why:** ARC-43 v1 leaves one horizontal slot open ("—") and makes the
    other swap contextually (Go to Album on tracks / Go to Artist on
    albums); both jumps are real destinations and deserve permanent
    squares. Four squares, the neutral center excluded from the count.
  - **Details:** the quadrants carry Favorite, Queue (Add to Queue),
    Jump to Artist and Jump to Album. Suggested placement keeps the v1
    anchors — N Queue, S Favorite — and fills W Jump to Artist, E Jump to
    Album. When the current selection **is an album**, Jump to Album is
    substituted by the open "—" slot (never offer the page already open),
    while Jump to Artist stays live; a track carries both jumps. No new
    backend: `altFavorite`, `altAddToQueue`, `altGoToArtist` and
    `altGoToAlbum` already exist — only the `quickActions` config in
    `SongList`/`AlbumList`/`ArtistEntry` changes, inside the same neutral
    machine, acrylic tiles and aim keys as ARC-43.
  - **Verify:** track rows show all four verbs; album rows show the
    Jump-to-Album "—" substitution with Jump to Artist still live; aiming
    and neutral-release behavior matches ARC-43; the ARC-43 acceptance list
    still passes.
  - **Now:** landed — `song_rows_json` emits `album_id` (locked by a Rust
    test) so the Songs list can feed the existing `altGoToArtist`; `SongList`
    shows all four verbs, `AlbumList`/`ArtistEntry` put Go to Artist on W
    with the open "—" on E. The overlay draws from one `board` rectangle
    (margin + option scale) with equal square tiles anchored to its edges
    (up/down horizontally centred, west/east vertically centred) and the
    neutral mark `centerIn` it — an inline `OptionBox` plus `pragma
    ComponentBehavior: Bound`. The board also tracks `Window.window` and
    clamps itself to the content rect inset by a 10 px no-cut margin: it
    shifts off a touched edge (right → left and vice versa) and scales down
    only when it cannot fit between both. The tile scale ramps with the
    window itself (cap 20% under the old 1.7, floor 0.5; wide targets are
    caught by the board's fit scale) and the target↔tile margin grows so the
    four squares never overlap. The verbs now
    live in one recycled `QuickActions.qml` (`song(item)`/`album(item)`)
    shared by the lists, `ArtistEntry` and the fullscreen queue, which arms
    the overlay too (`player::Track` gained `album_id`, emitted in the queue
    payload). Each tile carries its SVG verb (queue `baseline-playlist-add`,
    favorite `baseline-favorite`, artist jump `person-walking`, album jump
    `album`) recoloured through the shared `Icon`; the glyph set is baked
    white per `scripts/check-icons.sh`. `cargo test` 114 pass; QML parses
    clean under `qmllint`. Open: the manual acceptance pass on hardware.

### Surface search (ARC-UI-2)

- [ ] **ARC-UI-2 · Global `/` in-surface search (incremental, highlight, Enter jumps)** — P1 · (VI-14 generalised; N5b, CRITIQUE §2.6; SEARCHACTIONSTUDY.md)
  - **Why:** vi search is an in-buffer motion, not only a global palette: `/`
    must search the text of whatever surface is on top — the main lists, the
    album page, the artist grid, the queue drawer, fullscreen and even the
    search overlay's columns — highlight the matched words in place, and jump
    on Enter; today `/` is not tokenized at all (`keybind.rs:285-322`) and the
    only search is the server-side FTS palette.
  - **Details:** the Neovim reference (`pattern.txt`/`cmdline.txt`/
    `syntax.txt` quotes), the surface/selectable-text inventory, the `n`/`N`
    decision and the highlight-rendering study live in
    `SEARCHACTIONSTUDY.md`; the contract:
    - new `Action::SearchAction` + `/` token, and a bottom `LineSearch`
      prompt (`zCommand`, no scrim) that opens over the current
      overlay/location from any region; text fields keep typing `/`;
    - matching is client-side over the top surface's *selectable* cells only
      (title/subtitle/meta, …; headers/hints never match), literal smartcase
      substring; a matching row is the jump unit;
    - matches get a block overlay inside the text (shared `SearchText.qml`
      with `FontMetrics`-measured rects — `Text.RichText` breaks `elide`,
      documented in the study); the current candidate row is emphasized
      (`primary` vs `primaryContainer`) and `[i/N]` shows in the prompt;
    - `Enter` jumps the surface cursor (`searchRows()`/`searchJump(row)`
      contract on MediaList/SearchColumn/QueueDrawer/NowPlaying/AlbumEntry/
      ArtistEntry) and keeps the session; `n`/`N` cycle matching rows while a
      session is live, track skips otherwise (VI-10 preserved); no match =
      the UI-49 denied cue with the cursor untouched (E486);
    - `Esc` while typing abandons; `Esc` after commit is `:noh`.
  - **Milestones (SEARCHACTIONSTUDY.md §7):** M0 input skeleton + prompt ·
    M1 surface contract + matcher/counter · M2 highlight rendering · M3
    commit + `n`/`N` · M4 `?`/history/FTS + sidebar coverage · M5 docs.
  - **Verify:** the 14-case matrix in SEARCHACTIONSTUDY.md §8 (chrome never
    matches, drawer-over-main scoping, typing parity, abandon vs `:noh`,
    elided-title paint, live palette, held-key + palette churn).
  - **Now:** M0–M1 landed (650a4dd) — `/` opens a flat, full-width command bar
    at the bottom edge over a 50% black scrim (no blur); matching is literal
    smartcase over the top surface's rendered cells (`searchRows()`/
    `searchJump()` on `ScrollContent` + the six row surfaces,
    `SearchMatch.js` unit-tested); Tab / Shift-Tab cycle the matches with a
    live cursor preview, `[i/N]` counts, and 0 matches flash the red-underline
    E486 cue with the cursor untouched. Per user rule, Enter commits the
    row's own context-Enter (VI-16) on the match — this overrides the study's
    cursor-only Enter; `n`/`N` repeat is still open. Truly global: the
    sidebar's enabled segments and the status region's controls (Sync/Wipe)
    are targets from their own regions (jump moves the cursor/scope ring,
    never activates). The FTS box and `/` share one `SearchInput` (Ctrl-W
    word-delete, paste/select-all native, context keys as signals; named so
    because Qt 6.11's Controls ships its own `SearchField`, which shadowed
    ours) and both
    pop-ups ride the one spawn FX (`PopFx`, lifted from the Alt overlay).
    Now the whole window is the buffer (status region + sidebar + main at
    once; the modal's surface when one owns the context) and matching is
    **viewport-scoped** — scrolled-out rows never match, so `/` cannot
    afford a jump the eye cannot verify. M2 landed: `SearchText.qml` paints
    Neovim-style blocks (`FontMetrics` rects, elide-safe; `Search`
    primaryContainer, `CurSearch` primary on the current candidate) over
    every row surface's cells; `SearchMatch.js` gained `spans()` (tested).
    Open next: `n`/`N` repeat + Up/Down history + `?` backward (M3/M4).

### Scroll layouts (ARC-UI-3)

- [ ] **ARC-UI-3 · `ScrollContent` list/grid modes (Shift-C)** — P2 · (SCROLLGRIDSTUDY.md; SCROLLCONTENTSTUDY.md, UI-58, VI-4, ARC-UI-2)
  - **Why:** ArtistEntry's scrolling grinds while the shared scroll bodies
    follow smoothly — its bespoke `GridView` + `ListScroll` + `Selection`
    stack re-implements everything `ScrollContent` already owns (follow,
    memory, adaptive rows, selection, the ARC-UI-2 search contract, the Alt
    anchor) and drifts from it with every fix. One scroll body that can
    render a list *or* a cover grid de-forks ArtistEntry and lets Search, the
    album page and ArtistEntry ride the same machinery; the queue stays
    list-only (a queue has no grid metaphor).
  - **Details:**
    - `ScrollContent` gains a layout mode — `layout: "list"` (default) or
      `"grid"` with `columns` — and keeps one active view (`ListView` ↔
      `GridView`, one auto-created pair), carrying the same invisible 1×1
      highlight follow — `highlightMoveVelocity` on the list,
      `highlightMoveDuration` on the grid (GridView has **no** velocity
      property; SCROLLGRIDSTUDY §3/§5) — the shared preferred band,
      `ListScroll` memory/`ensureVisible`, the UI-58 adaptive squeeze,
      `Selection`, `searchRows()`/`searchJump(row)` (both-corners viewport
      probe) and the Alt anchor (`itemAtIndex` exists on both views); the
      grid's grind is today's per-move `ensureVisible` teleport
      (`ArtistEntry.qml:430`, SCROLLGRIDSTUDY §2) and disappears with the
      view-owned follow;
    - hosts keep supplying the delegate(s): a row delegate for list, a cell
      delegate for grid (cover + title underneath, like ArtistEntry today);
      `ArtistEntry` drops its `GridView`/`ListScroll`/`Selection` copies and
      becomes `ScrollContent { layout: "grid"; columns: 4 }`, preserving its
      `moveWithinGrid` edge rule (column edges still return false so `h` can
      leave for the sidebar) and its viewport-scoped search rows;
    - `Shift-C` toggles the mode where both layouts are wired: new
      `Action::LayoutToggle` (`"layout_toggle"`, `shift-c` token — currently
      unbound) consumed by the host's `toggleLayout()`; cursor index and
      viewport map 1:1, and the choice is remembered per `memoryKey`
      (`ScrollMemory`) so a reopen restores the last layout;
    - scope: ArtistEntry first (grid default, list on Shift-C); Search
      columns and the album page inherit the shared body but stay list-only
      until a grid cell delegate earns its keep there — the toggle is only
      advertised on a surface that declares both layouts; the queue never
      exposes it.
  - **Verify:** ArtistEntry behaves identically after the migration (cursor
    edges, `moveWithinGrid`, Enter, Alt anchor, `/` viewport-scoped matching,
    scroll memory) and the follow no longer grinds under held-key and wheel
    input (SCROLLCONTENTSTUDY.md matrix); `Shift-C` swaps list ↔ grid live
    without losing the cursor or viewport; queues never show a grid;
    `cargo test` + QML parse clean, existing QtQuickTest scroll/selection
    checks still pass.
  - **Now (partial):** M0–M2 landed (SCROLLGRIDSTUDY.md) — `ScrollContent`
    gained `layout`/`columns`/`cellDelegate` (`layouts` order + `toggleLayout`)
    with one shared stage, band, memory, selection, search contract and
    adaptive sizing; `ArtistEntry` and the Main segments (`MediaList`:
    Albums/Songs/Artists) both ride the shared `ScrollPanel` now — the wall
    is grid mode with a compact list fallback, the segments default list with
    a cover wall on Shift-C — so `gg`/`G`, the `/` contract, selection and
    the empty/loading states come from one modal. Movement is one shared
    `moveStep` (grid walks by columns, list ±1) exposed as `moveWithinGrid`,
    so Home's `h`-to-sidebar edge works everywhere; the grid grind is fixed
    by the view-owned follow (`highlightMoveDuration`, since GridView has
    **no** `highlightMoveVelocity`); the body's `currentIndex` is the single
    source of truth (view refill transients are overwritten, never adopted;
    `edgeJump` replaced direct `Theme.cursorEdge` writes); `ScrollMemory`
    carries the layout beside cursor + offset. UI-58's band edge moved
    0.7 → 0.82, so row reduction initiates 40% sooner (user call). A
    QtQuickTest harness covers list/grid sync, the swap carry, search rows,
    the memory round-trip and the panel aliases/toggle (12 pass); `cargo
    test` 121 pass, QML parses clean. Open: M3 (UI-58 adaptive cells for the
    grid), extending the toggle to Search columns, the hardware pass.

### Queue marks (ARC-UI-4)

- [ ] **ARC-UI-4 · Row queue add/remove SVG by membership (the favourite scheme)** — P2 · (UI-56 pattern; Q-1, ARC-43)
  - **Why:** a row cannot tell whether its song is already queued: the Alt
    menu's North verb is always "Add to Queue"
    (`baseline-playlist-add.svg`, QuickActions.qml:34/64/98/164) even for a
    track the player queue already holds, and the queue drawer is the only
    place membership shows. The favourite heart is the pattern to copy: one
    state-driven mark (`row.starred` from the payload) that flips with the
    user's action and reconciles on the next payload.
  - **Design:**
    - **one row mark, two glyphs** — not queued → `baseline-playlist-add`
      (the Alt North icon); queued → a removal glyph (`playlist-remove`,
      baked white through `scripts/check-icons.sh`; the set has
      `baseline-playlist-add` and the `heart-minus` minus shape to lean on,
      but no queue-remove SVG yet);
    - **state** — membership of the row's song id in the player queue. The
      `queue` channel already emits `id` per item (qml.rs `emit_queue`), so
      the surface keeps a queue-id set from that payload
      (`queueIndex`/`queueLength` only order it) and rows read it like they
      read `starred` today; album/artist rows would need an any/all rule —
      out of scope until that decision lands;
    - **action** — the same verb toggles: unqueued rows queue
      (`altAddToQueue`), queued rows remove (`queueRemove(index)` at the
      queue position(s)); the Alt North label/glyph follows the state too;
    - **update scheme** — flip the row's local flag at invoke time, exactly
      like ARC-I-4's `decorateFavorite` decoration: no reload, no DB check,
      the next `queue`/list payload reconciles. This is the same in-place
      "row state swap" the favourite decoration still needs for
      MediaList/SearchColumn/ArtistEntry (ARC-I-4's Now note), so both
      features should land on **one** shared row-update path instead of two
      copies — the reuse the sketch points at;
    - **scope** — song rows first: Main Songs (`MediaList`), AlbumEntry,
      SearchColumn songs, and the fullscreen queue rows (already queue-side,
      where the same mark can mean remove).
  - **Verify:** queueing from a row lights its mark immediately in every
    visible region; removing — from the same verb or the drawer — clears it;
    a reopened queue (or restart) shows the true membership; chrome/rows
    without a queue meaning never draw the mark.

### Lyrics experience (ARC-UI-5)

- [ ] **ARC-UI-5 · Spotify-grade lyrics stage — soothing, springy, dots + effects** — P2 · **"Research up what makes it good! Lyrics."** · (LYRICSEXPERIENCESTUDY.md; LYR-8/LYR-10/LYR-6, roadmap "Lyrics presentation")
  - **Why:** the lyrics pane is already synced and glides (LYR-8/LYR-9),
    but Spotify's stage reads *good* for reasons ours does not yet carry:
    the focus window has an affordance for non-lyric time (the **"…"** trio
    during intros and instrumental breaks), the motion settles with a spring
    instead of a linear ease, lines are addressable (tap to seek), and the
    scroll has a rubber-band edge. The research (quoted Newsroom/Support/
    Musixmatch sources + observed behaviour) is in
    `LYRICSEXPERIENCESTUDY.md` §1–§4.
  - **Details (LYRICSEXPERIENCESTUDY.md §3/§6):**
    - **L0 dots** — a pure `Lyrics.js` waiting-helper (`activeLyric < 0`
      before the first timed line, instrumental gaps past a ~4 s threshold,
      timed empty lines) drives three staggered dots in the focus slot;
      they dissolve into the first line; `reduceMotion` keeps them static.
    - **L1 spring** — the `glideToCenter` motion gains a spring settle
      (`Easing.OutBack`, bounded overshoot, or a damped follower) at the same
      `followMs: 300` and bezier the pane fade binds to; far/seek landings
      stay decelerate/instant; retarget in place mid-flight.
    - **L2 addressable lines** — click/tap a line seeks to its `t` with a
      hover state; `Enter`-on-lyrics parity decided in the study; the
      hold/resume window and j/k scrolling stay intact.
    - **L3 polish** — elastic `DragOverBounds` on the lyrics body only, and
      the top/bottom edge fades (closes the LYR-8 residual).
    - **L4 karaoke (stretch)** — per-word accent fill only when the
      OpenSubsonic `cueLine` payload exists (LYR-6/LYR-10); never fake timing.
  - **Verify:** the §7 matrix — helper cases in the harness, the spring
    glide trace (mid-flight samples, bounded overshoot, settle), real intros
    with dots, an instrumental break, click-seek + `,/.` seek, manual
    scroll + resume, unsynced/loading/no-lyrics states, track change
    mid-glide, palette swap while playing.
  - **Now (partial):** L1 landed (`fe4f3da`) — the shared glide settles with
    an `Easing.OutBack` spring (`overshoot: 1.7`, ~4 px on a row, monotonic
    under held keys, edges clamped), used by every cursor list and the
    lyrics pane alike; the glide is also the default follow now
    (SCROLLMODELANALYSIS §3). Open: L0 dots, L2 addressable lines, L3 edge
    fades + elastic bounds, L4 karaoke.

### Scroll revamp (ARC-UI-7)

- [ ] **ARC-UI-7 · ScrollRevamp — audit follow-through on the four scroll components** — P2 · (analysisModal.md; ARC-UI-2, ARC-UI-3, VI-4, UI-54)
  - **Why:** `analysisModal.md` audits `ScrollContent`, `ListScroll`,
    `RowState` and `SearchContract` across performance, coherence and upgrade
    potential: the contract is sound, but one scalability fault (dense
    `rows()`), two cheap performance wastes (a memory write on every
    `contentY` pixel; a `sel.count()` call on every cursor move) and a set of
    upgrades have no owner. Left as prose they drift; this parent turns them
    into one ordered plan with one verification surface. The parent closes
    only when its children do.
  - **Details:** one owner for four components. Phase 1 = the three
    highest-priority findings (7.1–7.3) plus the rename/doc cleanup (7.4);
    Phase 2 = memory and navigation (7.5–7.7); Phase 3 = selection/search
    and the large upgrades (7.8–7.12). Every child keeps the existing
    contract: the body owns the cursor, the view is a follower, the host
    gates writes, and `/` stays viewport-scoped by default.
  - **Verify:** each child carries its own check; each phase gate is
    `cargo test`, `qmllint`, the QtQuickTest scroll/selection harness, and a
    manual pass over Albums/Songs/Artists, AlbumEntry, ArtistEntry, the queue
    drawer, the fullscreen queue and `/` in both list and grid modes.
  - **Now (partial):** 7.1–7.9, 7.12 and 7.13 landed: sparse `/` snapshots,
    durable/TTL scroll memory, page jumps, auto row extents, grid scrolloff
    wiring, selection signals + Shift block select, the public landing flag,
    and the offscreen warning/100k-row harness. The SearchColumn row-height
    loop and the RowBody incubation warning are fixed too. `qmllint`, the
    145-test suite and `scripts/check-scroll.sh` are green. Open: 7.10
    (`searchAll`/fuzzy needs a user-facing toggle and ranking decision),
    7.11 (per-scope layouts/named sets need a command surface), both large
    opt-ins, and 7.14 (Vi scrolloff follow, the structural fix for the
    centre-follow instabilities).

  - [x] **ARC-UI-7.1 · Sparse `SearchContract` viewport snapshot** — P2
    - **Why:** `rows()` allocates a dense array for every model row
      (`0..model.count-1`) and pushes `[""]` for everything off screen; on a
      10k+ library that is an O(n) allocation on every `/` open — the
      analysis's #1 action item.
    - **Details:** return a sparse result (`{first, last, rows: {i: cells}}`)
      or expose `rowsInRange(first, last)` and let `LineSearch` keep only the
      live band. Keep the both-corner viewport probe, absent-role `""` cells
      and the "scrolled-out never matches" rule. Adapt
      `LineSearch.open()/refresh()` (today it flattens a dense global row
      array with `_offsets`) and the `ScrollContent.searchRows()` facade;
      preserve the dense shape only where a caller still needs it, and
      document the choice. The incremental range idea from the analysis
      folds into this API.
    - **Verify:** `/` candidates, Tab cycling, `[i/N]` and the Enter jump are
      identical on list and grid; off-screen rows never match; a 10k-row
      model opens `/` without the full-count loop (harness/profiler);
      `SearchMatch.js` tests stay green.
    - **Now:** `rows()` returns a sparse array whose `length` is the model
      count and only the visible band carries cells; `LineSearch` maps each
      target by absolute row and `SearchMatch.candidates()` walks assigned
      keys. `scripts/bench-searchcontract.qml` exercises 100k rows (visible
      cells only, <100 ms guard) and is run by `scripts/check-scroll.sh`;
      `qmllint` clean.

  - [x] **ARC-UI-7.2 · Short-circuit `RowState.selectionCount`** — P3
    - **Why:** `readonly property int selectionCount: sel.count(root.cursor)`
      re-evaluates on every cursor move and calls `Selection.count()` even
      when no selection is active.
    - **Details:** bind `sel.hasSelection ? sel.count(root.cursor) : 0`; keep
      `hasSelection`/`selected` semantics and the range math unchanged.
    - **Verify:** the count badge stays 0 without a selection and tracks the
      anchor→cursor range when one is live; QtQuickTest selection cases pass.
    - **Now:** `RowState.selectionCount` short-circuits to 0 while no
      selection is live, so a cursor move no longer calls `sel.count()`;
      `qmllint` clean.

  - [x] **ARC-UI-7.3 · Debounce `ScrollContent` memory writes** — P3
    - **Why:** `Connections.onContentYChanged` calls `root.remember()` on
      every pixel of a manual scroll/flick; each call is a JS write through
      `ScrollMemory`, so a flick writes duplicate offsets every frame.
    - **Details:** coalesce the write behind `Qt.callLater` (or the shared
      short timer) and flush on teardown, layout switch, `resetView`,
      `resetViewNow` and the last position tick so the final offset is never
      lost; the cursor/count handlers keep their immediate save or share the
      same scheduler. Do not debounce explicit host `remember()` calls.
    - **Verify:** a long flick leaves one write per batch, not per pixel;
      switching away and back restores the final offset; order/filter resets
      still zero the memory; no stale write survives destruction.
    - **Now:** `Connections.onContentYChanged` schedules `_flushRemember()`
      through `Qt.callLater`; one pending write per event-loop batch,
      flushed on destruction, while explicit host `remember()` calls stay
      immediate; `qmllint` clean.

  - [x] **ARC-UI-7.4 · Rename `ListScroll` + document the bleed stage** — P3
    - **Why:** `ListScroll` is a memory service, not a scroll primitive; the
      name sends new readers to the wrong concept. The bleed geometry
      (`stage` offsets by `-bleed`, widens by `2 * bleed`, views inset by
      `bleed`, clip ownership flips to the stage) is correct but opaque.
    - **Details:** rename `ListScroll.qml` → `ScrollMemory.qml` (or
      `ScrollMemoryStore`) and update every reference (`ScrollContent`,
      UI-54 notes, tasklist); add a short comment above the stage explaining
      the stage-as-clip trick. Pure rename/doc: no behavior change.
    - **Verify:** `qmllint` clean; `rg ListScroll` finds only historical
      notes (or none); UI-54 remember/recall tests pass.
    - **Now:** renamed to `ScrollMemory.qml` (the JS module keeps its name);
      `ScrollContent` uses the new type, `rg ListScroll` finds only this
      plan's historical notes, and the stage-as-clip comment explains the
      bleed geometry. Behavior unchanged.

  - [x] **ARC-UI-7.5 · Durable scroll memory — persist, evict, richer recall** — P2
    - **Why:** `ScrollMemory.js` is engine-lifetime only, so every restart
      forgets cursor/offset/layout; the dict also grows without bound across
      visited albums/artists, and `recall()` returns a bare bool.
    - **Details:** serialize the store through `QtCore.Settings` (or a small
      backend slot writing JSON under `~/.config/musika`) on a throttled
      save, load it once at startup and keep the JS dict as the hot cache;
      stamp each entry and prune stale keys on load/open (TTL plus a cap);
      return `{ok, index, y}` from `recall` and update
      `ScrollContent.restore()`/callers. Search columns stay query-bound and
      out, as they are today.
    - **Verify:** scroll, quit, relaunch → cursor/offset/layout restored on
      Albums/Songs/Artists, AlbumEntry, ArtistEntry, queue drawer and
      fullscreen; a corrupt/missing store starts clean; stale keys evicted;
      callers can distinguish "no memory" from "memory applied".
    - **Now:** the JS dict is flushed through `Backend.saveScrollMemory()` to
      `scroll.json` under the config dir (atomic temp+rename, 500 ms
      debounce, teardown flush) and hydrated by `Backend.scrollMemory()` on
      first use. Entries carry `savedAt`, expire after 30 days and are
      capped at 200; `recall()` returns `{ok,index,y}` and `restore()` uses
      it. Smoke: page/queue entries landed in `scroll.json`; missing/corrupt
      files start empty (`qmllint` clean).

  - [x] **ARC-UI-7.6 · Page jump (`Ctrl-D` / `Ctrl-U`)** — P2
    - **Why:** long lists only have row steps, counts and `gg`/`G`;
      Neovim's half-page motion is the natural way to traverse a long
      album/song list.
    - **Details:** add `Action::PageDown`/`PageUp` bound to `ctrl-d`/`ctrl-u`
      (the tokenizer already emits both; `pageup`/`pagedown` may alias) and
      one `ScrollContent.pageJump(fraction)` that moves by half a viewport in
      rows (`columns`-aware in grid), clamps at the ends and reuses the
      glide/`reduceMotion`/memory rules. Host handlers follow `moveStep`'s
      return-value convention so Home can still hand `h` to the sidebar; do
      not collide with the VI-8 `d` operator.
    - **Verify:** `Ctrl-D`/`Ctrl-U` walk a long list by roughly half a screen,
      clamp cleanly at both ends, keep the cursor gliding, work in list and
      grid, and respect `reduceMotion`; neither token is bound elsewhere
      before landing.
    - **Now:** `Action::PageUp`/`PageDown` bound to `ctrl-u`/`ctrl-d` (plus
      `pageup`/`pagedown`); `ScrollContent.pageJump(0.5)` reuses `moveStep`
      (grid row-aware, clamped, glides). Wired through ScrollPanel,
      AlbumEntry, QueueDrawer, NowPlaying queue/lyrics, SearchColumn/Overlay
      and SideBar; keybind test covers the tokens.

  - [x] **ARC-UI-7.7 · Auto-calculate `contentHeightHint`** — P3
    - **Why:** fixed-row hosts must wire `count * adaptiveRowHeight`
      manually; a missed wiring makes the clamp/glide math fall back to Qt's
      lazy estimate.
    - **Details:** when `contentHeightHint` is 0 and `rowRatio > 0`, derive
      the extent from `count × adaptiveRowHeight` (plus `rowSpacing` where
      the view applies it) in one shared helper used by `_glideTo` and
      `_clampY`; keep the explicit override for variable-row hosts (lyrics,
      queue). Watch the binding loop: `adaptiveRowHeight` depends on
      `rowViewHeight`, not on the hint.
    - **Verify:** the fullscreen queue and other fixed-row hosts can drop the
      manual hint; first-open/goto/`G` clamps to the true end before
      delegates realise; variable-row hosts behave as today.
    - **Now:** `contentExtent()` derives the list extent as `count ×
      adaptiveRowHeight` (+ row spacing) and the grid extent as whole rows ×
      cell size, adding the scrolloff margin. The manual hints were removed
      from SearchColumn, AlbumEntry, QueueDrawer and NowPlaying; the
      explicit `contentHeightHint` override remains for variable rows. Live
      `contentHeight` is authoritative once measured; the derived extent is
      only the pre-layout fallback (the count multiplication overran the
      real 558-album extent by ~223 px and let `k` glide past the bottom).

  - [x] **ARC-UI-7.8 · Grid scrolloff parity** — P3
    - **Why:** `unlimitedScrolloff` uses ListView `header`/`footer` spacers,
      so the grid cannot centre its first/last row; the analysis flags this
      as a known list-only limit.
    - **Details:** branch the spacer strategy per layout (GridView has no
      header/footer; try Flickable `topMargin`/`bottomMargin` or a leading/
      trailing cell row), clamp with `originY` as the list path does, and
      keep the list behaviour untouched. Only advertise
      `unlimitedScrolloff` where the active layout supports it.
    - **Verify:** grid row 0 and the final row can centre like any other row;
      the list/lyrics pane still centres row 0 and the last line; `_clampY`
      never exposes empty space outside the intended margin.
    - **Now:** the grid carries the scrolloff as Flickable `topMargin`/
      `bottomMargin`; the list keeps its header/footer spacer. The extent
      math includes the margins. No grid host opts into `unlimitedScrolloff`
      yet (the lyrics pane stays list), so the grid branch is wired but not
      exercised in-app.

  - [x] **ARC-UI-7.9 · Selection extension — change signal + block select** — P2
    - **Why:** `RowState` has no `onSelectionChanged`, so hosts must poll to
      update a badge/status; the VI selection is anchor-only, with no
      Shift+Click / Shift+Arrow range extension.
    - **Details:** emit `selectionChanged` from `Selection` (or `RowState`)
      on start/all/clear/anchor moves; add `extendTo(index)` and wire
      Shift+Click on row delegates plus Shift+Arrow through new arrow-key
      bindings and the VI-2 mode machine (builds on the solved VI-4) while
      keeping `toggle`'s enter/exit contract intact.
    - **Verify:** a status badge updates without polling; visual mode still
      starts/ends on `v`; Shift+Click and Shift+Arrow widen the range from
      the anchor; the Alt batch verbs act on the same range; selection tests
      pass.
    - **Now:** `Selection` emits `selectionChanged` (start/all/clear and a
      live range moving with the cursor); `RowState` → `ScrollContent` →
      `ScrollPanel`/`SearchColumn` relay it. `extendTo()` plus
      `extendSelection()`/`extendSelectionTo()` back Shift+Arrow (new
      `extend_*` actions) and Shift+Click on MediaList, SearchColumn,
      AlbumEntry, ArtistEntry, QueueDrawer and the fullscreen queue.
      `scripts/check-scroll.sh` drives both paths; `qmllint` clean.

  - [ ] **ARC-UI-7.10 · Search modes — `searchAll` + fuzzy/scored matching** — P3
    - **Why:** `/` deliberately matches only the viewport, and matching is a
      literal smartcase substring loop in the host; power users cannot find
      off-screen matches and relevance is unranked.
    - **Details:** add an opt-in `searchAll` mode that ignores the viewport
      constraint (viewport-only stays the default and is never silently
      widened); move matching into `SearchContract` behind a scored/fuzzy
      matcher that returns ranked rows, with spans still coming from
      `SearchMatch.js`. Keep the "no match → E486, cursor untouched"
      contract and the `[i/N]` counter.
    - **Verify:** default `/` stays viewport-only; `searchAll` finds
      off-screen rows and jumps to them; ranked results still paint the
      Search/CurSearch blocks; zero-match feedback unchanged.
    - **Open:** needs a user-facing toggle (`?` is reserved for backward
      search) and a ranking decision for fuzzy spans; the sparse range API
      from 7.1 is ready for it. Land after VI-13/VI-15 or a search setting.

  - [ ] **ARC-UI-7.11 · Large-surface memory/selection upgrades** — P3
    - **Why:** the analysis lists two deliberately large extensions: layout
      memory keyed by scope as well as surface, and named selection sets
      ("mark group A/B" playlist assembly).
    - **Details:** extend `ScrollMemory` keys with `scope` (one surface can
      remember different layouts per sub-section) and add named selection
      sets on `Selection`/`RowState` (create/switch/clear, ids per set)
      without changing the default single-range path. Both are opt-in; no
      existing memory key or selection call changes meaning.
    - **Verify:** two scopes under one surface restore their own layouts; the
      default path is unchanged; named sets survive a cursor move and the
      Alt verbs still read the active set; tests cover create/switch/clear
      and the no-set fallback.
    - **Open:** both are opt-in APIs without a UI surface; named sets need a
      create/switch verb (VI-13's `:` line is the natural home) and
      per-scope layout needs a host with genuinely different sub-sections.
      No existing key or selection call changes until then.

  - [x] **ARC-UI-7.12 · Expose the landing state** — P3
    - **Why:** `_instantFollow` is private, so outer surfaces (overlay fades
      and other landing-sensitive chrome) cannot react to a landing without
      polling or re-deriving it.
    - **Details:** expose a readonly `landing`/`isLanding` property or
      `landingStarted`/`landingFinished` signals; keep `_instantFollow` as
      the writer and release it in the same tick. Add one consumer that
      actually needs it (e.g. an overlay fade) rather than shipping a dead
      API.
    - **Verify:** the consumer reacts once per landing, not per cursor move;
      no behaviour change for existing follow/glide; tests cover land/goto/
      reset paths.
    - **Now:** `readonly property bool isLanding` mirrors `_instantFollow`;
      the index/follow guards consume it instead of reaching into the
      private flag. The release tick is unchanged.

  - [x] **ARC-UI-7.13 · ScrollRevamp regression harness — warnings + large models** — P3
    - **Why:** `qmllint` and `cargo test` cannot see the two runtime
      regressions this pass already fixed — the SearchColumn row-height
      binding loop and the RowBody async Loader dying mid-incubation — and
      the 7.1/7.3 O(n) and per-pixel claims have no measured guard. The
      fixes currently live only in the parent's Now note.
    - **Details:** extend the headless QML harness (or add an offscreen app
      script) to open `/` and the global search in list/grid, exercise
      results, partial no-match, all-empty and a queue clear+refill while
      delegates load, and fail on `Binding loop detected`, `Object or context
      destroyed during incubation` or any QML warning. Add a 10k/100k-row
      fixture that times `SearchContract.rows()`/`/` open and counts
      `ScrollMemory` writes during a simulated flick.
    - **Verify:** the harness is green on the current tree and fails when the
      async Loader or the content-height baseline is reintroduced; the
      large-model numbers are recorded in the parent's Now.
    - **Now:** `scripts/check-scroll.sh` drives search/page/selection/
      fullscreen over the IPC socket offscreen and fails on binding-loop,
      incubation or JS Reference/Type errors. `scripts/bench-searchcontract.qml`
      opens 100k rows through `SearchContract.rows()` and guards the sparse
      snapshot (<100 ms; visible cells only); `check-scroll.sh` runs it when
      the QML runtime is on PATH. Memory-write counting was not added (7.3's
      coalescing has its own teardown/restore checks).

  - [ ] **ARC-UI-7.14 · ScrollContent remake — centred follow, clamped at the ends (no gap)** — P1 · (reported; SCROLLMODELANALYSIS §3, v0.3.1 baseline)
    - **Why:** the old body carried an edge-scrolloff experiment plus
      derived-extent repairs; height changes and small viewports kept leaving
      the cursor unfollowed.
    - **Idea (user call, revised):** rebuild the body around one rule — the
      cursor follows in the centre, but the viewport stops at the content
      bounds. Row 0 sits flush with the top and the last row flush with the
      bottom; near those ends the cursor moves inside the edge instead. No
      spacers, no empty gap.
    - **Details:** no header/footer spacers and no Flickable margins; the
      follow is a live frame retarget clamped to `[0, contentHeight - height]`.
      Normal moves glide over `followMs`; `gg`/`G`, search, restore and reset
      land instantly. `unlimitedScrolloff` is kept as a no-op for host
      compatibility. A resize snaps to the cursor, then re-snaps from
      `contentHeight` changes until the row unit settles. Memory, search,
      selection, adaptive rows and grid cursor movement are preserved.
    - **Verify:** down from row 0 pins the top (`contentY = 0`) until the
      cursor reaches the centre, centres while scrolling, then pins the last
      row flush at the bottom; tall/squeezed/variable-height lists and grids
      follow on every step; a height change keeps the cursor visible; `gg`/`G`,
      search, memory restore, selection extension and grid columns still land.
    - **Now:** rewritten in `src/qml/ScrollContent.qml`; `check-qml` and
      headless harnesses pass (list, grid, variable-height, small viewport,
      resize at top/middle/bottom). Awaiting a live pass over the five segments,
      album/artist pages, queue drawer and fullscreen queue. Memory recalls the
      raw saved `contentY`, so a re-opened row may not sit dead-centre.

### Search revamp (ARC-SEARCH)

- [ ] **ARC-SEARCH · SearchRevamp — a hybrid Nucleo + FTS index for the global palette** — P2 · (reported; S-8, S-9, PATH-10; DEPCRITIQUE)
  - **Why:** the palette still pays for three engines per keystroke: FTS5 over
    `search.db` (`src/search.rs`), a `LIKE '%…%'` full scan for artists/albums
    (`src/library.rs:335`), and a full scan of the FTS virtual table for the
    empty-query "Most played" surface (`src/search.rs:109`). The FTS query
    also sorts on an expression (`rank - play_count * 0.1`,
    `src/search.rs:95`), so FTS5 cannot stop after the first N matches. None
    of this involves files: `fd`/`fff` are filesystem walkers and have no
    place here — the catalog is already in SQLite, and shelling out would
    add a process, a sync mirror and stdout parsing for a worse result. The
    next step is an in-process fuzzy index (Nucleo, the matcher behind
    Helix) fed from `library.db`: ranked search over title/artist/album with
    typo tolerance, smartcase and Unicode normalization, and no SQL or IPC in
    the typing loop.
  - **Why Nucleo (and not fd/fff/fzf):**
    - `nucleo` 0.5 (`nucleo-matcher` 0.3 core) is the high-performance matcher
      used by Helix. Its high-level worker matches on a background threadpool
      and supports incremental matching (`MultiPattern::reparse(…, append)`),
      so extending a query reuses the previous candidate set instead of
      rescanning the catalog;
    - it scores fuzzy matches with word-boundary/prefix bonuses and returns
      match indices for highlighting; smartcase and diacritic folding are
      built in (`CaseMatching::Smart`, `Normalization::Smart`);
    - it is a library, not a subprocess: no tmpfs mirror, no `fd`/`fff`
      spawning, no process lifecycle, no filesystem to keep in sync;
    - license: MPL-2.0, compatible with this GPL-2.0-or-later project. The
      lock cost is small (`nucleo-matcher`, `rayon`, `rayon-core`,
      `crossbeam-deque/utils`; `parking_lot`/`memchr`/`either` are already in
      the tree). Record it in DEPCRITIQUE with the marginal count.
  - **Details:**
    - **Index (`src/search/index.rs`, new).** `SearchIndex` owns:
      - `Nucleo<Arc<SongHit>>` for songs (the only large list), one match
        column per item holding a haystack `title · artist · album` (fields
        joined by a separator; store each field's char range so match indices
        can be mapped back for highlighting);
      - `Vec<Arc<ArtistHit>>` / `Vec<Arc<AlbumHit>>` matched with
        `Pattern::match_list` (those tables are small; one `Matcher` per
        search thread, no extra worker threads);
      - `most_played: Vec<Arc<SongHit>>` sorted once at build
        (`play_count DESC, title`) so the empty query costs no scan;
      - `by_album: HashMap<String, Vec<Arc<SongHit>>>` sorted by
        `(disc_number, track, title)` for S-8;
      - `epoch: u64`, bumped on every rebuild so a payload can never mix
        catalogs.
      `SongHit` carries everything `search_json` needs from `library.db`:
      id/title/artist/album/duration/play_count/album_id/cover_art/starred/
      disc/track. Build it in one blocking pass over `library.db`
      (`song_search_rows` grows into a catalog query; artists/albums queried
      once). Store `Arc`s: snapshots and payloads only clone pointers.
    - **Query.** `Pattern::new(clean_query(raw), CaseMatching::Smart,
      Normalization::Smart, AtomKind::Fuzzy)` — literal fuzzy over
      whitespace tokens, preserving S-9's "punctuation is prose" contract;
      fzf operators (`^`, `'`, `!`, `$`) can be an opt-in later. Reparse the
      song worker with `append = true` only when the new text starts with the
      previous text; `tick(10)` until `status.running` is false (`Status` is
      a struct, not an enum), checking `SEARCH_GEN` between ticks so a
      superseded keystroke stops within one tick. Run the whole operation in
      `spawn_blocking`; `request_search` keeps its generation guard but no
      longer touches SQLite. The `Mutex<Nucleo<…>>` is unavoidable
      (`tick`/`reparse` take `&mut self`); artist/album matching is lock-free
      over immutable `Arc` slices.
    - **Ranking.** Songs: walk `snapshot.matched_items(..)` in score order;
      effective score = `score + PLAY_BONUS * min(play_count, CAP)`, with
      `PLAY_BONUS` calibrated so a strong text match still beats a heavily
      played weak one (the S-9 rule). Stop at 20 after the offline filter.
      Artists/albums: `Pattern::match_list` already returns score-ordered
      rows; truncate to 20. Offline filtering happens while walking the
      ordered hits (no rebuild on an offline toggle).
    - **Most played.** Empty/whitespace query returns the pre-sorted
      `most_played` prefix; no Nucleo tick, no SQLite read.
    - **S-8.** If exactly one album matched, the Songs column switches to
      `by_album[album_id]` (track order) instead of the fuzzy ranking; the
      payload keeps `album_context`. No DB round-trip.
    - **Payload.** Keep the existing `search_json` shape (`query`,
      `most_played`, `artists`, `albums`, `songs`) so QML is untouched in
      M1–M3. Only the cover URL join remains (`covers::url_map_of` over ≤20
      ids) plus the tiny `artist_user` overlay reads; the per-keystroke
      `LIKE`/FTS/most-played queries go away. Add an optional `spans` role
      per row (character ranges per displayed cell, derived with
      `Pattern::indices` on the top 20 only) so fuzzy matches still paint
      `SearchText` blocks; `SearchText` keeps its literal
      `SearchMatch.spans` as the fallback for the in-surface `/` search.
    - **Build/refresh.** Build lazily on first palette use (startup stays
      unblocked), then rebuild after every successful library sync in
      `index::rebuild_search` (rename to `rebuild_search_index`) on a
      blocking thread. A failed build logs and keeps the old index (stale
      until the next sync), matching today's "search is a disposable cache"
      contract. On logout/wipe, drop the index. Keep `search.db` untouched
      in M1–M3 as the fallback.
    - **Engine switch.** `MUSIKA_SEARCH_ENGINE=hybrid|nucleo|fts` — `hybrid`
      is the default: Nucleo answers first and FTS5 fills any column that
      came back short; `nucleo` and `fts` pin one engine. `search_json` falls
      back to FTS whenever the in-memory build or query fails. The merge is
      paid only for sparse queries, never for a broad hit set.
    - **Threads/memory.** `Nucleo::new(Config::DEFAULT, noop_notify,
      Some(4), 1)` — a small worker pool for songs (the 100k bench measured
      6 ms for a common prefix vs 19 ms on one thread); the notify callback
      only needs to wake the tick loop. Memory ≈ item structs + one UTF-32
      haystack per song (~4 bytes/char): measure at 10k/50k/100k and record;
      if the 100k figure is unacceptable, cap the index (e.g. 150k songs) and
      fall back to FTS.
  - **Milestones:**
    1. **M0 — baseline.** A `#[ignore]` Rust bench plus a small script over a
       synthetic 10k/100k catalog records p50/p95 for `search_json`
       (cold/warm), `most_played` and artist/album `LIKE`; record the numbers
       in **Now**. No production change.
    2. **M1 — index core.** Dependency + `SearchIndex` (build, query,
       most_played, by_album, artist/album matching); unit tests for fuzzy
       order, smartcase, diacritics, punctuation, play boost, S-8, offline
       filtering and append-reparse correctness.
    3. **M2 — wire behind the flag.** `request_search`/`search_json` read the
       index; rebuild on startup/sync; parallelize the remaining hydration
       reads (`try_join!`); smoke-test the overlay against `nucleo`.
    4. **M3 — parity.** Highlight spans, S-8/S-9 parity, offline, empty
       state, error fallback; lower the 120 ms debounce only if p95 says it
       is safe.
    5. **M4 — default + cleanup.** Flip the default, delete the dead FTS/LIKE
       hot paths (keep the migrations for a later cleanup), update docs and
       DEPCRITIQUE.
  - **Verify:** `rdo`/`radiohd` finds Radiohead (fuzzy); `bjork` finds Björk
    (normalization); `KID A` respects smartcase; punctuation still behaves as
    prose; one-album queries list tracks in track order (S-8); an equal-text
    tie is ordered by play count and a strong text match survives a small
    boost (S-9); offline shows only downloaded songs; the empty query paints
    the same Most-played rows with no DB read. At 100k songs, warm
    `search_json` p95 < 10 ms and `most_played` < 1 ms (baseline recorded in
    M0), with no work on the Qt thread and no `LIKE`/FTS query in the hot
    path; memory recorded. `cargo test`, `qmllint`,
    `scripts/check-scroll.sh` and the overlay smoke tests green;
    `MUSIKA_SEARCH_ENGINE=fts` still works.
  - **Risks:** fuzzy ranking changes habit (weak matches appear that FTS
    never showed); mitigate with the M0/M3 comparison and a conservative
    play boost. The high-level `nucleo` API is pre-1.0 (0.5), but the matcher
    core is stable and widely used. Memory is the real trade; the cap and
    fallback keep a large library usable. All `tick` loops live in
    `spawn_blocking`, so the Qt thread never waits.
  - **Now (partial):** M1 + M2 and the M4 default flip landed — `nucleo 0.5`
    is in `Cargo.toml`; `src/search/index.rs` builds the catalog from
    `library.db` (songs on a 4-thread Nucleo worker, artists/albums via
    `nucleo-matcher`), keeps `most_played` and `by_album` sorted, and
    `browse::search_json` serves the hybrid by default: Nucleo answers
    first, FTS5 fills any column that came back under 20, and
    `MUSIKA_SEARCH_ENGINE=nucleo|fts` pins one engine. `index::rebuild`
    refreshes the cache after a sync. S-8 (`album_context` + track order)
    and S-9 (play boost + Most played) ride the in-memory index;
    hearts/ratings are read live; every wipe clears it. Tests cover fuzzy
    matching, smartcase/diacritics, the play boost, Most-played order,
    track order, hybrid dedupe/order and one end-to-end `search_json`
    payload. M0 (index-only, release, 100k synthetic): build 38 ms, `rdo`
    3.4 ms, `track` 6.0 ms, `radio` 3.1 ms on a 4-thread pool; the FTS
    comparison baseline is still to record. Open: M3 highlight spans (fuzzy
    hits still use the client-side literal `SearchMatch.spans`), FTS
    cleanup.

### Incremental (ARC-I)

- [x] **ARC-I · Alt menu in the Search overlay** — P3 · (ARC-UI-1 follow-up)
  - **Why:** the Alt verbs stop at the lists, `ArtistEntry` and the fullscreen
    queue; search hits are the fastest path to a song/album/artist and should
    offer the same fixed menu instead of forcing Enter + navigation.
  - **Details:**
    - arm from `SearchOverlay` on the active column's cursor row
      (`songsColumn`/`albumsColumn`/`artistsColumn` via `activeColumn`),
      exposing a `quickAction(target, actions)` signal like
      `MediaList`/`NowPlaying`; `SearchColumn` relays it from
      `list.itemAtIndex(cursor)` + `model.get(cursor)`;
    - routing: `Home.beginQuickActions` must target the search while it is
      open, and `Home.armQuickActions`/`AltActionsOverlay.available` currently
      block on `Backend.searchOpen` — the search arm path has to bypass that
      without letting the scrim dismiss the overlay; `Esc` keeps
      `quick_cancel`;
    - verbs: songs → `QuickActions.song`, albums → `QuickActions.album`; the
      hits key their ids as `id` (not `songId`/`albumId`), so map at arm time
      or alias in `QuickActions`;
    - backend gaps: `browse::search_json` song rows need `album_id` for
      Go to Artist (same as `song_rows_json` in ARC-UI-1); artist hits need
      their own set — at least Favorite (`altFavorite` kind "artist" →
      `starArtist`, ARC-42) and Add to Queue ("artist" → every indexed song),
      with both jumps as open "—" slots;
    - the acrylic backdrop stays the search panel/scrim.
  - **Verify:** Alt arms on the active column's row; song hits show the four
    verbs (Go to Artist resolving), album hits keep Go to Album open, artist
    hits show the chosen subset; h/j/k/l aim, neutral fires nothing, Alt
    release invokes without closing the search; Esc still exits the overlay.
  - **Now:** landed — `SearchColumn` arms on its cursor row
    (`quickActionKind` song/album/artist through the recycled `QuickActions`)
    and `SearchOverlay` relays the active column; Home routes it to the shared
    overlay while the search is open (the overlay paints above the search
    panel, backdrop = the search, `available` no longer blocks
    `Backend.searchOpen`), and the two jumps close the search so the page is
    shown. The search song payload carries `album_id`
    (`library::song_album_ids`); artist hits queue every song of the artist's
    albums (`library::artist_songs` → `quick_songs("artist")`) and favourite
    through `altFavorite("artist")` → `star`, with both jumps as open "—".
    `cargo test` 114 pass; `scripts/check-icons.sh` ok; QML parses clean under
    `qmllint`. Open: the manual acceptance pass on hardware.

- [x] **ARC-I-2 · Alt menu in `AlbumEntry` tracks + the album's favourite mark** — P3 · (ARC-I follow-up)
  - **Why:** the album page's song list is the last track surface without the
    Alt menu, and the album's own favourite state is invisible there even
    though the fixed verbs can star it (`altFavorite("album")`).
  - **Details:**
    - `AlbumEntry` gets `armQuickActions()` on the song cursor —
      `songs.get(songList.currentIndex)` + `songList.itemAtIndex(...)`, with a
      `quickAction` signal and the `QuickActions.song` set; its rows need
      `songId`/`albumId` mapped from the payload (`load_album_songs_json`
      already emits them) and Home's `albumEntryComponent` relays
      `onQuickAction` like the other stack views; the toolbar cursor
      (`onToolbar`) is a no-op, empty lists too;
    - the header shows the album's favourite with the same white
      `baseline-favorite` leading the header meta; `queue_album_json` (the
      page payload: ARC-43 shape) must carry the album row's `starred`, and
      the Alt Favorite verb lights it optimistically (P-2 parity).
  - **Verify:** Alt on an album-page track arms the overlay on that row with
    all four verbs; the header heart matches the server album flag and flips
    with the Alt Favorite verb; the toolbar case is a no-op.
  - **Now:** landed — `AlbumEntry` arms on its song cursor (`songs.get` +
    `songList.itemAtIndex`; toolbar/empty no-op) with the `quickAction` relay
    wired into Home's `albumEntryComponent`, and its rows carry
    `songId`/`albumId`. The header heart rides `albumStarred`, threaded from
    every opener (`AlbumList`/`ArtistEntry`/`SearchOverlay` `albumActivated`,
    the ARC-43 `open_album` payload and the restored-session album all carry
    `starred`; `queue_album_json` emits it). ArtistEntry cards moved their
    favourite mark inside the cover's bottom-right corner, side derived from
    the cover (~30% over the old meta-line mark). `cargo test` 115 pass;
    `scripts/check-icons.sh` ok; qmllint clean. Open: optimistic toggle parity
    with `altFavorite`/P-2 and the manual pass.

- [ ] **ARC-I-3 · `ArtistEntry` action chips on keybinds** — P3 · (P-8 B–D, UI-44, ARC-42)
  - **Why:** `<stars> <download> <shuffle> <favorite>` are the only way to
    rate, cache or favourite an artist, and the product is keyboard-first —
    the chips must never require the mouse (the shuffle chip is inert today).
  - **Details:** keys follow the existing mnemonics/action-name scheme
    (`keybind.rs` `Action` + `Action::name()` consumed by
    `ArtistEntry.handleAction`, region-scoped like the list actions):
    - `s` toggles favorite (`starArtist`, the chip's optimistic flow);
    - `1`…`5` set the rating (`rateArtist`), `0` clears;
    - `c` toggles the cache rule (`downloadArtist`);
    - `z` shuffles the artist (queue every indexed track shuffled and start
      it — mpd/ncmpcpp's random key; `shift-s` is the fallback if `z` is
      claimed) and wires the inert shuffle chip;
    - each binding gets its insight/hint per UI-44 (footer or which-key), so
      no keybind ships without its visible counterpart.
  - **Verify:** from the artist page each key does exactly what its chip
    does (hover/press state reacts), `0` clears the rating, no binding leaks
    to other regions/`Backend.keyPending`/Alt aiming, and the chips still
    work by mouse.

- [ ] **ARC-I-4 · Favourite toggle: UI ↔ db ↔ server in one verb** — P2 · (UI-56, P-2, ARC-42)
  - **Why:** the UI-56 hearts reflect the last sync — `altFavorite` stars on
    the server without writing the local flag, and a second Favorite on an
    already-favourited row can't unstar, so the mark never tracks the user's
    action and the db drifts from the server.
  - **Details:**
    - db: on a successful star/unstar, write the local flag (`songs`/`albums`
      `starred` = now/NULL, or a small `library` helper); artist favourites
      keep their `artist_user` flow (ARC-42) and only need the toggle;
    - toggle: Favorite on a favourited song/album calls `client.unstar`
      (api.rs already wraps it) and clears the flag — songs, albums and
      artists (artist ids for the API, `artist_user` locally);
    - UI: flip the row's `starred` optimistically and reconcile on the
      payload (re-emit the affected list or update in place) so every region
      — Main lists, album page, search columns, ArtistEntry covers and the
      fullscreen queue (whose `player::Track.favorite` must follow) — shows
      the same state without a manual sync; revert on failure with the toast;
    - P-2's `s`/`*` binding reuses this one verb instead of a second path.
  - **Now (partial):** the decoration-only rule (no DB check; the next payload
    reconciles) landed on `ArtistEntry` album cards; the verb became a real
    toggle (`altFavorite(kind, id, on)` → star/unstar) with state-aware
    hearts (heart-add-fill / heart-minus, baked).
    **New piece (this batch):** the Alt favourite verb writes the target's
    state back into the owning scroll model *in place* — one shared
    `ScrollContent.markStarred(on)` (cursor row, or the whole selected range)
    invoked from every host's armed verb, so the affected rows' hearts track
    addition/removal with no reload: MediaList rows, SearchColumn, AlbumEntry
    songs, the fullscreen queue, ArtistEntry cards.
    Pending still: db writes (`songs`/`albums` `starred`), revert-on-failure
    with the toast, and `player::Track.favorite` following.
  - **Verify:** favouriting a song/album lights the mark immediately in every
    region and survives a restart (db); a second Favorite unstars on the
    server and db and the mark clears; a failed call reverts with the error
    toast; artists keep the chip/`artist_user` flow intact.

- [x] **CACHE-1 · DB4 `media.db` + `media/` blobs** — P2 · (roadmap Media cache; MEDIACACHESTUDY.md §3/§5)
  - **Why:** audio is never stored; playback dies without network. Research
    picked an independent reqwest fetch (`download` original / `stream` with
    transcode params) over mpv `--stream-record` (seek/switch breaks the file).
  - **Details:** schema `media(song_id PK, kind, file, size, quality,
    fingerprint, rule_id, last_used, created_at)` + `media_rule` + `media_meta`;
    blobs split permanent/rule (`~/.config/musika/media/`) vs playback/rolling
    (`~/.cache/musika/media/`); `.part` + atomic rename; `media::cached_path`
    and a batch resolver for `build_track`; purge only after a successful sync
    (covers.rs pattern).
  - **Now:** M0 landed — `migrations_media` (media / media_rule / media_meta
    with kind CHECKs and last_used/rule_id indexes), `Dbs.media`
    (`MUSIKA_MEDIA_FILE`); `src/media.rs`: tier split (`~/.config/musika/media`
    vs XDG `~/.cache/musika/media`, `MUSIKA_MEDIA_DIR` /
    `MUSIKA_MEDIA_CACHE_DIR` overrides), `record`/`store` (`.part` → atomic
    rename, replacing the previous blob), `cached_path`, `path_map` batch
    resolver, `touch`, `used_bytes`, the `fingerprint(size|suffix|bit_rate|
    server|quality)` helper, and `purge` (vanished songs + rule-orphans,
    hooked after the sync's destructive phase next to lyrics/colors). `queue.rs`
    `build_track` is local-first: a cached blob becomes a `file://` URL, else
    the stream URL (prefer-server-online is CACHE-6). 3 unit tests (fingerprint,
    tier split, purge). Open: the fetch/prefetch path (CACHE-2), caps
    (CACHE-3/4), rules (CACHE-5), status UI (CACHE-7).

- [ ] **CACHE-2 · Playback cache + prefetch next N** — P2 · (MEDIACACHESTUDY.md §4)
  - **Why:** stutter-free playback; Symfonium-style rolling buffer.
  - **Details:** reuse `Player::upcoming(n)` (lyrics already prefetches 3);
    trigger on load/next/prev/queueJump/track-change tick; bounded concurrency
    2-4 with an in-flight set; current uncached track fetched in parallel while
    it plays; prefetch never swaps the playing file, only future `loadfile`s.
  - **Now (partial):** the single-track path landed with CACHE-8: `downloadOne`
    in `qml.rs` checks the artifact first (cached blob → ledger `done`), skips
    an `active` fetch, otherwise marks → fetches the original (`download`
    endpoint) → `media::store` (explicit downloads are `permanent`) → ledger
    `done` / `error` with the reason (plus the error toast). Slots:
    `downloadSong` + the `download_song <id>` IPC route; `download` channel
    carries `{id, state}`. Open: prefetch next N on queue events, the shared
    in-flight set, playback-tier kind.

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
  - **Now (partial):** the artist container landed as the first slice:
    `media::set_rule/rule_for/delete_rule` (deterministic rule ids, selecting
    twice never duplicates), `downloadArtist(id, on)` marks every artist song
    (`library::artist_song_ids`) in the ledger and fetches the missing ones in
    `FETCH_CONCURRENCY` chunks through `downloadOne`; OFF deletes the rule, its
    marks and its rule-owned blobs (`media::remove_rule_media`; explicit
    permanent downloads have no rule id and survive). `requestArtistCache` +
    the `artist_cache` channel drive ArtistEntry's In-cache chip, whose three
    glyph states landed: empty `outline-download.svg` (unselected), the
    spinning `loading-loop.svg` (fetch pass in flight, `active` payload) and
    the filled `baseline-download.svg` in the MY3 accent (`Palette.tertiary`)
    once selected and finished. The sync's
    purge phase also runs `media::prune_uncovered`: a rule-owned blob whose
    song no longer falls inside a live artist/album rule is removed (scope
    changed / stale ref), while explicit downloads and kinds without a scope
    check yet stay untouched. Open: album /
    playlist / genre / favourites kinds, sync-time re-materialization of new
    content, and proper overlap pruning (currently a
    `remove_rule_media` shortcut refetches on re-selection).

- [ ] **CACHE-6 · Quality & network policy** — P2 · (MEDIACACHESTUDY.md §2/§8)
  - **Details:** transcode via `stream?maxBitRate=&format=` (fingerprint includes
    quality), `download` for original; prefer-server-online toggle; max parallel
    fetches; Wi-Fi-only has no desktop meaning until metering detection exists.

- [x] **CACHE-8 · `download.db`: download intent/state ledger** — P2 · (CACHE-1/2/5/7; MEDIACACHESTUDY.md §5/§6)
  - **Why:** `media.db` answers two of the three questions — `media` is what
    exists on disk, `media_rule` is which containers were selected — but
    per-song intent and progress (queued / in flight / failed-with-reason /
    done, plus the selecting rule and timestamps) had nowhere to live;
    CACHE-2's fetcher needs a work list, CACHE-5's materializer marks,
    CACHE-7's UI counts.
  - **Details:** DB9 `download.db`:
    `download(song_id PK, state CHECK queued|active|done|error, error,
    rule_id, requested_at, updated_at)` + indexes on `state`/`rule_id`; API
    `mark_queued` (done rows are never downgraded; re-selection clears a
    failure), `set_state`, `get`, `queued_ids(limit)` (oldest first),
    `counts` (queued/active/done/error/total), `unmark`, `unmark_rule`
    (removal drops pending rows, keeps done artifacts); no blob logic —
    artifacts stay media.db's; vanished songs pruned with the other caches
    via ARC-36's `cache::purge_missing`.
  - **Verify:** queued → active → done | error round trip; done never
    downgraded; counts drive "n of N"; rule removal drops only pending rows;
    vanished songs purged after a sync.
  - **Now:** landed — migration + `src/download.rs` (states, transitions,
    counts, 3 tests), tenth pool (`MUSIKA_DOWNLOAD_FILE`), sync-phase purge
    via `cache::purge_missing`. Open: CACHE-2's fetch loop writes it,
    CACHE-5 materializes rules into it, CACHE-7 renders the counts.

- [ ] **CACHE-7 · Cache status UI** — P2
  - **Details:** per-item state (cached/partial/queued) + used-bytes vs cap in
    the status bar; offline filter; permanent-tier warning threshold.

- [x] **CACHE-9 · Offline mode: downloaded-only library** — P2 · (request)
  - **Why:** with music in `download.db` the app should still open when the
    server is unreachable, and everything it shows must be playable — i.e.
    only what the download ledger marks done.
  - **Details:** offline-first auto-login — with a stored account and at
    least one `done` row in `download.db`, the app opens immediately as
    `Backend.offline` (every content surface filtered through the downloaded
    set: the Albums/Songs/Artists lists, albums and artists surviving only
    through downloaded songs with shrunken counts, album pages, ArtistEntry,
    search, the playback queues and the restored session queue/page), while
    the auth finishes **inside** the app; on success `noteOnline` flips the
    greeting to `Welcome, <user>`, clears the filter and reloads the full
    streaming library (the background sync then refreshes it). Without
    downloads the Login screen is the entry point and a failed manual login
    still falls back into offline mode when downloads exist.
  - **Verify:** with downloads and a dead server the app opens straight into
    `Offline, <user>`, lists show only downloaded songs/albums/artists and
    Enter plays from the local blobs; when the server is reachable the
    greeting becomes `Welcome, <user>` and the full library appears without a
    relaunch; without downloads the Login screen shows (prefilled); no
    loading screen or ping timeout exists.
  - **Now:** landed — `download::done_ids` + `library::downloaded_owners`
    (chunked), a shared `OfflineFilter` in `browse.rs` for all six loaders,
    `queue.rs` filtering (album, library, song-in-album, resume) and the
    `offlineLogin`/`noteOnline` slots wired into the offline-first startup
    (with downloads), the manual-login failure and the sync finish; on the
    online transition `noteOnline` reloads the library so the streaming
    catalog is available without a relaunch. Unit tests cover the ledger
    ids, the owner counts and
    the queue filter (111 pass). Caveat: availability follows `download.db`
    (a `done` row whose blob was purged elsewhere would fail playback —
    CACHE-3/4 keep the ledger and blobs consistent); covers/lyrics come from
    their caches when present.


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

### PATH-LIST — Performance & memory audit (2026-10-18)

Maintenance pass over the hot paths: unnecessary allocations, unbounded
state, async work that outlives its request, and full-table work per UI
action. Ordered by expected ROI; each item is independently shippable.

- [x] **PATH-1 · One shared API client** — P1 · (perf audit)
  - **Why:** every API action calls `client()` (`qml_backend/support.rs:21`),
    re-reads credentials from SQLite and constructs a new
    `reqwest::Client` (`http_client.rs:44`) — a fresh connection pool/TLS
    context per favorite, rating, download or metadata fetch.
  - **Details:** cache an `Arc<SubsonicClient>` (or a shared
    `reqwest::Client` plus credentials) behind the `Backend`; reset it on
    login/logout/account change. Route `browse::album_notes`,
    `queue::tracks_json` and `index`'s shared client through the same seam.
  - **Verify:** one pool reused across actions; no credentials query per
    call; repeated favorites/downloads show no new client allocations.
  - **Now:** landed — `SubsonicClient` clones one process-wide
    `reqwest::Client` (shared pool/TLS) and `support::client` caches one
    client per credential triple, invalidated on a full wipe.

- [x] **PATH-2 · Resolve only the payload's covers and media paths** — P1 · (perf audit)
  - **Why:** `covers::url_map` loads every cover and `media::path_map` loads
    every media row (and `is_file()`s each) on every list/search/queue
    payload (`browse.rs:62/87/100/114`, `queue.rs:459-460`).
  - **Details:** query the referenced cover ids only (the existing
    `library::song_cover_ids` shape) and the queue's song ids only; drop the
    per-row filesystem probe; chunk the `IN` lists. Cache the maps only if
    a payload genuinely needs the whole catalog.
  - **Verify:** memory/time per payload scales with the page, not the
    library; no stat storm while playing a large cached library.
  - **Now:** landed — `covers::url_map_of` and `media::path_map_of` query
    only the payload ids in 500-row chunks; the full-table maps are gone and
    queue paths no longer stat the filesystem.

- [x] **PATH-3 · Page the Songs list and hand the queue to the player in Rust** — P1 · (perf audit)
  - **Why:** `library::all_songs` materializes the whole table for the Songs
    tab (`browse.rs:83`), the library queue (`queue.rs:184`) and shuffle
    (`qml.rs:1969`); the queue then serializes to JSON, is parsed by
    `playerLoadLibrary` (`qml.rs:2148`), cloned by `emit_queue`
    (`qml.rs:2235`) and parsed again into the QML model
    (`NowPlaying.qml:297`). `songs_range` exists but is unused.
  - **Details:** serve the Songs tab in windows/pages (`songs_range` or a
    cursor window), build the player queue directly in Rust for
    `playSong`/shuffle (no JSON round-trip through QML), and publish queue
    changes by revision/delta rather than the whole row set.
  - **Verify:** 10k-song fixture: bounded payload memory, first paint not
    blocked by the full-table sort, queue load without a JSON detour.
  - **Now:** landed — `songs_page` serves `songs_range` pages (300 rows,
    preloaded through the remembered cursor) and `SongList` appends on
    approach; `playSong` still builds the full queue in Rust. Open: the
    queue's internal JSON hop and paging the queue model itself.

- [x] **PATH-4 · Keep the mark truth bounded and incremental** — P1 · (perf audit)
  - **Why:** `mark_states` (`qml.rs:124`) keeps every id ever queried;
    `rebuild_mark_sets` (`qml.rs:2385`) clones/sorts all states on every
    patch and the exposed vectors are searched by `indexOf` per visible row.
  - **Details:** retain only the union of active windows plus explicit
    per-id patches with a last-seen/generation stamp; update the
    done/cached/pending sets incrementally (membership sets, no full sort);
    make `RowMarks` membership O(1). Also fix
    `apply_downloaded_ids` so album-tracklist ids are not rewritten when
    they were not part of the requested song window
    (`qml_backend/download.rs:34-83`).
  - **Verify:** scrolling through the whole library leaves the map bounded;
    a rating/download patch costs O(changed), not O(history).
  - **Now:** landed — per-id patches update the exposed sets incrementally,
    window requests prune the mark map to the recent windows, and window
    reads rewrite only the requested songs (album tracklists feed coverage,
    not marks).

- [x] **PATH-5 · Coalesce and version the mark-window reads** — P1 · (perf audit)
  - **Why:** every cursor move calls `requestDownloads` (`MediaList.qml:127`)
    and spawns a DB read; fast scrolling stacks tasks and older snapshots
    can land after newer ones. `recent_mark_requests` also unions 12
    windows into one refresh (`qml.rs:2314-2330`) with unchunked `IN`
    lists (`download.rs:145/162`, `library.rs:200`).
  - **Details:** debounce/coalesce per surface, stamp each read with a
    generation and drop stale landings, refresh the latest window (or
    per-id deltas) instead of the 12-window union, and chunk the queries.
  - **Verify:** holding `j`/`k` through a long list stays responsive, no
    stale mark flicker, query size bounded.
  - **Now:** landed — requests accumulate into a pending union and a
    single debounced (60 ms), generation-stamped read applies it; a newer
    generation owns the result and retires only its own snapshot, so
    cursor sweeps and all visible surfaces coalesce without stale
    overwrites.

- [x] **PATH-6 · Make queue publishing cheap** — P2 · (perf audit)
  - **Why:** `emit_queue` clones the whole queue, queries the whole starred
    set (`library.rs:151`) and builds/hashes the full JSON on every publish
    (`qml.rs:2238-2280`).
  - **Details:** cache starred ids and update them from `favoriteChanged`;
    add a partial index on `songs(starred)`; skip the payload unless the
    row set changed (compare a row revision, not a full JSON string);
    expose an index-only/delta path for skips and advances.
  - **Verify:** skipping through a 10k queue does no full-table scan or
    full payload build.
  - **Now:** landed — the starred read is covered by the partial
    `idx_songs_starred` index; publishes still dedupe on the player
    revision and row hash (the queue clone/JSON build remains only for
    real row changes).

- [x] **PATH-7 · Move session-status serialization off the Qt thread** — P2 · (perf audit)
  - **Why:** `maybe_save_position` builds `session_status` on the Qt thread
    every 10 s while playing, including the full queue id JSON
    (`qml.rs:827/865/898`), before the debounce generation check.
  - **Details:** snapshot only the mutable scalar fields, mark the queue
    dirty on changes, and serialize the queue on the runtime thread after
    the debounce wins.
  - **Verify:** the 10 s tick does no O(queue) allocation on the UI thread.
  - **Now:** landed — the debounced writer clones the queue ids on the
    Qt thread but serializes the full id JSON in the runtime worker, after
    it wins the generation check; `flushStatus` keeps the last-chance
    synchronous path.

- [x] **PATH-8 · Serve album/artist metadata local-first with a TTL** — P2 · (perf audit)
  - **Why:** `requestAlbumInfo` (`qml.rs:1878` → `browse.rs:277`) and
    `requestArtistInfo` (`qml.rs:1523`) hit the server on every page open
    and fall back to the cache only on failure.
  - **Details:** emit the cached row immediately, refresh in the background
    behind a TTL (per album/artist), and coalesce duplicate opens.
  - **Verify:** reopening a page is instant/offline-safe; no request per
    navigation; refresh still lands when stale.
  - **Now:** landed — `album_info` is a new library cache (migration)
    and both album/artist metadata emit the cached row first, then refresh
    behind `INFO_TTL_SECS` (7 days); stale album rows are purged on sync.

- [x] **PATH-9 · Gate up-to-date cover revalidation** — P2 · (perf audit)
  - **Why:** an unchanged sync still runs `refresh_covers`
    (`index.rs:105-118`), issuing a conditional request for every
    referenced cover.
  - **Details:** only revalidate on first run, a library change, or a cover
    TTL / explicit refresh; report the skipped state in the sync message.
  - **Verify:** an up-to-date sync performs no cover fan-out; the next real
    sync still revalidates.
  - **Now:** landed — `covers_checked_at` in library meta gates the
    up-to-date sync: covers revalidate only when missing or older than
    `COVER_CHECK_TTL_SECS` (24 h), and a completed pass stamps the key.

- [x] **PATH-10 · Index and chunk the hot DB paths** — P2 · (perf audit)
  - **Why:** `all_songs` sorts the full table with no matching index
    (`library.rs:601-606`), `most_played` orders on an `UNINDEXED` FTS
    column, and several `IN` lists are not chunked (`cache::push_in_list`).
  - **Details:** add a composite index for the default song order and a
    partial index for `starred`; chunk `done/pending/album_song` ids (the
    500-row pattern from `downloaded_owners`).
  - **Verify:** `EXPLAIN QUERY PLAN` uses the indexes; large windows stay
    under the bind limit with bounded plans.

- [x] **PATH-11 · Trim QML hot paths** — P2 · (perf audit)
  - **Why:** `MediaList.qml:84` still logs on every mark request;
    `NowPlaying.applyQueue` builds an O(n) string signature
    (`NowPlaying.qml:306`) then rebuilds the whole model;
    `activeLyric` linearly scans all lines per position tick
    (`NowPlaying.qml:123`); `RatingBurst`'s particle system always runs
    (`RatingBurst.qml:48`).
  - **Details:** drop the debug log; send a backend row revision/hash and
    rebuild only on real changes; binary-search the active lyric; start/stop
    the particle system around bursts.
  - **Verify:** no per-move logging; queue apply is O(1) for index-only
    updates; lyric highlight stays cheap; idle particles cost nothing.
  - **Now:** landed — the per-move `console.log` is gone; queue
    payloads carry the backend row hash and QML compares that string; the
    active lyric uses binary search; `RatingBurst` runs its particle system
    only for 1.2 s after a burst.

- [x] **PATH-12 · Cache the offline catalog filter** — P2 · (perf audit)
  - **Why:** `browse::offline_filter` (`browse.rs:34-48`) reads the whole
    downloaded id set and owner maps for every offline payload and search.
  - **Details:** cache the filter in `Backend`, invalidate on download
    settles/sync/rule changes, or filter in SQL for the paged paths.
  - **Verify:** offline list/search paging does not re-read the whole
    catalog per request.

- [x] **PATH-13 · Batch the sync rating import** — P3 · (perf audit)
  - **Why:** `artist_user::import_ratings` (`artist_user.rs:104`) writes one
    row at a time on every sync.
  - **Details:** one transaction/batched upsert for the rated artists.
  - **Verify:** sync with many rated artists performs one write batch.
  - **Now:** landed — `import_ratings` filters the 0..=5 domain and
    writes one chunked, transactional multi-row upsert that preserves each
    existing `starred` flag.
  - **Now:** landed — one 2 s `Arc<OfflineFilter>` cache backs all
    offline payloads; download settles and sync reloads clear it eagerly.
  - **Now:** landed — `idx_songs_order` covers the default
    `(artist, album, disc_number, track, title)` sort; `done_ids_of`,
    `pending_ids_of` and `album_song_ids_of` now chunk their `IN` lists at
    500 ids.

- [ ] **PATH-14 · Shared GPU cover texture cache** — P3 · (cover review)
  - **Why:** `d238008` makes every thumbnail decode at its native 320 px edge,
    so the decoded pixmap is shared per cover, but each visible `CoverImage`
    still creates its own GPU texture node.
  - **Details:** a native texture provider / `QSGTexture` pool keyed by cover
    file, or a shared provider image with per-row texture coordinates; do not
    re-upload the same 320 px bitmap for every repeated album cover.
  - **Verify:** same cover in grid + search + queue uploads once; visual
    output unchanged; release build.

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

> **Easiest batch** — moved from Should have on 2026-10-14 (the smallest
> remaining work): UI-35, S-6, UI-42, CACHE-3, CACHE-4, P-1.

- [] **ARC-Whatever** While holding ALT in Search mode, Alt takes priority over the SearchBox, making it impossible to write text while using alt;

- [x] **UI-58 · Adaptive rows — cut a row past a 30% squeeze** — P1 · **top priority** · (design; window-height responsive; last design before the next release)
  - **Why:** every scroll list sizes its rows as a fraction of the live
    viewport, so shrinking the window squeezes the rows endlessly until the
    list is unreadable. The design rule instead: a row may lose at most 30%
    of its nominal (default-window) height; past that the squeeze trades
    whole rows — fewer, still-readable rows instead of a squashed list.
  - **Details:** one pure helper `Theme.adaptiveRow(viewHeight, ratio,
    baseline)` — the nominal row height while it stays ≥ 0.7× the captured
    baseline, else `viewHeight / max(1, floor(viewHeight / (0.7 ×
    baseline)))`, so rows oscillate in [0.7, 1]× the baseline and each step
    cuts exactly one row. `ScrollContent` captures each list's baseline on
    its first real measure and exposes `rowRatio`/`adaptiveRowHeight`
    (`ScrollPanel` forwards; MediaList 0.105, fullscreen queue 0.16). Hosts
    whose row height is page-relative capture their own baseline and call
    the same helper (AlbumEntry songs 0.055 of the page, SearchColumn
    columns 0.12, QueueDrawer rows 0.06 of the panel).
  - **Verify:** harness cases for the helper (in-band scaling, the cut on
    the 30% step, never above nominal, floor ≥ 0.7×) plus a PTY smoke;
    manual: resize the window down through several steps and watch rows
    drop one by one instead of squeezing.
  - **Now:** implemented — `Theme.adaptiveRow`, `ScrollContent` baseline
    capture + `rowRatio`/`adaptiveRowHeight` (`ScrollPanel` forwards;
    MediaList 0.105, fullscreen queue 0.16), host-level baselines for
    AlbumEntry/SearchColumn/QueueDrawer; harness case + PTY smoke green;
    the band edge sits at 0.82 (ARC-UI-3, 40% sooner). Landed (`42e4a6a`,
    `5a4cc4f`). 2026-10-20: the row-count step (and its resize jump) was
    removed — the row now clamps continuously at 0.82× and the panel snaps its
    body to whole rows instead; guarded by `scripts/bench-adaptive-row.qml`.

- [x] **UI-59 · Transport cluster: one horizontal row, squeeze-proof** — P1 · (design QuickWin; user-found while squeezing the bars)
  - **Why:** previous/pause/next must always read as one horizontal line. Today
    the play button carries a larger preferred size (InferiorBar 0.56 vs 0.46
    of the bar height; fullscreen 0.058 vs 0.05) and the cluster lives between
    fill zones / an absolute-x box, so when the bar or pane is squeezed the
    pause drifts out of alignment with the skips — precisely when space is
    tight, the thing that must stay crisp goes off-axis.
  - **Details:** one shared horizontal `Row` for the three everywhere they
    appear (InferiorBar centre zone; NowPlaying fullscreen transport bar), with
    the icons pinned to a common centre line and the play on the row's
    horizontal centre; make the buttons unsqueezable (`Layout.minimumWidth/
    Height` = preferred) so a tight layout elides/clips the surrounding zones
    first (the time box already elides before the volume cluster) instead of
    deforming the cluster; keep the fullscreen yield rule intact (the cluster
    slides left only when the volume would touch it, UI fix from
    2026-10-15).
  - **Verify:** squeeze both bars down through `compact` → `narrow` — the
    three stay on one line with steady relative sizes and the pause centred;
    no overlap with the time/volume clusters at any width.
  - **Now:** landed (`9841d14`, `d3f4b46`) — `TransportControls.qml` is the
    one previous/play/next core: common centre line, play at 1.22× the skips
    and the core centre, minimum == preferred so a tight bar clips around the
    cluster; the skip/play press feedback moved inside so both hosts dropped
    their copies. The repeat control is an accessory in the left gutter and
    never changes the core's implicit width, so play stays centred. The
    player bar and the fullscreen transport instantiate it (unit 0.46× /
    0.05× bar height). Open: the manual squeeze pass compact → narrow.

- [x] **UI-60 · Merging Favorite System** — P2 · (reported)
  - **Why:** favourite and rating are two grammars for one piece of user
    state: every row wears a heart, but Navidrome's 0..5 stars are only
    reachable by clicking `ArtistEntry`'s five-glyph cluster (UI-52/ARC-42)
    — mouse-only on a keyboard-first player — and albums have no rating
    affordance at all even though the server exposes `setRating` for songs,
    albums and artists. The rating should ride the favourite mark itself.
  - **Details:**
    - one Favorite mark: the heart wears a small `number-0..5.svg` badge in
      its bottom-right corner (0 = unrated, drawn with the number-0 glyph);
      favourite and rating stay independent — unstarring keeps the rating,
      rating never flips the favourite;
    - the Alt-hold overlay's south slot (`QuickActions.song`/`album`/
      `artist`, and `forSelection`) becomes rating-aware: releasing Alt over
      it keeps today's mere favourite toggle (the normal action == release),
      while aiming at it with Shift held turns `J`/`K` into rating down/up
      (`k` = +1, `j` = -1, clamped 0..5) and the badge updates live; `J`/`K`
      must not leak to the queue reorder while the overlay owns the hold;
    - the same badge rides the stationary favourite marks that mirror the
      state (`RowMarks` row heart, `ArtistEntry`'s Favorite chip,
      `AlbumEntry` header / fullscreen keybar heart), so a rating is visible
      while browsing, not only inside the overlay;
    - merge (retire) `ArtistEntry`'s separate five-star cluster: the
      Favorite chip becomes the single favourite + rating control; keep the
      cluster's hover/click star preview only if it survives as a second
      pointer path to the same state;
    - backend: artists already store `rating` (`artist_user`) with
      `rateArtist` + the `artist_user` payload; add the same 0..5 user
      rating for albums (and songs, if the shared Favorite mark covers track
      rows) — new `rateAlbum`/`rateSong` slots write the server through
      Subsonic `setRating`, index locally, emit the affected row/user
      payload on success and `errorRaised` on failure; album/song payloads
      (browse, search, queue as needed) gain `rating` so rows re-render
      without a full reload;
    - domain: 0..=5, optimistic on the keypress and reconciled by the
      emitted payload; no badge behaviour when the server user lacks the
      `setRating` capability.
  - **Verify:** keyboard-only pass: aim Alt south on an album/artist row,
    tap `Shift-K` five times → the badge climbs 1..5, `Shift-J` walks it
    back to 0 (the number-0 glyph) without unfavouriting; releasing Alt alone
    toggles only the favourite; the value survives a restart (local index)
    and matches Navidrome's `userRating`; the pointer path in `ArtistEntry`
    still works if kept; `qmllint` clean and the overlay still cancels over
    neutral.
  - **Now:** landed — albums gain a `rating` column (migration) fed by
    `AlbumID3.userRating` on sync; `rateAlbum` writes the server/local row and
    emits the new `ratingChanged` per-row patch plus `album_state` (rating
    included); artists reuse `artist_user`, with the index's `userRating`
    imported on sync. `RatingBadge.qml` (`number-0..5.svg`) rides `RowMarks`
    (album/artist rows only — song rows keep the heart alone), `MediaList`'s
    grid cells, `ArtistEntry`'s favourite chip and album cards,
    `AlbumEntry`'s header, the fullscreen keybar heart and all search rows;
    `ArtistEntry`'s five-star cluster is retired (the chip's outline/filled
    star is the one control). The Alt overlay's south slot carries the seed
    rating + badge and Shift-J/K steps it (180 ms debounced, release flushes
    and never double-fires the favourite); the Artists list now arms the
    overlay too. The "bright" FX (`RatingBurst.qml`, QtQuick.Particles) fires
    a few stars from the aimed heart on every step, above the overlay so the
    burst outlives the tile collapse. `cargo test` 141 pass, icon check
    clean, QML parses, an offscreen run opened the album/artist pages without
    QML warnings. Open: the live keyboard pass on Navidrome and capability
    gating for servers without `setRating`.

- [x] **UI-56 · Microdesign: favorites lead with a heart before their meta** — P3 · **top priority** · (design microtask; P-2, ARC-42)
  - **Why:** every album, song and artist row shows its `<track amount>` /
    `<track duration>` / `<album quantity>` meta with no way to tell a
    favourited row from a plain one — the heart reads only after opening the
    row (ArtistEntry chip) or arming the Alt overlay, so favourites are
    invisible while browsing. The data is already cached: `library.db` writes
    `starred` for all three tables (`library.rs:60-117`) and `artist_user.db`
    tracks the artist favourite (ARC-42), but the row structs (`AlbumRow`,
    `SongRow`, `ArtistRow`), `browse.rs` JSON and the QML row models drop it.
  - **Details:**
    - regions (all row surfaces): the Main lists (`MediaList` →
      `AlbumList`/`SongList`/`ArtistList`), `AlbumEntry`'s song list,
      `ArtistEntry`'s album cards, the fullscreen playback queue in
      `NowPlaying`, and all three search columns;
    - backend: select `starred` into the three row structs, thread it through
      `browse.rs` (`album_json`/`song_rows_json`/`artist_json`) as a boolean
      (`"starred": starred.is_some()`; artists: OR the `artist_user` flag so
      both surfaces agree), the search payloads (`search.rs` song hits need
      the library lookup `search_json` already does for `album_id`) and the
      queue (`player::Track` gains a `#[serde(default)]` favourite field, as
      `album_id` did in ARC-UI-1); unit test the JSON key on a starred
      fixture;
    - QML: `MediaList`'s delegate gains a `required property bool starred` and
      the heart is **anchored at the left edge of the meta label** (meta stays
      right-aligned; the title/subtitle's right anchor already points at the
      meta label, so nothing else moves): `Icon` (`name:
      "baseline-favorite"`, `symbolColor: Theme.favoriteStar`, fixed
      `0.8 × meta font` frame, meta-label spacing) visible only when
      `row.starred`; the wrappers (`AlbumList`, `SongList`, `ArtistList`) map
      the payload flag into their rows and set `starred: album.starred` etc.;
      the same spawn/spacing lands in `SearchColumn`, `AlbumEntry`'s song
      rows, `ArtistEntry`'s album cards and the `NowPlaying` queue delegate;
    - placement contract: the glyph sits in front of (left of)
      `<track count> tracks` / `<track duration>` / `<album count> albums` in
      every region; the search artist column carries no meta, so the heart
      leads its count subtitle instead;
    - the heart is not interactive (no hitbox, no cursor change, hover/cursor
      block unchanged) — it is a state mark, not a button;
    - live parity: `altFavorite` on a song/album stars on the server but the
      list payload is not re-fetched, so either toggle the row's flag locally
      when the call succeeds and let `onDataReady`/`onLibrary_changed` resync,
      or map `altFavorite` to unstar too (api.rs already wraps `unstar`) so
      the mark can be lightened as well as lit; artist rows read the
      `artist_user` payload.
  - **Verify:** star an album/song/artist on the server, sync, then browse:
    the heart leads the meta on exactly the favourited rows in the Main lists,
    the album page, `ArtistEntry`'s cards, the fullscreen queue and all three
    search columns, and unstarred rows show none; long titles/durations keep
    elide and the meta never shifts; hover, cursor, click and keyboard flows
    are unchanged; `mapRow`/JSON test passes and `qmllint`/offscreen run is
    clean.
  - **Now:** landed — `starred` rides every row payload (`AlbumRow`/`SongRow`/
    `ArtistRow` selects, `player::Track.favorite`), `browse.rs` emits
    `"starred"` for albums/songs/artists (artists OR the `artist_user`
    overlay) and the search payload joins it back from the library
    (`library::song_starred_ids`); the white `baseline-favorite` heart
    (`Theme.iconStrong`) leads the meta in `MediaList` (Albums/Songs/
    Artists), `SearchColumn` (artists via the count subtitle), `AlbumEntry`
    songs and the fullscreen queue — and sits inside the bottom-right corner
    of `ArtistEntry`'s covers (cover-derived side) — no hitbox, no gap when
    unstarred. `cargo test` 115 pass (row-JSON flag test);
    `scripts/check-icons.sh` ok; QML parses clean under `qmllint`. Open:
    optimistic toggle parity with `altFavorite`/P-2 and the manual pass.

- [x] **UI-57 · Responsive title ramp for list headers (Albums/Songs)** — P2 · (quick win; UI-56)
  - **Why:** in compressed (narrow) viewports the shared list header kept its
    full footprint: the title sat at `Theme.fontSize("headline", …)` while
    the band stayed at `headerRatio` (0.2 of the panel), so the title + hint
    row squashed the scroll body below.
  - **Now:** `Theme.widthScale(width)` is the one shared width ramp — 1 at
    `Theme.breakpoints.compact`, tapering linearly to 0.7 at the `narrow`
    breakpoint ArtistEntry uses. `ScrollPanel` scales its title
    (`clamp(full * widthScale, 18, full)`: the 18 px headline floor, never
    body-text weight, via the same linear-toward-a-clamp mechanism as
    `ArtistEntry`'s description) and its band
    (`headerRatio * widthScale`, with the default `headerRatio` cut 30%,
    0.2 → 0.14), so the reclaimed height goes to the list. The same ramp now
    also scales the status bar's identity/report type (`SuperiorBar`) and the
    now-playing title/artist/time (`InferiorBar`). Full-size viewports are
    byte-identical; the ramp is continuous in width, so resizing or crossing
    the breakpoint never jumps, and the right-anchored Shift-H/Shift-L cue and
    selection badge stay centred in the shrinking band. Applies to every
    `ScrollPanel` list (Albums/Songs/Artists) by construction. `qmllint`
    clean; offscreen run silent; **visual pass pending** (dials: the 0.7
    floor and the narrow→compact span).

- [x] **UI-13 · Scroll position hint** — P3 · **removed 2026-10-14**
  - **Why:** long lists have no scrollbar/position feedback for mouse users.
  - **Details:** one reusable `ScrollHint.qml` (opacity/transform only, never
    part of layout) attached to each long list: a slim bar pinned to the
    list's left edge, thumb height `max(h * h / contentHeight, h * 0.02)`,
    offset from `visibleArea.yPosition`, width
    `Math.max(1, h * Theme.hairlineRatio * 0.5)`, `Palette.outlineVariant` at rest
    and `onSurfaceVariant` while scrolling or hovered; fade `quick`, hidden
    when `contentHeight <= height`.
  - **Targets:** `MediaList` (Albums/Songs/Artists), `SearchColumn`, the
    fullscreen queue and lyrics in `NowPlaying.qml`, `QueueDrawer`,
    `AlbumEntry`'s song list, `ArtistEntry`'s album grid.
  - **Verify:** long lists show a proportional thumb that tracks wheel,
    trackpad and keyboard moves; short lists show nothing; no cursor/hover/
    click behaviour changes; `reduceMotion` shows/hides it without fading.
  - **Now:** retired on request (2026-10-14). The white thumb was the last
    iteration; `ScrollHint.qml` and every instantiation were deleted
    (ScrollPanel's `showHint` went with it), so lists are clean at the edge.
    Rationale: the hint read as chrome noise beside the keyboard-first
    cursor, and the vi motions make position feedback less necessary.

- [x] **UI-40 · Palette transition wrapper (finish THM-4)** — P2 · (POLISHAUDIT §1.3)
  - **Why:** `Palette.apply` snaps every colour in one frame; THM-4 promised
    an animated swap.
  - **Details:** a QML `Palette` wrapper (or per-role Behaviors) animating
    role changes `micro`/`standard`, no animation on first apply/reset.
  - **Now:** landed without a QML wrapper — a QML singleton named `Palette`
    can never shadow QtQuick's `Palette` attached type (probed with `qml`;
    only the C++ registration can), so the crossfade lives in `Main.qml`,
    where the `colors` payload already arrives: `fadePalette` snapshots the
    painted roles, `pushPalette` mixes old/new hex maps and re-applies them
    through `Palette.apply` on a `standard`-tier NumberAnimation, so a
    re-target mid-fade continues from the painted colours. First apply,
    reset/no-art and `reduceMotion` snap; `paletteCamel` maps the payload's
    snake_case roles (`inverse_on_surface` now updates — a latent key
    mismatch in `palette.rs` is fixed). Offscreen harness verified snap,
    fade, mid-fade retarget, reset and post-reset fade.

- [x] **UI-41 · List scroll easing + payload crossfades** — P3 · (POLISHAUDIT §4)
  - **Why:** keyboard moves teleport and model swaps flash.
  - **Details:** animate small `contentY` corrections for search/queue,
    crossfade lists on payload swap (`quick`), keep lyric follow in LYR-8.
  - **Now:** `ListScroll.qml` eases small `contentY` corrections
    (`quick`, Bezier standard, half-row context; instant `ListView.Contain`
    fallback for unrealised rows and under `reduceMotion`), wired into
    `SearchColumn`, `MediaList` (Albums/Songs/Artists), `AlbumEntry`,
    `ArtistEntry`, the `QueueDrawer` (reorder-guarded) and the fullscreen
    queue; payload swaps crossfade `quick` in the search columns and both
    queues, with queue reorders staying live. UI-54 layered the scroll
    memory on the same instance (`remember`/`recall`). The deeper
    distance-aware engine (frame-timing, follow guards, reuseItems) is
    UI-46.

- [ ] **LYR-8 · Smooth centre-follow + edge fades** — P2 · (roadmap "Lyrics presentation" P1)
  - **Why:** `positionViewAtIndex(Center)` teleports each line — once when a
    payload lands (NowPlaying.qml:156-157) and again on every active-line
    change (NowPlaying.qml:171-172), which fights manual scrolling whenever
    the 500 ms `Backend.position` tick moves `activeLyric` under the user's
    hand.
  - **Details:** the active phrase stays centred; animate `contentY` to the
    active delegate's centre (`standard`/`decelerate`, `quick` for tiny moves);
    pause follow on `onMovementStarted` and resume after 5 s idle or on track
    change — the idle wait drops to 2 s whenever the lyrics pane is not the
    focused/highlighted section (cursor elsewhere or no pane highlight), so
    re-centring happens sooner when the user is not reading there; top/bottom
    gradients from `surfaceContainerLow` to transparent.
  - **Verify:** the active line glides into centre; manual scroll is not
    yanked while playing; follow resumes after 5 s idle (2 s with the lyrics
    pane unfocused) or on track change; unsynced lyrics stay static;
    gradients only over the scrollable range; `reduceMotion` keeps an instant
    `positionViewAtIndex`.
  - **Now (partial):** the lyrics list rides the shared scroll body in
    `glideToCenter` mode — a harness probe showed the highlight-band follow
    snaps one-line moves instantly (ApplyRange enforces the band), so the
    body now animates the viewport itself to the active delegate's centre
    over `followMs: 300` (the pane fade shares the value and the
    decelerate bezier; variable heights measured, instant `Center` only for
    unrealised lines, `followSnapRows: 0`, drags stop it via
    `movementStarted`). j/k, drag and wheel pause it (`holdLyrics`), and a
    single-shot `lyricResume` timer re-centres the active phrase after the
    idle window — `Theme.lyricsResume` 5 s while the lyrics pane is focused,
    `Theme.lyricsResumeUnfocused` 2 s otherwise, re-armed when pane focus
    moves and dropped on track change. Crispness pass (`37294d3`): lyric
    lines use `Text.CurveRendering` and the masked stage layer is sized in
    device pixels. Open: edge fades (dropped after the first review pass) and
    a desktop pass for the timer feel.

- [x] **S-9 · Empty-query search surface + usage boost** — P2 · (roadmap mixed design, "Empty state" + "Selection, scope and ordering")
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
  - **Now:** landed — the `song_search` FTS table carries `play_count`
    (new migration recreates it; the up-to-date sync branch self-heals an
    empty index from the unchanged library, so an existing install never
    loses search), `SongSearchRow`/`rebuild` copy it, `search()` orders
    `rank - COALESCE(play_count,0) * 0.1, rank` and `most_played()` orders
    count desc then title. `search_json` returns `{query, most_played, …}`
    for the empty query (with per-song `play_count`); the overlay opens,
    clears and re-fills on that surface: the Songs column becomes "Most
    played" (play-count meta) with the album/artist columns and dividers
    hidden, `moveColumn` is a no-op there, zero-hit denial is skipped, and
    stale responses are dropped by the echoed query. Unit tests cover the
    boost (a strong match survives a small count, enough plays overtake),
    most_played ordering and the rows' play count. A new `build.rs` watches
    every `migrations_*` directory so cargo rebuilds when a migration is
    added/edited (sqlx's `migrate!` does not track them, which silently kept
    the old schema in a running binary).

- [x] **AUTH-3 · Login error/status roles** — P2
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
  - **Now:** landed — the wordmark, fields, button and status use the MY3
    type roles (`Theme.fontSize`, no raw type fractions left); the status is
    a role container (`errorContainer`/`onErrorContainer` logged out,
    `primaryContainer`/`onPrimaryContainer` on success, hairline border,
    `Theme.radius`, wrapped) whose height is reserved even when empty, so the
    form never jumps. Live login round-trip for the AA-contrast check is
    deferred to a desktop session.

- [x] **DOC-1 · Keep design docs in sync with the tree** — P2 · (A10)
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
  - **Now:** the bounded pass landed — status headers on `CRITIQUE.md`,
    `roadmap.md` and `materialyou3idea.md`; the `TASKS.md` reference became
    `TASKLISTMAJOR.md`; the duration/ratios/palette rows and the
    missing-roles paragraph now match `Theme.js`/`palette.rs` (motion tiers,
    breakpoints, 32 roles); CRITIQUE's key-dispatch claim now names
    `keybind.rs` (counts/operators still open); POLISHAUDIT §1.1 close fades,
    §1.3 palette snap, §1.4 loading bar and §4 list easing are ticked as
    consumed. Note: these docs are `.gitignore`d (`*.md` except
    `TASKLISTMAJOR.md`), so this pass is local-only by design.

- [x] **UI-47 · Microdesign: search column rule draws behind the next column's covers** — P3 · (design microtask)
  - **Why:** the structural rule (`|`) between search columns is a RowLayout
    sibling declared *before* the following column, so that column's covers
    paint over it — the divider is created behind the album cover.
  - **Details:** give the two divider Rectangles in `SearchOverlay.qml`
    (`columnsRow`, `:484-488` and `:508-512`) `z: 1` (or move them after the
    columns) so they paint above covers; keep them non-interactive and
    layout-neutral.
  - **Verify:** with results in every column the rules stay continuous over
    covers while scrolling; covers/rows stay clickable.
  - **Now:** both dividers carry `z: 1` (comments name UI-47); layout and
    hit-testing untouched. Visual confirmation with results in all three
    columns is deferred to a desktop session.

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

- [x] **UI-52 · ArtistEntry identity strip: biography slot + action chips** — P3 · (design microtask; ARTISTENTRYSTUDY.md §2, P-8, ARC-42)
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
  - **Verify:** at ≥narrow the header shows the biography over the chips on
    the right with no overlap with the avatar/name at compact widths; hovering
    a chip tints it and the cursor is a pointing hand; the favourite star
    toggles (gray ↔ `Theme.favoriteStar`) and star clicks set/clear the
    rating, both reconciled by the backend payloads; below narrow the strip is
    gone and the header reads exactly as today; biography text wraps to two
    lines and elides; `reduceMotion` still fades nothing.
  - **Now:** the strip landed (`ActionChip` inline component,
    stars · In-cache · Shuffle · Favorite; hover preview + hitbox slots; D-5
    ambience) and its metadata is wired to ARC-42: the entry requests
    `requestArtistUser` + `requestArtistInfo` on open, applies the
    `artist_user`/`artist_info` payloads (id-guarded), toggles the favourite
    star optimistically (`Theme.favoriteStar` yellow lit) and sets the rating
    by clicking a star (clicking the lit top clears to 0); the backend
    reconciles the optimistic state through the same payload on success and
    failure (error toast rides `errorRaised`). In-cache and Shuffle stay
    inert placeholders until their backends (CACHE-1/5, Q-3); keybar
    `s`/`1..5`/`c` and the rating capability gate belong to ARC-42.

- [x] **UI-53 · Header composition standard: one major + three containers** — P3 · (design standard; UI-52, ARTISTENTRYSTUDY.md §2, P-8)
  - **Why:** the artist header was a flowing `RowLayout`: a long biography
    negotiated width against the identity column and, when the description
    was allowed to expand, the whole segment grew and squashed the name/count
    (and the grid below via the layout). Containers with fixed anchors scale
    predictably — each zone owns its box and nothing to fight over.
  - **Standard (the issue):**
    - one **major rectangle** owns the header box: transparent, height = the
      identity cover/avatar side; it is the only element the outer layout
      negotiates with;
    - **left container**: anchored left/top/bottom, **30% of the major's
      width**; owns the round avatar (responsive:
      `min(avatarSide, 50% of the container)`) over the title + count (both
      elide; the identity can shrink but never be squeezed by the text zone);
    - **right stack**: the remaining width, from the left container's right
      edge to the major's right edge, split **70% height** (long-form text:
      biography) over **30% height** (action row); both transparent with
      fixed anchors, so the two never negotiate with each other;
    - **container-driven text sizing**: the 8 px floor defines the rectangle
      capacity via `FontMetrics` (chars per line × lines that fit at 8); the
      font ramps **8..14** by fill:
      `clamp(8 + (1 − len / capacity) × 6, 8, 14)` — short text grows toward
      14, text that exactly fills the box sits at 8, overflow clamps and
      elides;
    - below `narrow` the right stack hides and the left identity keeps the
      major.
  - **Applies to:** the ArtistEntry header now; future headers (AlbumEntry
    D-3, NowPlaying identity) adopt the same three-container rule.
  - **Verify:** a long biography fills its 70% box without moving the
    identity or the major's height; the ramp lands between 8 and 14; the
    chips stay inside the bottom 30%; identity wins below `narrow`.
  - **Now:** `ArtistEntry` header reworked to the standard: `header` (major)
    with `identityBox` (30%, avatar + name + count), `descriptionBox` (70%
    height, FontMetrics capacity ramp 8..14 from the box's own height/width)
    and `actionsBox` (30%, the star row + chips, vertically centred); the
    avatar is `min(avatarSide, 50% of the identity box)` so the 30% column
    never overflows; the right stack hides below `narrow`.

- [x] **UI-54 · Session scroll memory per page** — P2 · (request)
  - **Why:** the segment pages and the entry pages are destroyed when you
    leave them, so coming back reset the cursor and the viewport: the
    highlight was not visible at the point where the user stopped.
  - **Details:** the `ListScroll` `remember`/`recall` pair over the
    `ScrollMemory.js` `.pragma library` store (engine lifetime): `remember`
    saves cursor + `contentY` (and the focused scope where one exists),
    `recall` clamps the index to the fresh row count and restores the exact
    offset in one step (a `Qt.callLater` retry only when the list had no
    real geometry yet), so no intermediate `positionViewAt` frame can flick
    before the offset lands; the first payload after a reopen restores,
    later reloads keep the live state.
  - **Verify:** scroll a segment, switch away and back → cursor and offset
    return; reopen an album/artist page → its list/grid returns to where it
    stopped; reopen the queue drawer / fullscreen → highlight and pane
    return; order changes and resync reloads do not jump the list.
  - **Now:** landed through the shared `ListScroll` seam: Main segments
    (`albums`/`songs`/`artists`, saved continuously), `AlbumEntry`
    (`album:<id>`), `ArtistEntry` (`artist:<id>`), the queue drawer
    (`queue`, restored on open instead of snapping to the playing track)
    and the fullscreen player (`fullscreen` queue + the `pane` scope,
    restored on re-entry). Offscreen harness covers remember/recall, the
    scope field and fresh keys; search columns stay query-bound and are out.

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
    VI-8a removed the bare `n`/`p` aliases (`p` is queue paste-below now,
    `n` is reserved for VI-14); `Ctrl-N`/`Ctrl-P` are the standard.

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
    (ARC-43 later re-homed the Alt chords to the quick-action overlay, so the
    pane hint is `Shift-J/K move` and reorder is Shift-only.)

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

- [x] **ARC-8 · Merge duplicate sync/login pipelines** — P1 · (O3)
  - **Why:** the same client → `index::sync_library` → report orchestration
    was written three times in `qml.rs` (`login()` via `spawn_index_sync`,
    `startup()` via the same, and the manual `syncLibrary` slot), so fixes
    (re-entrancy, offline flag, toasts, progress) had to be applied per copy.
  - **Now:** landed — one `spawn_sync(finish, progress, client, emit_started)`
    free function owns the whole pipeline: optional `librarySyncStarted`
    emission, the progress closure, `index::sync_library`, the "library
    updated" log, `noteOnline` on success (CACHE-9), the `sync_summary`
    finish and the failure toast. Login and cached auto-login pass
    `SubsonicClient::new` from their raw credentials with
    `emit_started: true`; the manual slot keeps its re-entrancy guard and
    synchronous "Syncing…" state (UI-30) and passes the DB-loaded credentials
    with `emit_started: false`. `spawn_index_sync` deleted; the manual path
    now also logs the report and flips offline back on success. `cargo test`
    119 pass.

- [x] **ARC-7 · Trim `api.rs` to used endpoints** — P0 · (O2)
  - **Why:** `api.rs` was 1894 lines behind `#![allow(dead_code)]`: an endpoint
    layer for families the app never calls (playlists, shares, podcasts,
    jukebox, chat, radio, users, video, scanning, bookmarks, now-playing,
    similar/top songs, searching — FTS is local since S-9).
  - **Now:** landed — 1894 → **514 lines**; the file-level allow is gone and
    `cargo check` is warning-free. Compiler-guided fixpoint (temporary
    `#![warn(dead_code)]` + a brace-matching stripper) removed 121 items in
    one pass: 65 dead `SubsonicClient` endpoints, their response types (49
    structs), two parameter enums with impls, and the dead helpers
    (`string_or_number`, `search_params`, `album_list_params`, `append`,
    2× `as_str`); orphan section banners pruned; rustfmt clean. The live 14
    remain (`ping`, `get_indexes`, `get_artist(_info/_info2)`,
    `get_album(_info2)`, `get_lyrics(_by_song_id)`,
    `get_open_subsonic_extensions`, `star`, `unstar`, `set_rating`,
    `stream_url`). Tests: the one dead-type fixture (`ArtistsID3`) was
    rewritten against the live `Indexes` shape; 117 pass.

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
  - **Removed:** the loading-screen path was deleted (CACHE-9: offline-first
    auto-login — no countdown, no ping timeout); the Login screen remains the
    entry point for accounts without downloads.

- [x] **UI-35 · Queue drawer modal parity: slide + scrim + blur** — P2 · (POLISHAUDIT §1.2, D-2)
  - **Why:** the drawer is modal for keys but has no scrim and the shell blur
    ignores it (`Home.qml:254`), so cursors behind it stay visible — D-2 says
    an open overlay owns the only visible selection.
  - **Details:** translate the panel in/out (`standard`), add the scrim at
    `Theme.scrimOpacity`, include `queueDrawer.opened` in the shell layer
    condition.
  - **Now:** landed (`9841d14`-era) — the drawer slides from the right edge
    over `standard` (root visibility follows the slide) and carries the
    shared scrim with click-outside-to-close; the acrylic panel already owns
    the blur (UI-55 — the shell layer stack was retired with Ctrl-W), so no
    layer condition is needed.

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

- [ ] **CACHE-3 · Rolling cache (LRU, max size)** — P2
  - **Details:** cap configurable (default ~2 GiB), evict LRU by `last_used`;
    playback tier capped separately (~512 MiB); permanent/rule exempt.

- [ ] **CACHE-4 · Permanent cache + "Move to permanent" pin** — P2
  - **Details:** pin drops `kind='rule'`/`rule_id` and moves the blob to the
    config-root `media/` (never auto-removed; uncapped, warn threshold in UI).

- [ ] **P-1 · Scrobble back to Navidrome** — P2 · (M5, CRITIQUE Ph3.2)
  - **Why:** play counts / recently played never update server-side.
  - **Details:** `scrobble` at 50%/4 min and on completion; `api.rs:1318` ready.

### Recycling (REC-1 – REC-27)

> **All findings initialised** (2026-10-14) from `RECYCLECRITIQUE.md` (tree
> `e9419c9`); the critique's `REC-*` ids are canonical here. Phases: **0**
> mechanical dedup (REC-5…15, 17, 18) · **1** API seam (REC-1…4) · **2** QML
> macros & plumbing (REC-19…23, 25, 26) · **3** cross-module merges (REC-16,
> 24, 27). Work in phase order; every finding is independently shippable and
> behaviour-preserving. Verify with `cargo test` (111) + `cargo clippy` per
> commit; phases 2–3 also need a QML smoke run (search palette, album page,
> queue drawer metadata, palette swap) — tests cannot catch a renamed QML
> method or JSON key. REC-24 must reconcile `song_count`/`tracks` with the QML
> readers **before** extraction.

- [x] **REC-1 · API envelope helpers (`get_key`/`get_key_opt`/`call_ok`)** — P2 · (RECYCLECRITIQUE §A, phase 1)
  - **Why:** the `call::<Value>` + `extract` two-step repeats across ~43
    methods (49 `extract` + 2 `extract_opt` pairs) and 24 command endpoints
    end `map(|_| ())` — the single biggest repeat in the tree.
  - **Details:** add the three helpers beside `extract`/`extract_opt`/
    `one_or_many` (`api.rs:38-89`); convert the ~75 call sites; zero call
    semantics change.

- [x] **REC-2 · `push_all(params, key, values)` for multi-value params** — P3 · (RECYCLECRITIQUE §A, phase 1)
  - **Why:** `for id in ids { params.push(("id", id.to_string())) }` appears 9×
    (`api.rs:1236,1260,1263,1398,1428,1544,1743,1826-1833`).
  - **Details:** one helper taking `impl IntoIterator<Item = impl ToString>`;
    no call-site behaviour changes.

- [x] **REC-3 · Bounded fan-out `map_bounded(ids, f)`** — P2 · (RECYCLECRITIQUE §A, phase 1)
  - **Why:** `index.rs::fetch_artists` (`:310`) and `fetch_albums` (`:344`)
    are 32-line clones differing in one method call; `covers::refresh` and
    `lyrics::backfill` repeat the same chunk/`JoinSet`/`join_next` scaffold
    (~18 lines each).
  - **Details:** keep `FETCH_CONCURRENCY` chunking and per-item error
    collection in the helper; callers keep their `ensure_complete`/progress
    logic and pass only the fetch fn.

- [x] **REC-4 · HTTP pipeline + cover URL builders** — P3 · (RECYCLECRITIQUE §A, phase 1)
  - **Why:** `http_client.rs:101/123/166` each repeat
    send → `status().is_success()` → body; `api.rs:1289/1320` rebuild the same
    cover/download params that `http_client.rs:122/152` rebuild again.
  - **Details:** `get_checked(url) -> Result<Response>` (304 stays the
    cover-art special case); `download_media` takes the client's own
    `download_url`.

- [x] **REC-5 · Lyrics `Line` ↔ text codec** — P3 · (RECYCLECRITIQUE §A, phase 0)
  - **Why:** the same `lines.iter().map(...).join("\n")` / split appears 5×
    (`lyrics.rs:105,135,360,515,664`).
  - **Details:** `lines_to_text(&[Line]) -> String` and
    `text_to_lines(&str) -> Vec<Line>`.

- [x] **REC-6 · `delete_by` for the static `DELETE`s** — P3 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** the seven static `DELETE FROM <table> WHERE <col>` blocks repeat
    the same execute envelope (`covers.rs:177`; `media.rs:272,292,364,434`;
    `lyrics.rs:316,387`).
  - **Details:** `async fn delete_by(pool, sql: &'static str, key: &str)`;
    SQL stays literal per call site — never interpolate identifiers.

- [x] **REC-7 · `SONG_SELECT` + `song_order()`** — P2 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** the 4-line column list + `LEFT JOIN albums` repeats 9× and the
    three `ORDER BY` strings twice (`library.rs::all_songs` `:468-495`,
    `songs_range` `:539-566`, `song` `:571`, `songs_by_ids` `:635`,
    `album_songs` `:655`, `song_ordinal` `:504-522`); the next ORDER change
    must touch every copy.
  - **Details:** `const SONG_SELECT` and `fn song_order(order) -> &'static str`;
    expect ~80 lines removed.

- [x] **REC-8 · `ALBUM_SELECT`/`ARTIST_SELECT` + order** — P3 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** 11 album projections (`library.rs:261,386-408,432-442,605`),
    4 artist column copies (`:249,455,458,618`) and 9 order strings repeat the
    REC-7 shape.
  - **Details:** `const ALBUM_SELECT`, `const ARTIST_SELECT`,
    `album_order(order, by_artist)`.

- [x] **REC-9 · `db_path!` + `open_migrated`** — P2 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** 11 one-line path wrappers (`db.rs:136-174`), 3 env-override-else-
    `config_dir` copies (`:182-205`), 2 XDG-else-HOME copies (`:207-228`) and
    10 `open_file` + `sqlx::migrate!().run()` pairs (`:65-95`).
  - **Details:** `macro_rules! db_path`, `fn dir_from_env(env, fallback)`,
    `async fn open_migrated(path, max_connections, migrations, ignore_missing)`;
    removes the `set_ignore_missing` special case buried in `init()`.

- [x] **REC-10 · Upsert pairs (`artist_user`, `download`)** — P3 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** `set_starred`/`set_rating` (`artist_user.rs:38/56`) and
    `mark_queued`/`set_state` (`download.rs:54/77`) each rewrite the same
    `INSERT … ON CONFLICT DO UPDATE` COALESCE-preserve-column shape.
  - **Details:** `upsert_artist_user(pool, artist_id, starred, rating)` and
    `set_download_state(pool, song_id, state, …)`; callers pass only the field
    they change.

- [x] **REC-11 · Dynamic SQL assembly (`push_in_list`/`insert_many`)** — P2 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** 4 copies of the `builder.separated(", ") … push_bind …
    push_unseparated(")")` `IN`-list (`library.rs:203,284,634,697`) and 4
    row-by-row insert loops in a transaction (`library.rs:58,73,93`,
    `search.rs:57`); only `downloaded_owners` chunks at 500 today, so the rest
    can drift into oversized statements.
  - **Details:** `push_in_list(&mut qb, &ids)` and
    `insert_many(tx, const SQL, rows, bind)`; centralising makes chunking
    consistent.

- [x] **REC-12 · Test memory pools** — P3 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** 8 copies of `SqlitePoolOptions::new().max_connections(1)
    .connect_with("sqlite::memory:")` plus 5–6 `sqlx::migrate!` runs
    (`library.rs:823`, `search.rs:147`, `status.rs:97`, `artist_user.rs:123`,
    `download.rs:192`, `cache.rs:72`, `media.rs:455`, `lyrics.rs:711`) — test
    code is where the next schema change will hurt.
  - **Details:** one `#[cfg(test)] pub(crate) async fn memory_pool(migrations)`
    in `sqlite.rs`; ~50 test-only lines.

- [x] **REC-13 · Small persistence drifts** — P2 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** `library.rs:810` `unix_now()` duplicates `consts::now_secs`
    (`consts.rs:27`) with a different return type; `db.rs:112` vs
    `library.rs:786` both do `create_dir_all(parent).with_context(...)`;
    `library.rs:130` `touch_last_sync` vs `:794` `set_meta` are the same
    statement on different executors; `.bind(bool as i64)` at
    `artist_user.rs:47`/`status.rs:72` casts sqlx already does.
  - **Details:** delete `unix_now` and bind `now_secs()` (add
    `now_secs_text()` if the five `.to_string()` sites annoy);
    `ensure_parent_dir(path)`; `upsert_meta(executor, key, value)`; drop the
    bool casts. Reconcile the drifts before extracting.

- [x] **REC-14 · `index.rs` volume (`report`/`best_effort`)** — P3 · (RECYCLECRITIQUE §B, phase 0)
  - **Why:** 8 five-line `SyncProgress` literals
    (`index.rs:72,87,123,139,158,181,230,259`, `total` always 100) and 7
    fail-log-continue blocks (`:176-228`).
  - **Details:** `report(progress, message, done)` and
    `best_effort("lyric cache purge", fut)`; ~34 lines and the "always 100"
    assumption lives once.

- [x] **REC-15 · Socket plumbing (`config_socket`, `spawn_line_reader`)** — P2 · (RECYCLECRITIQUE §C, phase 0)
  - **Why:** `player.rs:487` and `ipc.rs:36` derive the same mpv socket path;
    `player.rs:391,482` and `ipc.rs:53` remove stale sockets; `player.rs:515`
    `spawn_reader` and `ipc.rs:70` `serve` are the same line-reader loop.
  - **Details:** `db::config_socket(name) -> PathBuf` and
    `ipc::spawn_line_reader(stream, on_line)`.

- [x] **REC-16 · Raw mpv commands** — P3 · (RECYCLECRITIQUE §C, phase 3)
  - **Why:** 9 sites push `json!(["set_property", …])` / `observe_property`
    by hand (`player.rs:219,220,240,248,349,359,452`); 7 sites take
    `self.shared.lock().unwrap()`; 3 sites clear the same `position/eof/error`
    trio (`:206,255,559`).
  - **Details:** `Player::set_property(name, value)`,
    `Player::observe(id, name)`, `Shared::reset()`, `Player::shared()`;
    command IDs stay literal — only the envelope dedupes.

- [x] **REC-17 · MPRIS minors (`put_nonempty`, `submit!`, `mpris_script`)** — P3 · (RECYCLECRITIQUE §C, phase 0)
  - **Why:** `mpris.rs:258-289` repeats the
    `if !empty { map.insert(key, owned(Value::from(clone))) }` shape six
    times; `:437-459` six action methods are one `submit("<verb>")` each; the
    `MUSIKA_MPRIS_SCRIPT` empty-check is duplicated at `mpris.rs:144` and
    `player.rs:407`.
  - **Details:** `put_nonempty(map, key, &str)`, a `submit!`-style macro and
    `mpris_script() -> Option`.

- [x] **REC-18 · State naming (`State::mpris_status()`)** — P3 · (RECYCLECRITIQUE §C, phase 0)
  - **Why:** `player.rs:64` `State::name()` and the rematch to
    `"Playing"/"Paused"/"Stopped"` in `qml.rs:2202-2206` are the same table.
  - **Details:** one `State::mpris_status()` so the strings live once.

- [x] **REC-19 · Palette role application (`apply_roles`)** — P3 · (RECYCLECRITIQUE §D, phase 2)
  - **Why:** `apply_role!` (`palette.rs:107-116`) is invoked 32× in `apply`
    (`:341-498`) and `restore!` (`:506-513`) is a copy of the same macro
    invoked 32× in `reset` (`:515-546`) — the role list exists twice.
  - **Details:** `fn apply_roles(&mut self, roles: &BTreeMap<String, String>)`;
    `apply` calls it and `reset` builds the default map once; ~36 lines.

- [x] **REC-20 · Enum string tables (`name_enum!`)** — P3 · (RECYCLECRITIQUE §D, phase 2)
  - **Why:** `seed.rs:69` `SourceMode`, `scheme.rs:42` `Spec`,
    `scheme.rs:78` `VariantChoice`, plus `scheme.rs:259` `variant_name`
    duplicating `VariantChoice::name`, all hand-roll `name()`/`from_name()`
    with the same `trim().to_ascii_lowercase()` prelude.
  - **Details:** one `name_enum!` macro removes ~25 lines and the fourth copy
    of the variant names.

- [x] **REC-21 · Colors minors (`spec_scheme`, `rgb` codec, `build_scheme`)** — P3 · (RECYCLECRITIQUE §D, phase 2)
  - **Why:** `DynamicScheme::from_spec(hct, variant, is_dark, Some(contrast),
    Platform::Phone, SpecVersion::Spec2025)` is written 7×
    (`scheme.rs:164-243`); `fetch_into` (`engine.rs:193`) and `build_scheme`
    (`:238`) construct `CoverScheme` twice; hex/ARGB conversion is split
    between `seed.rs:95,363`, `engine.rs:350` and `scheme.rs:327`.
  - **Details:** `spec_scheme(hct, variant, version, contrast, is_dark)`;
    `fetch_into` calls `build_scheme`; one `rgb` codec module.

- [x] **REC-22 · QML async-slot prelude (`spawn_data`)** — P2 · (RECYCLECRITIQUE §E, phase 2)
  - **Why:** 9+ slots repeat the same 15-line shape
    (`qml.rs:1069,1148,1170,1192,1220,1420,1519,1537,1551`): invoker, `dbs`
    clone, `runtime().spawn`, `emitData`/`eprintln!`.
  - **Details:** `spawn_data<F, Fut>(&self, channel, load)` where
    `F: FnOnce(Dbs) -> Fut`, `Fut: Future<Output = anyhow::Result<String>>`;
    keep a second path (or a bool) for slots that raise `errorRaised`; ~100
    lines, every new slot becomes one line.

- [x] **REC-23 · QML property plumbing (`qprop!`, `sync_field!`)** — P2 · (RECYCLECRITIQUE §E, phase 2)
  - **Why:** 31 `qproperty!` declarations (`qml.rs:194-319`), 29 matching
    `#[qsignal]` stubs (`:334-403`) and 3 hand-written setters (`:321-332`)
    must be written twice because of the derive; `sync_player` (`:2054`) adds
    21 more compare/assign/notify blocks.
  - **Details:** `qprop!(server_url, "serverURL", write)` collapses each
    property to one line; `sync_field!` covers `sync_player`; ~130 lines.
    Medium risk around macro hygiene and the `Notify` signal names — do it
    property-by-property with `cargo test` after each.

- [x] **REC-24 · Row→JSON builders + the `song_count`/`tracks` drift** — P2 · (RECYCLECRITIQUE §E, phase 3)
  - **Why:** `artist_json` (`browse.rs:172`), `album_json` (`:186`) and
    `song_rows_json` (`:202`) are re-implemented inline in `search_json`
    (`:297-339`); `queue_album_json` (`queue.rs:134`) copies `album_json`;
    `emit_queue` (`qml.rs:1771`) and the mpv metadata (`player.rs:195`) repeat
    the same key sets. The copies disagree on the count key — browse emits
    `"song_count"`, queue emits `"tracks"`, and QML reads both per context
    (`SearchOverlay.qml:273` and `ArtistEntry.qml:229,235` read `song_count`;
    `Home.qml:578,600` read `tracks`).
  - **Details:** reconcile the key in QML **first**, then give
    `AlbumRow`/`ArtistRow`/`SongRow`/`player::Track` a `qml_json(covers, …)`
    in one module; ~60 lines and the next key addition happens once.

- [x] **REC-25 · Error/emit plumbing (`raise`, `emit`)** — P3 · (RECYCLECRITIQUE §E, phase 2)
  - **Why:** `errorRaised` + `format!("...{err:#}")` appears 12×
    (`qml.rs:1246,1274,1306,1329,1408,1453,1596,1628,1666,2312` …) and
    `emitData` 23×.
  - **Details:** `raise(&invoker, kind, &err)` and
    `emit(&invoker, channel, payload)`; the 10 invoke sites become one line
    each; ~30 lines.

- [x] **REC-26 · Session, sync and auth handshake** — P3 · (RECYCLECRITIQUE §E, phase 2)
  - **Why:** `queue_status_save` (`qml.rs:709-753`) and `flush_status`
    (`:770-794`) build the same `SessionStatus` field-by-field; the
    `SyncProgress` closure (`:990`, `:2277`) and its `librarySyncFinished` +
    `raiseError("sync")` branch (`:1008`, `:2305`) are identical; `login`'s
    completion (`:443`) and `startup`'s (`:539`) run the same
    `noteServerType` → `noteOnline` → `spawn_index_sync` → `authenticated(...)`
    chain.
  - **Details:** `session_status()`, `sync_progress_cb(invoker)` and
    `authenticated(...)` helpers; ~50 lines, medium risk (QML-visible).

- [x] **REC-27 · Small QML clusters** — P2 · (RECYCLECRITIQUE §E, phase 3)
  - **Why:** `client(dbs)` already exists (`qml.rs:37`) but is re-inlined in
    `sync_library` (`:984`), `request_lyrics` (`:1872`), `browse::album_notes`
    (`:226`) and `queue::tracks_json` (`:297`); the offline filter is spelled
    out in `browse::offline_filter` (`:36`), `queue::filter_offline` (`:62`)
    and `queue::resume_payload` (`:215`); order defaulting
    `if self.x_order.is_empty() { ORDERS[0] }` appears 7×
    (`:484,1075,1154,1176,1198,1616`) and `stepped_order`/
    `stepped_artist_order` (`:2227-2243`) duplicate `library::cycle_order`/
    `cycle_artist_order` (`:361-378`); `star_artist` (`:1236`) and
    `rate_artist` (`:1288`) are 45-line clones; `v.max(0) as usize` 5×
    (`:1690,1715,1801,1808,1816`).
  - **Details:** make `client` `pub(crate)` and call it;
    `offline_song_ids(dbs, offline)`; one
    `effective_order(orders, current, delta)` in `library.rs`; one
    `apply_artist_user(...)`; `fn index(v: i64) -> usize`; ~80 lines.

## Solved (reference)

Kept for rationale; the tree already carries these.

- [x] **VI-1 · Mode indicator in the status bar** — P1 · (N4 partly, MY3 QW10)
  - **Why:** Vi users must always know the mode; silent mode changes break trust.
  - **Details:** colored `NORMAL`/`INSERT`/`VISUAL`/`COMMAND` chip in the bar.
  - **Now:** `NORMAL` chip replaced the "LIBRARY" title (RightBar.qml); the
    search overlay shows an `INSERT`/`NORMAL` chip too.

- [x] **VI-4 · `v`/`V` content selection** — P1 · (CRITIQUE Ph1.2/§2.5, N1)
  - **Why:** visual mode must mean "select rows", not "pick a pane"; batch
    queueing/deleting depends on it.
  - **Details:** row selection with `secondaryContainer` + count badge;
    operators (VI-8) act on the selection; the Alt menu adapts (batch "Add to
    Queue" live, single-row verbs gray).
  - **Now:** the selection state lives in the shared scroll model
    (`ScrollContent`/`Selection`), so every list carries it: Albums/Songs via
    `MediaList`/`ScrollPanel`, `AlbumEntry`, the three search columns and the
    fullscreen queue (`ArtistEntry`'s grid keeps its own — GridView has no
    follow API). `v` toggles at the cursor, `j/k`/`gg`/`G` extend, `V`
    selects the list, Esc clears first, and entering another surface
    (activation, search, sidebar, segment) trashes it. Selected rows wear the
    cursor fill; a ratcheting `SelectionBadge` shows the count without
    reflowing on every move. Alt on a selection shows the batch verb set
    (`QuickActions.forSelection`: Add to Queue live, the rest disabled) and
    `Backend.queueAddSelection` resolves the ids through
    `queue::quick_tracks_json_many`. `cargo test` 117 pass; Selection and
    ScrollContent selection APIs carry QtQuickTest coverage; qmllint clean.

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
  - **Follow-up:** VI-8a takes the row verbs — outside text fields `y`/`Y`/
    `p`/`P` operate on row selections; the system clipboard bridge and
    `Ctrl-V` text paste stay as landed here.


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

- [x] **Q-5 · Gapless + stream-error retry/auto-advance** — P1 · (M8, PLAYBACKSTUDY.md)
  - **Why:** end-of-track errors stopped the session silently, and every track
    change paid the tick-poll + stream-open gap.
  - **Details:** the current track plays as a small mpv *playlist* — the next
    queue entry is appended once the current reports `file-loaded` — with
    `--gapless-audio=yes`, `--prefetch-playlist=yes` and
    `--stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=5`;
    `playlist-pos` maps native advances back to `index`, `idle` advances the
    window or stops at the queue end. A failed load has no next entry to
    auto-advance to, so it is retried in place with `playlist-play-index`
    (2 attempts, 1 s backoff); a mid-track failure advances directly;
    `file_error` surfaces through the playback toast (manual retry button
    intact). `next()` uses the prefetched `playlist-next`, `previous()` replays
    the window entry, queue `remove` mirrors `playlist-remove`, and reorders
    re-anchor the window and seek back. Backend verdict: stay on the mpv
    subprocess (GStreamer only if Q-6 crossfade becomes a hard requirement).
  - **Now:** window engine + `pump()` in `player.rs` (replaces `poll_end`;
    `qml.rs::tick` emits the queue on an advance and raises the failure toast);
    `apply_event` records `playlist-pos`/`file-loaded`/`idle` and keeps
    `file_error`; unit tests cover the new event mapping. Measured with the
    local IPC harness (`PLAYBACKSTUDY.md` §3): transition 1.51 s → 1.01 s
    (playlist) → 0.003 s (prefetch); `playlist-play-index` reloads a failed
    entry in place. Real-server manual pass still pending.

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
  - **Now (superseded):** `LoadingScreen.qml`, the `loginPending` signal and
    the 5 s ping timeout are gone. The offline-first auto-login (CACHE-9)
    replaces the screen: with downloads the app opens immediately as
    `Offline, <user>` while the auth finishes in the background; without
    downloads the Login screen is shown, prefilled from the stored account.

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
| Must have | 6 | 2 |
| Should have | 58 | 16 |
| Quick wins | 32 | 62 |
| Solved (reference) | — | 40 |
| **Total** | **96** | **120** |

### Coverage by area

| Area | Open | Solved |
|------|------|--------|
| Vi modal engine | 10 | 11 |
| Queue & playback grammar | 2 | 8 |
| Player & media features | 5 | 2 |
| Lyrics pipeline | 2 | 7 |
| Search | 2 | 6 |
| UI microdesigns | 19 | 32 |
| Overlay & scroll (ARC-UI) | 4 | 1 |
| Incremental (ARC-I) | 2 | 2 |
| Dynamic color | 1 | 5 |
| Architecture & robustness | 21 | 21 |
| Recycling | 27 | 0 |
| Media cache / offline | 5 | 2 |
| Auth / startup | 0 | 2 |
| Docs & process | 0 | 2 |
| Packaging | 0 | 1 |
