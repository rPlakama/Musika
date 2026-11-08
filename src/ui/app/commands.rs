//! Socket / `:` command context resolution for shuffle, radio and download.

use super::super::ipc_command::{self, RadioSeed, ShuffleTarget};
use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn execute_shuffle(&mut self, target: ShuffleTarget) {
        match target {
            ShuffleTarget::Queue => {
                let queue = self.player.queue_snapshot();
                if queue.is_empty() {
                    self.push_toast("The queue is empty.".to_string(), ToastKind::Error, None);
                    return;
                }
                let ids = queue.iter().map(|track| track.id.clone()).collect();
                self.bridge.shuffle_queue(ids);
            }
            ShuffleTarget::Album(id) => {
                let id = id.or_else(|| {
                    self.page
                        .as_ref()
                        .filter(|page| page.descriptor.kind == state::PageKind::Album)
                        .map(|page| page.descriptor.id.clone())
                });
                match id {
                    Some(id) => self.bridge.shuffle_album(id),
                    None => self.push_toast(
                        "Open an album or pass an album id.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
            ShuffleTarget::Artist(id) => {
                let id = id.or_else(|| {
                    self.page
                        .as_ref()
                        .filter(|page| page.descriptor.kind == state::PageKind::Artist)
                        .map(|page| page.descriptor.id.clone())
                });
                match id {
                    Some(id) => self.bridge.shuffle_artist(id),
                    None => self.push_toast(
                        "Open an artist or pass an artist id.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
        }
    }

    pub(in crate::ui) fn execute_radio(&mut self, seed: RadioSeed) {
        let play = |kind: &'static str, id: String| (kind, id);
        let selected = match seed {
            RadioSeed::Current => self
                .player
                .current()
                .map(|track| play("song", track.id.clone())),
            RadioSeed::Album(id) => id
                .map(|id| play("album", id))
                .or_else(|| self.current_album_id().map(|id| play("album", id))),
            RadioSeed::Song(id) => id
                .map(|id| play("song", id))
                .or_else(|| self.current_song_id().map(|id| play("song", id)))
                .or_else(|| {
                    self.player
                        .current()
                        .map(|track| play("song", track.id.clone()))
                }),
        };
        match selected {
            Some((kind, id)) => self.bridge.play_radio(kind.to_string(), id, 0),
            None => self.push_toast(
                "No song or album is selected for radio.".to_string(),
                ToastKind::Error,
                None,
            ),
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
