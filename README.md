# Musika

<p align="center">
  <img src="src/icons/musika.svg" alt="Musika" width="144">
</p>

Musika is a Keyboard driven Navidrome/Subsonic Client made with AI assistance. Written in Rust and QT. It contains many functionalities, such as _vi motions_, _cache & download system_, an indexed db based search, caching in covers, and other performance extras. Everything is thoroughly tested by me. The development is mainly driven by specs -- review by me, and coding itself by AI.

# Features

1 - Alt-Menu -- A multipoint(Alt) call for Tracks, Albums. Which is capable of transporting through other sections, or executing context-driven actions (like bulk-download, favoriting, etc;). Four quadrants: Add to Queue (N), Favorite (S), Go to Artist (W), Go to Album (E — on an album row the east slot carries the download toggle instead), aimed with `h/j/k/l` or the arrows (mouse works too), fired on Alt release, cancelled by Esc or a neutral aim. With a visual selection up (`v`/`V`) the verbs apply to the whole range, including a bulk **Download** that toggles: a selection already cached is removed instead.

2 - Download System -- Local-first, two ledgers in `media.db` and two tiers: the artifact tables own the blobs + the container *rules*, the download ledger owns the intent and progress (queued → active → done/error). An explicit song download is `permanent`; selecting an album or an artist writes a rule, downloads every existing track and keeps catching new ones on sync — toggling it off removes the rule and its files. Permanent/rule blobs live under `~/.config/musika/media`, the purgeable playback tier under `~/.cache/musika/media`. Row glyphs show the state (outline → spinning loop → filled), and offline mode serves the downloaded catalog only.

3 - Search -- `/` searches the surface in front of you (literal, smartcase, viewport-scoped) with in-place match blocks and `[i/N]` counts; Enter jumps, Esc abandons. The global overlay searches the whole indexed library with a persistent Tantivy index (ARC-SEARCH).

4 - Playback -- MPV runs as the backend subprocess: gapless with prefetch, stream-error retry + auto-advance, native MPRIS, seek/volume, and synced lyrics (OpenSubsonic) that ride the shared scroll body.

5 - Covers, lyrics and palettes are each cached in their own DB and purged on sync — nothing derived is fetched twice.

# Installation

Currently, it's a nix flake based installation; However, I plan in the future to provide another packaging schemes for MUSIKA. The flake installation follows as below:

in yer flake inputs.

```
musika.url = "github:rPlakama/musika";

```

then as a user/system package.

```
inputs.musika.packages.${pkgs.system}.default
```

# Technical Information

It works with two databases:

- `library.db` -- the server and account, artists/albums/songs, artist stars
  and ratings, playlists, session resume and cover-derived colors
- `media.db` -- cover URLs + cached art, cached lyrics, download blobs + rules
  and the download ledger

# Sections

- Sidebar segments: Albums, Songs, Artists, Playlists (Radio is a placeholder for now).
- Fullscreen player (Enter on a song): transport, seek, volume, and the queue + lyrics panes.
- Queue drawer (`Ctrl-Q`): `j/k`, Enter jumps, `J/K` reorder, `dd` removes.
- Search overlay: the global fuzzy browser over the persistent Tantivy index.

# Building

```
nix run .       # build + launch
nix develop     # dev shell (Rust + Qt)
```

# Contributions

Pull Requests are welcome if properly tested. I have no issues with AI -- since it would be hypocritical. Besides that, the development is nix-first, as my favorite development tool. Besides that, many core tools, which are present in other players, still missing from here, and bugs that I haven't caught yet. Opening an issue for any problem is also very welcome.

GPL-2.0-or-later.
