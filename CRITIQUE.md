# Musika — Architecture, UIX & Vi-Motion Critique

An in-depth critical analysis of **Musika**, evaluating the project against its current technical scope and establishing the foundational framework for **what makes an exemplary fusion of a music player and Vi/Neovim modal ergonomics**.

---

## 1. Executive Summary & Current Scope

Musika is an ambitious desktop music player built with **Rust** and **Qt 6 / QML** via `qtbridge`, designed as a local frontend for Subsonic / Navidrome streaming servers. Its stated philosophy is to deliver a **keyboard-driven, Neovim-styled modal interface** wrapped in an unrounded, high-contrast, Material You 3-inspired dark aesthetic.

### 1.1 Architectural Inventory (As Built)

| Layer | Implementation Details | Current Status & Assessment |
| :--- | :--- | :--- |
| **GUI Shell** | Qt 6 Quick / QML (`Main.qml`, `Home.qml`). Responsive container sizing (percentage/ratio-based), 0px corner radii, Material You dark palette (`Theme.js`). Divided into Top Status Bar (`RightBar.qml`), Sidebar Segments (`SideBar.qml`), Central `StackView` (`AlbumList`, `SongList`, `ArtistList`), Bottom Player Bar (`InferiorBar.qml`), and Fullscreen Now Playing (`NowPlaying.qml`). | Structurally complete shell. Transitions tuned to 100ms. Floating search palette and toast notifications implemented. |
| **Keybinding Engine** | Custom Rust stateful token parser (`keybind.rs`) using prefix trees and 350ms timeouts. Maps raw Qt keys/modifiers to symbolic `Action` enums. | Functional for flat sequences (`ctrl-w`, `space`, `shift-space f`, `g g`, `h j k l`), but lacks counts, grammar, and a formal modal state machine. |
| **Audio Engine** | `player.rs` driving an independent `mpv` subprocess in `--idle` mode via UNIX domain socket JSON-IPC. Background reader thread polls property events (`time-pos`, `duration`, `pause`, `end-file`). | Solid, lightweight audio backend. Seeking and position reporting work accurately. MPRIS integration supported via `MUSIKA_MPRIS_SCRIPT`. |
| **Data & Persistence** | Four separate SQLite databases (`credentials.db`, `library.db`, `search.db`, `covers.db`) in `~/.config/musika/`. Client passwords encrypted with AES-256-GCM. Cover images cached on disk. Incremental library sync via Subsonic `getIndexes(ifModifiedSince)`. | Multi-database design avoids lock contention. Asynchronous database reads via Tokio worker pools deliver data to QML via `dataReady` signals without blocking the Qt GUI thread. |
| **Search Mechanism** | Dual implementation: FTS5 table in `search.db` for full-text queries, plus an in-RAM tmpfs file-tree (`/dev/shm` or `XDG_RUNTIME_DIR`) queried by a spawned `fd` binary via regex for the floating palette (`SearchPalette.qml`). | Novel and performant in practice, but architecturally split and unorthodox (running a CLI process over tmpfs files instead of querying SQLite FTS5 directly). |

---

## 2. Theory: What Makes a Good Mix of a Music Player with Vi Motions?

Many modern applications claim to be "Vim-like" simply by mapping `j` to scroll down and `k` to scroll up. This is a **superficial mimicry** that fails to capture why Vi motions are so extraordinarily effective. 

To create a genuine, elegant blend of a music player and Vi motions, one must understand how Vim's mental model maps onto the domain of digital music management and playback.

```
+-----------------------------------------------------------------------------------+
|                            THE VI MUSIC PLAYER GRAMMAR                            |
+-----------------------------------------------------------------------------------+
|    COUNT        +         OPERATOR          +             OBJECT / MOTION         |
|  (Quantity)              (Verb)                             (Noun)                |
|                                                                                   |
|  [3]                       d (delete)               w (window) / j (track down)   |
|  [10]                      l (seek +10s)            % (seek to 50%)               |
|  [ ]                       y (yank to queue)        ap (a playlist) / <CR> (play) |
+-----------------------------------------------------------------------------------+
```

### 2.1 The Seven Pillars of Vi-Music Fusion

