//! Palette and display-cover requests.

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
            let _ = tx.send(event);
            let mut inflight = inflight_shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inflight.as_deref() == Some(cover_art.as_str()) {
                *inflight = None;
            }
        });
    }

    pub fn request_display_cover(&self, key: String, cover_art: String, target: u32) {
        if cover_art.is_empty() || target == 0 {
            return;
        }
        let inflight = Arc::clone(&self.display_inflight);
        {
            let mut keys = inflight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !keys.insert(key.clone()) {
                return;
            }
        }
        let tx = self.tx.clone();
        let started = std::time::Instant::now();
        crate::debug_log!(
            "covers: display request cover={} target={} key={} at={}",
            cover_art,
            target,
            key,
            unix_ms_for_cover()
        );
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else {
                inflight
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&key);
                return;
            };
            let source = match crate::covers::cached_path(&dbs.covers, &cover_art).await {
                Ok(Some(source)) => source,
                _ => {
                    crate::debug_log!(
                        "covers: display failed cover={} target={} reason=no-source key={}",
                        cover_art,
                        target,
                        key
                    );
                    let _ = tx.send(Event::DisplayCover {
                        key: key.clone(),
                        cover_art: cover_art.clone(),
                        target,
                        path: None,
                    });
                    inflight
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&key);
                    return;
                }
            };
            let path = crate::covers::get_or_create_resized_cover(&source, target).await;
            crate::debug_log!(
                "covers: display ready cover={} target={} path={:?} elapsed_ms={:.1}",
                cover_art,
                target,
                path,
                started.elapsed().as_secs_f64() * 1000.0
            );
            let _ = tx.send(Event::DisplayCover {
                key: key.clone(),
                cover_art: cover_art.clone(),
                target,
                path,
            });
            inflight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
        });
    }
}
