# Musika — Material You 3 × Qt Sharpness

**A design study and codebase diagnostic: motion, style, and the Neovim layer.**

> Status: study + proposal. Everything under "Diagnostics" was read from the
> working tree at the time of writing (a moving target — the search overlay is
> being actively extended). Companions: `CRITIQUE.md` (UX review),
> `TASKS.md` (action items), `roadmap.md` (design intent).

---

## 1. Thesis

Material You 3 is **personal, tonal, expressive**: a seeded palette, surface
containers instead of chrome, generous shape, and motion that breathes.
Qt/KDE-sharp is the opposite pole: **square, dense, crisp, instant**, built for
keyboards and 1-device-pixel lines.

Musika sits exactly on that seam. The blend works when MY3 supplies the
*vocabulary* — color roles, tonal hierarchy, motion grammar — and Qt sharpness
supplies the *discipline* — zero radius, hairlines, never blocking the keys.

### 1.1 Seven rules

| # | Rule | Consequence |
|---|------|-------------|
| R1 | **Surfaces carry hierarchy, not shadows.** | Use the surfaceContainer ladder; never `DropShadow`. |
| R2 | **Elevation is drawn as a hairline.** | `Math.max(1, h * Theme.hairlineRatio)` in `outlineVariant`/`primary`; no blur "depth" except modal scrims. |
| R3 | **Shape is binary + one.** | `0` for containers, `Theme.radius` (3) for interactive surfaces, `full` for avatars only. |
| R4 | **Motion is decisive, never bouncy.** | Short fades/rises, one "emphasized" tier for structural changes, no springs. |
| R5 | **Input latency beats visual delight.** | Mode switches and cursor moves are `instant` or ≤80 ms; nothing queues behind animation. |
| R6 | **State is always visible.** | Hover/focus/press/disabled = state layer + border, never color alone. |
| R7 | **Proportional, with clamps.** | Fractions for layout; min/max caps for type and hairlines (device-px aware). |

---

## 2. Diagnostics — current tree

### 2.1 Theme tokens (`src/qml/Theme.js`)

| Token | Current | Diagnosis |
|-------|---------|-----------|
| `radius` | `3` | Good ("the only softness"); violated by literal `3` in `NowPlaying.qml:156,162`. |
| `duration` | `100` | One token for everything. Too abrupt for view transitions (U9), too slow for cursor state. |
| `easeOut` | `[0.2, 0, 0, 1]` | Correct standard curve; missing on a few behaviors and meaningless on progress bars. |
| ratios | `0.15 / 0.09 / 0.115` | Fine; but components re-derive their own fractions ad hoc. |
| `hairlineRatio` | `0.0018` + `Math.max(1, …)` | The best pattern in the codebase; the only sanctioned px. |
| palette | 18 roles | Solid MY3-ish dark seed, but incomplete — see below. |

**Missing color roles** (needed by already-implemented states):

`errorContainer` / `onErrorContainer`, `tertiaryContainer` /
`onTertiaryContainer`, `surfaceTint`, `inverseSurface` / `onInverseSurface`,
`scrim` (currently hardcoded `#000000`), `surfaceDim` / `surfaceBright`, plus
the **state-layer alpha set** (R6).

### 2.2 Motion inventory