#### 1. Strict Modality & Visible State (No Modal Ambiguity)
In Vim, you are always in a known mode: **NORMAL**, **VISUAL**, **INSERT**, or **COMMAND**. 
- In a music player, the default mode must **always be NORMAL**. 
- Typing letters in Normal mode should never accidentally trigger search inputs or form fields. 
- The current mode must be vividly displayed in the status bar (e.g., `-- NORMAL --`, `-- VISUAL --`, `-- QUEUE --`). 
- Focus must never get lost between the GUI focus ring and the keyboard cursor.

#### 2. The Invariant Escape (`<Esc>` or `<C-[>`)
In Vim, `<Esc>` is an inviolable invariant: it immediately cancels any pending sequence, leaves visual selection, dismisses command palettes or prompts, closes overlays, and returns the application unconditionally to **NORMAL** mode at the current cursor position. Requiring double-escape (`Esc Esc`) or overloading escape breaks user trust and muscle memory.

#### 3. True Grammar: Verb + Noun + Count (Composability)
Vi is not a collection of key shortcuts; it is a **compositional language**:
- **Counts**: Numbers prefixing motions. In a music player:
  - `5j` moves down 5 tracks or albums.
  - `15l` seeks 15 seconds forward; `10h` seeks 10 seconds back.
  - `3G` jumps directly to track #3.
- **Operators (Verbs)**:
  - `d` (Delete/Remove): `dd` removes the highlighted track from the playback queue. `3dd` removes the next 3 tracks.
  - `y` (Yank): `yy` copies the track/album link or adds it to the internal clipboard.
  - `p` / `P` (Put/Paste): `p` appends the yanked track(s) after the current playing song; `P` inserts them as "Play Next".
  - `m` (Mark): `ma` sets a bookmark at the current playback timestamp; `'a` jumps back to that timestamp (ideal for transcription, DJ practice, or A-B looping).
  - `.` (Repeat): Re-executes the last operation (e.g., repeating track deletion or queue appending).

#### 4. Dual-Axis Navigation (Structural vs. Temporal)
A text editor has two dimensions: lines (vertical) and characters (horizontal). A music player similarly possesses two natural orthogonal axes:
- **Vertical Axis (Structural Navigation)**: Navigating discrete items (tracks, albums, artists, playlists, queues) using `j`, `k`, `gg`, `G`, `<C-u>`, `<C-d>`.
- **Horizontal Axis (Temporal Navigation)**: Moving through the continuous timeline of the currently playing track using `h` (rewind), `l` (fast-forward), `0` (jump to beginning), `$` (jump to track end), `50%` (jump to middle).

#### 5. Spatial Window Control (`Ctrl-W`) vs. Content Selection (`Visual Mode`)
This is one of the most common pitfalls in keyboard UI design:
- **`Ctrl-W {h,j,k,l}`** is for moving focus across application panes (Sidebar $\leftrightarrow$ Tracklist $\leftrightarrow$ Queue $\leftrightarrow$ Transport Bar). It is instantaneous and directional.
- **`Visual Mode` (`v` / `V`)** is strictly for selecting multiple tracks/albums to perform a batch operation (e.g., selecting 4 tracks with `V 3j` and pressing `p` to enqueue them all, or `d` to drop them from the queue). Conflating window navigation with visual mode breaks the semantic model of Vim.

#### 6. Search as a Motion (`/` and `?`)
In Vim, `/` is not an external floating dialog; it is an in-buffer motion. 
- Typing `/radiohead<CR>` filters or highlights matches in the current list.
- Pressing `n` jumps to the next match; `N` jumps to the previous match.
- Global navigation is reserved for a fuzzy finder / command palette (like `telescope.nvim` or `fzf`), which fits the `<Leader>ff` or `<Leader>fa` pattern.

#### 7. The Command Line (`:`) as the Power Hatch
The `:` prompt provides a natural home for operations that do not require dedicated UI buttons:
- `:sync` — trigger background library indexing.
- `:q` / `:quit` — gracefully save state and exit.
- `:set volume=80` — adjust audio parameters.
- `:sort artist` / `:sort year` — reorder the active view.
- `:help [topic]` — open the keyboard cheatsheet.

---

## 3. Detailed Critique of Musika

### 3.1 Input Grammar & Vi Fidelity

