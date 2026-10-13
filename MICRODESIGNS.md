# Musika — Microdesigns

Small UI choices, redundant information and polish for the **current**
implementation. Audited at commit `d210011` (search typing/normal mode, ESC/i
binds, external command IPC). One bullet = one task; file:line refs point at
today's code.

Legend: `[ ]` todo · `[x]` done. **P0** the UI lies · **P1** clear win ·
**P2** polish · **P3** nice-to-have.

## Chrome & status

- [ ] **MD1 (P1)** Region cursor: four different frame colours, and PLAYER uses
  `Theme.error` (red) which reads as a failure. Unify on one accent, or only
  draw frames in visual mode (Home.qml:384-431).
- [ ] **MD2 (P1)** Drop the always-on "Shift Space F for Search" strip; it owns
  a full row and becomes noise after day one (Home.qml:220-234).
- [ ] **MD3 (P2)** "Syncing…" appears twice at once — status text and button
  label — plus a progress track that stays visible when idle. Keep one status,
  disable (don't relabel) the button, hide the track when not syncing
  (RightBar.qml:76-130).
- [ ] **MD4 (P2)** Counts live in three places: sidebar stats, each list header
  ("N indexed") and the list content. Pick one home for counts
  (SideBar.qml:111-151, AlbumList.qml:93-100, SongList.qml:89-96,
  ArtistList.qml:83-90).
- [ ] **MD5 (P2)** Two titles stack ("MUSIKA" sidebar brand + "LIBRARY" bar).
  Brand only on login; status bar says the segment or app name
  (SideBar.qml:27-45, RightBar.qml:61-70).
- [ ] **MD6 (P3)** Placeholder segments (Playlists, Radio) look selectable but
  only open an empty state; dim them or mark them "soon" (SideBar.qml:54-108).
- [ ] **MD7 (P3)** Rotated "LIBRARY" label in visual mode is unreadable at
  glance (Home.qml:420-429); replace with a small square tag or drop the label.

## Floating search

- [ ] **MD8 (P1)** Fullscreen is not escapable: hint says "ESC to exit
  fullscreen"/"Esc close", but ESC (now `Action::Back`, keybind.rs:83) only
  pops the content stack (Home.qml:130-135) — so in fullscreen it silently
  pops the album page *behind* the overlay. Make `back` close `nowPlaying`,
  and update the hint to the real keys (NowPlaying.qml:203-207, Home.qml:95-99).
- [ ] **MD9 (P2)** "Esc close" appears twice (input row and footer), and the
  two-stage ESC (typing → normal → close) isn't communicated anywhere. Keep
  one cheat sheet that describes the real modes (SearchOverlay.qml:302-306,
  :382-388).
- [ ] **MD10 (P2)** Column order contradicts the design and the file's own
  docstring: roadmap and the header comment say Songs | Albums | Artists, the
  layout renders Artists | Albums | Songs. Align one of the two
  (SearchOverlay.qml:7-10, :314-379, roadmap.md:77-93).
- [ ] **MD11 (P2)** Key model is inconsistent between modes: typing mode has
  Tab/arrows only (no left/right column move), normal mode has h/l/j/k only
  (arrows dead) — yet the footer advertises "Tab column · ↑/↓ row" unqualified.
  Make arrows and Tab work in both modes, or label the hints per mode
  (SearchOverlay.qml:283-295, :59-75).
- [ ] **MD12 (P3)** No visual mode indicator in the overlay: the only cue that
  typing stopped is the field losing its border. Show `typing` in the field or
  footer (SearchOverlay.qml:16, :274-279).
- [ ] **MD13 (P3)** Artist meta is "14 alb"; everywhere else it's "N albums"
  (SearchOverlay.qml:170). Use one wording.
- [ ] **MD14 (P3)** Row text is `title + "   " + subtitle` in one colour;
  separator differs from lists ("·"). Split title/subtitle styling
  (SearchColumn.qml:105-115).
- [ ] **MD15 (P3)** Hover silently steals the keyboard cursor (any mouse pass
  over a column re-targets `activeColumn`); only switch on click
  (SearchOverlay.qml:326-334, :348-356, :370-378).

## Lists (albums / songs / artists)

- [ ] **MD16 (P1)** Rows are fractions of the viewport (Album/Song/Artist
  heights 0.105/0.09/0.105), so row size should be a fixed unit per view: rows
  shouldn't grow with the window (AlbumList.qml:124, SongList.qml:120,
  ArtistList.qml:111).
- [ ] **MD17 (P2)** Three near-identical delegates (zebra + cursor + cover +
  two-line text). Extract one `ListRow` component so states stay in sync
  (AlbumList.qml:111-195, SongList.qml:107-187, ArtistList.qml:101-169).
