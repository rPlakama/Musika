//! Sync, wipe and container-maintenance requests.

use std::sync::PoisonError;

use tokio::sync::watch;

use super::*;

/// One queued library sync.
///
/// The daemon serialises runs: a request while one is live is folded into the
/// next pass, and a pass interrupted by an offline transition keeps its mode.
/// `generation` invalidates a pass that was running when the account changed,
/// so an old result can neither be re-queued nor announced.
#[derive(Debug, Default)]
struct SyncQueueState {
    running: bool,
    pending: Option<SyncMode>,
    generation: u64,
}

#[derive(Debug)]
pub(crate) struct SyncQueue {
    state: Mutex<SyncQueueState>,
}

impl SyncQueue {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(SyncQueueState::default()),
        }
    }

    /// Records `mode`. Returns `true` when the caller must spawn
    /// [`drive_syncs`] to own the queue.
    pub(crate) fn request(&self, mode: SyncMode) -> bool {
        let mut state = self.lock();
        state.pending = Some(merge_modes(state.pending, mode));
        if state.running {
            false
        } else {
            state.running = true;
            true
        }
    }

    /// Claims the next pass. When none is pending, releases the supervisor
    /// slot in the same lock so a concurrent request is never stranded.
    pub(crate) fn next(&self) -> Option<(SyncMode, u64)> {
        let mut state = self.lock();
        match state.pending.take() {
            Some(mode) => Some((mode, state.generation)),
            None => {
                state.running = false;
                None
            }
        }
    }

    /// Puts an interrupted pass back. The supervisor slot stays held, so the
    /// same driver resumes it once the server is reachable again.
    fn requeue(&self, mode: SyncMode, generation: u64) {
        let mut state = self.lock();
        if state.generation == generation {
            state.pending = Some(merge_modes(state.pending, mode));
        }
    }

    /// Drops queued work and invalidates a live pass (account switch/logout).
    /// The session watch cancels the run; its result is discarded.
    pub(crate) fn clear(&self) {
        let mut state = self.lock();
        state.pending = None;
        state.generation = state.generation.wrapping_add(1);
    }

    /// Ends the supervisor after a terminal failure or a lost session watch.
    /// The slot, queue and generation move under one lock so a request racing
    /// the failure either starts a fresh driver or is left for `next`.
    pub(crate) fn finish(&self) {
        let mut state = self.lock();
        state.pending = None;
        state.running = false;
        state.generation = state.generation.wrapping_add(1);
    }

    /// Whether `generation` still belongs to the current account/session.
    fn is_current(&self, generation: u64) -> bool {
        self.lock().generation == generation
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SyncQueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn mode_rank(mode: SyncMode) -> u8 {
    match mode {
        SyncMode::Incremental => 0,
        SyncMode::All => 1,
        SyncMode::Repair => 2,
    }
}

/// A request never downgrades a pass that is already queued: an automatic
/// incremental ping must not replace a user's `sync all` or `sync repair`.
fn merge_modes(pending: Option<SyncMode>, mode: SyncMode) -> SyncMode {
    match pending {
        Some(pending) if mode_rank(pending) >= mode_rank(mode) => pending,
        _ => mode,
    }
}

impl Bridge {
    /// Run an incremental, deep, or clean-rebuild library sync.
    ///
    /// The request is queued rather than spawned directly. The driver pauses
    /// a live pass when the session goes offline and replays the same mode
    /// when it returns; a ping-triggered pass is always incremental.
    pub fn sync(&self, mode: SyncMode) {
        if !crate::session::online() {
            _ = self.tx.send(Event::SyncPaused);
        }
        if !self.sync.request(mode) {
            return;
        }
        let queue = Arc::clone(&self.sync);
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let online = crate::session::watch_online();
            let run_tx = tx.clone();
            drive_syncs(queue, tx, online, move |mode| {
                run_sync_once(run_tx.clone(), mode)
            })
            .await;
        });
    }
}

/// Runs queued syncs until the queue drains.
///
/// `online` is the session's connectivity feed. A transition to offline drops
/// the live pass (which aborts its fetch tasks and rolls back an in-flight
/// transaction) and parks the mode; the next online transition claims it
/// again. A failure while the feed still reports online is a real error and
/// is surfaced once.
pub(crate) async fn drive_syncs<F, Fut>(
    queue: Arc<SyncQueue>,
    tx: UnboundedSender<Event>,
    mut online: watch::Receiver<bool>,
    run: F,
) where
    F: Fn(SyncMode) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    loop {
        if !*online.borrow() {
            // The pass is parked, not failed. The UI stops its spinner and
            // resumes when the first progress event follows reconnect.
            _ = tx.send(Event::SyncPaused);
        }
        if !wait_until_online(&mut online).await {
            queue.finish();
            break;
        }
        let Some((mode, generation)) = queue.next() else {
            break;
        };
        let result = tokio::select! {
            result = run(mode) => result,
            _ = online.wait_for(|online| !*online) => {
                // Dropping the run future cancels its network fetches; an
                // in-flight SQLite transaction rolls back on drop. Nothing
                // partial is committed, so the whole pass is replayed.
                queue.requeue(mode, generation);
                continue;
            }
        };
        if !queue.is_current(generation) {
            // The session changed under the pass; discard its result instead
            // of announcing a sync for the previous account.
            continue;
        }
        match result {
            Ok(()) => {
                _ = tx.send(Event::LibrarySynced);
            }
            Err(error) => {
                crate::linux::malloc_trim();
                if !*online.borrow() {
                    queue.requeue(mode, generation);
                    continue;
                }
                _ = tx.send(Event::Error(error.to_string()));
                queue.finish();
                break;
            }
        }
    }
}

