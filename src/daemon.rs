//! Headless core daemon (daemon split phase 2).
//!
//! The daemon owns the databases, the native player, the backend bridge, the
//! external IPC socket, MPRIS and the tray. It never creates a GPUI window;
//! `musika-ui` will attach over a separate control socket in a later phase.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::bridge::{Bridge, Event, SyncMode};
use crate::ipc_command::{self, IpcCommand, RadioSeed, ShuffleTarget};
use crate::player::{Player, State};

/// Last page hint loaded from the status row; preserved when the daemon
/// flushes so a later UI session can reopen it.
#[derive(Debug, Clone, Default)]
struct LastPage {
    kind: String,
    id: String,
}

pub fn run() -> ! {
    if !crate::entry::init_backend() {
        std::process::exit(1);
    }
    crate::app::runtime().block_on(run_async());
    std::process::exit(0)
}

async fn run_async() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Event>();
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<String>();
    let bridge = Bridge::new(event_tx.clone());
    crate::ipc::serve(crate::ipc::socket_path(), move |line| {
        let _ = command_tx.send(line);
    });
    crate::tray::start();
    crate::mpris::init();

    let mut player = Player::new();
    let mut page = LastPage::default();
    restore_status(&mut player, &mut page).await;

    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_status = Instant::now();
    let mut quit = false;

    while !quit {
        tokio::select! {
            Some(event) = event_rx.recv() => apply_event(&mut player, &mut page, event),
            Some(line) = command_rx.recv() => {
                quit = handle_command(&mut player, &bridge, &line).await;
            }
            _ = ticker.tick() => {
                if let Some(error) = player.pump() {
                    eprintln!("daemon: playback error: {error}");
                }
                publish_mpris(&player);
                for line in crate::mpris::drain() {
                    quit = handle_command(&mut player, &bridge, &line).await || quit;
                }
                if last_status.elapsed() >= Duration::from_secs(2) {
                    flush_status(&player, &page).await;
                    last_status = Instant::now();
                }
            }
        }
    }

    flush_status(&player, &page).await;
    crate::tray::shutdown().await;
    let _ = std::fs::remove_file(crate::ipc::socket_path());
    eprintln!("daemon: stopped");
}

async fn restore_status(player: &mut Player, page: &mut LastPage) {
    let Some(dbs) = crate::app::dbs() else {
        return;
    };
    let Ok(Some(status)) = crate::status::load(&dbs.status).await else {
        return;
    };
    page.kind = status.page_kind.clone();
    page.id = status.page_id.clone();
    player.set_volume(status.volume);
    player.set_muted(status.muted);
    let Ok(payload) = crate::queue::resume_payload(&dbs, &status, false).await else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
        return;
    };
    let tracks = value["queue"]
        .as_str()
        .and_then(|json| serde_json::from_str::<Vec<crate::player::Track>>(json).ok())
        .unwrap_or_default();
    let start = value["start"].as_u64().unwrap_or(0) as usize;
    let position = value["position"].as_f64().unwrap_or(status.position);
    if !tracks.is_empty() {
        player.load_paused(tracks, start);
        if position > 0.0 {
            player.seek(position);
        }
    }
}

fn apply_event(player: &mut Player, page: &mut LastPage, event: Event) {
    match event {
        Event::Queue { tracks, start } => player.load(tracks, start),
        Event::QueueAppend { tracks } => player.append(tracks),
        Event::QueuePaste { tracks, at } => player.insert_many(at, tracks),
        Event::Restore {
            volume,
            muted,
            tracks,
            start,
            position,
            ..
        } => {
            player.set_volume(volume);
            player.set_muted(muted);
            if !tracks.is_empty() {
                player.load_paused(tracks, start);
                if position > 0.0 {
                    player.seek(position);
                }
            }
        }
        Event::LoginRequired { .. } | Event::LoggedOut => {
            player.load_paused(Vec::new(), 0);
        }
        Event::AccountRemoved { .. } => {
            player.load_paused(Vec::new(), 0);
        }
        Event::OpenPage { descriptor } => {
            page.kind = match descriptor.kind {
                crate::model::PageKind::Album => "album",
                crate::model::PageKind::Artist => "artist",
                crate::model::PageKind::Playlist => "playlist",
                crate::model::PageKind::Radio => "radio",
            }
            .to_string();
            page.id = descriptor.id;
        }
        Event::OpenPageReveal { descriptor, .. } => {
            page.kind = match descriptor.kind {
                crate::model::PageKind::Album => "album",
                crate::model::PageKind::Artist => "artist",
                crate::model::PageKind::Playlist => "playlist",
                crate::model::PageKind::Radio => "radio",
            }
            .to_string();
            page.id = descriptor.id;
        }
        Event::Error(message) => eprintln!("daemon: backend error: {message}"),
        Event::Header { server_url, .. } => {
            crate::debug_log!("daemon: header for {server_url}");
        }
        _ => {}
    }
}

