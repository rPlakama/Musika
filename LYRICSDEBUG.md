# Lyrics Debug — "lyrics disappeared"

> **Status: FIXED** — RC-1 and RC-2 applied on 2026-10-13: the delegate role is
> now `line` (`NowPlaying.qml:103,460,463`) and `handleAction` uses
> `lyricsView` (`NowPlaying.qml:160,162`); `qmllint` no longer reports
> `[property-override]`. Evidence below was gathered from the pre-fix tree at
> `793d665` and the live runtime databases under `~/.config/musika/`
> (read-only queries, 2026-10-13). Runtime checks in §6 are still pending.

## 1. Symptom

The fullscreen **Lyrics** pane stopped showing lyrics (empty panel) even though
the pipeline had previously fetched and cached real lyrics.

## 2. Runtime evidence — the backend pipeline works

`lyrics.db` (read-only):

```
sqlite> SELECT source, synced, COUNT(*) FROM lyrics GROUP BY source, synced;
none|0|13
subsonic|1|6

sqlite> SELECT * FROM lyrics_meta;
song_lyrics|yes

sqlite> SELECT COUNT(*) FROM lyric_line;
133
```

| song id | lines | first start | last start |
|---|---|---|---|
| `1nFvMTfYQHFq6dWSbRsyPP` (Everything In Its Right Place) | 29 | 34.73 s | 218.0 s |
| `2vjfonSzYBk4t5yYt8lgbQ` (Optimistic) | 36 | 42.67 s | 231.8 s |
| `3DYbyHBNBEd6d3vTu6RFSu` (The National Anthem) | 13 | 96.03 s | 221.3 s |
| `3PlpTsrmYRDfmuo3fe3PEJ` (Space Dementia) | 20 | 47.33 s | 252.0 s |
| `5uxb8BrLl68FVggGSDrhao` (How to Disappear Completely) | 22 | 41.57 s | 275.3 s |
| `6ezMP7Ll9cLPLHxatexFGS` (Kid A) | 13 | 61.91 s | 248.1 s |

Sample rows (`3PlpTsrmYRDfmuo3fe3PEJ`): `0 47330 "H8 is the one for me"`,
`1 57660 "It gives me all I need"`, …

Conclusion: OpenSubsonic probe, `getLyricsBySongId`, structured parsing,
`lyrics.db` storage and purge all work. The 13 `none` rows are genuine server
misses. **The failure is on the QML rendering path.**

## 3. Pipeline map (verified)

```
NowPlaying visible ──> Backend.requestLyrics()            qml.rs:822
                    ──> lyrics::fetch (override→cache→    lyrics.rs
                        getLyricsBySongId→legacy)
                    ──> dataReady("lyrics", json)          qml.rs:853
                    ──> NowPlaying.applyLyrics(payload)    NowPlaying.qml:88
                    ──> lyricModel (roles t, text)         NowPlaying.qml:95
                    ──> ListView lyricsView                NowPlaying.qml:446
                        delegate FadeText                  NowPlaying.qml:456
```

Verified along the way (not causes):

- `applyLyrics` stale-guard uses `Backend.currentSongId`, which is emitted only
  on change (`qml.rs:901-904`) — no 500 ms tick churn.
- `Connections.onCurrent_song_id_changed` (`NowPlaying.qml:199`) matches the
  project's snake-case notify convention (`onLibrary_changed`,
  `onLogged_in_changed`), so re-request on track change is wired.
- `activeLyric` offset math (`NowPlaying.qml:31-44`) is correct.

## 4. Confirmed root causes

### RC-1 (primary) — delegate role `text` shadows `Text.text`

`src/qml/NowPlaying.qml:456-461`

```qml
delegate: FadeText {
    id: lyricLine
    required property int index
    required property real t
    required property string text      // <-- shadows QQuickText.text
    ...
    text: index === root.activeLyric ? "> " + lyricLine.text + "  <" : lyricLine.text
```

`qmllint`:

```
Warning: src/qml/NowPlaying.qml:460:58: Member "text" shadows member "text"
from base type "QQuickText", use a different name or add a final or override
specifier. [property-override]
```

Effect: the required property gets the model role, while the drawn
`QQuickText.text` is never assigned → each delegate renders an **empty string
with ~zero height**. The model is full (`lyricsAvailable == true`, ListView
visible) but the pane paints blank — exactly the observed "disappeared".

**Fix applied:** renamed the role/property to `line`:

```qml
// applyLyrics
lyricModel.append({ t: …, line: data.lines[i].text })
// delegate
required property string line
text: index === root.activeLyric ? "> " + lyricLine.line + "  <" : lyricLine.line
```

### RC-2 (secondary) — stale `lyricsFlick` id in `handleAction`

`src/qml/NowPlaying.qml:160,162`

```qml
scrollLyrics(-lyricsFlick.height * 0.3);
scrollLyrics(lyricsFlick.height * 0.3);
```

`lyricsFlick` was the old `Flickable`; the pane is now a `ListView` called
`lyricsView` (`NowPlaying.qml:447`). Evaluating `lyricsFlick.height` throws a
`ReferenceError`, so **j/k scrolling of the lyrics pane is dead** (the whole
branch aborts before `scrollLyrics` runs). **Fix applied:** use
`lyricsView.height`.

## 5. Secondary observations (not the disappearance)

- Header tag renders `subsonic · synced · xxx`: Navidrome’s `lang: "xxx"` means
  "unspecified"; suppress `xxx`/`und` in `lyricsTag()` (cosmetic).
- 13 cached `none` misses use a 7-day TTL with no manual refresh (`LYR-5` gap);
  if the server later gains lyrics they stay hidden until the TTL lapses.
- `~/.config/musika/lyrics/` does not exist → no local `.lrc`/`.txt` override
  is shadowing the cache.
- `lyricsAvailable` starts `false`, so "No lyrics" flashes before the first
  response (cosmetic).

## 6. Verification plan (after the fix is authorized)

1. Play `3PlpTsrmYRDfmuo3fe3PEJ` (Space Dementia), open fullscreen → lines
   render, active line marked `> … <`, auto-centered.
2. `j`/`k` in the lyrics pane scrolls without a QML `ReferenceError`.
3. `qmllint src/qml/NowPlaying.qml` no longer reports `[property-override]`.
4. Play a `none` track → centered "No lyrics"; play a synced track → lyrics
   switch without reopening fullscreen.

## 7. Repro commands (read-only)

```sh
sqlite3 -readonly ~/.config/musika/lyrics.db \
  "SELECT source, synced, COUNT(*) FROM lyrics GROUP BY 1,2;"
sqlite3 -readonly ~/.config/musika/lyrics.db \
  "SELECT song_id, COUNT(*), MIN(start_ms), MAX(start_ms) FROM lyric_line GROUP BY song_id;"
qmllint -I "$QML_IMPORT_PATH" src/qml/NowPlaying.qml | grep property-override
```
