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

use qtbridge::QmlMethodInvoker;

/// Resolve one cover id to a cached file, fetching it on demand, then return
/// a `file://` URL for a display-sized copy. `target_size <= 0` returns the
/// original; a missing cache/fetch leaves the caller without a payload (the
/// QML surface keeps its original URL until a display copy lands).
///
/// `target_size` comes from `CoverSize.js` — a screen/DPR bucket, never the
/// provider's full-resolution image. The resize runs off the Qt thread.
pub(super) async fn resized_cover_url(
    dbs: &crate::db::Dbs,
    cover_id: &str,
    target_size: i32,
) -> Option<String> {
    if cover_id.is_empty() {
        return None;
    }
    let mut path = crate::covers::cached_path(&dbs.covers, cover_id)
        .await
        .ok()
        .flatten();
    if path.is_none()
        && let Ok(client) = crate::session::client(dbs).await
    {
        let client = std::sync::Arc::new(client);
        let ids = [cover_id.to_string()];
        if let Err(err) = crate::covers::fetch_ids(&dbs.covers, &client, &ids).await {
            eprintln!("covers: display fetch failed for {cover_id}: {err:#}");
        }
        path = crate::covers::cached_path(&dbs.covers, cover_id)
            .await
            .ok()
            .flatten();
    }
    let path = path?;
    let path = if target_size > 0 {
        crate::covers::get_or_create_resized_cover(&path, target_size as u32)
            .await
            .unwrap_or(path)
    } else {
        path
    };
    reqwest::Url::from_file_path(path)
        .ok()
        .map(|url| url.to_string())
}

pub(super) fn emit(invoker: &QmlMethodInvoker, channel: &str, payload: String) {
    qtbridge::invoke_method!(invoker, "emitData", channel.to_string(), payload);
}

pub(super) fn raise(invoker: &QmlMethodInvoker, kind: &str, message: String) {
    qtbridge::invoke_method!(invoker, "errorRaised", kind.to_string(), message);
}

/// The artist-container toggle payload (`artist_cache` channel, CACHE-5);
/// `active` marks a fetch pass in flight for the chip's loading loop.
pub(super) fn artist_cache_payload(artist_id: &str, on: bool, active: bool) -> String {
    serde_json::json!({ "id": artist_id, "on": on, "active": active }).to_string()
}

/// The album container state for the fullscreen keybar's Download button:
/// `on` = selected (an album rule, or an artist rule that covers the album),
/// `active` = a fetch pass is in flight, `covered` = the artist owns it.
pub(super) fn album_cache_payload(album_id: &str, on: bool, active: bool, covered: bool) -> String {
    serde_json::json!({ "id": album_id, "on": on, "active": active, "covered": covered })
        .to_string()
}

/// The album favourite flag and user rating for the favourite marks.
pub(super) fn album_state_payload(album_id: &str, starred: bool, rating: i64) -> String {
    serde_json::json!({ "id": album_id, "starred": starred, "rating": rating }).to_string()
}