/// Returns true when the daemon should quit.
async fn handle_command(player: &mut Player, bridge: &Bridge, line: &str) -> bool {
    let command = match ipc_command::parse(line) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("daemon: ignored command {line:?}: {error}");
            return false;
        }
    };
    match command {
        IpcCommand::Quit => return true,
        IpcCommand::Show => eprintln!("daemon: show requested before the UI split is wired"),
        IpcCommand::PlayPause => player.toggle(),
        IpcCommand::Play => player.resume(),
        IpcCommand::Pause => player.pause(),
        IpcCommand::Stop => player.stop(),
        IpcCommand::NextTrack => player.next(),
        IpcCommand::PrevTrack => player.previous(),
        IpcCommand::Mute => player.toggle_mute(),
        IpcCommand::VolumeUp => player.set_volume(player.volume() + 5.0),
        IpcCommand::VolumeDown => player.set_volume(player.volume() - 5.0),
        IpcCommand::SetVolume(volume) => player.set_volume(volume),
        IpcCommand::Seek(seconds) => player.seek(seconds),
        IpcCommand::SeekBack => player.seek(player.position() - 5.0),
        IpcCommand::SeekForward => player.seek(player.position() + 5.0),
        IpcCommand::SeekRelative(delta) => player.seek(player.position() + delta),
        IpcCommand::RepeatCycle => {
            player.cycle_repeat();
        }
        IpcCommand::Shuffle {
            target: ShuffleTarget::Queue,
        } => player.shuffle_queue(),
        IpcCommand::Shuffle {
            target: ShuffleTarget::Album(Some(id)),
        } => bridge.shuffle_album(id),
        IpcCommand::Shuffle {
            target: ShuffleTarget::Artist(Some(id)),
        } => bridge.shuffle_artist(id),
        IpcCommand::Shuffle { target } => {
            eprintln!("daemon: shuffle target needs an id in daemon mode: {target:?}");
        }
        IpcCommand::Radio { seed } => {
            let (kind, id) = match seed {
                RadioSeed::Current | RadioSeed::Song(None) => match player.current() {
                    Some(track) => ("song".to_string(), track.id.clone()),
                    None => {
                        eprintln!("daemon: radio current needs a playing track");
                        return false;
                    }
                },
                RadioSeed::Song(Some(id)) => ("song".to_string(), id),
                RadioSeed::Album(Some(id)) => ("album".to_string(), id),
                RadioSeed::Album(None) => match player.current().and_then(|track| {
                    (!track.album_id.is_empty()).then(|| track.album_id.clone())
                }) {
                    Some(id) => ("album".to_string(), id),
                    None => {
                        eprintln!("daemon: radio album needs a playing album");
                        return false;
                    }
                },
            };
            bridge.play_radio(kind, id, 0);
        }
        IpcCommand::RandomAlbum => bridge.play_random_album(),
        IpcCommand::Sync(mode) => bridge.sync(mode),
        IpcCommand::Refresh => bridge.sync(SyncMode::Incremental),
        IpcCommand::PlaySong(id) => bridge.play_song(id, String::new()),
        _ => {}
    }
    false
}

fn publish_mpris(player: &Player) {
    let current = player.current();
    let status = match player.state() {
        State::Playing => "Playing",
        State::Paused => "Paused",
        State::Stopped => "Stopped",
    };
    crate::mpris::publish(crate::mpris::Snapshot {
        playback_status: status.to_string(),
        title: current.map(|track| track.title.clone()).unwrap_or_default(),
        artist: current.map(|track| track.artist.clone()).unwrap_or_default(),
        album: current.map(|track| track.album.clone()).unwrap_or_default(),
        track_id: current.map(|track| track.id.clone()).unwrap_or_default(),
        duration_us: (player.duration() * 1_000_000.0) as i64,
        position_us: (player.position() * 1_000_000.0) as i64,
        volume: player.volume() / 100.0,
        can_next: player.len() > 0,
        can_prev: player.len() > 0,
        art_url: current.map(|track| track.cover_url.clone()).unwrap_or_default(),
    });
}

async fn flush_status(player: &Player, page: &LastPage) {
    let Some(dbs) = crate::app::dbs() else {
        return;
    };
    let account = crate::accounts::active_account();
    let queue: Vec<&str> = player.queue().iter().map(|track| track.id.as_str()).collect();
    let queue = serde_json::to_string(&queue).unwrap_or_default();
    let current = player.current();
    let status = crate::status::SessionStatus {
        server_url: account
            .as_ref()
            .map(|account| account.server_url.clone())
            .unwrap_or_default(),
        username: account
            .as_ref()
            .map(|account| account.username.clone())
            .unwrap_or_default(),
        album_id: current.and_then(|track| {
            (!track.album_id.is_empty()).then(|| track.album_id.clone())
        }),
        song_id: current.map(|track| track.id.clone()),
        page_kind: page.kind.clone(),
        page_id: page.id.clone(),
        queue,
        queue_index: player.index() as i64,
        position: player.position(),
        volume: player.volume(),
        muted: player.muted(),
        updated_at: crate::consts::now_secs(),
    };
    if let Err(error) = crate::status::save(&dbs.status, &status).await {
        eprintln!("daemon: status flush failed: {error:#}");
    }
}
