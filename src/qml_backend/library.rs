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

//! Album/artist state, info-cache and container-selection bodies for the
//! bridge. The QML slots stay in `qml.rs` as thin delegators.

use qtbridge::QmlObject;

use crate::browse::album_notes;
use crate::session::client;

use super::Backend;
use super::artist::{ArtistUserAction, apply_artist_user};
use super::download::artist_covers_album;
use super::support::{album_cache_payload, album_state_payload, artist_cache_payload, emit, raise};

impl Backend {
    pub(super) fn star_artist_impl(&mut self, artist_id: String, on: bool) {
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let result = apply_artist_user(&dbs, &artist_id, ArtistUserAction::Star(on)).await;
            let json = crate::artist_user::payload(&dbs.artist_user, &artist_id).await;
            emit(&invoker, "artist_user", json);
            if let Err(err) = result {
                raise(
                    &error_invoker,
                    "error",
                    format!("Could not toggle favourite: {err:#}"),
                );
            }
        });
    }
    pub(super) fn rate_artist_impl(&mut self, artist_id: String, rating: i32) {
        let rating = match u8::try_from(rating) {
            Ok(rating) if rating <= 5 => rating,
            _ => {
                eprintln!("artist: rating {rating} is outside the 0..=5 domain");
                return;
            }
        };
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let result = apply_artist_user(&dbs, &artist_id, ArtistUserAction::Rate(rating)).await;
            let json = crate::artist_user::payload(&dbs.artist_user, &artist_id).await;
            emit(&invoker, "artist_user", json);
            // UI-60: patch the artist rows in place (list/search), also on
            // failure — the stored value reverts the optimistic badge.
            let stored = crate::artist_user::get(&dbs.artist_user, &artist_id)
                .await
                .ok()
                .flatten()
                .map(|row| row.rating)
                .unwrap_or(0);
            qtbridge::invoke_method!(
                invoker,
                "ratingChanged",
                "artist".to_string(),
                artist_id.clone(),
                stored
            );
            if let Err(err) = result {
                raise(
                    &error_invoker,
                    "error",
                    format!("Could not rate artist: {err:#}"),
                );
            }
        });
    }
    pub(super) fn request_artist_info_impl(&mut self, artist_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let cached = crate::library::artist_info(&dbs.library, &artist_id)
                .await
                .ok()
                .flatten();
            if let Some(row) = &cached {
                emit(
                    &invoker,
                    "artist_info",
                    serde_json::json!({
                        "id": artist_id,
                        "biography": row.biography,
                        "url": row.url,
                    })
                    .to_string(),
                );
                if let Ok(fetched_at) = row.fetched_at.parse::<i64>()
                    && crate::consts::now_secs() - fetched_at < crate::consts::INFO_TTL_SECS
                {
                    return;
                }
            }
            let fetched = async {
                let client = client(&dbs).await?;
                let (biography, url) =
                    match client.get_artist_info2(&artist_id, Some(10), None).await {
                        Ok(info) => (info.info.biography, info.info.last_fm_url),
                        Err(err) => {
                            eprintln!("artist: getArtistInfo2 failed for {artist_id}: {err:#}");
                            let info = client.get_artist_info(&artist_id, Some(10), None).await?;
                            (info.info.biography, info.info.last_fm_url)
                        }
                    };
                let biography = biography.unwrap_or_default();
                let url = url.unwrap_or_default();
                crate::library::upsert_artist_info(&dbs.library, &artist_id, &biography, &url)
                    .await?;
                Ok::<_, anyhow::Error>((biography, url))
            }
            .await;

            match fetched {
                Ok((biography, url)) => {
                    emit(
                        &invoker,
                        "artist_info",
                        serde_json::json!({
                            "id": artist_id,
                            "biography": biography,
                            "url": url,
                        })
                        .to_string(),
                    );
                }
                Err(err) => {
                    eprintln!("artist: info fetch failed for {artist_id}: {err:#}");
                    if cached.is_none() {
                        emit(
                            &invoker,
                            "artist_info",
                            serde_json::json!({
                                "id": artist_id,
                                "biography": "",
                                "url": "",
                            })
                            .to_string(),
                        );
                    }
                }
            }
        });
    }
    pub(super) fn request_artist_cache_impl(&mut self, artist_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            // The artist is "on" when every album of it holds an album rule
            // (the batch is per album now; a legacy artist rule has no role).
            let albums = crate::library::artist_album_ids(&dbs.library, &artist_id)
                .await
                .unwrap_or_default();
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
            let json = artist_cache_payload(&artist_id, on, false);
            emit(&invoker, "artist_cache", json);
        });
    }
    pub(super) fn request_album_cache_impl(&mut self, album_id: String) {
        self.spawn_data("album_cache", move |dbs| async move {
            let album_rule = matches!(
                crate::media::rule_for(&dbs.media, "album", &album_id).await,
                Ok(Some(_))
            );
            let artist_covered =
                match crate::library::album_artist_id(&dbs.library, &album_id).await {
                    Ok(Some(artist_id)) => artist_covers_album(&dbs, &artist_id, &album_id).await,
                    _ => false,
                };
            Ok(album_cache_payload(
                &album_id,
                album_rule || artist_covered,
                false,
                artist_covered,
            ))
        });
    }
    pub(super) fn request_album_state_impl(&mut self, album_id: String) {
        self.spawn_data("album_state", move |dbs| async move {
            let starred = crate::library::album_starred(&dbs.library, &album_id)
                .await
                .unwrap_or(false);
            let rating = crate::library::album_rating(&dbs.library, &album_id)
                .await
                .unwrap_or(0);
            // Re-resolve the header cover here too: a sync can replace the
            // art (or a wipe can drop it) while the page stays open.
            let cover_ids: Vec<String> = crate::library::album(&dbs.library, &album_id)
                .await
                .ok()
                .flatten()
                .and_then(|album| album.cover_art)
                .into_iter()
                .collect();
            let covers = crate::covers::url_map_of(&dbs, &cover_ids)
                .await
                .unwrap_or_default();
            let cover = cover_ids
                .first()
                .and_then(|id| covers.get(id))
                .cloned()
                .unwrap_or_default();
            Ok(serde_json::json!({
                "id": album_id,
                "starred": starred,
                "rating": rating,
                "cover": cover,
            })
            .to_string())
        });
    }
    pub(super) fn rate_album_impl(&mut self, album_id: String, rating: i32) {
        let rating = match u8::try_from(rating) {
            Ok(rating) if rating <= 5 => rating,
            _ => {
                eprintln!("album: rating {rating} is outside the 0..=5 domain");
                return;
            }
        };
        let error_invoker = self.get_qml_method_invoker();
        let state_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let result = async {
                let client = client(&dbs).await?;
                client.set_rating(&album_id, rating).await?;
                crate::library::set_album_rating(&dbs.library, &album_id, rating).await
            }
            .await;
            // Re-read the stored row: on failure this reverts the optimistic
            // badge to the last accepted value.
            let stored = crate::library::album_rating(&dbs.library, &album_id)
                .await
                .unwrap_or(0);
            let starred = crate::library::album_starred(&dbs.library, &album_id)
                .await
                .unwrap_or(false);
            qtbridge::invoke_method!(
                state_invoker,
                "ratingChanged",
                "album".to_string(),
                album_id.clone(),
                stored
            );
            emit(
                &state_invoker,
                "album_state",
                album_state_payload(&album_id, starred, stored),
            );
            if let Err(err) = result {
                eprintln!("album: could not rate {album_id}: {err:#}");
                raise(
                    &error_invoker,
                    "error",
                    format!("Could not rate album: {err:#}"),
                );
            }
        });
    }
    pub(super) fn request_album_info_impl(&mut self, album_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let cached = crate::library::album_info(&dbs.library, &album_id)
                .await
                .ok()
                .flatten();
            if let Some((notes, fetched_at)) = &cached {
                emit(
                    &invoker,
                    "album_info",
                    serde_json::json!({ "notes": notes }).to_string(),
                );
                if crate::consts::now_secs() - fetched_at < crate::consts::INFO_TTL_SECS {
                    return;
                }
            }
            let fresh = album_notes(&dbs, &album_id).await;
            if fresh.is_empty() {
                if cached.is_none() {
                    emit(
                        &invoker,
                        "album_info",
                        serde_json::json!({ "notes": "" }).to_string(),
                    );
                }
                return;
            }
            if let Err(err) =
                crate::library::upsert_album_info(&dbs.library, &album_id, &fresh).await
            {
                eprintln!("album: could not cache notes for {album_id}: {err:#}");
            }
            emit(
                &invoker,
                "album_info",
                serde_json::json!({ "notes": fresh }).to_string(),
            );
        });
    }
}
