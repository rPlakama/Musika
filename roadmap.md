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

## Implementation (static shell, no movementation yet)

    -> Theme.js: Material You 3-like dark palette, radius 3; _[X]_
    -> Home shell: SideBar (segments) | AlbumList (central) | RightBar (status) | InferiorBar; _[X]_
    -> Mainscreen mode: cover left + lyrics right (NowPlaying.qml); _[X]_
    -> Album entry: cover top, description, songs (AlbumEntry.qml); _[X]_
    -> Flexible layout: every size/margin/font is a fraction of its container (no fixed px); _[X]_
    -> RightBar status: live counts, sync state/message, staged progress, last sync (click-to-sync); _[X]_
    -> Keybind engine `keybind.rs`: sequence-aware, Qt token mapping, Backend.keyEvent, global Keys forwarding; _[X]_
    -> Feed AlbumList/SideBar/search from library.db; _[]_
    -> Keyboard/vi movementation, block focus (double-ESC), global search binding; _[]_



