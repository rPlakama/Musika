# Changelog

All notable changes to Musika, newest first. `TASKLISTMAJOR.md` tracks the
open work; this file is the release history.

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
