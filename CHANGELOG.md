# Changelog

All notable changes to Musika, newest first. `TASKLISTMAJOR.md` tracks the
open work; this file is the release history.

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
