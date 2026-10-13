# Where do we go now?

[X] -- Init QT;
[X] -- Init Rust;
[X] -- Init HTTP;
[] -- Bridge to Navidrome API
    -> Usrname/Passwd write using Sqlite; _[X]_
    -> Login Screen in QT; _[X]_
    -> Wireup qml.rs <--> main.rs <--> sqlite.rs <--> http.rs, objective: Functional Login Screen that can ping if password is wrong, notice, use ENC to write down the DB; _[X]_ (password AES-256-GCM + hex, key file `~/.config/musika/key`) [X]
    -> Elaborating current ideia:

    User types into QML fields → set_username/set_password/set_server_url fire → fields update on Backend, still on the Qt thread.
    User clicks login → QML invokes the login slot → Qt thread, synchronous.
    login() clones pool, rt, and the three String fields out of self (no more borrow of self needed) and calls rt.spawn(async move { ... }).
    login() returns immediately — Qt thread is free, UI stays responsive.
    On a tokio worker thread, authenticate() runs the actual query, .awaits on I/O without blocking anything.
    When it resolves, the result needs to cross back to the Qt thread via whatever safe mechanism qtbridge provides, and only then does something call self.login_succeeded() or self.login_failed() to update the UI.

[] -- Indexing Library in .db -- local Sync of Library;
    -> Four DBs under `~/.config/musika`: credentials.db (DB1), library.db (DB2a), search.db (DB2b, FTS5), covers.db (DB3); _[X]_
    -> Incremental sync via `getIndexes(ifModifiedSince)`; auto-run after login + `Backend.syncLibrary()`; _[X]_
    -> Cover cache in `~/.config/musika/covers`: conditional HTTP + atomic swap, purge unreferenced only after a successful sync; _[X]_
    -> Plain-text mirror `index.txt` for `rg`; _[X]_
    -> Feed AlbumList/SideBar/RightBar/search from library.db; _[]_
[] -- Local Playback thought streaming using FFMPEG or MPV, maybe.;
    -> mpv subprocess + JSON IPC engine (`player.rs`): queue, play/pause/stop, next/prev; _[X]_
    -> Real position/duration from mpv (observed properties) + click/drag seek; _[X]_
    -> Wired to QML: InferiorBar transport + now-playing + seek; AlbumList/AlbumEntry click-to-play; _[X]_
[] -- Init Playable System; _[X]_
[] -- Initial Design <-- Working in that now.
...

# Features that I want

[] -- Sync (Indexing the Navidrome with QLite?); _[X]_
[] -- Lyrics
[] -- Download Caching
[] -- Cover Caching; _[X]_
[] -- Auto-Sync; _[X]_

# Per Design

DB1 = usrname stuff
DB2a = index of content (library.db)
DB2b = search index (search.db, FTS5)
DB3 = cover cache (covers.db + covers/)
DB4 = media cache (media.db + media/), see "Media cache" design below

# Design itself (UI/UIX)

The objective of Musika is a Keyboard driven friendly player with Neovim keybinds style, where:

...
Ctrl-N ScrollDown
Crtrl-P ScrollUP
Space+F Search(Global) Artists>Albums>Songs (Per this priority)
Movementation thought the UI: hjkl (vi) with support for Visual/Insert mode
Double ESC to jump into UI Visual Mode, where the user can jump betwheen the main user blocsk (aka, Side bar, Inferior bar, Main, right bar...)
...

Design:

UI with 0 Roundness, square KDE like (QT) main. Using Material You 3(like) colors. Where:

Side bar being the Album/Songs/Artists/Playlist/Radio Segments
Central (main) being the Albums in list (indexed)
right bar (status) Amount of albums/songs/artists, sync progress, if its syncing, how much of sync it is currently doing (clickable to sync)
inferior bar being the play itself.

If user (enter) in inferior bar (Clicks it) it enter (mainscreen) mode, where album cover (by the left side) and Lyris by the right side.

