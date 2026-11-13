//! Palette requests.

use super::*;

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
        let inflight_shared = Arc::clone(&self.palette_inflight);
        crate::app::runtime().spawn(async move {
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
            let mut inflight = inflight_shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inflight.as_deref() == Some(cover_art.as_str()) {
                *inflight = None;
            }
        });
    }
}
