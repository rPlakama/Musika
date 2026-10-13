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
DB5 = lyric cache (lyrics.db), see "Lyrics" design below

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

# MPRIS node -- design (control that survives any environment)

Goal: the player node owns `org.mpris.MediaPlayer2.musika` on the session bus
itself, so panels, media keys, `playerctl`, Bluetooth stacks and lock screens
work on a stock install -- no mpv-mpris script, no per-DE keybind fiddling.

Today: metadata only reaches MPRIS when the user installs mpv's mpris.lua/so
and sets `MUSIKA_MPRIS_SCRIPT` (or drops it in `~/.config/mpv/scripts`);
otherwise the node is invisible to the desktop.

Design:
    -> new `mpris.rs`: a bridge task owns the service; `player.rs` stays the
       source of truth (`Shared` snapshot + queue) and is published from there;
    -> async D-Bus (zbus) on the tokio runtime; zero work on the Qt thread;
    -> start only if a session bus exists (`DBUS_SESSION_BUS_ADDRESS`); else
       log once and no-op (SSH, TTY, bare WSL, containers);
    -> own `org.mpris.MediaPlayer2.musika` with allow-replacement; when the
       mpv-mpris script is detected, do not double-publish (it owns ...mpv);
    -> cover art is published as a `file://` URL straight from the cover cache.

Interface surface (MPRIS 2.2):
    root:   Identity, CanRaise, CanQuit, Desktops, Raise, Quit
    player: PlaybackStatus, LoopStatus, Rate, Shuffle, Metadata, Volume,
            Position, MinimumRate, MaximumRate, CanControl, CanGoNext,
            CanGoPrevious, CanPlay, CanPause, CanSeek, CanStop
    methods: Play, Pause, PlayPause, Stop, Next, Previous, Seek, SetPosition,
             OpenUri (optional)
    signals: Seeked, PropertiesChanged

