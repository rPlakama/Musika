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

//! Unified search and the shared row-text helpers.
//!
//! List surfaces receive typed [`crate::model::Row`] windows from
//! [`crate::rows`]; this module owns the search engine merge
//! ([`search`]) and the album/song subtitle and duration rules every
//! surface inherits.

pub(crate) mod rows;
pub(crate) mod search;

pub use search::search;
#[cfg(test)]
pub(crate) use search::{merge_hits, search_results_index};

pub async fn album_notes(dbs: &crate::db::Dbs, album_id: &str) -> String {
    let Ok(client) = crate::session::client(dbs).await else {
        return String::new();
    };
    match client.get_album_info2(album_id).await {
        Ok(info) => info.notes.unwrap_or_default(),
        Err(err) => {
            eprintln!("album: could not fetch info for {album_id}: {err:#}");
            String::new()
        }
    }
}
