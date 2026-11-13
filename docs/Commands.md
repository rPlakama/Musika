# Musika commands

Musika exposes one command registry in two places:

- the interactive command line (`:`)
- the Unix IPC socket

Both parse exactly the same syntax, so every `:` command can be scripted and
every IPC command can be typed. Commands are one line each; the socket reads
one command per line.

## IPC socket

The socket lives in the app config dir as `ipc.sock`. `MUSIKA_IPC_SOCKET`
overrides the path. The usual manual test is:

```sh
printf 'play_pause\n' | nc -U "$MUSIKA_IPC_SOCKET"
```

or, with the default config location:

```sh
printf 'shuffle queue\n' | nc -U ~/.config/musika/ipc.sock
```

With a window attached, every command that runs is echoed in the UI as a toast
(`IPC · <canonical command>`): commands the core executes from `ipc.sock`
(including the tray items, which go through the same socket) are announced to
the attached window, and commands typed on the `:` line are echoed when they
execute. Without a window there is nothing to toast. MPRIS controls are not IPC
and stay silent.

## Window and daemon

Musika's core daemon keeps playing after the window closes. These commands are
routed by the core and work even with no window open:

| Command | Usage | What it does |
| --- | --- | --- |
| `show` | `show` | Spawn the `musika-ui` child window, or raise it if it is already open. |
| `random` | `random` | Pick a random non-empty album from `library.db` and play it in album order. |
| `quit` | `quit` | True quit: flush the session, close the UI child, remove sockets and stop the core. |

The transport/volume/shuffle/radio/sync commands below execute in the core, and
so do the status queries. Presentation commands (navigation, search, queue
overlay, ...) are forwarded to the attached `musika-ui`; without a UI they are
ignored. See `docs/Daemon.md` for the process and socket architecture.

## Status queries

Two read-only commands are answered by the core and work with no window open.
Each returns exactly one JSON line on the same connection, so `nc -U` can read
it back. Typing them on the `:` line also works: the window asks the same
socket in the background and shows the reply in a toast.

```sh
printf 'status\n' | nc -U ~/.config/musika/ipc.sock
printf 'queue\n' | nc -U ~/.config/musika/ipc.sock
```

| Command | Answer (one JSON line) |
| --- | --- |
| `status` | `state` (`playing`/`paused`/`stopped`), `index`, `queue_rev`, `duration`, `position`, `volume`, `muted`, `loading`, `normalization` (0 off / 1 track / 2 album), `crossfade`, `engine_available`, `current` (`{id,title,artist,album}` or `null`), `last_error` |
| `queue` | `rev`, `index`, `len`, `ids` (queue ids in order) |

Every other `ipc.sock` command stays fire-and-forget and writes no reply, which
keeps the plain `printf ... | nc -U` snippets working unchanged. The core keeps
the query connection open, so on an `nc` that waits for EOF add `-N` (or
`-q 0`) to exit after the reply.

## Syntax

- Command names are case-sensitive and use the canonical names below.
- Arguments are whitespace-separated; wrap names containing spaces in quotes:
  `"Pink Floyd"`, `"Atom Heart Mother"`, `"Fat Old Sun"`.
- `playlist add` and `download` use a shell-like tokenizer supporting double
  quotes, single quotes and backslash escapes.
- Commands that accept free text (`search`, `insert_mode`) receive the rest of
  the line verbatim.
- The command line (`:`) additionally offers `Tab` completion and scoped search
  suggestions for command arguments.

Aliases are available where noted: `cancel` → `back`, `volume` → `set_volume`,
`fullscreen` → `toggle_fullscreen`, `lyrics` → `lyrics_overlay`,
`insert` → `insert_mode`, `search_action` → `line_search`.

MPRIS compatibility forms are also accepted:

```text
mpris_play
mpris_pause
mpris_play_pause
mpris_stop
mpris_volume:40
mpris_seek:5000000
mpris_position:5000000
```

## Accounts and library maintenance

