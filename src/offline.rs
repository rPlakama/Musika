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

//! CACHE-9: the offline catalog shared by list loaders and queue builders.
//!
//! Only songs whose download finished are listable (and therefore playable)
//! while the server is unreachable; albums and artists survive only through
//! their downloaded songs.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;

/// The downloaded catalog, with the album/artist counts derived from it.
pub(crate) struct Filter {
    songs: HashSet<String>,
    albums: HashMap<String, i64>,
    artists: HashMap<String, i64>,
}

impl Filter {
    pub(crate) fn contains_song(&self, id: &str) -> bool {
        self.songs.contains(id)
    }

    pub(crate) fn contains_album(&self, id: &str) -> bool {
        self.albums.contains_key(id)
    }

    pub(crate) fn contains_artist(&self, id: &str) -> bool {
        self.artists.contains_key(id)
    }

    pub(crate) fn album_count(&self, id: &str) -> Option<i64> {
        self.albums.get(id).copied()
    }

    pub(crate) fn artist_count(&self, id: &str) -> Option<i64> {
        self.artists.get(id).copied()
    }
}

/// PATH-12: the offline catalog is the same for every payload between ledger
/// changes; a short TTL coalesces paging/search bursts, and download settles
/// clear it eagerly.
static CACHE: OnceLock<Mutex<Option<(Instant, Arc<Filter>)>>> = OnceLock::new();
const TTL: Duration = Duration::from_secs(2);

pub(crate) fn clear_cache() {
    if let Some(cache) = CACHE.get() {
        *cache.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}

/// `None` while online; the downloaded set while offline.
pub(crate) async fn load(dbs: &crate::db::Dbs, offline: bool) -> Result<Option<Arc<Filter>>> {
    if !offline {
        return Ok(None);
    }
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    {
        let guard = cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((at, filter)) = guard.as_ref()
            && at.elapsed() < TTL
        {
            return Ok(Some(Arc::clone(filter)));
        }
    }
    let Some(songs) = crate::download::offline_song_ids(dbs, offline).await? else {
        return Ok(None);
    };
    let ids: Vec<String> = songs.iter().cloned().collect();
    let (albums, artists) = crate::library::downloaded_owners(&dbs.library, &ids).await?;
    let filter = Arc::new(Filter {
        songs,
        albums,
        artists,
    });
    *cache.lock().unwrap_or_else(|error| error.into_inner()) =
        Some((Instant::now(), Arc::clone(&filter)));
    Ok(Some(filter))
}

/// Keeps only downloaded songs in `songs`; a no-op while online.
pub(crate) async fn retain_songs(
    dbs: &crate::db::Dbs,
    offline: bool,
    songs: &mut Vec<crate::library::SongRow>,
) -> Result<()> {
    if let Some(filter) = load(dbs, offline).await? {
        songs.retain(|song| filter.contains_song(&song.id));
    }
    Ok(())
}
