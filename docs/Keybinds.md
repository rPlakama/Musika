# Musika keybinds

Musika is keyboard-first. The keymap is Vim-flavoured: bindings can be
sequences (`g g`), motions take a count prefix (`3j`, `10l`, `5G`), and a
half-typed sequence times out after 800 ms.

`Ctrl-J` / `Ctrl-K` are universal list-navigation keys: down / up in any list,
search, suggestion list, and insert mode. They work wherever the arrow keys
work, including the command line.

## Global

| Keys | Action |
| --- | --- |
| `Space` | Play/pause |
| `Ctrl-N` / `Ctrl-P` | Next / previous track |
| `,` / `.` | Seek back / forward 5 s. Holding repeats, and alternating the pair keeps counting in one bubble: the toast shows the repeat count plus the net distance (e.g. `Seek forward x3 (+15 s)`, or `Seek x4 (0 s)` when a `,.` spam nets out). While held, the progress bar shows a ghost band where the playhead will land. |
| `-` / `=` (`+`) | Volume down / up 5%, with the same held-key counter, a direction chevron and the resulting level. |
| `m` | Mute. A sticky toast stays on screen until you unmute. |
| `Ctrl-R` | Cycle repeat (off → all → one) |
| `r` | Sync the library |
| `Shift-Enter` | Toggle the fullscreen player |
| `Ctrl-Q` | Toggle the queue drawer |
| `?` | Toggle the lyrics overlay in fullscreen, or the page description overlay on album/artist pages |
| `[b` / `]b` | Previous / next place in the travel log (cleared on reaching Home) |
| `Shift-Q` | Home: close overlays, leave fullscreen and pop every page |
| `q` | Back; reaching Home ends the travel session. Press twice at Home for `qq`, the true quit (closing the window only closes the UI and leaves the core daemon playing in the tray). |
| `Esc` / `Ctrl-[` | Cancel and hierarchical Back |
| `1`–`9` prefix | Count for the next motion or action |
| `Ctrl-J` / `Ctrl-K` | Move down / up in the current list or text selector |

## Lists and pages

| Keys | Action |
| --- | --- |
| `j` / `k` | Move the cursor down / up |
| `h` / `l` | Move left / right between panes where a surface has them |
| `g g` / `G` | Jump to the first / last row |
| `Ctrl-D` / `Ctrl-U` or `PageDown` / `PageUp` | Page down / up |
| `v` / `V` | Toggle visual selection / select all rows |
| `Shift-↑` `Shift-↓` `Shift-←` `Shift-→` | Extend the visual selection |
| `y` / `Y` | Yank the visual range (or cursor row) / the cursor row |
| `p` / `P` | Paste the row clipboard below / above the queue cursor; on a playlist page, append the yanked rows (paste consumes the clipboard — yank again to paste again) |
| `Ctrl-V` | Paste text into the focused search field |
| `Enter` | Open/play the row, confirm, or use the focused control |
| `Tab` / `Shift-Tab` | Walk panes, header stops and status-bar actions |
| `Shift-J` / `Shift-K` | Next / previous library scope (Albums, Songs, Artists, Playlists) |
| `Shift-L` / `Shift-H` | Next / previous list ordering |
| `/` | In-surface line search |
| `Shift-Space F` or `i` | Open the floating search |
| Mouse click | Place the cursor and open/play the row |
| `Shift` + click | Extend the visual selection to the row |
| Mouse Back / Forward | Previous / next place in the travel log |

Visual selections are also addressable from the command line and the IPC
socket: `: playlist add selected; "Rated R Mix"` appends every selected row to
a playlist, expanding album and artist rows to their songs.

Each page visit is a place in the travel log (`[b` / `]b`, mouse Back/Forward).
Backing out of the last page reaches Home and clears the log: opening another
page from Home starts a fresh trip, so its Back returns Home rather than the
previous page.

On a playlist page, `y`/`Y` yank its song rows, `p` inserts the row clipboard
below the cursor and `P` above it (local M3U files insert exactly; server
playlists rebuild their order because `updatePlaylist` cannot insert), and
`d d` deletes the selected rows or the cursor row. `Ctrl-Shift-K` /
`Ctrl-Shift-J` move the cursor entry up / down, or the whole visual selection
as one block, like the queue drawer.

On album and artist pages, `Tab` cycles the page header stops. `?`, or Enter/click
on the inline description, opens the shared full-window description reader:
`j`/`k` scroll it a line at a time, `Ctrl-D`/`Ctrl-U` and `PageDown`/`PageUp`
page it, `g g`/`G` jump to the start/end, and `Esc`/`q` closes it. The wheel
scrolls it too. On an artist page, `Shift-J`/`Shift-K` switches between the
album wall and the artist's full track list.

## Queue drawer

| Keys | Action |
| --- | --- |
| `j` / `k`, `g g` / `G`, `Ctrl-D` / `Ctrl-U`, `PageDown` / `PageUp` | Move through the queue |
| `↑` / `↓`, `Ctrl-J` / `Ctrl-K` | Move through the queue |
| `Enter` | Jump to the cursor track |
| `Ctrl-Shift-J` / `Ctrl-Shift-K` | Move the track, or the whole selected block, down / up |
| `d d` | Remove the cursor track, or the whole visual range |
| `v` / `V`, `Shift-arrows` | Select rows / select all |
| `y` / `Y`, `p` / `P` | Yank the range or row / paste below or above |
| `q` or `Ctrl-Q` | Close the drawer |
| `Esc` / `Ctrl-[` | End the visual selection (does not close the drawer) |