| Command | Usage | What it does |
| --- | --- | --- |
| `sync` | `sync [all|repair]` | Incremental sync by default; `all` performs the deep pass; `repair` rebuilds the search/library indexes. |
| `refresh` | `refresh` | Start an incremental sync. |
| `wipe` | `wipe <album|lyrics|covers>` | Clear downloaded album files, cached lyrics, or cached covers. |
| `wipe_databases` | `wipe_databases` | Open the destructive database wipe chooser; this is not an immediate wipe. |
| `logout` | `logout` | End the live session and show the login screen. The account and its data stay stored. |
| `add_account` | `add_account` | Open the login screen in add-account mode. Escape cancels without touching the current account. |
| `remove_account` | `remove_account [--keep-data]` | Remove the current account. The account-local data directory is removed unless `--keep-data` is supplied (legacy config-root data is preserved). If another account exists it becomes active; otherwise the login screen is shown. |

## Playlists, radio, shuffle and downloads

### Playlists

| Command | Usage |
| --- | --- |
| `playlist create` | `playlist create <name> [--offline]` |
| `playlist add` | `playlist add <target>...; <playlist>`, `playlist add selected; <playlist>` or `playlist add queue; <playlist>` |
| `playlist rm` | `playlist rm <target>...; <playlist>` or `playlist rm; <playlist>` |

`playlist create` creates a playlist containing the currently playing track.
By default it is posted to the server; `--offline` writes only the account-local
M3U file.

```text
playlist create "Rated R Mix"
playlist create "Rated R Mix" --offline
```

`playlist add` appends one or more targets to an existing playlist. Targets are
resolved through the same search engine as the Search overlay:

- an artist expands to all of its songs;
- an album expands to all of its tracks;
- a track adds that one song;
- the literal `current` adds the currently playing track;
- the literal `selected` adds the rows currently selected in the UI;
- the literal `queue` saves the entire current playback queue, in queue order.

`queue` is resolved from the core's authoritative playback queue, so it also
works over `ipc.sock` with no window attached (an attached UI forwards its
mirror of the same queue). It is appended through the same resolved-id path as
every other target, so a song that appears several times in the queue keeps
only its last position, exactly like a repeated name target.

`selected` is the one target that is not resolved by name: it uses the live
selection of the active surface. The floating search's range wins when the
overlay is open; otherwise the queue selection, the open page's rows or the
library rows are used. An active `visual` range contributes every selected row;
without one the cursor row counts as a single selected row (like `yank` and
`delete_row`). Selected album and artist rows expand to their songs, and
playlist rows are skipped. Because the selection lives in the UI process,
`selected` must be the only target and, like the navigation commands, needs an
attached window — an `ipc.sock` client forwards it to `musika-ui`.

The destination playlist name or id comes after the first unquoted `;`; it may
be unquoted even when it has spaces. Before the `;` sits the target list
(comma-separated and whitespace-separated targets are both accepted; a quoted
target may itself contain `;`). The legacy form without a cutoff — playlist is
the last argument — is still accepted.

Command-line suggestions follow the cutoff: before `;` you get `current`,
`selected`, `queue` plus search hits for the targets, after `;` only existing
playlist names (matched against the name typed so far, so a two-word name completes in
one Tab).

```text
playlist add current; "Rated R Mix"
playlist add selected; "Rated R Mix"
playlist add queue; "Rated R Mix"
playlist add "Pink Floyd", "King Crimson"; Road Trip
playlist add "Rated R" "Leg of Lamb"; Rated R Mix
# legacy form, still accepted:
playlist add current "Rated R Mix"
```

Local M3U playlists are appended to and rescanned. Server playlists are updated
with the server's `updatePlaylist` endpoint and refreshed afterwards. Local mix
files use the sanitized origin/playlist name (`<name>.m3u`) and overwrite an
existing mix of the same name instead of creating timestamped duplicates.

Playlists never hold the same song twice: a rescan removes duplicates from an
app-owned M3U file (the first occurrence stays, and duplicate unresolved path
lines are cleaned by normalized path), a server playlist found with duplicates
is rebuilt in deduplicated order, and an open playlist page never shows
duplicate rows even when the cleanup has to wait for the network.

`playlist rm; <playlist>` (the cutoff with no targets before it) deletes the
whole playlist. The `;` is the declaration that what follows is a playlist, so
the command line suggests playlist names after it. Local M3U files are removed
and the folder rescanned; server playlists use `deletePlaylist` and refresh the
list. The open playlist page closes and the Playlists scope reloads.

