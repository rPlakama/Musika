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

//! `sqlx::migrate!` embeds the migration files at compile time but does not
//! ask cargo to watch their directories, so a new or edited migration would
//! not trigger a rebuild and the binary would keep running the old schema.
//! Emit the watch list explicitly (sqlx's recommended build.rs).

fn main() {
    for dir in [
        "migrations",
        "migrations_artist_user",
        "migrations_colors",
        "migrations_covers",
        "migrations_download",
        "migrations_library",
        "migrations_lyrics",
        "migrations_media",
        "migrations_playlist",
        "migrations_search",
        "migrations_status",
        // qtbridge embeds these trees at compile time; without the watch a
        // QML-only edit keeps `cargo run` on the previous engine payload.
        "src/qml",
        "src/icons",
    ] {
        println!("cargo:rerun-if-changed={dir}");
    }
}
