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

//! Toolkit-free media download actions shared by the GPUI bridge.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use crate::session::client;

/// One playback-cache fetch per song at a time: a play/retry burst must not
/// launch overlapping downloads of the same blob.
static STREAM_CACHE_INFLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// One *download* per song at a time in this process (N-05). The ledger's
/// `active` state can be stale after a crash, so this in-memory claim is the
/// authoritative in-flight marker; it also closes the check-then-set race two
/// concurrent `download_one` calls had.
static ONGOING_DOWNLOADS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn lock_set(set: &Mutex<HashSet<String>>) -> std::sync::MutexGuard<'_, HashSet<String>> {
    set.lock().unwrap_or_else(|error| error.into_inner())
}

/// Owned claim on one in-flight fetch. Dropping it releases the song — also
/// on panic or task cancellation (N-05/N-10).
struct FetchGuard {
    set: &'static Mutex<HashSet<String>>,
    song_id: String,
}

impl Drop for FetchGuard {
    fn drop(&mut self) {
        lock_set(self.set).remove(&self.song_id);
    }
}

fn claim(set: &'static Mutex<HashSet<String>>, song_id: &str) -> Option<FetchGuard> {
    if !lock_set(set).insert(song_id.to_string()) {
        return None;
    }
    Some(FetchGuard {
        set,
        song_id: song_id.to_string(),
    })
}

fn claim_stream_cache(song_id: &str) -> Option<FetchGuard> {
    claim(&STREAM_CACHE_INFLIGHT, song_id)
}

fn claim_download(song_id: &str) -> Option<FetchGuard> {
    claim(&ONGOING_DOWNLOADS, song_id)
}