## Fullscreen player

| Keys | Action |
| --- | --- |
| `Tab` / `Shift-Tab` | Cycle the album and lyrics panes (lyrics is skipped while hidden) |
| `j` / `k`, `Ctrl-D` / `Ctrl-U`, `PageDown` / `PageUp` | Scroll the lyrics pane (and the floating overlay when it is open) |
| `g g` / `G` | Jump to the first / last lyric |
| `h` / `l` | Move between the lyrics and album panes |
| `Enter` | Open the current album or track actions |
| `?` | Toggle the floating lyrics overlay (its body scrolls with the same grammar) |
| `Esc` / `Ctrl-[` | Close the lyrics overlay, otherwise leave fullscreen |

The lyrics pane/overlay scrolls by hand for a few seconds and then returns to
following the played line (5 s when the lyrics are in focus, 2 s otherwise);
the wheel scrolls it as well. Only the lyric column glides when the phrase
changes: the highlighted (focused) pane stays fixed, and starting a manual
scroll stops an in-flight follow glide instead of letting it drag the column.

## Floating search

| Mode | Keys | Action |
| --- | --- | --- |
| Typing | text | Filter the current scope |
| Typing | `Ctrl-H` / `Ctrl-L`, `Tab` / `Shift-Tab`, `Ctrl-Backspace` | Previous / next scope (wraps: Shift-Tab from Songs goes to Playlists) |
| Typing | `↑` / `↓`, `Ctrl-P` / `Ctrl-N`, `Ctrl-K` / `Ctrl-J` | Previous / next result row |
| Typing | `Enter` | Open/play the result |
| Typing | `Ctrl-W` | Delete the previous word |
| Typing | `Esc` | Leave typing for normal mode |
| Normal | `h` / `l`, `Tab` / `Shift-Tab`, `Shift-H` / `Shift-L`, `Shift-J` / `Shift-K` | Previous / next scope |
| Normal | `j` / `k`, `g g` / `G`, `Ctrl-D` / `Ctrl-U`, `PageDown` / `PageUp` | Move through the results |
| Normal | `↑` / `↓`, `Ctrl-J` / `Ctrl-K` | Move through the results |
| Normal | `v` / `V`, `Shift-arrows` | Select / extend the selection |
| Normal | `y` / `Y` | Yank the range or row into the row clipboard |
| Normal | `i` | Return to typing |
| Normal | `Enter` | Open/play the result |
| Normal | `Esc` / `Ctrl-[` | Clear the selection first, then close the search |

## Line search (`/`)

| Keys | Action |
| --- | --- |
| text | Literal, smartcase search over the visible surface |
| `Tab` / `Shift-Tab`, `Ctrl-L` / `Ctrl-H`, `Ctrl-Backspace` | Next / previous match |
| `↑` / `↓`, `Ctrl-J` / `Ctrl-K` | Next / previous match |
| `Enter` | Commit the current match |
| `Esc` | Abandon the search |
| `Ctrl-W` | Delete the previous word |

## Command line (`:`)

The command line is the interactive form of the IPC command registry. See
[Commands.md](Commands.md) for command syntax and examples.

| Keys | Action |
| --- | --- |
| `:` | Open the command line |
| text | Type a command |
| `Tab` / `Shift-Tab` | Complete the highlighted suggestion, then cycle completions |
| `↑` / `↓`, `Ctrl-P` / `Ctrl-N`, `Ctrl-K` / `Ctrl-J` | Move the highlighted suggestion |
| `Enter` | Execute the command |
| `Esc` | Close the command line |
| `Ctrl-C` / `Ctrl-X` | Copy / cut the command-line selection |
| `Ctrl-V` | Paste into the command line |
| `Ctrl-W` | Delete the previous word |

Argument autosuggestions use the same search engine as the floating search
overlay. The previous argument scopes them, for example:

```text
: shuffle artist "P
: radio album "K
: download "Pink Floyd" album "A
: playlist add "P
```

## Alt quick actions

| Keys | Action |
| --- | --- |
| Hold `Alt` | Open the quick-action menu on the cursor row |
| `h` / `j` / `k` / `l` or the arrow keys | Aim west / south / north / east |
| `Ctrl-J` / `Ctrl-K` | Move down / up through the menu entries |
| `Shift-H` / `Shift-L` | Within an aimed slot: step the favourite's rating, or cycle its verbs (album/song N cycles Add to Queue → Start Radio) |
| Release `Alt` | Fire the aimed action; release over the neutral centre cancels |
| Click a tile / click outside | Fire a tile / cancel a sticky menu |

Each rating increase/decrease is sent to the core as it happens (a clamped
step at 0 or 5 sends nothing), so the header star row follows every step
instead of updating only when the hold ends. A rating step owns the release:
it does not also toggle the favourite underneath it.

Single-song rows and album rows/cards seed **Start Radio** from that row; a
visual selection keeps the range verbs (no radio).

Automation: the IPC socket drives the same gesture — `quick_hold on`,
`quick_aim north`, `quick_cycle next`, `quick_hold off` — for scripted checks.

While `Alt` is held, `Esc` is swallowed so it cannot leak into the keymap; the
neutral release is the cancel. Sticky menus (the trashcan wipe menu) aim with
`h`/`j`/`k`/`l` or the arrows and fire with `Enter`; `Enter` on the neutral
centre or a click outside closes them.
