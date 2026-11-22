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

//! Shared row text: the one duration format and the one artist/album
//! subtitle rule every library surface inherits (lists, pages, search,
//! queue rows).

/// The one row-duration format: m:ss, or h:mm:ss past the hour.
pub(crate) fn format_duration(seconds: Option<i32>) -> String {
    let total = seconds.unwrap_or(0).max(0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The album row subtitle: `artist · year`, dropping the parts it lacks.
///
/// N-21: an empty artist must not leave a leading `" · "` separator.
pub(crate) fn album_subtitle(album: &crate::library::AlbumRow) -> String {
    let artist = album.artist.as_deref().unwrap_or("").trim();
    match (artist.is_empty(), album.year) {
        (true, Some(year)) => year.to_string(),
        (true, None) => String::new(),
        (false, Some(year)) => format!("{artist} · {year}"),
        (false, None) => artist.to_string(),
    }
}

/// The song row subtitle: `artist · album`, dropping the parts it lacks.
///
/// N-21: whichever part is missing (or blank) is dropped together with its
/// separator, so untagged songs render no dangling `" · "`.
pub(crate) fn song_subtitle_from_parts(artist: Option<&str>, album: Option<&str>) -> String {
    let artist = artist.unwrap_or("").trim();
    let album = album.unwrap_or("").trim();
    match (artist.is_empty(), album.is_empty()) {
        (true, true) => String::new(),
        (true, false) => album.to_string(),
        (false, true) => artist.to_string(),
        (false, false) => format!("{artist} · {album}"),
    }
}
