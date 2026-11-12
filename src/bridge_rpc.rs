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

//! The bridge RPC table: one declaration of every request the UI client can
//! send to the core.
//!
//! The UI-side `RemoteBridge` methods and the core-side `dispatch_bridge`
//! arms are both generated from this list, so a request's name, its argument
//! names and their types exist once. The core handler bodies stay explicit in
//! `Bridge`, which the generated dispatcher calls and the compiler checks.
//!
//! The callback receives the whole list as `name(arg: Type, ...),` entries:
//!
//! ```ignore
//! crate::for_each_bridge_rpc!(generate_my_side);
//! ```

#[macro_export]
macro_rules! for_each_bridge_rpc {
    ($callback:ident) => {
        $callback! {
            // Session / bootstrap.
            refresh_header(),
            restore(),
            play_random_album(),
            set_offline(offline: bool),
            logout(),
            // Library and search.
            load_library(scope: $crate::model::Scope, order: usize),
            request_library_window(
                generation: u64,
                scope: $crate::model::Scope,
                order: usize,
                start: usize,
                count: usize
            ),
            preload_library(scope: $crate::model::Scope, order: usize),
            search(query: String),
            request_command_suggestions(
                generation: u64,
                scope: $crate::ipc_command::SuggestScope,
                query: String,
                start: usize,
                end: usize
            ),
            load_page(descriptor: $crate::model::PageDescriptor),
            request_notes(album_id: String),
            request_artist_notes(artist_id: String),
            request_lyrics(song_id: String),
            request_marks(songs: Vec<String>, albums: Vec<String>),
            request_container_state(kind: $crate::model::PageKind, id: String),
            request_album_state(album_id: String),
            request_colors(cover_art: String),
            request_display_cover(key: String, cover_art: String, target: u32),
            // Playback and queues.
            play_song(song_id: String, order: String),
            play_song_in_album(song_id: String),
            play_album(album_id: String, start: usize),
            play_radio(kind: String, id: String, start_index: usize),
            play_radio_query(kind: String, query: String, start_index: usize),
            quick_radio(kind: String, id: String),
            shuffle_album(album_id: String),
            shuffle_artist(artist_id: String),
            shuffle_album_query(query: String),
            shuffle_artist_query(query: String),
            shuffle_radio(seed_kind: String, seed_id: String),
            queue_add_radio(seed_kind: String, seed_id: String),
            save_radio(seed_kind: String, seed_id: String, name: String),
            save_radio_local(seed_kind: String, seed_id: String, name: String),
            download_radio(seed_kind: String, seed_id: String),
            // Row actions.
            quick_queue_add(kind: String, id: String),
            paste_queue(kind: String, ids: Vec<String>, at: usize),
            quick_favorite(kind: String, id: String, on: bool),
            quick_rate(kind: String, id: String, level: u8),
            quick_download(kind: String, id: String, on: bool),
            quick_go_album(song_id: String),
            quick_go_artist(album_id: String),
            quick_play_playlist(id: String),
            open_album_artist_reveal(album_id: String),
            download_artist(artist_id: String, on: bool),
            // History, accounts and maintenance.
            sync(mode: $crate::bridge::SyncMode),
            wipe(scope: String),
            download_request(
                request: $crate::ipc_command::DownloadRequest,
                current: Option<$crate::ipc_command::CurrentTrack>
            ),
            login(server_url: String, username: String, password: String),
            offline_login(server_url: String, username: String),
            remove_account(keep_data: bool),
            // Playlists.
            create_playlist(name: String, song_ids: Vec<String>, offline: bool),
            playlist_add(
                targets: Vec<String>,
                playlist: String,
                current_song_id: Option<String>
            ),
            playlist_add_selected(kind: String, ids: Vec<String>, playlist: String),
            playlist_remove(
                targets: Vec<String>,
                playlist: String,
                current_song_id: Option<String>
            ),
            delete_playlist(playlist: String),
            paste_playlist(
                playlist_id: String,
                kind: String,
                ids: Vec<String>,
                position: usize
            ),
            move_playlist_row(playlist_id: String, from: usize, to: usize),
            move_playlist_block(
                playlist_id: String,
                start: usize,
                len: usize,
                delta: i32,
                cursor: usize
            ),
            delete_playlist_positions(playlist_id: String, positions: Vec<usize>),
        }
    };
}
