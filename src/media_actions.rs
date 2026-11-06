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

use crate::session::client;

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