- [ ] **MD18 (P2)** Hover feedback is inconsistent: AlbumList tints the cover
  only, other lists have none; cursor starts at -1 so no row is highlighted
  until the first keypress. Settle row hover + initial cursor
  (AlbumList.qml:145, ArtistList.qml:124-143, SongList.qml:133-152).
- [ ] **MD19 (P2)** `FadeText` on every delegate means rows fade while
  scrolling and re-created rows blink. Reserve the fade for view-level
  transitions (FadeText.qml:5-12).
- [ ] **MD20 (P3)** Album row shows "N tracks" right-aligned; artist row shows
  "N albums" under the name. Pick one metadata line position
  (AlbumList.qml:176-183, ArtistList.qml:156-160).
- [ ] **MD21 (P3)** No scroll position hint in long lists; a thin proportional
  scrollbar would help mouse users (all three lists).

## Album page

- [ ] **MD22 (P2)** Notes are clamped to 3 lines with no way to expand; allow
  Enter/click to show full text (AlbumEntry.qml:217-228).
- [ ] **MD23 (P3)** "‹ Back" toolbar adds chrome that duplicates `q`/ESC after
  MD8; keep it only if it gains a real toolbar (AlbumEntry.qml:110-153).
- [ ] **MD24 (P3)** Cover fallback says "COVER"; use a music glyph or blank
  surface (AlbumEntry.qml:179-186).

## Player bars

- [ ] **MD25 (P1)** Remove or wire inert controls: shuffle `⤨` does nothing,
  stop `■` ends without a clear purpose, and the volume bar is a static 60%
  fill with an emoji speaker that ignores the player volume
  (InferiorBar.qml:133-138, :161-167, :174-202).
- [ ] **MD26 (P2)** Pause glyph differs between bars (`❚❚` vs `⏸`) and the
  speaker is an emoji, which breaks the monochrome palette
  (InferiorBar.qml:149, InferiorBar.qml:186, NowPlaying.qml:191).
- [ ] **MD27 (P2)** Fullscreen transport breaks the no-fixed-px rule
  (height 6, radius 3, spacing 8, buttons 32/36); derive from bar height
  (NowPlaying.qml:151-162, :180-201).
- [ ] **MD28 (P2)** Fake hardcoded lyrics render for every track; show an
  empty state until real lyrics land (NowPlaying.qml:119-127).
- [ ] **MD29 (P2)** Equal split in fullscreen: cover takes 50% of the pane,
  lyrics the other 50% (today cover is 0.4 / lyrics float the rest). When
  synced lyrics exist, animate the active line (highlight + auto-scroll to
  center); plain text otherwise (NowPlaying.qml:30-33, :79-130).
- [ ] **MD30 (P3)** Duplicate `formatTime` in six places; centralise it
  (InferiorBar.qml:12, NowPlaying.qml:12, SongList.qml:16, AlbumEntry.qml:26,
  PlaybackNode.qml:14, SearchOverlay.qml:123).
- [ ] **MD31 (P3)** Delete unused `PlaybackNode.qml` (nothing instantiates it;
  it duplicates InferiorBar/NowPlaying).

## Key routing / modes

- [ ] **MD32 (P2)** Bare Space still plays through the modal search: the
  global `play_pause` shortcut is dispatched before Home sees the overlay
  (Main.qml:24-29), so Space in search normal mode toggles playback behind the
  panel. Route it through the overlay first.
- [ ] **MD33 (P3)** `i` (`Action::InsertMode`, keybind.rs:82) is dead outside
  the search overlay; either consume it everywhere (a mode indicator) or scope
  the binding where it's meaningful.
- [ ] **MD34 (P3)** The keyTimer comment claims it resolves "bare Space =
  play/pause" (Main.qml:65-68); Space actually matches immediately and only
  `g g`/`shift-space …` are deferred. Fix the comment or the mechanism.

## Consistency sweep

- [ ] **MD35 (P2)** Mixed icon language: unicode triangles/boxes/emoji and the
  `⌕` search glyph all at once; pick one family and one sizing rule
  (IconButton.qml:4, SearchOverlay.qml:262, InferiorBar.qml:136-165).
- [ ] **MD36 (P2)** Subtitle separators drift ("  ·  " in lists, "   " in
  search, "·" in headers); define one (AlbumList.qml:170,
  SearchColumn.qml:111).
- [ ] **MD37 (P3)** Font ratios drift between views for the same role (title
  0.24/0.26/0.26; subtitle 0.19/0.2/0.2); pin them in Theme.js
  (AlbumList.qml:167-172, SongList.qml:160-168, ArtistList.qml:151-159).
- [ ] **MD38 (P2)** Toast retry exists only for `kind === "sync"`; playback
  errors get no action. Either offer retry for both or drop the button
  (Toast.qml:59-83).
