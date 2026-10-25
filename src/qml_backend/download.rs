// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

use super::support::raise;
use crate::session::client;

/// Reads the download ledger for the requested window and applies the
/// finished subset to the backend's `downloadedIds` property plus the albums
/// whose every track is cached to `cachedAlbumIds` (through the invoker, so
/// the write lands on the main thread). The window is the rows the UI can
/// actually show (±15 around a cursor); the whole ledger is never loaded, no
/// preload happens at open.
pub(super) async fn apply_window(
    dbs: &crate::db::Dbs,
    songs: &[String],
    albums: &[String],
    generation: u64,
    invoker: qtbridge::QmlMethodInvoker,
) {
    // Two batched queries, never point-by-point: one library read for every
    // window album's tracklist, one ledger read over the window songs plus
    // those tracklists (album coverage needs them all).
    let tracklists = crate::library::album_song_ids_of(&dbs.library, albums)
        .await
        .unwrap_or_default();
    let mut all = songs.to_vec();
    for ids in tracklists.values() {
        all.extend(ids.iter().cloned());
    }
    let (done, pending) = tokio::join!(
        crate::download::done_ids_of(&dbs.download, &all),
        crate::download::pending_ids_of(&dbs.download, &all),
    );
    let done: std::collections::HashSet<String> = done.unwrap_or_default().into_iter().collect();
    let pending: std::collections::HashSet<String> =
        pending.unwrap_or_default().into_iter().collect();
    let songs_done: Vec<String> = songs
        .iter()
        .filter(|song| done.contains(*song))
        .cloned()
        .collect();
    let albums_done: Vec<String> = albums
        .iter()
        .filter(|album| {
            tracklists
                .get(*album)
                .is_some_and(|ids| !ids.is_empty() && ids.iter().all(|song| done.contains(song)))
        })
        .cloned()
        .collect();
    let songs_pending: Vec<String> = songs
        .iter()
        .filter(|song| pending.contains(*song))
        .cloned()
        .collect();
    crate::debug_log!(
        "downloads: {} of {} visible songs cached; {} of {} albums fully cached; {} in flight",
        songs_done.len(),
        songs.len(),
        albums_done.len(),
        albums.len(),
        songs_pending.len()
    );
    // PATH-5: a newer request won while this read ran; its result lands
    // instead (a stale snapshot would flip the marks backwards).
    if super::MARK_WINDOW_GEN.load(std::sync::atomic::Ordering::Relaxed) != generation {
        return;
    }
    let json = serde_json::json!({
        "songs": songs_done,
        "albums": albums_done,
        "downloading": songs_pending,
        // The window's queried ids: their states get rewritten, everything
        // else keeps its own (reads never clobber other surfaces).
        "queried_songs": songs,
        "queried_albums": albums,
    })
    .to_string();
    qtbridge::invoke_method!(invoker, "applyDownloadedIds", json);
}

