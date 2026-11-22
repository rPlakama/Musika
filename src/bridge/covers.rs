//! Palette requests.

use super::*;

struct PaletteInflightGuard {
    slot: Arc<Mutex<Option<String>>>,
    cover_art: String,
}

impl Drop for PaletteInflightGuard {
    fn drop(&mut self) {
        let mut inflight = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inflight.as_deref() == Some(self.cover_art.as_str()) {
            *inflight = None;
        }
    }
}

impl Bridge {
    pub fn request_colors(&self, cover_art: String) {
        if cover_art.is_empty() {
            return;
        }
        {
            let mut inflight = self
                .palette_inflight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inflight.as_deref() == Some(cover_art.as_str()) {
                return;
            }
            *inflight = Some(cover_art.clone());
        }
        let tx = self.tx.clone();
        let guard = PaletteInflightGuard {
            slot: Arc::clone(&self.palette_inflight),
            cover_art: cover_art.clone(),
        };
        crate::app::runtime().spawn(async move {
            let _guard = guard;
            let Some(dbs) = crate::app::dbs() else { return };
            let event = match crate::colors::fetch(&dbs, &cover_art).await {
                Ok(Some(scheme)) => Event::Palette {
                    cover_art: cover_art.clone(),
                    roles: scheme.roles,
                },
                _ => Event::PaletteReset {
                    cover_art: cover_art.clone(),
                },
            };
            _ = tx.send(event);
        });
    }
}

impl Bridge {
    /// Source-file metadata for the topbar. The library owns the provider
    /// bitrate/suffix; the UI never touches the database.
    pub fn request_track_info(&self, song_id: String) {
        if song_id.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let info = crate::library::song_file_info(&dbs.library, &song_id)
                .await
                .ok()
                .flatten()
                .map(|(size, suffix, bit_rate)| TrackInfo {
                    song_id: song_id.clone(),
                    suffix,
                    bit_rate,
                    size,
                })
                .unwrap_or_else(|| TrackInfo {
                    song_id,
                    ..TrackInfo::default()
                });
            _ = tx.send(Event::TrackInfo(info));
        });
    }
}