#### Critical Flaw: Conflating Window Management with Visual Mode
- **Current State**: Musika currently uses `Ctrl-W` to enter an overlay modal called `visualMode`. In this mode, the main panes blur, title headers pop up, and the user must press `Enter` to commit focus to a pane.
- **The Issue**: In Neovim, `Ctrl-W` is never a modal lock. `Ctrl-W l` jumps focus to the right split immediately without blurring, blocking input, or requiring an `Enter` confirmation. Furthermore, calling pane switching "Visual Mode" misuses standard Vi terminology, leaving no room for actual multi-track selection.
- **Remedy**: 
  1. Implement immediate window motions: `Ctrl-W h` (Sidebar), `Ctrl-W l` (Main stack), `Ctrl-W j` (Player bar), `Ctrl-W k` (Top status bar).
  2. Reserve `v` and `V` for selecting continuous rows of tracks for batch queueing or deletion.

#### The `<Space>` Dilemma: Play/Pause vs. Leader Key
- **Current State**: Bare `Space` triggers `PlayPause` globally with a 350ms deferral timer to allow `Shift+Space F` for search.
- **The Issue**: 
  1. In the Neovim ecosystem, `<Space>` is universally adopted as the user `<Leader>` key. 
  2. Because `Space` is bound to `PlayPause`, any sequence starting with `Space` incurs a noticeable delay (350ms), creating sluggish playback response.
  3. Conversely, in media players, `Space` is the universal muscle-memory button for play/pause.
