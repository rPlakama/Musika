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

use crate::http_client::SubsonicClient;

pub(crate) async fn client(dbs: &crate::db::Dbs) -> anyhow::Result<SubsonicClient> {
    let credentials = crate::sqlite::load_credentials(&dbs.credentials)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no saved credentials"))?;
    Ok(SubsonicClient::new(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    ))
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