Mapping:
    PlaybackStatus <- State (playing/paused/stopped)
    mpris:trackid  <- /org/musika/track/<song id>
    mpris:length   <- duration * 1_000_000 (µs)
    mpris:artUrl   <- cover_url (file://...)
    xesam:title/artist/album <- Track fields
    Position       <- polled snapshot (µs); Seeked only on real seeks
    CanGoNext/Prev <- queue index bounds

Environment matrix:
    GNOME/Cinnamon/MATE/Budgie: media keys + panel via MPRIS, nothing else;
    KDE Plasma: plasma media controller, works out of the box;
    XFCE: MPRIS-aware; keys may need one xfconf binding and we document it;
    sway/Hyprland/i3: no daemon; acceptance = `playerctl` works + documented
        bindings for the sample config;
    no session bus (SSH/TTY/WSL/container): detected, no-op, playback intact;
    Flatpak (future): needs `--talk-name=org.mpris.MediaPlayer2.*`;
    multiple players: unique bus name, `Identity` disambiguates in panels;
    mpv-mpris present: fallback path disabled to avoid two MPRIS nodes.

Fallbacks:
    -> keep `MUSIKA_MPRIS_SCRIPT` as an opt-in fallback for older builds;
    -> internal XF86Audio keys only where Qt delivers them (X11); Wayland
       always relies on the DE/portal.

Robustness:
    -> bridge failures log + backoff, never block or panic the UI;
    -> rate-limit Position/PropertiesChanged (>=250 ms, plus transitions);
    -> art URLs pinned while a track is current (cache purge must not break it).

Milestones:
    -> M0 spike: add zbus, own the name, static Identity, `playerctl -l` lists us;
    -> M1 transport: Play/Pause/PlayPause/Stop/Next/Previous + PlaybackStatus;
    -> M2 metadata: trackid/length/artUrl/xesam; panel shows title + art;
    -> M3 extras: LoopStatus, Shuffle, Volume, Position/Seeked, Can* flags;
    -> M4 hardening: no-bus path, mpv fallback toggle, env matrix, docs;
    -> M5 optional: playlist interface for the queue, OpenUri.

Testing:
    playerctl -l / -p musika status / metadata
    busctl --user introspect org.mpris.MediaPlayer2.musika /org/mpris/MediaPlayer2
    dbus-monitor "interface='org.mpris.MediaPlayer2.Player'"
    one pass per environment row, plus `DBUS_SESSION_BUS_ADDRESS= playerctl -l`.

Open questions:
    -> zbus vs a ready-made `mpris-server` crate (weight vs control);
    -> Rate fixed at 1.0 for v1 (fine unless speed/pitch lands);
    -> whether to expose the queue as the MPRIS playlist or keep it internal.

# Lyrics -- design (Subsonic / OpenSubsonic, multiDB)

Research:
    -> `getLyrics` (Subsonic 1.2.0): optional artist/title in, plain
       `lyrics{artist,title,value}` out. No id, no timestamps, fuzzy match --
       fallback only.
    -> `getLyricsBySongId` (OpenSubsonic extension `songLyrics` v1): song id in,
       `lyricsList.structuredLyrics[]` out with `lang`, `offset`, `synced`,
       `line[{start(ms), value}]`. Primary endpoint. `enhanced=true` (v2) adds
       `kind`, `cueLine` and `agents` for karaoke -- out of scope for v1.
    -> discovery: `getOpenSubsonicExtensions` lists `songLyrics`; responses set
       `openSubsonic: true`. Servers without it may still serve `getLyrics`.
    -> `lang` is free-form: `xxx`/`und` mean unspecified; never assume ISO.
    -> server-side sources: embedded SYLT/USLT tags, `.lrc`/text sidecars,
       external providers; `synced` is true only when timings exist.
    -> Navidrome implements the extension in current releases (verify per
       version); synced when the source carries timestamps.

MultiDB placement:
    -> DB5 = lyrics.db (text only, no blob directory), new pool in `Dbs`,
       `migrations_lyrics`; purge domain independent, exactly like covers.
    -> library.db stays lean (no lyric blobs); search.db untouched.
    -> schema:
       lyrics(song_id TEXT PRIMARY KEY, source TEXT, synced INTEGER,
              lang TEXT, offset_ms INTEGER, text TEXT, fetched_at INTEGER)
       lyric_line(song_id TEXT, idx INTEGER, start_ms INTEGER, value TEXT)
       -- lines only for synced lyrics; plain text lives in `text`.
    -> user override: `~/.config/musika/lyrics/<song_id>.lrc` (or .txt) wins
       over the DB and is never purged.

Fetch policy:
    -> on-demand: NowPlaying asks for the current song; misses cached as
       source='none' and retried after a TTL or manual refresh; hits kept
       (songs carry no `changed` field, so there is no per-song invalidation).
    -> prefetch: when the queue loads or a track starts, request lyrics for the
       next N tracks, bounded concurrency (reuse FETCH_CONCURRENCY=8).
    -> no full-library sweep (request amplification); `:lyrics sync` can
       backfill per album/artist later.
    -> fallback chain: local override -> getLyricsBySongId ->
       getLyrics(artist, title) -> none.

Read path:
    -> Backend.requestLyrics(songId) -> tokio task -> dataReady("lyrics", json)
       with {source, synced, lang, offset_ms, lines:[{t, text}]}.
    -> NowPlaying: synced -> active-line highlight + auto-scroll to center
       (`quick` scroll / `micro` highlight, motion tokens); plain -> static
       scrollable text; none -> "No lyrics" empty state.
    -> playback position already flows through `Backend.position`; apply the
       OpenSubsonic `offset` before comparing.

Sync & purge:
    -> after a successful library sync, delete lyric rows whose song id no
       longer exists in library.db (same policy as covers); overrides kept.
    -> capability (`songLyrics` yes/no) cached in lyrics.db meta at login.

Milestones:
    -> L0: `getLyricsBySongId` + types + extension probe in api.rs;
    -> L1: DB5 + migrations_lyrics + `Dbs` pool; store/read path;
    -> L2: `requestLyrics` + plain rendering in NowPlaying (kills the hardcoded
       placeholder, MD28);
    -> L3: synced rendering (highlight, auto-scroll, offset) + empty state;
    -> L4: prefetch next N, miss TTL, purge-on-sync, local `.lrc` override;
    -> L5 optional: karaoke v2 (word-level cueLine, translations), lyric search
       via search.db FTS5, `:lyrics` commands.

Open questions:
    -> store raw LRC vs normalized lines (normalize at write for O(1) lookup);
    -> miss TTL default (7 days?) and the manual refresh key;
    -> language choice when several `structuredLyrics` exist (first synced main);
    -> whether to expose an offline "lyrics available" filter.

# Review findings (see CRITIQUE.md)

Master task list: TASKLISTMAJOR.md

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
[] -- Native MPRIS service (see "MPRIS node" design); playerctl acceptance test;