/// CACHE-5/8: the pass, three songs at a time, in order (the 2-D shape: the
/// caller's list arrives album-major, so a chunk usually *is* one album's
/// head). The loading glyph is **existence-based, not queue-based**: a
/// chunk's songs and the albums owning them are marked in-flight only while
/// that chunk runs, then settle — a row spins while one of its tracks is
/// actually fetching, never while it waits in line. Each finished song
/// flips its own row (and its album's, when the tracklist completes); the
/// container id stays held for the whole pass.
pub(super) async fn fetch_marked(
    dbs: &crate::db::Dbs,
    songs: &[String],
    container_id: &str,
    album_container: bool,
    invoker: qtbridge::QmlMethodInvoker,
) {
    for chunk in songs.chunks(crate::consts::DOWNLOAD_CONCURRENCY) {
        // The chunk's albums: their rows light up alongside its songs.
        let scopes = crate::library::song_scopes(&dbs.library, chunk)
            .await
            .unwrap_or_default();
        let mut chunk_albums: Vec<String> = Vec::new();
        for song in chunk {
            if let Some((_, Some(album))) = scopes.get(song) {
                if !chunk_albums.contains(album) {
                    chunk_albums.push(album.clone());
                }
            }
        }
        let mut batch_ids: Vec<String> = chunk.to_vec();
        batch_ids.extend(chunk_albums.iter().cloned());
        let batch = serde_json::to_string(&batch_ids).unwrap_or_else(|_| "[]".to_string());
        qtbridge::invoke_method!(invoker, "noteDownloadStates", batch);

        let mut set = tokio::task::JoinSet::new();
        for song in chunk {
            let dbs = dbs.clone();
            let song = song.clone();
            set.spawn(async move {
                let result = download_one(&dbs, &song).await;
                (song, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            // `download_one` already logged the per-song outcome.
            let Ok((song, result)) = joined else {
                continue;
            };
            let state = match result {
                Ok(state) => state,
                Err(_) => crate::download::ERROR,
            };
            qtbridge::invoke_method!(
                invoker,
                "noteDownloadState",
                song.clone(),
                state.to_string()
            );
            if state == crate::download::DONE {
                if let Some(album) = completed_album(dbs, &song).await {
                    qtbridge::invoke_method!(invoker, "markAlbumCached", album);
                }
            }
        }
        // The chunk is over: its albums stop loading unless later chunks
        // hold their tracks (the container id stays held all pass).
        for album in &chunk_albums {
            if album == container_id {
                continue;
            }
            qtbridge::invoke_method!(invoker, "settleDownloadRequest", album.clone());
        }
    }
    // The pass is over: an album container is re-checked against its whole
    // tracklist (the per-song path misses it when a track's library album
    // changed) and flips to the filled mark; anything else just settles its
    // loading glyph. The container id must never ride `noteDownloadState`:
    // that path stamps it as a finished *song*, erasing the album's cached
    // state.
    if !container_id.is_empty() {
        if album_container && album_complete(dbs, container_id).await {
            qtbridge::invoke_method!(invoker, "markAlbumCached", container_id.to_string());
        } else {
            qtbridge::invoke_method!(invoker, "settleDownloadRequest", container_id.to_string());
        }
    }
}

/// True when an artist rule truly owns the whole album: the rule exists
/// **and** its song set contains every track of the album. An album's
/// `artist_id` can disagree with its songs' `artist_id` (server metadata —
/// compilations, stale libraries), so bare rule existence must never skip
/// the album request: that skip left albums marked "covered" while none of
/// their tracks were downloaded (Pablo Honey, 0/12 — probed).
pub(super) async fn artist_covers_album(
    dbs: &crate::db::Dbs,
    artist_id: &str,
    album_id: &str,
) -> bool {
    if !matches!(
        crate::media::rule_for(&dbs.media, "artist", artist_id).await,
        Ok(Some(_))
    ) {
        return false;
    }
    let album_songs = crate::library::album_song_ids(&dbs.library, album_id)
        .await
        .unwrap_or_default();
    if album_songs.is_empty() {
        return false;
    }
    let artist_songs: std::collections::HashSet<String> =
        crate::library::artist_song_ids(&dbs.library, artist_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    album_songs.iter().all(|song| artist_songs.contains(song))
}

/// Selects an album container: the rule and its song marks, returning the
/// marked songs (the caller runs the fetch pass). Shared by the album
/// download and the artist batch's per-album loop.
pub(super) async fn album_container_on(
    dbs: &crate::db::Dbs,
    album_id: &str,
) -> anyhow::Result<Vec<String>> {
    let rule_id = crate::media::set_rule(&dbs.media, "album", album_id).await?;
    let songs = crate::library::album_song_ids(&dbs.library, album_id).await?;
    for song in &songs {
        if let Err(err) = crate::download::mark_queued(&dbs.download, song, Some(&rule_id)).await {
            eprintln!("media: could not mark {song}: {err:#}");
        }
    }
    Ok(songs)
}

/// Drops a container rule: its marks and every config-tier blob its songs
/// own — the rule-owned rows and the rows stored before the fetch recorded
/// the ownership (the ledger still names the rule then). One helper for the
/// album, artist and legacy OFF paths.
pub(super) async fn rule_off(dbs: &crate::db::Dbs, rule_id: &str) {
    let songs = crate::download::rule_song_ids(&dbs.download, rule_id)
        .await
        .unwrap_or_default();
    if let Err(err) = crate::download::unmark_rule(&dbs.download, rule_id).await {
        eprintln!("media: could not clear the rule's marks: {err:#}");
    }
    if let Err(err) = crate::media::remove_rule_media(&dbs.media, rule_id).await {
        eprintln!("media: could not remove the rule-owned blobs: {err:#}");
    }
    if let Err(err) = crate::media::remove_songs_media(&dbs.media, &songs).await {
        eprintln!("media: could not remove the rule's blobs: {err:#}");
    }
    if let Err(err) = crate::media::delete_rule(&dbs.media, rule_id).await {
        eprintln!("media: could not drop the rule: {err:#}");
    }
}

/// Drops an album container: its rule, marks and blobs.
pub(super) async fn album_container_off(dbs: &crate::db::Dbs, album_id: &str) {
    match crate::media::rule_for(&dbs.media, "album", album_id).await {
        Ok(Some(rule)) => rule_off(dbs, &rule.id).await,
        Ok(None) => {}
        Err(err) => eprintln!("media: could not read the album rule: {err:#}"),
    }
}

/// The Alt selection's Download toggle (VI-4), converged like the Favorite
/// verb: when every target already holds its content the selection is
/// removed (blob/rule), otherwise only the targets still missing are queued
/// and fetched. Containers follow the ArtistEntry chip's rule-state
/// semantics; songs follow the config-tier blob (`media::downloaded`).
pub(super) async fn selection_toggle(
    dbs: &crate::db::Dbs,
    kind: &str,
    ids: &[String],
    invoker: qtbridge::QmlMethodInvoker,
    error_invoker: qtbridge::QmlMethodInvoker,
) -> anyhow::Result<()> {
    match kind {
        "song" => {
            let mut missing: Vec<String> = Vec::new();
            for id in ids {
                if !crate::media::downloaded(&dbs.media, id).await? {
                    missing.push(id.clone());
                }
            }
            if missing.is_empty() {
                for id in ids {
                    if let Err(err) = crate::download::unmark(&dbs.download, id).await {
                        eprintln!("download: could not clear {id}'s mark: {err:#}");
                    }
                }
                let removed = crate::media::remove_songs_media(&dbs.media, ids).await?;
                eprintln!("download: selection (song) — removed {removed} blobs");
                qtbridge::invoke_method!(invoker, "ResetRowMarks");
            } else {
                fetch_marked(dbs, &missing, "", false, invoker).await;
            }
        }
        "album" => {
            let mut missing: Vec<String> = Vec::new();
            for id in ids {
                let cached = matches!(
                    crate::media::rule_for(&dbs.media, "album", id).await,
                    Ok(Some(_))
                );
                if !cached {
                    missing.push(id.clone());
                }
            }
            if missing.is_empty() {
                for id in ids {
                    album_container_off(dbs, id).await;
                }
                eprintln!(
                    "download: selection (album) — removed {} targets",
                    ids.len()
                );
                qtbridge::invoke_method!(invoker, "ResetRowMarks");
            } else {
                let mut songs: Vec<String> = Vec::new();
                for id in &missing {
                    match album_container_on(dbs, id).await {
                        Ok(list) => songs.extend(list),
                        Err(err) => {
                            eprintln!("download: could not select the album container: {err:#}")
                        }
                    }
                }
                if songs.is_empty() {
                    raise(
                        &error_invoker,
                        "error",
                        "Nothing to download here.".to_string(),
                    );
                    return Ok(());
                }
                fetch_marked(dbs, &songs, "", false, invoker).await;
            }
        }
        "artist" => {
            // The artist is "on" when every album of it holds a rule (the
            // `requestArtistCache` state); a mixed selection converges to on.
            let mut all_on = !ids.is_empty();
            let mut albums_of: Vec<(String, Vec<String>)> = Vec::new();
            for id in ids {
                let albums = crate::library::artist_album_ids(&dbs.library, id).await?;
                let mut on = !albums.is_empty();
                for album in &albums {
                    if !matches!(
                        crate::media::rule_for(&dbs.media, "album", album).await,
                        Ok(Some(_))
                    ) {
                        on = false;
                        break;
                    }
                }
                all_on = all_on && on;
                albums_of.push((id.clone(), albums));
            }
            if all_on {
                for (artist, albums) in &albums_of {
                    for album in albums {
                        album_container_off(dbs, album).await;
                    }
                    // Legacy cleanup: the pre-album design kept an artist rule.
                    if let Ok(Some(rule)) =
                        crate::media::rule_for(&dbs.media, "artist", artist).await
                    {
                        rule_off(dbs, &rule.id).await;
                    }
                }
                eprintln!(
                    "download: selection (artist) — removed {} targets",
                    ids.len()
                );
                qtbridge::invoke_method!(invoker, "ResetRowMarks");
            } else {
                let mut songs: Vec<String> = Vec::new();
                for (_artist, albums) in &albums_of {
                    for album in albums {
                        match album_container_on(dbs, album).await {
                            Ok(list) => songs.extend(list),
                            Err(err) => {
                                eprintln!("download: could not select the album container: {err:#}")
                            }
                        }
                    }
                }
                if songs.is_empty() {
                    raise(
                        &error_invoker,
                        "error",
                        "Nothing to download here.".to_string(),
                    );
                    return Ok(());
                }
                fetch_marked(dbs, &songs, "", false, invoker).await;
            }
        }
        other => return Err(anyhow::anyhow!("unknown download kind: {other}")),
    }
    Ok(())
}

/// True when every track of `album_id` is finished in the ledger.
pub(crate) async fn album_complete(dbs: &crate::db::Dbs, album_id: &str) -> bool {
    let tracklist = crate::library::album_song_ids(&dbs.library, album_id)
        .await
        .unwrap_or_default();
    if tracklist.is_empty() {
        return false;
    }
    let done = crate::download::done_ids_of(&dbs.download, &tracklist)
        .await
        .unwrap_or_default();
    done.len() == tracklist.len()
}

/// The song's album, when its tracklist is now completely cached — the
/// album-side of a targeted completion (the caller flips that row).
pub(super) async fn completed_album(dbs: &crate::db::Dbs, song_id: &str) -> Option<String> {
    let scopes = crate::library::song_scopes(&dbs.library, &[song_id.to_string()])
        .await
        .unwrap_or_default();
    let album = scopes
        .get(song_id)
        .and_then(|(_, album)| album.clone())
        .unwrap_or_default();
    if album.is_empty() {
        return None;
    }
    album_complete(dbs, &album).await.then_some(album)
}

/// Fetches one song into the media cache unless it is already available or a
/// fetch is in flight (CACHE-1/8). Returns the resulting ledger state.
///
/// The conditional the UI cares about lives here: a cached blob wins (the
/// ledger flips to `done`); an `active` row is a no-op; anything else is
/// marked, fetched, stored and closed out (`done` / `error` with the reason).
pub(super) async fn download_one(
    dbs: &crate::db::Dbs,
    song_id: &str,
) -> anyhow::Result<&'static str> {
    if crate::media::cached_path(&dbs.media, song_id)
        .await?
        .is_some()
    {
        eprintln!("download: {song_id} already cached");
        crate::download::set_state(&dbs.download, song_id, crate::download::DONE, None).await?;
        return Ok(crate::download::DONE);
    }
    // The blob's owner: the container rule whose mark selected this song
    // (the ledger's rule id), or none for an explicit download. A
    // rule-owned blob leaves with its rule — the media row must carry the
    // ownership, or removing the target leaves the file behind (CACHE-5).
    let mark = crate::download::get(&dbs.download, song_id).await?;
    if mark
        .as_ref()
        .is_some_and(|row| row.state == crate::download::ACTIVE)
    {
        eprintln!("download: {song_id} fetch already in flight");
        return Ok(crate::download::ACTIVE);
    }
    let rule_of_mark = mark.and_then(|row| row.rule_id);

    crate::download::mark_queued(&dbs.download, song_id, None).await?;
    crate::download::set_state(&dbs.download, song_id, crate::download::ACTIVE, None).await?;

    let fetched = async {
        let client = client(dbs).await?;
        let (size, suffix, bit_rate) = crate::library::song_file_info(&dbs.library, song_id)
            .await?
            .unwrap_or((None, None, None));
        let quality = "original";
        let fingerprint =
            crate::media::fingerprint(size, suffix.as_deref(), bit_rate, client.base(), quality);
        let extension = suffix
            .as_deref()
            .filter(|suffix| !suffix.is_empty())
            .unwrap_or("bin");
        // A rule-marked song is `rule`-owned (the rule's removal deletes the
        // blob); an explicit download is `permanent`: it persists until the
        // user removes it (pins/rolling tiers are CACHE-3/4).
        let (kind, rule) = match rule_of_mark.as_deref() {
            Some(rule) => (crate::media::KIND_RULE, Some(rule)),
            None => (crate::media::KIND_PERMANENT, None),
        };
        // Stream the body into the tier's `.part` file: a fetched song is
        // never held whole in memory, and the adoption is one atomic rename.
        let temp = crate::media::temp_path(kind)?;
        let written = match client.download_media_to(song_id, &temp).await {
            Ok(written) => written,
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                return Err(err);
            }
        };
        crate::media::store_streamed(
            &dbs.media,
            song_id,
            &temp,
            i64::try_from(written).unwrap_or(i64::MAX),
            kind,
            quality,
            &fingerprint,
            extension,
            rule,
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    match fetched {
        Ok(()) => {
            crate::download::set_state(&dbs.download, song_id, crate::download::DONE, None).await?;
            eprintln!("download: {song_id} finished");
            Ok(crate::download::DONE)
        }
        Err(err) => {
            let message = format!("{err:#}");
            eprintln!("download: {song_id} failed: {message}");
            let _ = crate::download::set_state(
                &dbs.download,
                song_id,
                crate::download::ERROR,
                Some(&message),
            )
            .await;
            Err(err)
        }
    }
}

/// CACHE-9: does the download ledger hold at least one finished song? Only
/// then can the app enter offline mode instead of failing the login.
pub(super) async fn downloaded_music_available() -> bool {
    match crate::app::dbs() {
        Some(dbs) => crate::download::counts(&dbs.download)
            .await
            .map(|counts| counts.done > 0)
            .unwrap_or(false),
        None => false,
    }
}

use qtbridge::QmlObject;

use super::Backend;
use super::support::{album_cache_payload, artist_cache_payload, emit};

impl Backend {
    pub(super) fn download_song_impl(&mut self, song_id: String) {
        // Optimistic and targeted: the row's loading glyph spins from the
        // first frame, the completion flips that row (and its album's) only.
        self.note_download_state(song_id.clone(), crate::download::ACTIVE.to_string());
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            match download_one(&dbs, &song_id).await {
                Ok(state) => {
                    qtbridge::invoke_method!(
                        invoker,
                        "noteDownloadState",
                        song_id.clone(),
                        state.to_string()
                    );
                    if state == crate::download::DONE {
                        if let Some(album) = completed_album(&dbs, &song_id).await {
                            qtbridge::invoke_method!(invoker, "markAlbumCached", album);
                        }
                    }
                }
                Err(err) => {
                    qtbridge::invoke_method!(
                        invoker,
                        "noteDownloadState",
                        song_id.clone(),
                        crate::download::ERROR.to_string()
                    );
                    raise(&error_invoker, "error", format!("Download failed: {err:#}"));
                }
            }
        });
    }
    pub(super) fn download_artist_impl(&mut self, artist_id: String, on: bool) {
        // The request is initiated now: the row's loading glyph lights the
        // same frame (the pass' batch and completions follow per id).
        if on {
            self.note_download_states(serde_json::json!([artist_id]).to_string());
        }
        let invoker = self.get_qml_method_invoker();
        let fetch_invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let albums = match crate::library::artist_album_ids(&dbs.library, &artist_id).await {
                Ok(albums) => albums,
                Err(err) => {
                    eprintln!("media: could not list the artist's albums: {err:#}");
                    return;
                }
            };
            if on {
                let mut songs: Vec<String> = Vec::new();
                let mut queued_albums: Vec<String> = Vec::new();
                for album in &albums {
                    match album_container_on(&dbs, album).await {
                        Ok(album_songs) => {
                            eprintln!(
                                "download: artist {artist_id} album {album} queued ({} songs)",
                                album_songs.len()
                            );
                            songs.extend(album_songs);
                            queued_albums.push(album.clone());
                        }
                        Err(err) => {
                            raise(
                                &error_invoker,
                                "error",
                                format!("Could not select the album for download: {err:#}"),
                            );
                        }
                    }
                }
                // No bulk in-flight batch: the fetch pass lights each chunk
                // (three songs) and their albums as it reaches them — a
                // row's loading glyph means its track is fetching *now*.
                eprintln!(
                    "download: artist {artist_id} pass started ({} albums, {} songs)",
                    queued_albums.len(),
                    songs.len()
                );
                // The pass is now in flight: the chip shows its loading loop.
                let json = artist_cache_payload(&artist_id, true, true);
                emit(&invoker, "artist_cache", json);
                fetch_marked(&dbs, &songs, &artist_id, false, fetch_invoker).await;
                // The albums that had nothing to fetch never emitted a
                // completion: settle their rows here.
                for album in &queued_albums {
                    qtbridge::invoke_method!(invoker, "settleDownloadRequest", album.clone());
                }
                eprintln!("download: artist {artist_id} pass finished");
                let json = artist_cache_payload(&artist_id, true, false);
                emit(&invoker, "artist_cache", json);
            } else {
                for album in &albums {
                    album_container_off(&dbs, album).await;
                }
                // Legacy cleanup: the pre-album design kept an artist-kind
                // rule; it has no role anymore.
                if let Ok(Some(rule)) =
                    crate::media::rule_for(&dbs.media, "artist", &artist_id).await
                {
                    rule_off(&dbs, &rule.id).await;
                }
                eprintln!("download: artist {artist_id} removed from the cache");
                let json = artist_cache_payload(&artist_id, false, false);
                emit(&invoker, "artist_cache", json);
                // The marks lose the artist's songs: hard reset + re-read.
                qtbridge::invoke_method!(fetch_invoker, "ResetRowMarks");
            }
        });
    }
    pub(super) fn download_album_impl(&mut self, album_id: String, on: bool) {
        // The request is initiated now: the row's loading glyph lights the
        // same frame (the pass' batch and completions follow per id).
        if on {
            self.note_download_states(serde_json::json!([album_id]).to_string());
        }
        let invoker = self.get_qml_method_invoker();
        let fetch_invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            if on {
                let artist_covered =
                    match crate::library::album_artist_id(&dbs.library, &album_id).await {
                        Ok(Some(artist_id)) => {
                            artist_covers_album(&dbs, &artist_id, &album_id).await
                        }
                        _ => false,
                    };
                if artist_covered {
                    // Redundant: the artist container already owns these songs.
                    eprintln!(
                        "download: album {album_id} already covered by the artist container; skipping"
                    );
                    let json = album_cache_payload(&album_id, true, false, true);
                    emit(&invoker, "album_cache", json);
                    return;
                }
                let songs = match album_container_on(&dbs, &album_id).await {
                    Ok(songs) => songs,
                    Err(err) => {
                        raise(&error_invoker, "error", format!("Could not select the album for download: {err:#}"));
                        return;
                    }
                };
                eprintln!(
                    "download: album {album_id} pass started ({} songs)",
                    songs.len()
                );
                // The pass is now in flight: the chip shows its loading loop.
                let json = album_cache_payload(&album_id, true, true, false);
                emit(&invoker, "album_cache", json);
                fetch_marked(&dbs, &songs, &album_id, true, fetch_invoker).await;
                eprintln!("download: album {album_id} pass finished");
                let json = album_cache_payload(&album_id, true, false, false);
                emit(&invoker, "album_cache", json);
            } else {
                album_container_off(&dbs, &album_id).await;
                eprintln!("download: album {album_id} removed from the cache");
                let json = album_cache_payload(&album_id, false, false, false);
                emit(&invoker, "album_cache", json);
                // The marks lose the album's songs: hard reset + re-read.
                qtbridge::invoke_method!(fetch_invoker, "ResetRowMarks");
            }
        });
    }
}