async fn wait_until_online(online: &mut watch::Receiver<bool>) -> bool {
    if *online.borrow() {
        return true;
    }
    online.wait_for(|online| *online).await.is_ok()
}

async fn run_sync_once(tx: UnboundedSender<Event>, mode: SyncMode) -> anyhow::Result<()> {
    let Some(dbs) = crate::app::dbs() else {
        return Err(anyhow::anyhow!("Sync needs a live session."));
    };
    let Ok(client) = crate::session::client(&dbs).await else {
        return Err(anyhow::anyhow!("Sync needs a live session."));
    };
    let client = Arc::new(client);
    if mode == SyncMode::Repair {
        _ = tx.send(Event::SyncProgress {
            message: "Repairing library…".to_string(),
            done: 0,
            total: 100,
        });
        if let Err(err) = crate::db::wipe_library(&dbs).await {
            return Err(anyhow::anyhow!("Library repair failed: {err:#}"));
        }
    }
    let deep = mode != SyncMode::Incremental;
    let progress_tx = tx.clone();
    let report = crate::index::sync_library(&dbs, client.clone(), deep, |progress| {
        _ = progress_tx.send(Event::SyncProgress {
            message: progress.message,
            done: progress.done,
            total: progress.total,
        });
    })
    .await
    .map_err(|err| anyhow::anyhow!("Library sync failed: {err:#}"))?;
    if mode != SyncMode::Incremental && report.covers_due {
        _ = tx.send(Event::SyncProgress {
            message: "Refreshing covers…".to_string(),
            done: 90,
            total: 100,
        });
        crate::index::cache_covers(&dbs, &client, &report.forced_covers).await;
    }
    // A sync holds large JSON bodies and cover buffers transiently; return
    // the freed pages now instead of keeping the high-water mark resident
    // for the daemon's whole lifetime.
    crate::linux::malloc_trim();
    Ok(())
}

