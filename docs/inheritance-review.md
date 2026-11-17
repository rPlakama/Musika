# Inherit Maxxing review — `v0.7.3..a4d8d11`

Status: **review complete; R1/release blockers and the high-impact R3 items landed; R2/R4 tracked below**.
Baseline tag: `v0.7.3` (`a927ebd`). Review target: `a4d8d11` (28 commits,
108 files, +12,731/−878). Current tree is larger (186 production files,
~78k LOC) and green: 621 tests, 19 ignored, Clippy clean.

This complements `docs/mds/inheritance-audit.md`. The three audit questions
apply: **Shape** (schema declared twice), **Identity** (loose strings for a
stable id), **Rule** (a child restates a policy/default).

## Verdict

The batch added strong parent abstractions (`PcmSink`, `PlaybackError`,
`bridge_rpc`, `ItemMeta`, `ListState`, `CoverStore`), but several features were
added as parallel adapters. The runtime failures in this batch were inheritance
failures, not isolated bugs:

- production status tables were declared in a second migration tree;
- UI settings requests travelled through the core→UI `Event` enum;
- `stream`/`output` command metadata drifted from their parser;
- playlist artwork selected a provider's low-resolution mosaic at hero size.

## Findings and remediation status

| # | Kind | Finding | Parent / mechanism | Status |
|---|------|---------|--------------------|--------|
| F1 | Rule/Shape | Playback settings have four owners and three clamp/default copies (`playback_settings`, service, daemon, `CURRENT`, `CrossfadeConfig`/`Shared`) | policy structs + one live service owner; daemon read-only snapshot; remove `CURRENT` from queue builders | partial: crossfade/stream validation delegated; settings still split |
| F2 | Shape | `Event` carried both UI requests and core events; each settings request was declared in the RPC table, Event, Bridge, daemon match/exclusion, `PlaybackInput`, and text dispatch | `Event` is core→UI only; Bridge attaches `PlaybackControl` and sends `PlaybackInput` directly | **done** (`5a45e27`, protocol v4; v5 adds the settings snapshot) |
| F3 | Shape | `migrations_status` duplicated status DDL against the production `migrations_library`; caused missing `playback_settings` table | one schema owner: the library tree; tests use the same tree | **done** (`5a45e27`) |
| F4 | Shape/Identity | Cover identity is parallel fields/vectors (`cover_url`, `cover_remote`, `cover_art`, `collage`, `collage_remote`, `collage_art`); Row has no cover id; artist/playlist/search resolve differently | `CoverRef { art, url, remote }`; `PageHeader { cover, collage: Vec<CoverRef> }`; one resolver per item kind | open |
| F5 | Shape | Command metadata is a second grammar; `stream fixed` and output modes/backend were absent from usage/hints | command spec owns usage/hints; parity test | **done** (`5a45e27`) |
| F6 | Identity | Connectivity was mirrored by `Bridge::offline`, `session::ONLINE`, daemon probe state, UI flag, `Event::Connection` | session owns online; Bridge/UI derive; transition event only | **done for Bridge** (`b3c3e8c`); daemon/UI still mirrors |
| F7 | Rule | Settings overlay hardcodes seven rows and misses output/stream/scrobble/crossfade policy | field descriptor table fed by a settings snapshot | **done** (`7e9bb57`): full snapshot event + generated rows/actions; Ctrl-, handler registered |
| F8 | Shape | `Sink`/`AlsaSink` duplicate ring/shared/forwarding methods; `PcmSink` only unifies callers | shared `SinkCore` or trait default methods | open |
| F9 | Rule | Playlist/radio/cover rules diverged (provider mosaic vs entry collage; cache-gated/uncapped radio) | one `CoverPlan` for playlist/radio/list/search | playlist/radio fixed; search/list need same resolver |
| F10 | Shape | Cover lookup/caching families duplicate unique/batch/conditional loops; `artist-image:` prefix restated | `CoverOrigin` + `CoverFetcher`/`CoverLookup` | partial: inverse key helper added; fetcher still duplicated |
| F11 | Shape | `recommend::SongFeature`/SQL restates `SongRow` + fields | shared song projection/SQL columns | open |
| F12 | Rule/Identity | Image role was implicit; provider playlist mosaic was low-res. `CoverImage` retention keyed by URL can keep stale row art during grace | `CoverRole` source selection; retain only on stable cover identity | **role + live fix done** (`9d8613e`, `0eb7aaa`); retention identity open |
| F13 | Shape | Dead `collage_*` JSON helpers; `PageState::rebuild_covers` duplicates resolution; test fixtures repeat DDL | remove/route through one resolver | dead helpers removed (`fdc91d8`); rest open |

## Intentional mirrors (do not dedupe)

- Core/UI playback snapshot + queue deltas.
- MPRIS/tray command vocabulary.
- `fast_image` display derivatives vs. core row thumbnails (different
  consumers); the two must agree on a **role**, not share a decoder.
- Storage columns keep string page identities; derive from `PageKind`.

## Remediation phases

1. **R1 — correctness/release blockers** (mostly done): status schema, event
   request split, command metadata, high-res runtime verification.
2. **R2 — cover identity**: `CoverRef`, `PageHeader`/`Row` ownership, one
   `CoverPlan`, `CoverRole` retention by identity, shared artist/playlist/search
   resolution. This is the remaining low-resolution/flicker class.
3. **R3 — settings/connectivity ownership**: one live settings owner with
   policy structs; remove `CURRENT` from queue construction; one connectivity
   state; settings overlay generated from the model.
4. **R4 — surface cleanup**: `SinkCore`, `CoverFetcher`, shared song projection,
   test-fixture hoisting, dead-helper removal.

## Evidence/acceptance

- `cargo test` 621 pass, 19 ignored; `cargo clippy --all-targets` clean.
- Live playlist smoke on the real library:
  - provider playlist cover is a ~600 px pre-rendered 2×2 mosaic;
  - page now builds a two-pane full-resolution entry collage
    (`mf-…`/`dc-…:1`), and `fast_image` produced a 768 px venue derivative;
  - header no longer changes pane count as covers warm.
- Protocol v5: stale UI fails at Welcome version check instead of `BadPayload`;
  settings changes stream as a snapshot event.
- The playlist header live check selects the two album-entry originals
  (`mf-…`, `dc-…:1`), and the provider playlist mosaic is only a fallback.
- Ctrl-,/Ctrl-E handlers are registered; modified punctuation no longer becomes
  a seek hold. (The overlay was compile/unit verified; a UI key-injection smoke
  was inconclusive on this compositor.)
- Status schema regression test opens the production library migrations and
  asserts `playback_settings`/`pending_scrobble`.

## Assumptions

- Multichannel output and native DSD remain documented gates, not part of this
  review.
- `a4d8d11` is the review target; subsequent commits are the remediation above.