`playlist rm` with targets mirrors `playlist add`: it accepts exactly the same targets
(`current`, artist, album, track) and the same `;` cutoff / legacy grammar. M3U
entries resolving to the target songs are dropped and the file is rescanned;
server playlists are updated with repeated `songIdToRemove` parameters and
refreshed afterwards. Removing a target that is not present reports an error
instead of rewriting the playlist.

```text
playlist rm current; "Rated R Mix"
playlist rm "Pink Floyd"; Road Trip
playlist rm "Rated R"; Rated R Mix
```

### Radio

| Command | Usage |
| --- | --- |
| `radio` | `radio [current|album|song] [name]` |
| `radio save` | `radio save [--offline]` |

`radio` starts a radio queue:

```text
radio                 # currently playing track
radio current         # currently playing track
radio album           # current album page, or pass a name
radio album "Kid A"
radio song "Idioteque"
```

`radio save` saves the currently open radio page. It requires a radio page to
be open; the M3U is named from that page's origin title. By default it also
creates the server playlist; `--offline` writes only the local M3U.

```text
radio save
radio save --offline
```

### Shuffle

| Command | Usage |
| --- | --- |
| `shuffle queue` | Shuffle the current queue and restart it. |
| `shuffle album [name]` | Shuffle the current album page, or an album resolved by name/id. |
| `shuffle artist [name]` | Shuffle the current artist page, or an artist resolved by name/id. |

```text
shuffle queue
shuffle album "Rated R"
shuffle artist "Queens of the Stone Age"
```

Album/artist names use the unified search engine; exact library ids also work.

### Downloads

```text
download
download current
download rm
download artist "Pink Floyd"
download album "Atom Heart Mother"
download track "Fat Old Sun"
download "Pink Floyd"
download "Pink Floyd" album "Atom Heart Mother"
download "Pink Floyd" album "Atom Heart Mother" "Fat Old Sun"
download rm "Pink Floyd"
```

Rules:

- No arguments (or `current`) downloads/removes the currently playing track.
- `rm` removes local download marks/media; it never deletes anything on the
  server.
- Scope keywords `artist`, `album`, `track` force a direct search at that level.
- Without keywords, names are positional: artist, then album, then track.
- Multiword names must be quoted.
- Album/artist downloads expand to all matching songs; removal uses the same
  scope and clears the relevant local rules/marks/media.
- `download_song <song-id>` remains available for a single known song id.

## Playback

| Command | Usage | What it does |
| --- | --- | --- |
| `play_pause` | `play_pause` | Toggle playback. |
| `play` | `play` | Resume playback. |
| `pause` | `pause` | Pause playback. |
| `stop` | `stop` | Stop playback (`mpris_stop`). |
| `track` | `track <next\|previous>` | Move to the next or previous track. |
| `mute` | `mute` | Toggle mute. |
| `volume_up` | `volume_up` | Raise volume by 5. |
| `volume_down` | `volume_down` | Lower volume by 5. |
| `set_volume` | `set_volume <0..100>` | Set absolute volume (`volume`). |
| `seek_back` | `seek_back` | Seek back 5 seconds. |
| `seek_forward` | `seek_forward` | Seek forward 5 seconds. |
| `seek` | `seek <seconds>` | Seek to an absolute position. |
| `seek_relative` | `seek_relative <±seconds>` | Seek relative to the playhead. |
| `crossfade` | `crossfade [on\|off]` | Toggle or explicitly set the optional 5 s track crossfade. |
| `normalize` | `normalize [off\|track\|album]` | Cycle or explicitly set ReplayGain loudness normalization (default off). |
| `toggle_fullscreen` | `toggle_fullscreen` | Toggle fullscreen playback (`fullscreen`). |
| `lyrics_overlay` | `lyrics_overlay` | Toggle the fullscreen lyrics overlay (`lyrics`). |
| `show` | `show` | Spawn/raise the GPUI window (core keeps running headless). |
| `random` | `random` | Play a random album from `library.db`. |

## Navigation and selection