pub(crate) async fn rule_off(dbs: &crate::db::Dbs, rule_id: &str) {
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

/// Drops every downloaded album: album media rules plus explicit downloads
/// for songs that belong to an album. Library metadata is untouched.
pub(crate) async fn wipe_albums(dbs: &crate::db::Dbs) -> anyhow::Result<()> {
    let rules: Vec<String> = sqlx::query_scalar("SELECT id FROM media_rule WHERE kind = 'album'")
        .fetch_all(&dbs.media)
        .await?;
    for rule_id in rules {
        rule_off(dbs, &rule_id).await;
    }

    let done = crate::download::done_ids(&dbs.download).await?;
    if done.is_empty() {
        return Ok(());
    }
    let albums = crate::library::song_album_ids(&dbs.library, &done).await?;
    let album_songs: Vec<String> = done
        .into_iter()
        .filter(|song| albums.contains_key(song))
        .collect();
    for song in &album_songs {
        _ = crate::download::unmark(&dbs.download, song).await;
    }
    crate::media::remove_songs_media(&dbs.media, &album_songs).await?;
    Ok(())
}

/// Drops an album container: its rule, marks and blobs.
pub(crate) async fn album_container_off(dbs: &crate::db::Dbs, album_id: &str) {
    match crate::media::rule_for(&dbs.media, "album", album_id).await {
        Ok(Some(rule)) => rule_off(dbs, &rule.id).await,
        Ok(None) => {}
        Err(err) => eprintln!("media: could not read the album rule: {err:#}"),
    }
}
pub(crate) async fn download_one(
    dbs: &crate::db::Dbs,
    song_id: &str,
) -> anyhow::Result<&'static str> {
    if crate::media::downloaded(&dbs.media, song_id).await?
        && crate::media::cached_path(&dbs.media, song_id)
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
    let rule_of_mark = mark.and_then(|row| row.rule_id);
    // N-05: the in-process RAII claim is the authoritative in-flight marker
    // (the ledger check alone raced between two concurrent calls). A ledger
    // row still `active` here is a leftover from a crash and is taken over
    // below instead of locking the song out forever.
    let Some(_inflight) = claim_download(song_id) else {
        eprintln!("download: {song_id} fetch already in flight");
        return Ok(crate::download::ACTIVE);
    };

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
                _ = tokio::fs::remove_file(&temp).await;
                return Err(err);
            }
        };
        // N-01: a zero-byte body is a failed transfer, never a cached song.
        if written == 0 {
            _ = tokio::fs::remove_file(&temp).await;
            anyhow::bail!("the server returned an empty download");
        }
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
            _ = crate::download::set_state(
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

/// Caches the currently streamed bytes into the purgeable playback tier so the
/// next play of the same song (and the same quality) is local.
///
/// This is deliberately not a download: it writes no download-ledger row, is
/// never used by the offline catalog, and is evicted by the LRU cap. A song
/// already downloaded in the config tier is left alone.
pub(crate) async fn cache_stream(
    dbs: &crate::db::Dbs,
    song_id: &str,
    settings: &crate::playback_settings::PlaybackSettings,
) -> anyhow::Result<bool> {
    if !settings.stream_cache || song_id.is_empty() {
        return Ok(false);
    }
    if crate::media::downloaded(&dbs.media, song_id).await?
        && crate::media::cached_path(&dbs.media, song_id)
            .await?
            .is_some()
    {
        return Ok(false);
    }
    let quality = settings.stream_quality_key();
    if crate::media::playback_path(&dbs.media, song_id, &quality)
        .await?
        .is_some()
    {
        _ = crate::media::touch_playback(&dbs.media, song_id).await;
        return Ok(true);
    }
    // N-10: the RAII claim is released on every exit path, cancellation too.
    let Some(_inflight) = claim_stream_cache(song_id) else {
        return Ok(false);
    };
    cache_stream_inner(dbs, song_id, settings, &quality).await
}

async fn cache_stream_inner(
    dbs: &crate::db::Dbs,
    song_id: &str,
    settings: &crate::playback_settings::PlaybackSettings,
    quality: &str,
) -> anyhow::Result<bool> {
    let client = client(dbs).await?;
    let (size, suffix, bit_rate) = crate::library::song_file_info(&dbs.library, song_id)
        .await?
        .unwrap_or((None, None, None));
    let fingerprint =
        crate::media::fingerprint(size, suffix.as_deref(), bit_rate, client.base(), quality);
    let extension = settings
        .stream_format
        .as_deref()
        .filter(|format| !format.is_empty() && *format != "original")
        .or(suffix.as_deref())
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or("bin");
    let temp = crate::media::temp_path(crate::media::KIND_PLAYBACK)?;
    let written = match client
        .download_stream_to(
            song_id,
            settings.stream_max_bitrate,
            settings.stream_format.as_deref(),
            &temp,
        )
        .await
    {
        Ok(written) => written,
        Err(error) => {
            _ = tokio::fs::remove_file(&temp).await;
            return Err(error);
        }
    };
    if written == 0 {
        _ = tokio::fs::remove_file(&temp).await;
        anyhow::bail!("the server returned an empty stream");
    }
    // N-05: a download may have completed while this stream was in flight;
    // never overwrite the user's config-tier blob with a purgeable copy.
    // (`media::record` refuses the downgrade atomically as the last line of
    // defence.)
    if crate::media::downloaded(&dbs.media, song_id).await?
        && crate::media::cached_path(&dbs.media, song_id)
            .await?
            .is_some()
    {
        _ = tokio::fs::remove_file(&temp).await;
        return Ok(false);
    }
    crate::media::store_streamed(
        &dbs.media,
        song_id,
        &temp,
        i64::try_from(written).unwrap_or(i64::MAX),
        crate::media::KIND_PLAYBACK,
        quality,
        &fingerprint,
        extension,
        None,
    )
    .await?;
    let cap = settings.stream_cache_mib.saturating_mul(1024 * 1024);
    if let Err(error) = crate::media::enforce_playback_cap(&dbs.media, cap).await {
        crate::debug_log!("stream cache: eviction failed: {error:#}");
    }
    crate::debug_log!("stream cache: stored {song_id} ({quality}, {written} bytes)");
    Ok(true)
}