| Where | What | Timing | Note |
|-------|------|--------|------|
| `Theme.js:12` | every animation | 100 ms | no tiers |
| `FadeText.qml:6` | every created label fades in | 100 ms | charming, but applied globally and unconditionally |
| `Home.qml:243-262` | StackView push/pop/replace, 6 transitions | 100 ms | duplicated 4×; exits lack the Bézier curve; should be one factory + emphasized tier |
| `Home.qml:339-341` | NowPlaying fullscreen fade | 100 ms | acceptable, retune with tokens |
| `Home.qml:415-417` | region focus border width | 100 ms | mode/cursor changes should be `instant`/`micro` (R5) |
| `Home.qml:177-213,327-331` | visualMode: 4× MultiEffect blur `0.35` | static | expensive, semantically wrong (N1) — replace with scrim + dim |
| `SearchOverlay.qml:197` | search debounce | 120 ms | correct for type-ahead |
| `SearchOverlay.qml:~224` | backdrop fade | 100 ms | no easing declared |
| `SearchOverlay.qml:~242-250` | panel fade + scale 0.98 | 100 ms | should be standard/emphasized; prefer rise over scale |
| `Toast.qml:22` | toast opacity | 100 ms | needs quick-in/lazy-out |
| `PlaybackNode.qml:114`, `RightBar.qml:94` | progress width | 100 ms easeOut | progress morphing should be `linear` |
| `Main.qml:68` | deferred key resolver | **800 ms** | latent bare-key action; directly feeds the Space dilemma (N6) |

Missing entirely: motion for mode changes (N4), count accumulation (N2a),
`showcmd`/which-key (N8b), command line (N5a) — none of those features exist
yet, so this document defines their motion before the code exists.

### 2.3 Style inventory / drift

| Kind | Location | Issue |
|------|----------|-------|
| Hardcoded scrim | `SearchOverlay.qml:221` | only literal color outside Theme.js; should be `Theme.scrim` |
| Hardcoded opacities | blur `0.35` (Home), scrim `0.5`, disabled/skeleton `0.6` (RightBar:109, SearchColumn:165) | no tokens |
| Fixed px | `NowPlaying.qml:154,183-197` (`6/32/36`) | violates "fractions only" (R7/U10) |
| Layout weights | `SearchOverlay.qml:317,340,362` (`28/32/40`) | these are RowLayout weights, not px — document as intentional |
| z-index | `Home.qml:351` (20), `:407` (10), `SearchOverlay.qml:24` (30) | ad hoc; command line/which-key will need a ladder |
| Typography | 16 QML files with inline `font.pixelSize` | no role scale, no clamps (U10) |
| Focus states | `IconButton.qml:22-25` (hover only), `RightBar.qml:43` (empty `handleAction`) | keyboard focus invisible/trapped (U2/U11) |
| Region colors | `Home.qml:371-376` primary/tertiary/secondary/error | identity labels, not semantic states |
| Shape exceptions | avatars full-circle (`ArtistList.qml:131`, `SearchColumn` `roundedArt`) | fine — codify as the only exception (R3) |
| Cursor style | list rows use `secondaryContainer` | good; make it the canonical "cursor" token |

### 2.4 Neovim layer

`keybind.rs` has grown to 15 flat actions (`Action` enum, `keybind.rs:13-31`)
and now includes `InsertMode`; `escape`/`q` map to `Back`. There are still **no
counts, no operators, no mode state**. `ctrl-w` still toggles the blurring
`visual_mode` overlay (N1), `gg`/`G` have no handler (N8a), and sequences are
resolved by an 800 ms timer (`Main.qml:68`).

The important new development: **`SearchOverlay.qml` is the first real modal
sub-machine in the app**. It has `typing` ↔ "search normal" states
(`SearchOverlay.qml:16,42-75`): `Esc` steps typing → normal → closed, `i`
re-enters typing, `hjkl` navigates results, `Enter` opens/plays, and
`Home.qml:90-93` routes global actions into it while open. This is exactly the
pattern to generalize into a global mode machine (N4).

Still missing, mapped to the task list:

| Gap | Task | Design consequence |
|-----|------|--------------------|
| Global mode state + status display | N4 | no mode colors, no motion vocabulary |
| which-key / showcmd | N8b | pending sequences invisible (800 ms of silence) |
| Counts `3j`, `5l` | N2a | no count register chip |
| Operators `d/y/p` | N2b | no visual feedback for queue edits |
| `:` command line | N5a | no style/motion for a command surface |
| `/` in-list search | N5b | search lives only in the overlay |
| Pane navigation | N1 | blur overlay instead of `Ctrl-w h/j/k/l` |
| `gg`/`G` wiring | N8a | dead bindings |
| Enter/`q` disambiguation | N9 | false affordances (`NowPlaying` advertises an unbound ESC) |

