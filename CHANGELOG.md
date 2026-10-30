# Changelog

All notable changes to Musika, newest first. `TASKLISTMAJOR.md` tracks the
open work; this file is the release history.

## 0.4.12 - _Shared QML ownership and screen-sized covers_

- `c8908ea` Share the QML row, motion and mark seams
- `70183b0` Reset the key register when the navigation context changes
- `c6adc6f` Load display-sized cover art for large surfaces

  Cover loading for the album page, artist page and fullscreen player now
  picks a screen/DPR bucket from the painted box instead of decoding the
  provider's full-resolution art. Buckets are reused across resizes, and a
  late full-size payload can no longer overwrite an already-loaded display
  copy.

- Keep display-sized covers stable across width changes: resized files no
  longer derive a missing `.thumb.jpg`, and a late reply for an older size
  bucket is discarded instead of replacing the current cover.

- A song search hit is now a navigation jump: its album page opens with the
  cursor parked on the hit. No queue is built and nothing plays until Enter
  on the album row.

- Open that page from the hit metadata itself, scope album row/info payloads
  to their album id, and keep a cold display cover from swapping in after
  the original has painted.
- Let the cover box settle before decoding, and never swap a display copy in
  after the original has painted: the session-restored AlbumEntry cover now
  loads once instead of reloading through the startup geometry changes.
- Use the old settled resize-anchor pass (next frame + 100 ms) instead of
  snapping on every height event: write contentY directly, preserve a
  manually scrolled Library viewport by its visual top row, and finish the
  repair when an idle window becomes active.
- Rebuild the list follow from the row unit: pin the first/last screenful,
  derive maxY from the row extent (the realised tail wins once it exists),
  clamp contentHeight churn, and keep the cursor visible on settle — so the
  bottom row stays flush and height changes no longer chase stale delegate
  geometry.
- Land `gg`/`G` through the reveal path again: pre-position to the
  row-unit extent, let the target row realise, then settle it, so `G`
  actually loads the bottom instead of waiting for the delegate.
- Capture the row base once and derive rows through Theme.adaptiveRow again,
  so a height resize cannot change the row grid underneath the viewport;
  restore the old count repair and stride-based mark band too.
- After a height resize, if the manual top-row anchor left the cursor
  outside the viewport, recentre it on its target row.
- Remove the grid/cover-wall layout mode: Albums, Songs and ArtistEntry
  are list-only, and Shift-C no longer applies.
- Keep dormant library loaders dormant behind a restored page and skip
  full reloads on preload heads; only the visible scope loads on return to
  Home, and hidden heads warm afterward.
- Share row FontMetrics per surface, move row hover/tap to one list-level
  handler, create mark buttons only when needed, and drop the duplicate
  delegate display margins.
- Give the AlbumEntry identity header a 40% dark backdrop so the title,
  stars and metadata stay legible over the album art.

## 0.4.11 - _Cover improvements and other minor_

-  `54beb51`  Use DPI for CoverLoading
-  `f9b588c`  Bound QML cover and search result memory
-  `f2ff9c3`  Keep album track rows out of the Tab cycle
-  `7c80aae`  Add a Recent order backed by created timestamps
-  `74c883f`  Fix for index err
-  `63507c9`  Load the album cover on a background task
-  `d918965`  Restore the saved page and spawn the queue as an overlay
-  `73f1a7e`  Warm the library tabs behind the restored page
-  `9617df0`  Restore the saved album before the first Home frame
-  `f27c552`  Open Home directly when stored credentials exist

## 0.4.10 - _Internal Mechanism, Polishing Search and smaller things_

Notable changes since `v0.4.9`:


