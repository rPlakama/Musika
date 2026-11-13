# Radio — local similar-song expansion

`radio` (and the Radio Tool board it opens) is a core, local-first feature. The
page and the playback queue are generated from `library.db`; no
`getSimilarSongs` request is made, so a radio can be opened and played while
the server is unreachable. Only downloaded songs are offered while offline.

## Seeds

| seed | source |
| --- | --- |
| `radio` / `radio current` | the currently playing track (song seed) |
| `radio album [name\|id]` | the open album page, or a name resolved through the local search index |
| `radio song [name\|id]` | a song row, or a name resolved through the local search index |

The seed must exist in the synced local library. A server-only row that never
made it into `library.db` cannot be expanded offline and reports that instead
of falling back to the network.

## How the mix is built

1. **Similar artists** (up to 15, in order):
   - the cached `getArtistInfo2.similarArtist` graph for the seed artist,
     filtered to artists that still exist locally. The graph is stored next to
     the cached artist biography (`artist_info.similar`, JSON) and is refreshed
     when the artist page is opened while online. A missing/stale graph also
     triggers one background refresh when online — the expansion itself never
     waits on HTTP;
   - local metadata ranking for whatever the graph does not cover:
     - normalized genre tokens with inverse-document-frequency weighting
       (`Rock` ↔ `Rock progressivo` share `rock`; one-song artists are
       suppressed by a catalog-support factor),
     - mean-year proximity,
     - co-occurrence in the user's own playlists.
2. **Songs**: the seed artist (`artistWeight = 10`) and each similar artist
   (`0`) contribute their most-starred/most-played songs to a weighted chooser
   that matches Navidrome's fallback (`topCount * (4 + artistWeight)`, decaying
   by 4 per song, `topCount = max(count, 20)`).
3. **Top-ups** when the chooser is short: songs sharing a seed genre string,
   then the seed's four-year era, then random local songs. The seed song and
   the seed album are excluded from the normal mix.

The result is deduplicated, capped at 100 rows for the radio page/queue (the
caller can ask for up to 500), and cached in memory per seed so page and queue
always share one order. A library sync clears that cache.

## Offline

With the offline flag set, every candidate step is restricted to songs whose
download ledger state is `done` before selection, and the page/queue apply the
same filter to the cached expansion. If nothing is playable the page shows the
usual empty state; the seed identity stays visible with a `Radio (offline)`
subtitle. `radio save` and `radio save --offline` keep working from the cached
expansion; saving a server playlist still needs the server, and
`--offline` writes only the app-owned M3U.

## Limitations

- The cached artist graph depends on the server's Last.fm configuration; when
  it is empty or unavailable the mix is metadata-driven and intentionally
  approximate (genre tags are free text and can be localized/fragmented).
- Local `play_count` is a sync snapshot; playlists, starred flags and era carry
  more personalization weight on small-sparse histories.
- There is no per-track Last.fm similarity offline (Navidrome's agent path);
  the local engine is the fallback shape, not a replacement for agent quality.
- Audio-feature "sonic similarity" is server-side only and not part of this
  path.