| Command | What it does |
| --- | --- |
| `top` | Jump to the first row. |
| `bottom` | Jump to the last row. |
| `move_up` / `move_down` | Move the cursor up/down one row. |
| `scroll_up` / `scroll_down` | Scroll the active surface up/down one row. |
| `page_up` / `page_down` | Move one page up/down. |
| `move_left` / `move_right` | Move left/right within the active surface. |
| `prev_place` / `next_place` | Previous/next entry in the travel log. |
| `home` | Return to Home and close overlays/fullscreen/pages. |
| `back` | Back out of the current surface (`cancel`). |
| `enter` | Activate the current selection. |
| `scope_next` / `scope_prev` | Next/previous library scope. |
| `order_next` / `order_prev` | Next/previous library ordering. |
| `visual` | Toggle visual selection. |
| `visual_all` | Select every visible row. |
| `pane_cycle [back]` | Cycle the active pane; `back` reverses. |

## Queue

| Command | What it does |
| --- | --- |
| `queue_toggle` | Open/close the queue drawer. |
| `queue_remove` / `delete_row` | Remove the queue cursor row or the selected range; on a playlist page, delete the selected entries. |
| `queue_up` / `queue_down` | Move the queue row (or selected block) up/down, or the playlist-page cursor/block up/down. |
| `yank` / `yank_row` | Yank the visual range (or cursor row) / the cursor row into the row clipboard. |
| `paste_below` / `paste_above` | Paste the row clipboard below/above the queue cursor, or insert it below/above the cursor on an open playlist page. |
| `repeat_cycle` | Cycle repeat mode (off → all → one). |

## Search

| Command | Usage | What it does |
| --- | --- | --- |
| `search` | `search [query]` | Open floating search, optionally pre-filled. |
| `insert_mode` | `insert_mode [query]` | Open search directly in insert/typing mode (`insert`). |
| `search_next` / `search_prev` | | Next/previous floating-search row. |
| `search_scope_next` / `search_scope_prev` | | Next/previous floating-search scope. |
| `line_search` | `line_search` | Open the in-surface `/` line search (`search_action`). |

## Direct row/song commands

| Command | Usage | What it does |
| --- | --- | --- |
| `play_song` | `play_song <song-id>` | Play one song in the current order. |
| `play_song_in_album` | `play_song_in_album <song-id>` | Play a song in its album queue. |
| `open_song_album` | `open_song_album <song-id>` | Open the song's album page. |
| `download_song` | `download_song <song-id>` | Download one song by id. |

## Alt quick actions and quit

| Command | Usage | What it does |
| --- | --- | --- |
| `quick_hold` | `quick_hold <on|off>` | Arm/release the Alt quick-action gesture. |
| `quick_aim` | `quick_aim <north|south|east|west>` | Aim a quick-action slot. |
| `quick_cycle` | `quick_cycle <next|prev>` | Cycle the aimed slot's verbs. |
| `quick_cancel` | `quick_cancel` | Cancel the quick-action menu. |
| `quit` | `quit` | True quit: stop the core daemon and its UI child. |

## Automation examples

```sh
socket="${MUSIKA_IPC_SOCKET:-$HOME/.config/musika/ipc.sock}"

printf 'shuffle album "Rated R"\n' | nc -U "$socket"
printf 'radio album "Kid A"\n' | nc -U "$socket"
printf 'playlist add current; "Road Trip"\n' | nc -U "$socket"
printf 'playlist add selected; "Road Trip"\n' | nc -U "$socket" # needs an attached window
printf 'playlist add queue; "Road Trip"\n' | nc -U "$socket"   # works headless
printf 'playlist create "Road Trip" --offline\n' | nc -U "$socket"
printf 'radio save --offline\n' | nc -U "$socket"
printf 'download "Pink Floyd" album "Atom Heart Mother"\n' | nc -U "$socket"
printf 'download rm "Pink Floyd"\n' | nc -U "$socket"
printf 'quick_hold on\nquick_aim north\nquick_hold off\n' | nc -U "$socket"
```

`download`, `playlist add`, `shuffle album/artist`, `radio album/song` and the
command-line suggestions resolve names through the same local search engine as
the Search overlay, so quoted names do not need to be ids.