## Floating search (design; replaces the single-list palette)

One floating panel centered over the shell, dim backdrop behind it, square
(radius 3) surfaceContainerHigh with a primary hairline border. Search is a
modal layer: the current view stays behind the dim, nothing navigates away.

Column order left -> right: Songs | Albums | Artists. Each column is a stack
of result rows; the active column's header takes the primary accent while the
others dim. Lookup priority (Artists > Albums > Songs) only picks the column
the cursor starts in.

    +----------------------------------------------------------------------+
    |  ⌕  pink floyd                                       Esc close        |
    +---------------------+----------------------+-------------------------+
    | Songs           3   | Albums           2   | Artists            1    |
    |---------------------|----------------------|-------------------------|
    | [art] Time   4:32   | [art] The Wall 1979  | (o) Pink Floyd  14 alb  |
    | [art] Money  6:22   | [art] Meddle  1971   | (o) Pink Floyd...       |
    | [art] Comfortably   |                      |                         |
    |       Numb   5:06   |                      |                         |
    +---------------------+----------------------+-------------------------+
    | Enter open/play    h/l column    j/k row    / query    Esc close      |
    +----------------------------------------------------------------------+

Row anatomy: square art thumb (round avatar for artists), title, muted
subtitle (song: artist · album; album: artist · year; artist: N albums),
right-aligned kind tag; highlighted row uses secondaryContainer.

Widths: Songs 40%, Albums 32%, Artists 28%; panel 70% width / 65% height of
the window, every size/margin/font a fraction of its container (no px).

Flow: type filters all three columns from one `search.db` (FTS5) request,
debounced ~120ms; per-column "no matches" line; skeleton while in flight;
mouse parity (click row activates, hover mirrors cursor, backdrop dismisses);
100ms cubic-bezier fade/rise for the panel, results refresh in place.

Keys: type filters; Tab or h/l moves between columns; j/k walks rows; Enter
plays a song, opens an album or artist; Esc closes and returns focus to the
previous region. Counts (3j, 5l) and n/N in-results come later with the rest
of normal mode.

## Implementation (static shell, no movementation yet)

    -> Theme.js: Material You 3-like dark palette, radius 3; _[X]_
    -> Home shell: SideBar (segments) | AlbumList (central) | RightBar (status) | InferiorBar; _[X]_
    -> Mainscreen mode: cover left + lyrics right (NowPlaying.qml); _[X]_
    -> Album entry: cover top, description, songs (AlbumEntry.qml); _[X]_
    -> Flexible layout: every size/margin/font is a fraction of its container (no fixed px); _[X]_
    -> RightBar status: live counts, sync state/message, staged progress, last sync (click-to-sync); _[X]_
    -> Keybind engine `keybind.rs`: sequence-aware, Qt token mapping, Backend.keyEvent, global Keys forwarding; _[X]_
    -> Feed AlbumList/SideBar/search from library.db; _[X]_
    -> Cached covers shown in AlbumList/AlbumEntry/InferiorBar (`coverUrl`); _[X]_
    -> Album navigation: central StackView album list -> album page -> back; _[X]_
    -> Keybind dispatch: global handler forwards actions to the current screen; _[X]_
    -> Focus cursor: active region stays highlighted; region mode (Ctrl-W) picks a region, j/k cycles, Enter accepts; _[X]_
    -> Scoped movement: each region owns its cursor; keys only affect the active region; _[X]_
    -> Main region: album list Enter opens album, song Enter plays; sidebar cursor; _[X]_
    -> Player region: h/l seek, j/k prev/next, Enter play/pause; _[X]_
    -> Sidebar segments switch the main view (Albums/Songs/Artists; Playlists/Radio placeholders); _[X]_
    -> Artist opens its filtered albums; global search field (Shift+Space F, focused from any region); _[X]_
    -> Search query/results (Artists > Albums > Songs priority) wired to library.db; _[]_
    -> Motion: 100ms cubic-bezier fades/slides between views; soft color transitions; words/titles fade in; _[X]_
    -> Status bar moved to the top; central pane expands into the freed space; _[X]_
    -> Visual mode: regions blur, selection squares show their titles; unselected not highlighted; _[X]_
    -> Keys: Shift+Enter fullscreen player; Space play/pause anywhere; Shift+Space F search; ESC leaves search; _[X]_
    -> Keyboard/vi movementation, block focus (double-ESC), global search binding; _[]_