### 2.5 Severity rollup

| Sev | Meaning | Examples |
|-----|---------|----------|
| **S0** | Blocks input trust | 800 ms defer (N6), invisible/trapped focus (U2/U11), dead `gg`/`G` (N8a) |
| **S1** | Wrong motion semantics | blur overlay (N1), 100 ms structural transitions (U9), no mode motion (N4/N8b) |
| **S2** | Token/role drift | hardcoded scrim/opacities, literal radius/px, ad-hoc z, type roles (U10) |
| **S3** | Cosmetic completeness | missing state layers, container colors, icon polish (U3 done) |

---

## 3. Proposed token system

### 3.1 Motion

**Durations** (`Theme.motion.duration`):

| Token | ms | Use |
|-------|----|-----|
| `instant` | 0 | mode switches, cursor moves, `showcmd`, key echo |
| `micro` | 80 | hover/press layers, region focus border, which-key appear |
| `quick` | 120 | toast in, list row cursor color, tab/column switch |
| `standard` | 160 | view push/pop/replace, overlay in/out, command line rise |
| `emphasized` | 240 | fullscreen player, first paint of a new context |
| `lazy` | 320 | toast out, destructive confirmation, nothing keyboard-critical |

**Easings** (`Theme.motion.ease`): `standard` `[0.2, 0, 0, 1]`,
`emphasized` same curve at longer duration, `decelerate` `[0.05, 0.7, 0.1, 1]`,
`accelerate` `[0.3, 0, 0.8, 0.15]`; progress bars use `linear` (no curve).

**Recipes** (what to apply where):

| Event | Properties | Tokens |
|-------|-----------|--------|
| Mode change (NORMAL↔…) | mode chip color, border | `instant` |
| Cursor row move / hover | row color | `quick` (or `instant` in lists, per taste) |
| Region/pane focus (`Ctrl-w`) | border color | `micro`, width `instant` |
| View push/pop | opacity, translate-y 2% | `standard` in / `accelerate` out |
| Fullscreen player | opacity, scale 0.99→1 | `emphasized` |
| Floating search open | opacity + rise 4% | `standard` |
| Backdrop/scrim | opacity | `standard` |
| Search result refresh | none (swap in place) | `instant` |
| Toast | opacity | `quick` in / `lazy` out |
| Progress/seek/volume | width | `linear`, ≤120 ms |
| which-key / showcmd | opacity + rise | `micro` in, `instant` out |

Rule of thumb: **if the user's next keystroke can race the animation, make it
instant.**

### 3.2 Shape

| Token | Value | Applies to |
|-------|-------|-----------|
| `shapeSquare` | `0` | panels, lists, rows, dividers, region boxes, tooltips |
| `shapeSoft` | `Theme.radius` (3) | inputs, buttons, focusable rows, thumbs, toasts |
| `shapeFull` | `width * 0.5` | avatars only (artists) |

Never nest a soft radius inside a square clip without padding — QML clipping
shows the seam. Covers stay `shapeSoft` (3) for consistency with their frames.

### 3.3 Color roles & state layers

Add to `Theme.js`:

```
errorContainer / onErrorContainer     // toasts, destructive
tertiaryContainer / onTertiaryContainer // sync, progress, insert mode
scrim (#000000)                        // modal backdrop, opacity token 0.5
inverseSurface / onInverseSurface      // future tooltips
surfaceTint                            // reserved (no shadows)
```

**State layers** (overlay color on the base surface):

| State | Layer alpha | Border |
|-------|-------------|--------|
| hover | 8% `onSurface` | none |
| focus (keyboard) | 10% `primary` | 1 device-px `primary` |
| pressed | 10% `onSurface` | none |
| dragged | 16% `onSurface` | none |
| disabled | 12% container, 38% content | none |
| selected/cursor | `secondaryContainer` fill | none |