impl Bridge {
    /// Wipe a maintenance scope. Library/index/cover scopes are independent.
    pub fn wipe(&self, scope: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let result = match scope.as_str() {
                "library" => crate::db::wipe_library(&dbs).await,
                "indexes" => crate::db::wipe_indexes(&dbs).await,
                "covers" => crate::db::wipe_covers(&dbs).await,
                "lyrics" => crate::db::wipe_lyrics(&dbs).await,
                "album" => crate::media_actions::wipe_albums(&dbs).await,
                other => Err(anyhow::anyhow!("unknown wipe scope: {other}")),
            };
            match result {
                Ok(()) => {
                    _ = tx.send(Event::Wiped { scope });
                }
                Err(err) => {
                    _ = tx.send(Event::Error(format!("Wipe failed: {err:#}")));
                }
            }
            crate::linux::malloc_trim();
        });
    }

    /// Download coverage + favourite/rating for an Album or Artist page.
    pub fn request_container_state(&self, kind: crate::model::PageKind, id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let (cached, starred, rating) = match kind {
                crate::model::PageKind::Album => {
                    let cached =
                        crate::download::window_state(&dbs, &[], std::slice::from_ref(&id))
                            .await
                            .map(|marks| marks.albums_done.iter().any(|album| album == &id))
                            .unwrap_or(false);
                    let state =
                        crate::library::album_states(&dbs.library, std::slice::from_ref(&id))
                            .await
                            .ok()
                            .and_then(|states| states.get(&id).copied())
                            .unwrap_or((false, 0));
                    (cached, state.0, state.1.clamp(0, 5) as u8)
                }
                crate::model::PageKind::Artist => {
                    let albums = crate::library::artist_album_ids(&dbs.library, &id)
                        .await
                        .unwrap_or_default();
                    let cached = if albums.is_empty() {
                        false
                    } else {
                        crate::download::window_state(&dbs, &[], &albums)
                            .await
                            .map(|marks| marks.albums_done.len() == albums.len())
                            .unwrap_or(false)
                    };
                    let user = crate::artist_user::get(&dbs.artist_user, &id)
                        .await
                        .ok()
                        .flatten();
                    let (starred, rating) = user
                        .map(|user| (user.starred, user.rating.clamp(0, 5) as u8))
                        .unwrap_or((false, 0));
                    (cached, starred, rating)
                }
                crate::model::PageKind::Playlist | crate::model::PageKind::Radio => {
                    (false, false, 0)
                }
            };
            _ = tx.send(Event::PageContainerState {
                kind,
                id,
                cached,
                starred,
                rating,
            });
        });
    }

    /// Download or remove the whole artist container.
    pub fn download_artist(&self, artist_id: String, on: bool) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            _ = tx.send(Event::DownloadStarted {
                kind: "artist".to_string(),
                id: artist_id.clone(),
                on,
            });
            let ids = crate::library::artist_song_ids(&dbs.library, &artist_id)
                .await
                .unwrap_or_default();
            if ids.is_empty() {
                _ = tx.send(Event::Error(
                    "This artist has no songs to download.".to_string(),
                ));
                return;
            }
            if on {
                for song in &ids {
                    if let Err(err) = crate::media_actions::download_one(&dbs, song).await {
                        _ = tx.send(Event::Error(format!("Download failed: {err:#}")));
                    }
                }
            } else {
                for song in &ids {
                    _ = crate::download::unmark(&dbs.download, song).await;
                }
                if let Err(err) = crate::media::remove_songs_media(&dbs.media, &ids).await {
                    _ = tx.send(Event::Error(format!(
                        "Could not remove the artist media: {err:#}"
                    )));
                }
            }
            send_download_settled(&tx);
        });
    }

    pub fn set_offline(&self, offline: bool) {
        let _ = crate::session::set_online(!offline);
    }

    pub(crate) fn offline(&self) -> bool {
        !crate::session::online()
    }

    /// UI request: retarget the output stream (or follow the default).
    pub fn set_output_device(&self, device: Option<String>, follow_default: bool) {
        self.send_playback(crate::daemon::playback::PlaybackInput::SetOutputDevice {
            device,
            follow_default,
        });
    }

    /// UI request: enumerate the PipeWire sinks. Discovery blocks on a short
    /// private PipeWire connection, so it runs on the blocking pool.
    pub fn request_output_devices(&self) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let result = tokio::task::spawn_blocking(crate::player::pipewire::list_output_devices)
                .await
                .unwrap_or_else(|error| Err(format!("output discovery task: {error}")));
            match result {
                Ok(devices) => {
                    _ = tx.send(Event::OutputDevices { devices });
                }
                Err(error) => {
                    _ = tx.send(Event::Error(format!("Could not list outputs: {error}")));
                }
            }
        });
    }

    /// UI request: change one stream-quality/cache field.
    pub fn set_stream_change(&self, change: crate::playback_settings::StreamChange) {
        self.send_playback(crate::daemon::playback::PlaybackInput::ApplyStreamChange(change));
    }

    pub fn set_color_contrast(&self, contrast: f64) {
        self.send_playback(crate::daemon::playback::PlaybackInput::SetColorContrast(contrast));
    }

    pub fn set_color_saturation(&self, saturation: f64) {
        self.send_playback(crate::daemon::playback::PlaybackInput::SetColorSaturation(
            saturation,
        ));
    }

    /// UI request: arm/cancel the sleep timer.
    pub fn set_sleep_timer(&self, minutes: Option<u64>) {
        self.send_playback(crate::daemon::playback::PlaybackInput::SetSleepTimer(minutes));
    }

    /// UI request: enable/disable scrobbling.
    pub fn set_scrobble(&self, enabled: bool) {
        self.send_playback(crate::daemon::playback::PlaybackInput::SetScrobble(enabled));
    }

    /// UI request: cycle the EQ profile through OFF.
    pub fn cycle_eq(&self) {
        self.send_playback(crate::daemon::playback::PlaybackInput::CycleEq);
    }

    pub fn eq_changed(&self, name: Option<String>) {
        _ = self.tx.send(Event::EqChanged(name));
    }

    pub fn settings_changed(&self, settings: crate::playback_settings::PlaybackSettings) {
        _ = self.tx.send(Event::Settings(Box::new(settings)));
    }

    /// UI request: change one crossfade field.
    pub fn set_crossfade_change(&self, change: crate::playback_settings::CrossfadeChange) {
        self.send_playback(crate::daemon::playback::PlaybackInput::SetCrossfadeConfig(change));
    }

    /// UI request: change exclusive/bit-perfect output mode.
    pub fn set_output_mode(&self, exclusive: Option<bool>, bit_perfect: Option<bool>) {
        self.send_playback(crate::daemon::playback::PlaybackInput::SetOutputMode {
            exclusive,
            bit_perfect,
        });
    }

    /// UI request: select the backend/ALSA device.
    pub fn set_output_backend(
        &self,
        backend: Option<crate::playback_settings::OutputBackend>,
        device: Option<String>,
    ) {
        self.send_playback(
            crate::daemon::playback::PlaybackInput::SetOutputBackend { backend, device },
        );
    }

    pub(crate) fn is_offline(&self) -> bool {
        self.offline()
    }

    /// Applies a connectivity verdict from the background health probe.
    ///
    /// The bridge flag and the session flag move together: list loaders read
    /// the former, the cover/network paths read the latter. Only a transition
    /// reaches the UI, and it is deliberately toast-free — the 5 s probe must
    /// never spam the user.
    pub(crate) fn note_connection(&self, online: bool) {
        let was_online = crate::session::set_online(online);
        if was_online != online {
            _ = self.tx.send(Event::Connection { online });
            if online {
                self.refresh_header();
            }
        }
    }
}