# Media cache -- design (Symfonium-inspired)

Goal: play from disk when possible and survive flaky/absent network, without
unbounded disk growth.

DB4 = media cache (media.db + media/), one row per downloaded song:

    song_id UNIQUE, file, size, kind, rule_id, hash, etag, last_modified, last_used
    kind = playback | rolling | permanent | rule
    rule_id points at the auto-download rule that owns the file (NULL otherwise)

Cache tiers (mirroring Symfonium):
    -> playback: caches the current track while it plays and pre-caches the
       next N queue tracks (player.rs owns the queue); purged freely; never
       counted as offline-available;
    -> rolling: fixed max size, evicts least-recently-used first;
    -> permanent: explicit "Keep offline", never auto-removed, no size cap;
    -> rule: files owned by an auto-download rule; re-evaluated on every sync
       and removed when no longer covered (promoted to permanent if pinned).

Auto-download rules (the user-facing part):
    -> Favorites: starring a song/album/artist caches it (and later additions);
    -> Artist in download cache: flag an artist ("Configure auto cache") and
       every album/song of that artist is downloaded -- existing ones right
       away, new ones on each sync;
    -> Playlist and Genre rules likewise (album-level for artists: all songs);
    -> all rule downloads are visible in the offline filter and removable by
       clearing the rule; "Move to permanent" pins a rule/rolling file.

Offline playback flow:
    -> build_track checks media::cached_path(song_id) first, then falls back to
       the stream URL; when a cached copy exists mpv gets the local file path;
    -> downloads: original or transcoded per settings (bitrate cap); written to
       .tmp and atomically renamed, exactly like covers.rs;
    -> "prefer server when online" / "prefer server on Wi-Fi" so a low-bitrate
       cached copy does not shadow the stream;
    -> Wi-Fi-only downloads, max simultaneous downloads (reuse FETCH_CONCURRENCY=8);
    -> status: per-item cache state (cached / partial / queued) + global
       used-bytes vs cap shown in the right bar.

Sync integration:
    -> after a successful library sync, evaluate rules -> queue missing songs;
    -> prune rule-owned files no longer covered and rolling overflow;
    -> failures retried next sync, never fatal (same policy as covers).

Start: playback cache only (no UI). Then permanent/rolling + rules (favorite
artist auto-download first), then offline filter + storage UI.

# Review findings (see CRITIQUE.md)

Per-critic task list: TASKS.md

# P0 -- Correctness / robustness

[] -- In-app error surfacing for sync/playback (toasts + retry), not eprintln;
[] -- Playback context: any play builds a sensible queue (album/list), not 1 track;
[] -- Scope the cover cache to album/artist art (stop per-song covers);
[] -- Remove block_on from the UI thread for lists/covers (async + model);
[] -- mpv: MPRIS/D-Bus + cleanup on abnormal exit;

# P1 -- Nvim fidelity

[] -- Single ESC / Ctrl-[ returns to normal; show mode in the status bar;
[] -- Counts (3j, 5l) and . repeat;
[] -- / search-in-list with n/N; wire global search to real results;
[] -- which-key/showcmd for pending sequences; implement gg/G; ?/help;
[] -- Reconsider Space (leader vs play/pause); drop Ctrl-N/P duplication;

# P2 -- Player features

[] -- Queue view + shuffle/repeat + volume + mute;
[] -- Playlists, radio, lyrics, download caching;
[] -- Scrobble back to Navidrome; resume-on-launch;
[] -- Fullscreen controls + a proper now-playing node;