**Mode colors** (once N4 lands): NORMAL `primary`, INSERT `tertiary`, VISUAL
`secondary`, COMMAND `outline`, SEARCH `primaryContainer`.

**Tonal elevation ladder** (R1):

| Level | Role | Components |
|-------|------|-----------|
| 0 | `surface`/`background` | window, Home |
| 1 | `surfaceContainerLow` | sidebar, player bar |
| 2 | `surfaceContainer` | list rows (alternating) |
| 3 | `surfaceContainerHigh` | cards, search panel, toast |
| 4 | `surfaceContainerHighest` | inputs, secondary regions |

### 3.4 Typography roles

Keep proportional sizing, but name the roles and clamp them (R7/U10):

| Role | Fraction | Clamp | Current use |
|------|----------|-------|-------------|
| `display` | `h * 0.055` | 22–44 px | NowPlaying title |
| `headline` | `h * 0.04` | 18–32 px | album/section titles |
| `title` | `h * 0.032` | 14–24 px | region headers, column headers |
| `body` | `h * 0.024` | 12–18 px | list rows, subtitles |
| `label` | `h * 0.02` | 10–14 px | hints, keys, meta, counts |
| `mono` | `h * 0.02` | 10–14 px | timecodes, counts, showcmd (monospace) |

`label` is the workhorse for the Neovim layer: mode chip, showcmd, which-key
rows, footer hints.

### 3.5 Layering

| Token | z | Scrim? |
|-------|---|--------|
| `zRegion` | 10 | no |
| `zToast` | 20 | no |
| `zOverlay` | 30 | yes (`scrim` @ 0.5) |
| `zCommand` | 40 | yes |
| `zWhichKey` | 50 | no (floating card) |
| `zTooltip` | 60 | no |

---

## 4. Neovim × MY3

### 4.1 The mode machine

```
        ┌───────────────────────────── ESC (invariant) ─────────────────────────────┐
        v                                                                          │
   ┌─────────┐   v / V   ┌──────────┐   :   ┌───────────┐   /   ┌──────────┐        │
   │ NORMAL  │──────────>│  VISUAL  │──────>│  COMMAND  │──────>│  SEARCH  │────────┘
   └─────────┘<──────────└──────────┘       └───────────┘       └──────────┘
        │  i / Enter(text fields)                  ^                   │ typing
        v                                          └── Esc (step out) ──┘
   ┌─────────┐
   │ INSERT  │   (only in inputs: search field, login, command line)
   └─────────┘
```

The search overlay already implements the right *local* version of this
(`typing` ↔ search-normal). The global machine just needs the same shape, with
the active mode rendered in the status bar.

### 4.2 Motion semantics per mode

