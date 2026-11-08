# GPUUI port — index

**Goal:** full QML (64 files) → GPU immediate-mode UI, preserving all behaviour,
animation and effects. Round protocol: **Design → Mechanization → Review →
Continue**.

## Status

| round | artifact | state |
| --- | --- | --- |
| Design | `00-design.md` | accepted |
| Design | `01-mechanization.md` | accepted (M0 ready) |
| Design | `02-review.md` | contract defined |
| Design | `03-decisions.md` | M0-blocking rows resolved |
| Design | 8 × `map-*.md` | **64/64 QML files mapped** |
| Review | `04-design-review.md` | NOT READY as specified → fixes applied |
| Review | `05-primitives.md` | single-owner primitive table |
| Review | `06-design-gate.md` | defects closed, M0 exit criteria |
| Retarget | `09-gpui-retarget.md` | **GPUI confirmed; eframe phases overridden by G0–G7** |
| Mechanization | `11-gpui-g1-report.md` | G1/G2 rows, pages, covers, selection, marks, scroll memory |
| Review | `12-gpui-behavior-review.md` | behaviour/animation matrix; search first slice added |
| Mechanization | `07-m0-report.md` | M0 foundation (theme/fonts/anim/clock/bridge/layout) verified; remaining spikes + M1 next |
| Review | `parity-matrix.md` | 64/64 rows seeded |
| Mechanization | `src/ui/**` + `gpuui` feature | **framework retargeted to GPUI** (branch `gpui`; `09-gpui-retarget.md`) |

## Docs

- [`00-design.md`](00-design.md) — architecture, layers, frame loop, effects,
  animation, seams, parity definition.
- [`01-mechanization.md`](01-mechanization.md) — phases M0–M7, feature/build
  mechanics, module plan, eGUI salvage map.
- [`02-review.md`](02-review.md) — parity matrix contract + review gates.
- [`03-decisions.md`](03-decisions.md) — design/oracle mismatches, M0 rulings.
- [`04-design-review.md`](04-design-review.md) — independent pre-M0 review
  (NOT READY verdict, 6 blocking defects).
- [`05-primitives.md`](05-primitives.md) — authoritative primitive ownership.
- [`06-design-gate.md`](06-design-gate.md) — review disposition, M0 exit
  criteria, first-week slice.
- [`parity-matrix.md`](parity-matrix.md) — 64/64 rows seeded (status todo/waived).
- Per-file maps (all 64 QML files):
  - [`map-a-shell.md`](map-a-shell.md) — Main/Home/Superior/Inferior/LibraryList/
    MediaList/ScrollPanel/PageEntry
  - [`map-b-rows.md`](map-b-rows.md) — ScrollContent/RowWindow/RowBody/
    RowHighlight/RowMarks/RowState/RowMetrics/MarkWindow
  - [`map-c-search.md`](map-c-search.md) — SearchOverlay/SearchInput/SearchText/
    LineSearch/SearchColumn/SearchContract/QuickActions/AltActionsOverlay
  - [`map-d-player.md`](map-d-player.md) — NowPlaying/TransportControls/ScrubBar/
    QueueDrawer/Toast/ToastModal/ExitChip/ExitOverlay/ScopeRing
  - [`map-e-motion.md`](map-e-motion.md) — Anim/ColorAnim/AppText/FadeText/
    PressBounce/PopFx/ShakeFx/FpsCounter
  - [`map-f-imagery.md`](map-f-imagery.md) — CoverImage/CoverBackdrop/
    PlaylistCollage/AcrylicPanel/Scrim/ShuffleChip/PlaceholderList/SkeletonList
  - [`map-g-widgets.md`](map-g-widgets.md) — StarRating/RatingBadge/RatingBurst/
    Selection/SelectionBadge/ScrollMemory/Icon/SymbolIcon
  - [`map-h-controls.md`](map-h-controls.md) — IconButton/ActionBadge/ActionChip/
    DownloadButton/FavoriteButton/Login/DescriptionOverlay

## Target assumption

**GPUI 0.2.2** ([gpui.rs](https://gpui.rs)) — Zed's hybrid immediate/retained
GPU UI framework — as an opt-in `gpuui` feature on current `master`; Qt/QML stays
buildable until parity. `09-gpui-retarget.md` is the authoritative framework/API
override. The eframe `gpuui` branch and the older `eGUI` branch are prototypes,
not merge bases.
