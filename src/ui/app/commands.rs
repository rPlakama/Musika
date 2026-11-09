//! Socket / `:` command context resolution for shuffle, radio and download.

use super::super::ipc_command::{self, RadioSeed, ShuffleTarget};
use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn execute_shuffle(&mut self, target: ShuffleTarget) {
        match target {
            ShuffleTarget::Queue => {
                if self.player.len() == 0 {
                    self.push_toast("The queue is empty.".to_string(), ToastKind::Error, None);
                    return;
                }
                // The queue is a plain in-memory array: permute it directly
                // and restart from the shuffled head (docs/Commands.md).
                self.player.shuffle_queue();
                self.playback_error = None;
                self.request_current_lyrics();
                if self.queue_open {
                    self.queue_selection.clear();
                    self.visual_mode = false;
                    self.land_queue(self.player.index());
                }
            }
            ShuffleTarget::Album(Some(query)) => self.bridge.shuffle_album_query(query),
            ShuffleTarget::Album(None) => {
                let id = self
                    .page
                    .as_ref()
                    .filter(|page| page.descriptor.kind == state::PageKind::Album)
                    .map(|page| page.descriptor.id.clone());
                match id {
                    Some(id) => self.bridge.shuffle_album(id),
                    None => self.push_toast(
                        "Open an album or pass an album name.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
            ShuffleTarget::Artist(Some(query)) => self.bridge.shuffle_artist_query(query),
            ShuffleTarget::Artist(None) => {
                let id = self
                    .page
                    .as_ref()
                    .filter(|page| page.descriptor.kind == state::PageKind::Artist)
                    .map(|page| page.descriptor.id.clone());
                match id {
                    Some(id) => self.bridge.shuffle_artist(id),
                    None => self.push_toast(
                        "Open an artist or pass an artist name.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
        }
    }

    pub(in crate::ui) fn execute_radio(&mut self, seed: RadioSeed) {
        let play = |kind: &'static str, id: String| (kind, id);
        match seed {
            RadioSeed::Album(Some(query)) => {
                self.bridge.play_radio_query("album".to_string(), query, 0)
            }
            RadioSeed::Song(Some(query)) => {
                self.bridge.play_radio_query("song".to_string(), query, 0)
            }
            RadioSeed::Current => match self
                .player
                .current()
                .map(|track| play("song", track.id.clone()))
            {
                Some((kind, id)) => {
                    self.open_descriptor(PageDescriptor::radio(kind, id.clone()));
                    self.bridge.play_radio(kind.to_string(), id, 0);
                }
                None => self.push_toast(
                    "Nothing is playing to start radio from.".to_string(),
                    ToastKind::Error,
                    None,
                ),
            },
            RadioSeed::Album(None) => {
                let id = self
                    .page
                    .as_ref()
                    .filter(|page| page.descriptor.kind == state::PageKind::Album)
                    .map(|page| page.descriptor.id.clone())
                    .or_else(|| self.current_album_id());
                match id {
                    Some(id) => {
                        self.open_descriptor(PageDescriptor::radio("album", id.clone()));
                        self.bridge.play_radio("album".to_string(), id, 0);
                    }
                    None => self.push_toast(
                        "Open an album or pass an album name.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
            RadioSeed::Song(None) => {
                let id = self.current_song_id().or_else(|| {
                    self.player
                        .current()
                        .map(|track| track.id.clone())
                        .filter(|id| !id.is_empty())
                });
                match id {
                    Some(id) => {
                        self.open_descriptor(PageDescriptor::radio("song", id.clone()));
                        self.bridge.play_radio("song".to_string(), id, 0);
                    }
                    None => self.push_toast(
                        "Select a song or pass a song name.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
        }
    }

    pub(in crate::ui) fn execute_download(&self, request: ipc_command::DownloadRequest) {
        let current = self
            .player
            .current()
            .map(|track| ipc_command::CurrentTrack {
                id: track.id.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                album_id: track.album_id.clone(),
            });
        self.bridge.download_request(request, current);
    }

    fn current_album_id(&self) -> Option<String> {
        self.page
            .as_ref()
            .filter(|page| page.descriptor.kind == state::PageKind::Album)
            .map(|page| page.descriptor.id.clone())
            .or_else(|| self.player.current().map(|track| track.album_id.clone()))
            .filter(|id| !id.is_empty())
    }

    fn current_song_id(&self) -> Option<String> {
        self.page
            .as_ref()
            .and_then(|page| page.rows.get(page.cursor))
            .filter(|row| row.kind == state::RowKind::Song)
            .map(|row| row.song_id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| {
                self.search
                    .as_ref()
                    .and_then(|search| {
                        let rows = search.scoped();
                        rows.get(search.cursor())
                            .filter(|row| row.kind == state::RowKind::Song)
                            .map(|row| row.song_id.clone())
                    })
                    .filter(|id| !id.is_empty())
            })
            .or_else(|| {
                self.view
                    .rows
                    .row(self.view.cursor)
                    .filter(|row| row.kind == state::RowKind::Song)
                    .map(|row| row.song_id.clone())
                    .filter(|id| !id.is_empty())
            })
    }
}

impl AppRoot {
    /// Continuous rotation for the transport loading spinner while the
    /// decoder is opening/fetching a stream.
    pub(in crate::ui) fn stream_loading_angle(&self) -> f32 {
        ((self.clock.now * 260.0) as f32).rem_euclid(360.0)
    }
}