| Mode | Motion rule |
|------|-------------|
| NORMAL | `instant`: every keystroke produces immediate cursor/state change. No deferred actions >120 ms (kills N6's 800 ms). |
| VISUAL | selection extends with `quick` color fill; count badge pops `micro` |
| COMMAND | prompt rises `quick`; matching commands filter without animation |
| SEARCH | overlay `standard`; result swap `instant` (already true); ESC step-out `micro` |
| INSERT | caret only; no chrome motion |

### 4.3 Status bar (N4 + N8b)

RightBar currently shows `LIBRARY`, counts, sync (RightBar.qml). Add a left
cluster, `label` typography:

```
[ NORMAL ]  d2j        ← mode chip + showcmd + count register
```

- mode chip: `surfaceContainerHighest`, 1px `primary` hairline, colored label;
- count register: a small chip that appears with the first digit, fades `micro`;
- showcmd: monospace `label`, right of the chip, cleared instantly;
- which-key: a square MY3 card (`surfaceContainerHigh`, hairline) anchored
  above the player bar, rows in `label`/`mono`; appears `micro`, disappears
  `instant`.

### 4.4 Pane navigation vs visual selection (N1)

Retire the blur overlay. `Ctrl-w h/j/k/l` moves focus instantly; the region
border (already there, `Home.qml:415-417`) becomes the only feedback, animated
`micro` on color, `instant` on width. `v`/`V` then means *content* selection:
`secondaryContainer` rows plus the count badge, exactly like the search
overlay's cursor. Blur is reserved for modal scrims only (R2).

### 4.5 Component notes

- **Search overlay** — reference implementation. Retune panel/backdrop to
  `standard`/`scrim`; keep 120 ms debounce; column/row switches `instant` or
  `quick`; artists | albums | songs widths are weights (28/32/40), document them.
- **Toast** — map `sync` → `tertiaryContainer`, errors → `errorContainer`; retry
  is a `shapeSoft` button with the focus state layer.
- **Transport (M4/U6)** — volume/mute need the same state layers as IconButton
  plus focus rings; play/pause already uses glyphs (U3), keep it that way.
- **Queue (M1)** — when the drawer lands, rows reuse the canonical cursor token;
  `dd` removal animates `quick` collapse, no bounce.

---

## 5. Implementation plan

| Step | Work | Satisfies |
|------|------|-----------|
| 1 | Add `motion`, `shape`, `scrim`, state-layer and container roles to `Theme.js` | R1–R7 |
| 2 | Sweep hardcoded values: scrim, blur opacity, radius literals, NowPlaying px, z-index | §2.3 |
| 3 | Motion factory: single StackView transition builder + `FadeText` opt-out | U9 |
| 4 | `FocusableControl` pattern (state layer + focus hairline) for IconButton/Sync/transport | U2/U11 |
| 5 | Global mode machine (N4) + mode chip + showcmd/which-key (N8b) using §4.3 | N4/N8b |
| 6 | `Ctrl-w h/j/k/l` panes; visual mode becomes selection (N1) | N1 |
| 7 | Type scale with clamps; replace inline `pixelSize` fractions | U10 |
| 8 | Retune durations per §3.1 (U9); toast/error roles per §4.5 | U9/U8 |
| 9 | Command line `:` (N5a) styled with `zCommand` + §4.2 motion | N5a |
| 10 | `?`/`:help` cheatsheet built from the same card style as which-key | N8c |

Acceptance for step 5: pressing `d` shows a count/pending chip within one
frame, `Esc` clears it instantly, and the status bar always states the mode.

---

## Appendix A — file index

| File | Role in this study |
|------|--------------------|
| `src/qml/Theme.js` | tokens; target of steps 1–2, 7 |
| `src/qml/FadeText.qml` | global text motion; step 3 |
| `src/qml/Home.qml` | StackView transitions, region borders, blur overlay |
| `src/qml/SearchOverlay.qml` | first modal sub-machine; motion reference |
| `src/qml/SearchColumn.qml` | cursor rows, skeleton, hover parity |
| `src/qml/RightBar.qml` | status bar; future mode/showcmd host |
| `src/qml/IconButton.qml` | focus-state gap (U2) |
| `src/qml/NowPlaying.qml` | px/radius drift, fullscreen motions |
| `src/qml/Toast.qml` | error/sync state roles |
| `src/keybind.rs` | flat grammar; target of N2a/N2b/N4 |
| `src/qml/Main.qml` | 800 ms deferred key resolver (N6) |

## Appendix B — quick wins

1. `Theme.scrim` role + use it in the search backdrop.
2. Replace `NowPlaying` `6/32/36` with fractions and `radius: Theme.radius`.
3. Add `easing` to the search backdrop and Home exit transitions.
4. Single function for the four StackView transition blocks.
5. `linear` easing for progress/seek width animations.
6. Add a 1-px focus hairline to `IconButton` (hover is not focus).
7. Cap `font.pixelSize` fractions with a `clamp()` helper (U10).
8. Give RightBar a `handleAction` that at least forwards sync (U11).
9. Replace the 800 ms key timeout with per-binding deferral (N6).
10. Paint the mode chip `NORMAL` in the status bar — even before modes exist
    elsewhere (N4).
11. Document search column widths as weights, not pixels.
12. Kill the visual-mode blur in favor of a scrim + focus border (N1/R2).