- `e027ed5` Sanitize Nucleo query syntax and cover typo matches
- `7709c8e` Log search result latency in debug builds
- `51110a2` proper debug tag in Download/Render sights
- `6919758` Patch only changed search result rows
- `889c4f6` Run DB search readers in parallel
- `35ded39` Render search scopes as raw text
- `2f2872f`  Give the queue drawer a dark 80 percent tint
- `c593951`  Return to Home in the scope an entry was opened from
- `c961c84`  Fix album tab activation after preload
- `65254bf`  Centralize row typography in one row-size function
- `3e2349e`  Preload the first 50 rows of each library tab
- `4c4a366` Load scopes in parallel and make Esc a vi cancel

    - Songs scope loads the whole list instead of a 300-row page.
    - Album, artist and full song payloads shape rows on rayon's full-core
      pool, bypassing the quarter-core background budget (order preserved).
    - Queue drawer: Esc/Ctrl-[ cancel the selection; only q/Ctrl-Q close.
    - Add ARC-46: preload the first 150 rows of each scope at startup.

- `90a9df1ab` Rowbody increase minimal
- `2148e58e3` Scale up Rowbody fontSize

## 0.4.9 — _Rust scroll memory, fullscreen playback, queue/MPV fixes_

Notable changes since `v0.4.8`:

- `5e124b9` Move scroll memory and row shaping into Rust
- `c30c933` Add the fullscreen playback scene with a black stage
- `b511ef5` Fix lyric band follow, track-title actions, and the Alt/Search handoff
- `11e0734` Shape queue rows, throttle astats metadata, and let the cover take the freed lyric width
- `9d73979` Fix the astats filter label and drop the cover underlay
- `5726a2c` Center the lyrics overlay
- `e2e48c1` Reset lyrics scroll on new-track init

## 0.4.8 — _Native QML controls, lighter rendering, quarter-core background cap_

Notable changes since `v0.4.7`:

- Restore the lyric edge fade so the stage no longer paints opaque gradient bands over the text
- Fix the cover backdrop loader by using the low-resolution GPU blur instead of missing `.blur.jpg` files
- Remove the QML WorkerScript row pipeline (Qt 6.11 null QJSEngine warning)
- `b638cdc` Cap background work at a quarter of the cores
- `c9bcdfb` Trim QML rendering and row layout work
- `d766d8a` Show writers in queue rows too
- `716ef82` Show track writers under album track titles
- `0da5f50` Base the icon button on native AbstractButton
- `447156b` Replace the last mouse areas with native scrub sliders
- `ad7d8c4` Start the native QML audit
- `0552c70` Raise the row and cell size clamps by forty percent
- `36138b9` Clamp row text scaling at a readable minimum
- `f38e3ac` Lay out scroll panel segments with Qt Layouts
- `754d806` Route all app text through one smooth AppText

## 0.4.7 — _Well, UI still needs loads of work;_

All commits since `v0.4.6`:

- `37294d3` Render lyrics with curve rasterization at device resolution
- `cae968c` Bottom-align short album track lists
- `a7d396a` Center row titles in their row
- `716a197` Fit and wrap row titles inside their row
- `21b532f` Paint selected rows above their neighbours
- `e4bb449` Use a blurred cover copy with an 80% shadow
- `fbed9ed` Let the blurred cover read through the tone
- `1f67ada` Tone the blurred backdrop with the second-darkest surface
- `06cef85` Stop washing the NowPlaying cover

## 0.4.6 — _Small fixes, small ribcages, small hips, aren't all they easy to grab?_

All commits since `v0.4.5`:

- `d238008` Share one thumbnail decode per cover
- `d3f4b46` Keep the transport core centered around repeat
- `a1677fe` Make album tracks Tab-selectable and deepen the cover shadow
- `a69630d` Anchor the album track scroller to the bottom
- `9c7bff5` Restore a blurred, shadowed cover backdrop
- `becc25f` Restore smooth cover filtering
- `26ef704` Stop washing the page covers
- `b713298` Remove the cover vignette

## 0.4.5 — _Under the flesh, under the clothes..._

All commits since `v0.4.4`:

- `9b84550` Show transient action badges in the lower-right corner
- `ac54f0a` Keep q immediate and quit after the bye animation
- `4db7c34` Shut down as soon as the bye animation ends
- `42fa90e` Add an elegant qq shutdown
- `07501e2` Scale search rows to their available height
- `092f7ee` Log idle transitions in debug builds
- `504e32c` Pace idle frames while the window is unfocused
- `f7662c4` Bounce and fade the repeat control
- `aed6c1a` Add repeat modes and a transport control
- `e5f8f1b` Trim FPS overlay precision
- `1962abf` Add a debug FPS overlay
- `ffa4874` Apply numeric counts to track skipping
- `1eb1f15` Add a debug startup report under the test module
- `534d7fb` Add vulkan-tools to the development shell
- `9f7b995` Add a Vulkan and GPU probe to the test module
- `c40ffb0` Add database and album benchmarks to the test module
- `663b262` Reuse list delegates and skip cover smoothing
- `82d3df8` Extract library state and info slots from the bridge
- `a96c551` Extract auth and download slot bodies from the bridge
- `d1d8341` Propagate digest failures instead of panicking
- `6626ce1` Replace production unwraps with handled failures
- `24acf53` Give each row segment its own box

## 0.4.4 — _I am really starting to like it now, good work Deepseek-chan_

All commits since `v0.4.3`:

- `5ad92b6` Keep rows and grid cells above their minimum size
- `abfd375` Use gradient vignette instead of canvas
- `ba23c5f` Replace the blurred surround with a vignette
- `ed9cd9e` Scale the cover to the rows band
- `7e599bf` Cap the album cover push at three quarters
- `336ece1` Push the album rows below the displayed cover
- `559b59c` Contain the page covers and blur their surround

## 0.4.3 — _Somewhere with defined style_

All commits since `v0.4.2`:

- `7b768a3` Make the artist title and biography Tab stops
- `0caa6eb` Restyle the artist header after the album header
- `1a6395e` Finish paged lists before jumping to the true end
- `82b572a` Teleport numeric scrolls above fifteen rows
- `721043c` Record the bridge helper extraction
- `5e4e5b8` Extract queue publishing and player sync from the bridge
- `3332cd3` Record the mark-window extraction
- `463b674` Extract the mark window from the QML bridge
- `1551451` Record the modularization pass in the task list
- `155152e` Split the mpv process out of the player queue
- `93064dc` Split browse rows and search payloads
- `97339a7` Split library sync and info caches out of the query module
- `aa20d7b` Move process state out of main into app
- `59f7370` Extract session, digest, offline and cache primitives

## 0.4.2 — _Wouldn't that be strange to have so much information?_

All commits since `v0.4.1`:

- `00f5e73` Polish the album page and consolidate tests into test.rs

## 0.4.1 — _Leaner enough to grab by the hips_

All commits since `v0.4.0`:

- `d6d2e4c` Reduce the Rust memory footprint across cache, sync and payloads

## 0.4.0 — _Now its quite charming, how cute_

All commits since `v0.3.16`:

- `0445f61` Add the VI-8a row clipboard: yank anywhere, paste in the queue
- `5ac2d56` Decouple the queue drawer from the fullscreen player
- `a48a215` Close the FIX-UPS audit: reap mpv, dedupe async work

## 0.3.16 — _Scopes, masks and seated headers_

All commits since `v0.3.15`:

- `34def41` rm quit chip
- `d05b142` Seat the album Tracks header against its rows
- `9da668b` Hide artists and playlists with no rows
- `a50fc34` Fade the lyrics edges and hide empty artists
- `61e7a79` Unify library scopes behind Shift-J/K

## 0.3.15 — _A native scroll modal and covers on demand_

All commits since `v0.3.14`:

- `87a3895` Keep deferred scroll work alive with owned timers
- `fbec9e0` Increase lyric fontSize(clamp)
- `fd2becb` Start switched segments at the first row
- `ce651aa` Drop the ambient cover vignette
- `35b7f1b` Fill large list payloads in event-loop batches
- `6fbcdfa` Rebuild the scroll modal on native Qt positioning
- `592a689` Cache covers on demand and decouple the pass from sync
- `28d536d` Rework the fullscreen player around the album name and a queue drawer
- `566c9dc` non-ambient instead
- `ab10402` Rm Q/ESc chip

## 0.3.14 — _Fixups and better AlbumEntry desing_

All commits since `v0.3.13`:

- `111dac4` Make the album art the full-page ambience
- `440bde6` Keep the sync progress bar collapsed
- `e01649c` Keep the sidebar as a collapsed icon rail
- `34a771e` Compute uniform scroll extent from expected rows

## 0.3.13 — _Better UI, and more general stuff_

All commits since `v0.3.12`:

- `7fbbbd2` Rework fullscreen playback layout and glide queue skips

## 0.3.12 — _Leaning, reducing, correcting_

All commits since `v0.3.11`:

- `b13b00d` Keep the startup auto-login when no music is downloaded
- `b8867ff` Wrap the search overlay keybind tips when the panel is narrow
- `109f631` Drive scroll metrics and flags from context profiles
- `a5e9939` Cache cover thumbnails and fix decode DPR
- `653b70a` Use CoverImage in the inferior bar
- `9bc2782` Track MPRIS playback progress and emit seeks
- `bb9f899` Add an mpv Lua MPRIS service
- `c81aaad` Fold four small dependencies into the source
- `bdae1f9` Keep library scroll steady across resizes
- `320b51c` Unify search into one scoped result list
- `0a9d5ef` Bold the title of the cursor row

## 0.3.11 — _A prefetched scroll band and an invisible remainder_

All commits since `v0.3.10`:

- `104b171` Keep the snapped scroll remainder transparent
- `5a4bb72` Prefetch the scroll band and break the row-unit resize loop

## 0.3.10 — _A continuous row unit and a plain scroll follow_

All commits since `v0.3.9`:

- `e920245` Cap rendering and marks to the visible rows
- `abdcae7` Raise the search row height by 30%
- `8a09736` Fold the queue drawer into the fullscreen player
- `34eedd5` Floor adaptive rows during transient pane layout
- `361bad4` Animate queue removals and keep the cursor steady
- `8f75f72` Make adaptive rows continuous and simplify the scroll follow

## 0.3.9 — _The scroll, rebuilt from the row up_

All commits since `v0.3.8`:

- `1b3c891` Lay the search results out in columns
- `7c3fef3` Rebuild the scroll body around whole-row centring
- `4dce9dc` Stabilise the scroll follow across rebuilds, resizes and the content end
- `2ee3c85` Pin the scroll follow at the content ends instead of padding it
- `d072d75` Remake the scroll body around an always-centred follow
- `713fbe4` Restore Qt contentHeight as the scroll extent authority
- `bab6e0f` Glue the end of the scroll to the last row
- `a19b4f9` Re-snap the cursor through multi-frame resize relayouts
- `4d4957f` Make the scroll follow a live frame retarget instead of a fixed animation
- `e9ee8af` Snap the scroll position after a resize instead of gliding
- `e23a005` Reload library lists on width changes, keeping the cursor
- `abad2d3` Refresh derived caches with album changes; fix squeezed scroll tail

## 0.3.8 — _Improving general search and row conformity_

All commits since `v0.3.7`:

- `a588f7e` Scroll: re-centre the cursor after a row-unit resize
- `659ea69` Search: default to the hybrid Nucleo + FTS engine
- `81af5af` Restore the FTS table and keep debug logs gated
- `43623b8` Revert "Search: make Tantivy the only engine"
- `bf31f11` Search: make Tantivy the only engine

## 0.3.7 — _Rouding the Edge... Balling the steel... Run._

All commits since `v0.3.6`:

- `bb462b9` Re-centre lyrics quickly on width changes
- `8da2a5b` ARC-SEARCH: default to the in-process Nucleo index
- `5be6eda` Fix the G/k end-of-list gap in ScrollModal
- `516ed0e` ScrollRevamp phases 1-3 and smoother search/glide feedback
- `a4d95ea` Decouple search and finish the search empty state
- `40c2a3f` Start ScrollRevamp: scroll quick wins and QML warning fixes

## 0.3.6 — _Navigation history and one scroll row model_

All commits since `v0.3.5`:

- `4bfa646` ARC-45 travel history and one scroll row model

## 0.3.5 — _Performance and playback pass_

All commits since `v0.3.4`:

- `3ed4304` Launch mpv with safe hardware decoding
- `de4b10b` PATH: index and chunk the DB paths, trim QML, cache offline, batch ratings
- `a2cef7f` PATH: coalesce mark reads and trim queue, status and metadata paths
- `0704fd8` PATH: share the API client, target maps, page Songs, bound marks

## 0.3.4 — _Local-first rating stars, cover actions and lenient search_

All commits since `v0.3.3`:

- `da196e8` Local-first rating stars, cover cycles and lenient search
- `ac2b5c7` Share the favourite button and put album actions on the cover

## 0.3.3 — _Improve in Favorite System_

All commits since `v0.3.2`:

- `4beeacc` Merge favourite and rating into one system
- `8f625a3` Fix album download mark erased at pass end

## 0.3.2 — _Extras and engine stuff_

All commits since `v0.3.1`:

- `cc14325` Engine optimizations: Vulkan default, lazy segments, NowPlaying loader and acrylic gating
- `4c63971` ARC-IDK1: search UI breathing room and opaque acrylic tint
- `8aa7fc5` Anchor album song title to the duration box

## 0.3.1 — _Optimizing and reducing_

All commits since `v0.3.0`:

- `ff826b0` Combine logical databases into two
- `981704f` Forget the previous queue scroll memory
- `3c4d3c2` Predict offscreen rows and fix queue edge jumps
- `b571eb5` Share one row body across lists and queue
- `bd58920` Split QML backend helpers by purpose

## 0.3.0 — _Reciclagem eh amor_

All commits since `v0.2.2`:

- `85dd21c` Track ARC note and note icon
- `799f391` Share QML spawn, auth and song JSON helpers
- `3e9b786` Share time, directory, atomic-write and QML slot helpers
- `010a398` Recycle colors, QML plumbing and row builders (REC-20..27)
- `9871c50` Recycle persistence, sockets, player and palette (REC-12..19)
- `cadff52` Recycle selects, DB setup and SQL assembly (REC-8..11)
- `973ec2e` Recycle HTTP, lyrics, deletes and song queries (REC-4..7)
- `c4c6424` Dedup API envelopes and fan-out (REC-1..3)
- `724a543` Config flag and configurator groundwork

## 0.2.2 — _Small fixes and patches_

All commits since `v0.2.1`:

- `a7d4bda` Order swaps force a true ScrollContent reset (#bug 4)
- `b99d55a` security build path
- `6373988` tracking

## 0.2.1 — _Scary downloads (and Super Corrections)_

All commits since `v0.2.0`:

- `8ffdf62` README: features, databases, sections, contributions, installation
- `83b22e8` Theme.radius everywhere; favourite lands per row (#bug 3)
- `a10145e` Selection download toggle; wipe all media; bigger biography

## 0.2.0 — _Woah, its pretty usable now! From Garbage to Trash_

All commits since `v0.1.9`:

- `99c7d3d` Queue revision gate; rule-owned downloads; whole-queue shuffle
- `f07aa5f` Tab walks the shell's small actions; ARC-REV filed
- `5e22614` Tab stop for the shuffle chip; endless passes of 30
- `b834c61` VI-7 count register; reusable tab shuffle (rolling 30)

## 0.1.8 — _Push push push!_

All commits since `v0.1.7`:

- `2aecca8` Login: log the online mode on a successful ping

## 0.1.7 — _Talk! Lyrics, Arguments...!_

All commits since `v0.1.61`:

- `67951ac` NowPlaying: lyrics ride the shared scroll modal — soothing 300
  ms follow
- `60a3284` TASKLIST: LYR-8 now rides the shared scroll body (followMs 300)
- `051bbd7` Lyrics: soothing follow 500 ms (was 300)

## 0.1.61 — _Small fix Patch_

All commits since `v0.1.6`:

- `bcdfe1f` AlbumEntry: "Tracks" wording; land row 0 on open (the second-J
  bug)
- `9fb0412` Icon: launcher wordmark black-on-white (white bg, black
  letters)

## 0.1.6 — _Patching a worried heart!_

All commits since `v0.1.5`:

- `7958335` Scroll: keep-alive segment host + far-landing snap
  (SCROLLMODELANALYSIS.md)
- `64746d3` ScrollContent: rewrite the body around SCROLLMODELANALYSIS.md
- `4643500` Scroll: instant landing — a far cursor never walks back
  (analysis §3)

## 0.1.5 — _I am no Model Man_, but here I give everything for you! (A Lie!)

All commits since `v0.1.4`:

- `0d3eff5` Wipe: reload the UI after any wipe — queue covers, palette,
  counts
- `d097c0f` Fullscreen: contained transport/volume bar; lyrics re-centre on
  seek
- `0763115` Search: instant gg/G snaps; retire the mode chip (centred
  placeholder, outline flash)
- `5decec2` TASKLIST: fold the REC batch into Quick wins
- `ccd41fe` Fullscreen: centre the transport until the volume box pushes it
  left
- `1968f0a` AlbumEntry: blink-and-fade the hero cover across the compact flip
- `a6dfcc2` AlbumEntry: dim-and-blur the hero cover across the compact flip
- `03599a5` Search: open the / command line over every surface (ARC-UI-2 M0)
- `8a98d80` NowPlaying: ramp the track title with the window width (UI-57)
- `6967313` Search: row matching with Tab cycling and context-Enter commit
  (ARC-UI-2 M1)
- `1e69c0f` Search: shared input/pop FX, global targets, brand artwork, cache
  probes
- `56a35c5` Search: whole-window buffer, viewport matching, hlsearch blocks,
  slide pop-ups
- `42e4a6a` ScrollContent: adaptive rows — cut a row past a 30% squeeze
  (UI-58)
- `0f028fc` Chrome: retire Ctrl-W (up-edge, H/L scope); ramp bar type; breathe
  collapsed rows
- `fe1421a` Chrome: batch favorite on visual selections; LeftCenter the bar
  title
- `92247ba` Chrome: optimistic favourite heart; centre the empty bar title
- `49a8136` TASKLIST: ARC-44 — playlist.db plan (Subsonic + M3U), P-4
  absorbed
- `d558c44` ScrollContent: ARC-UI-3 list/grid modes — ArtistEntry rides the
  shared body
- `5a4cc4f` ScrollContent: one modal everywhere — MediaList grid,
  ArtistEntry on the panel, sooner squeeze
- `04a5ba2` Icons: rebake the set white; add heart-add-fill, heart-minus and
  the Spinlogo source
- `0875321` TASKLIST: ARC-UI-4 — queue add/remove row marks on the favourite
  scheme
- `5f8662d` Favourites: local flag write + in-place hearts; Playlists:
  playlist.db M0 + the Subsonic list (ARC-44)
- `e1f33be` Lists: filter/order swaps reset to row 0 on an instant snap (no
  follow walk)

## 0.1.4 — _Inertia Creeps_ (Width! Height! All that moves, creeping in our eyes, perhaps, recollect me!)

All commits since `v0.1.3`:

- `93ac03b` UI-57: shared widthScale ramp — status/player chrome; header band
  −30%
- `f5c2ba9` rm unecessary copy of navidrome ref
- `3c5545b` UI-57: responsive list header — title ramp + shrinking band in
  narrow viewports
- `bbcb975` ARC-8: one sync pipeline (spawn_sync) for manual, login and
  auto-login

## 0.1.3 — _Actions! Actions! It's all talk! Talk! Chitat! <-- ifuk, uk_

All commits since `v0.1.2`:

- `031832e` ArtistEntry: Tab cycles the Download/Shuffle/Favorite chip scope
  (shared ring)
- `6817ea4` Alt menu: release-over-neutral cancels (Esc swallowed); click
  tiles fire, click outside closes
- `cff7020` Toast: grows with the message — wrap, near-fullscreen cap, scroll
  fallback
- `3a6ee16` Wipe menu: scoped quadrants via the trashcan Alt menu; SuperiorBar
  panes + Tab scope
- `c5cbfab` DB: fix the artists INSERT (6 values for 7 columns); replace
  regression test
- `a9bc3d8` ARC-7: trim api.rs to the 14 used endpoints — 1894 → 514 lines
- `03b2517` ARC-UI-1: springy zoom-in spawn; arrow-move neutral glyph
- `12a279c` UI-46/VI-4: view-owned follow + model-owned selection; ARC-7/8
  inits
- `9f5da76` VI-4: visual row selection (v/V) — count badge, batch Alt
  queue-add
- `4ea3695` Q-5: windowed mpv playlist — gapless transitions, stream-error
  retry/auto-advance
- `0603994` UI-56: +30% cover-corner favourite mark; ARC-I-4 favourite toggle
  task
- `e7eed95` ARC-I-2: Alt menu in AlbumEntry, album favourite mark;
  cover-corner star in ArtistEntry
- `7bdff13` ARC-I-2/3: initialise AlbumEntry Alt menu and ArtistEntry chip
  keybinds
- `b2f1e90` UI-56: favourites lead with a white heart before the meta in every
  region
- `ff6bdda` ARC-I: Alt menu in the Search overlay (recycled verbs, artist
  hits)

## 0.1.2 — _Alting_ my way into shortcuts!

All commits since `v0.1.1`:

- `10abacd` ARC-UI-1: square Alt frame with bounded gap; halve the option tiles
- `81c1c55` ARC-UI-1: fullscreen Alt menu, recycled QuickActions, SVG verb tiles
- `da6d2d1` ARC-UI-1: board-owned overlay layout — square tiles, window clamp +
  scale ramp
- `bfcedc6` ARC-UI-1: four fixed Alt menu verbs; album_id in song rows for Go
  to Artist
- `7b8a9d5` REC-1..27: initialise all recycler findings in TASKLISTMAJOR
- `ac75c98` Initial Alt_Actions Commit

## 0.1.1 — MUSIKA, Sloppy Release! Amazing Update!

All commits since `v0.1.0`:

- `f81668f` CACHE-9: offline-first auto-login, downloaded-only catalog; UI-55
  quick-win reorg
- `726a462` UI-49/UI-13: search mode chip beside the bar; retire the scroll
  hint
- `891964e` S-9/AUTH-3/DOC-1: Most-played empty search + usage boost, login
  status roles, docs sync
- `bb39e88` UI-55: acrylic panes over ambience; tiled-PNG grain;
  statusRestored fix
- `c6c8c2f` UI-55/LYR-8: acrylic drawer, lyric resume timeout, shared
  ScrollPanel
- `ea78ed3` UI-54: one-step scroll recall (kill the restore flick); held-key
  acceleration
- `aae5d65` UI-54/UI-47: shared ListScroll memory seam, highlight recall;
  search divider z-order
- `221fa26` UI-13/UI-40/UI-54: scroll hints everywhere, palette crossfade,
  page scroll memory