- **Remedy**: 
  - Embrace standard media player conventions for bare `Space` (zero latency play/pause).
  - Use `\` (standard Vim default) or `<Leader>` with leader-sequences, or make `<Leader>` configurable. Alternatively, use `Space` as leader in NORMAL mode, and use `c` or `<CR>` for transport toggle when navigating library lists.

#### Missing Count Register & Operator Grammar
- **Current State**: Keys are dispatched as monolithic, hardcoded actions (`MoveDown`, `MoveUp`, `ScrollDown`).
- **The Issue**: Typing `5j` does not move down 5 rows; it simply moves down 1 row (dropping the `5`). Typing `10l` does not seek 10 seconds. There are no operators (`d`, `y`, `p`). This reduces Vi motions to arrow-key aliases rather than an expressive grammar.

#### Single `<Esc>` Invariant is Broken
- **Current State**: Single `<Esc>` is unmapped in the global keymap; closing modals or views relies on ad-hoc QML handlers or `q`.
- **The Issue**: Users instinctively hit `<Esc>` or `Ctrl-[` to leave search palettes, close fullscreen Now Playing, or unfocus input fields. Leaving it unhandled makes the app feel unresponsive.

#### Discoverability & "Which-Key" Deficit
- **Current State**: Multi-key sequences (`g g`, `shift-space f`) have zero visual feedback during typing. Undocumented bindings (`shift-enter`, `q`) require source-code inspection to discover.
- **The Issue**: Vi interfaces rely heavily on either well-known standards or instant discoverability. Without a bottom `showcmd` indicator or a `which-key` popup when keys are pending, beginners and experienced users alike are left guessing.

---

### 3.2 Music Player Domain & Ergonomics

#### Complete Absence of Queue Inspection & Control
- **Current State**: Playback queues are maintained internally in `player.rs` as a flat `Vec<Track>`, but the user has **no UI view of the queue**.
- **The Issue**: In a music player, the queue is the primary working document (comparable to the active buffer in an editor). Without a queue view:
  - You cannot see what song plays next.
  - You cannot reorder upcoming tracks.
  - You cannot clear or dequeue tracks using Vi commands (`dd`).
  - You cannot use `p` / `P` to insert songs into your listening session.

#### Stubbed & Placeholder Features
- **Now Playing Screen**: `NowPlaying.qml` contains hardcoded Radiohead "Weird Fishes / Arpeggi" placeholder lyrics and a static "COVER" box. The real cover art and real track metadata are not bound to this view, and lyrics synchronization is nonexistent.
- **Sidebar Segments**: "Playlists" and "Radio" switch to `PlaceholderList.qml` ("Playlists are not indexed yet"). 
- **Volume & Mute**: Although `mpv` has full volume control and `Player.volume` exists in Rust, there is no volume slider, mute toggle, or keybinding (`+`/`-`, `[v`/`]v`).

#### Navidrome / Subsonic Lifecycle Gaps
- **No Scrobbling**: When tracks finish playing, Musika does not call the Subsonic `scrobble` endpoint. Consequently, listening activity in Musika does not update play counts or "recently played" stats on the Navidrome server.
- **Favorites / Starred**: No mechanism to view or toggle starred/favorite tracks (`s` or `*`).

---

### 3.3 UI / UX & Visual Design System

```
+-----------------------------------------------------------------------------------+
| Top Status Bar (Region 2): LIBRARY [Syncing...] [Progress Bar] [Sync Button]     |
+-------------------+---------------------------------------------------------------+
| Sidebar (Reg 0)   | Main StackView (Region 1)                                     |
|                   |                                                               |
| MUSIKA            | [SearchBar] (Focusable stub)                                  |
| - Albums          |                                                               |
| - Songs           | Central Content Area:                                         |
| - Artists         | List of Albums / Songs / Artists                              |
| - Playlists (stub)| (Cursor moves here via j/k)                                   |
| - Radio (stub)    |                                                               |
|                   |                                                               |
| Stats:            |                                                               |
| 120 Artists       |                                                               |
| 350 Albums        |                                                               |
| 4,200 Songs       |                                                               |
+-------------------+---------------------------------------------------------------+
| Player Bar (Reg 3): [Cover] Title - Artist   [Seek Bar]   [02:14 / 04:32]         |
+-----------------------------------------------------------------------------------+
```

#### The Ghost Focus in Region 2 (Top Status Bar)
- **Current State**: In `Home.qml`, Region 2 is defined as the top status bar. When entering pane navigation, Region 2 can be selected and focused.
- **The Issue**: In `RightBar.qml`, `handleAction(action)` is an empty function that discards all keystrokes. Focusing Region 2 renders all keyboard commands inert—the user cannot even press `Enter` to trigger the "Sync library" button. A non-interactive pane should never capture modal focus.

#### Search Field vs. Floating Search Palette
- **Current State**: `Home.qml` contains an inline `SearchBar` component sitting above the `StackView`, while `Shift+Space F` opens `SearchPalette.qml` (a floating modal overlay).
- **The Issue**: Having an inline, unfocused search field on every page alongside a separate floating search palette creates visual confusion. The inline search bar is non-functional, while the floating palette steals focus completely.

#### Motion & Animation Timing
- **Current State**: Transitions were lowered from 250ms to 100ms.
- **The Issue**: At 100ms, cubic-bezier easing loses its tactile elegance and feels like abrupt frame skipping. The sweet spot for modern responsive desktop applications is 160ms–200ms with smooth cubic-bezier decay (`(0.2, 0.0, 0.0, 1.0)`).

---

### 3.4 Architecture & Systems Engineering

#### The `/dev/shm` + `fd` Search Architecture
- **Current State**: In `search.rs`, `rebuild_palette` creates thousands of 0-byte files in tmpfs (`/dev/shm/musika-search` or `XDG_RUNTIME_DIR`) formatted as `[id] Title - Artist - Album`, and `palette_search` executes an external `fd` binary via `std::process::Command`.
- **The Critique**: 
  - While creative and fast due to Linux filesystem caching, running CLI processes to parse file names is an anti-pattern when an embedded SQLite database (`search.db`) with **FTS5** is already linked to the binary!
  - It adds an implicit runtime dependency on `fd` (`fd-find`). If `fd` is not installed on the host system, search fails silently.
  - Running FTS5 queries or maintaining an in-memory prefix trie (e.g., `fst` or `nucleo-matcher` crate) in pure Rust would be faster, zero-dependency, and memory-safe.

#### Token Security in Process Arguments
- **Current State**: Streaming URLs sent to `mpv` contain plaintext Subsonic authentication tokens (`u=...&t=...&s=...`).
- **The Critique**: On multi-user Linux machines, any local user can inspect arguments of running processes via `ps aux` or `/proc/<pid>/cmdline`. While standard for local workstations, passing sensitive credentials via HTTP headers (`--http-header-fields`) in `mpv` is substantially more secure.

---

## 4. Prioritized Action Plan & Roadmap

```
+-----------------------------------------------------------------------------------+
|                        MUSIKA STRATEGIC IMPLEMENTATION MAP                        |
+-----------------------------------------------------------------------------------+
|  PHASE 1: Vi Modal Engine   --> Formal Mode State Machine (NORMAL, VISUAL, EX)   |
|                                 Direct Ctrl-W window jumps & Single <Esc> reset   |
|                                 Showcmd & Status Mode Indicator                   |
+-----------------------------------------------------------------------------------+
|  PHASE 2: Queue & Grammar   --> Queue drawer (split pane) with j/k navigation     |
|                                 Counts (5j, 10l) and Operators (dd, p, P, yy)     |
|                                 In-buffer search (/ and ?) with n/N cycling       |
+-----------------------------------------------------------------------------------+
|  PHASE 3: Player Completeness-> Volume & Mute control (+ / -, ]v / [v)            |
|                                 Subsonic Scrobbling & Starred tracks              |
|                                 Live NowPlaying bindings (real cover & lyrics)    |
+-----------------------------------------------------------------------------------+
|  PHASE 4: Architecture Clean -> Pure-Rust in-memory search / FTS5 (retire tmpfs)  |
|                                 Secure mpv stream headers                         |
+-----------------------------------------------------------------------------------+
```

### Phase 1: Restoring Modal Sanity (P0)
1. **Explicit Mode State Machine**:
   - Introduce an explicit `Mode` enum in Rust (`Normal`, `Visual`, `Command`, `Search`).
   - Reflect the active mode directly in the top status bar (`[ NORMAL ]`, `[ VISUAL ]`, `[ COMMAND ]`).
2. **Standardize Window Movement**:
   - Replace the blurring `visualMode` overlay with immediate `Ctrl-W {h,j,k,l}` window navigation.
   - `Ctrl-W h` $\rightarrow$ Sidebar; `Ctrl-W l` $\rightarrow$ Central List; `Ctrl-W j` $\rightarrow$ Player Bar.
   - Strip Region 2 (Top Status Bar) from the window focus ring.
3. **The Invariant `<Esc>`**:
   - Single `<Esc>` or `<C-[>` unconditionally drops any active modal, closes the search palette, and resets to `NORMAL` mode.
4. **Discoverability (`showcmd` / which-key)**:
   - Render pending key sequences in the bottom corner of the screen so users can see `g...` or `d...` as they type.

### Phase 2: The Queue & The Grammar (P1)
1. **Queue View & Buffer Management**:
   - Add a collapsible Queue pane (accessible via `Ctrl-W q` or `:queue`).
   - Allow reordering and removing tracks using Vi operators: `dd` to delete track, `K`/`J` to move up/down.
2. **Count Accumulation**:
   - Update `keybind.rs` to parse numerical prefixes (`[count][motion]`).
   - Enable `3j` (skip 3 rows), `10l` (seek forward 10 seconds), `10h` (seek backward 10 seconds).
3. **In-Buffer Search (`/` and `?`)**:
   - Pressing `/` opens an in-line search bar at the bottom of the active list.
   - Pressing `n` jumps to the next match in the view; `N` jumps to the previous match.

### Phase 3: Player Domain Completeness (P2)
1. **Volume & Audio Transport**:
   - Expose volume control via keys: `+` / `-` or `]v` / `[v` for volume stepping; `m` for mute toggle.
   - Display a volume percentage indicator in `InferiorBar.qml`.
2. **Subsonic Protocol Completeness**:
   - Send `scrobble` requests to the Navidrome server when a track surpasses 50% or 4 minutes of playback.
   - Support starring tracks (`s` or `*`) and fetching starred playlists.
3. **Now Playing Screen Wiring**:
   - Connect `NowPlaying.qml` to real album art (`currentCoverUrl`) and real track metadata.
   - Fetch lyrics via Subsonic `getLyrics` API or LRC file parser.

### Phase 4: Architectural Consolidation (P3)
1. **Retire the `/dev/shm` + `fd` Pipeline**:
   - Migrate palette search to use SQLite FTS5 directly or use an in-memory fuzzy matcher in Rust (`nucleo-matcher` or `skim`), removing external binary dependencies.
2. **Transition Timing Polish**:
   - Normalize UI transition speeds to 160ms–180ms for a more refined, fluid feel.
