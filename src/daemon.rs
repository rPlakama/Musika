//! Headless core daemon (daemon split phase 2).
//!
//! The daemon owns the databases, the native player, the backend bridge, the
//! external IPC socket, MPRIS and the tray. It never creates a GPUI window;
//! `musika-ui` will attach over a separate control socket in a later phase.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::bridge::{Bridge, Event, SyncMode};
use crate::ipc_command::{
    self, CurrentTrack, DownloadRequest, IpcCommand, RadioSeed, ShuffleTarget, SuggestScope,
};
use crate::model::{PageDescriptor, PageKind, Scope};
use crate::player::{Player, State};
use crate::protocol::{
    CoreMessage, PlayerCommand, PlayerSnapshot, PlayerTick, RepeatWire, StateWire, UiMessage,
    VERSION,
};

/// Last page hint loaded from the status row; preserved when the daemon
/// flushes so a later UI session can reopen it.
#[derive(Debug, Clone, Default)]
struct LastPage {
    kind: String,
    id: String,
}

struct UiIncoming {
    conn: u64,
    message: UiMessage,
}

enum UiStateEvent {
    Connected(u64, mpsc::UnboundedSender<CoreMessage>),
    Gone(u64),
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
    let (ui_in_tx, mut ui_in_rx) = mpsc::unbounded_channel::<UiIncoming>();
    let (ui_state_tx, mut ui_state_rx) = mpsc::unbounded_channel::<UiStateEvent>();
    tokio::spawn(serve_ui(
        crate::ipc::daemon_socket_path(),
        ui_in_tx,
        ui_state_tx,
    ));
    crate::tray::start();
    crate::mpris::init();

    let mut player = Player::new();
    let mut page = LastPage::default();
    let mut header: Option<Event> = None;
    let mut ui_writer: Option<mpsc::UnboundedSender<CoreMessage>> = None;
    let mut ui_conn: u64 = 0;
    let mut ui_child: Option<std::process::Child> = None;
    let mut last_revision = player.revision();
    restore_status(&mut player, &mut page).await;

    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_status = Instant::now();
    let mut quit = false;

    while !quit {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                if matches!(event, Event::Header { .. }) {
                    header = Some(event.clone());
                }
                let forward = !matches!(
                    event,
                    Event::Queue { .. }
                        | Event::QueueAppend { .. }
                        | Event::QueuePaste { .. }
                        | Event::Restore { .. }
                        | Event::LoginRequired { .. }
                        | Event::LoggedOut
                        | Event::AccountRemoved { .. }
                );
                apply_event(&mut player, &mut page, event.clone());
                if forward
                    && let Some(writer) = ui_writer.as_ref()
                {
                    let _ = writer.send(CoreMessage::Bridge(event));
                }
            }
            Some(line) = command_rx.recv() => {
                if line.trim() == "show" {
                    if ui_writer.is_some() {
                        send_ui(&ui_writer, CoreMessage::Activate);
                    } else {
                        ensure_ui_child(&mut ui_child);
                    }
                } else {
                    quit = handle_command(&mut player, &bridge, &line).await;
                }
            }
            Some(incoming) = ui_in_rx.recv() => {
                if incoming.conn == ui_conn {
                    match incoming.message {
                        UiMessage::Hello { version } => {
                            eprintln!("daemon: UI hello v{version}");
                            let messages = bootstrap_messages(version, header.as_ref(), &player, &page);
                            for message in messages {
                                send_ui(&ui_writer, message);
                            }
                        }
                        UiMessage::Show => {
                            if ui_writer.is_some() {
                                send_ui(&ui_writer, CoreMessage::Activate);
                            } else {
                                ensure_ui_child(&mut ui_child);
                            }
                        }
                        UiMessage::Shutdown => {
                            send_ui(&ui_writer, CoreMessage::Shutdown);
                            quit = true;
                        }
                        UiMessage::Ping => send_ui(&ui_writer, CoreMessage::Ready),
                        UiMessage::Bridge { method, args } => dispatch_bridge(&bridge, &method, &args),
                        UiMessage::Player(command) => apply_player_command(&mut player, command),
                    }
                }
            }
            Some(state) = ui_state_rx.recv() => match state {
                UiStateEvent::Connected(conn, writer) => {
                    ui_conn = conn;
                    ui_writer = Some(writer);
                    eprintln!("daemon: UI connected");
                }
                UiStateEvent::Gone(conn) => {
                    if conn == ui_conn {
                        ui_writer = None;
                        ui_conn = 0;
                        eprintln!("daemon: UI disconnected");
                    }
                }
            },
            _ = ticker.tick() => {
                if let Some(error) = player.pump() {
                    eprintln!("daemon: playback error: {error}");
                }
                publish_mpris(&player);
                for line in crate::mpris::drain() {
                    quit = handle_command(&mut player, &bridge, &line).await || quit;
                }
                if ui_writer.is_some() {
                    if player.revision() != last_revision {
                        last_revision = player.revision();
                        send_ui(&ui_writer, CoreMessage::PlayerFull(snapshot(&player)));
                    }
                    send_ui(&ui_writer, CoreMessage::PlayerTick(tick(&player)));
                }
                if let Some(child) = ui_child.as_mut()
                    && child.try_wait().ok().flatten().is_some()
                {
                    ui_child = None;
                }
                if last_status.elapsed() >= Duration::from_secs(2) {
                    flush_status(&player, &page).await;
                    last_status = Instant::now();
                }
            }
        }
    }

    send_ui(&ui_writer, CoreMessage::Shutdown);
    flush_status(&player, &page).await;
    crate::tray::shutdown().await;
    if let Some(mut child) = ui_child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_file(crate::ipc::socket_path());
    let _ = std::fs::remove_file(crate::ipc::daemon_socket_path());
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

fn send_ui(writer: &Option<mpsc::UnboundedSender<CoreMessage>>, message: CoreMessage) {
    if let Some(writer) = writer {
        let _ = writer.send(message);
    }
}

fn bootstrap_messages(
    version: u32,
    header: Option<&Event>,
    player: &Player,
    page: &LastPage,
) -> Vec<CoreMessage> {
    let mut messages = Vec::new();
    if version != VERSION {
        messages.push(CoreMessage::Error(format!(
            "protocol version {version} does not match core {VERSION}"
        )));
    }
    messages.push(CoreMessage::Ready);
    if let Some(header) = header {
        messages.push(CoreMessage::Bridge(header.clone()));
    }
    messages.push(CoreMessage::PlayerFull(snapshot(player)));
    let page_kind = match page.kind.as_str() {
        "album" => Some(PageKind::Album),
        "artist" => Some(PageKind::Artist),
        "playlist" => Some(PageKind::Playlist),
        "radio" => Some(PageKind::Radio),
        _ => None,
    };
    if let Some(kind) = page_kind
        && !page.id.is_empty()
    {
        messages.push(CoreMessage::Bridge(Event::OpenPage {
            descriptor: PageDescriptor::new(kind, page.id.clone()),
        }));
    }
    messages
}

fn snapshot(player: &Player) -> PlayerSnapshot {
    PlayerSnapshot {
        queue: player.queue().to_vec(),
        index: player.index(),
        state: StateWire::from(player.state()),
        repeat: RepeatWire::from(player.repeat()),
        position: player.position(),
        duration: player.duration(),
        volume: player.volume(),
        muted: player.muted(),
        loading: player.is_loading(),
    }
}

fn tick(player: &Player) -> PlayerTick {
    PlayerTick {
        index: player.index(),
        state: StateWire::from(player.state()),
        repeat: RepeatWire::from(player.repeat()),
        position: player.position(),
        duration: player.duration(),
        volume: player.volume(),
        muted: player.muted(),
        loading: player.is_loading(),
    }
}

fn apply_player_command(player: &mut Player, command: PlayerCommand) {
    match command {
        PlayerCommand::PlayPause => player.toggle(),
        PlayerCommand::Play => player.resume(),
        PlayerCommand::Pause => player.pause(),
        PlayerCommand::Stop => player.stop(),
        PlayerCommand::Next => player.next(),
        PlayerCommand::Previous => player.previous(),
        PlayerCommand::SetVolume { volume } => player.set_volume(volume),
        PlayerCommand::ToggleMute => player.toggle_mute(),
        PlayerCommand::SetMuted { muted } => player.set_muted(muted),
        PlayerCommand::Seek { seconds } => player.seek(seconds),
        PlayerCommand::Jump { index } => player.jump(index),
        PlayerCommand::Load { tracks, start } => player.load(tracks, start),
        PlayerCommand::LoadPaused { tracks, start } => player.load_paused(tracks, start),
        PlayerCommand::Append { tracks } => player.append(tracks),
        PlayerCommand::InsertMany { at, tracks } => player.insert_many(at, tracks),
        PlayerCommand::Remove { index } => player.remove(index),
        PlayerCommand::RemoveMany { indices } => {
            player.remove_many(&indices);
        }
        PlayerCommand::Move { from, to } => player.move_item(from, to),
        PlayerCommand::MoveBlock { start, len, delta } => {
            player.move_block(start, len, delta);
        }
        PlayerCommand::ShuffleQueue => player.shuffle_queue(),
        PlayerCommand::CycleRepeat => {
            player.cycle_repeat();
        }
        PlayerCommand::SetTrackFavorite { id, favorite } => {
            player.set_track_favorite(&id, favorite);
        }
    }
}

fn arg<T: DeserializeOwned>(args: &serde_json::Value, key: &str) -> Option<T> {
    args.get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn dispatch_bridge(bridge: &Bridge, method: &str, args: &serde_json::Value) {
    match method {
        "refresh_header" => bridge.refresh_header(),
        "load_library" => {
            if let (Some(scope), Some(order)) = (arg::<Scope>(args, "scope"), arg::<usize>(args, "order")) {
                let _ = bridge.load_library(scope, order);
            }
        }
        "request_library_window" => {
            if let (Some(generation), Some(scope), Some(order), Some(start), Some(count)) = (
                arg(args, "generation"),
                arg::<Scope>(args, "scope"),
                arg(args, "order"),
                arg(args, "start"),
                arg(args, "count"),
            ) {
                bridge.request_library_window(generation, scope, order, start, count);
            }
        }
        "preload_library" => {
            if let (Some(scope), Some(order)) = (arg::<Scope>(args, "scope"), arg(args, "order")) {
                bridge.preload_library(scope, order);
            }
        }
        "search" => {
            if let Some(query) = arg(args, "query") {
                bridge.search(query);
            }
        }
        "request_command_suggestions" => {
            if let (Some(generation), Some(scope), Some(query), Some(start), Some(end)) = (
                arg(args, "generation"),
                arg::<SuggestScope>(args, "scope"),
                arg(args, "query"),
                arg(args, "start"),
                arg(args, "end"),
            ) {
                bridge.request_command_suggestions(generation, scope, query, start, end);
            }
        }
        "load_page" => {
            if let Some(descriptor) = arg::<PageDescriptor>(args, "descriptor") {
                bridge.load_page(descriptor);
            }
        }
        "request_notes" => {
            if let Some(album_id) = arg(args, "album_id") {
                bridge.request_notes(album_id);
            }
        }
        "request_artist_notes" => {
            if let Some(artist_id) = arg(args, "artist_id") {
                bridge.request_artist_notes(artist_id);
            }
        }
        "request_lyrics" => {
            if let Some(song_id) = arg(args, "song_id") {
                bridge.request_lyrics(song_id);
            }
        }
        "request_marks" => {
            if let (Some(songs), Some(albums)) = (arg(args, "songs"), arg(args, "albums")) {
                bridge.request_marks(songs, albums);
            }
        }
        "request_container_state" => {
            if let (Some(kind), Some(id)) = (arg::<PageKind>(args, "kind"), arg(args, "id")) {
                bridge.request_container_state(kind, id);
            }
        }
        "request_colors" => {
            if let Some(cover_art) = arg(args, "cover_art") {
                bridge.request_colors(cover_art);
            }
        }
        "request_display_cover" => {
            if let (Some(key), Some(cover_art), Some(target)) = (
                arg(args, "key"),
                arg(args, "cover_art"),
                arg(args, "target"),
            ) {
                bridge.request_display_cover(key, cover_art, target);
            }
        }
        "restore" => bridge.restore(),
        "play_song" => {
            if let (Some(song_id), Some(order)) = (arg(args, "song_id"), arg(args, "order")) {
                bridge.play_song(song_id, order);
            }
        }
        "play_song_in_album" => {
            if let Some(song_id) = arg(args, "song_id") {
                bridge.play_song_in_album(song_id);
            }
        }
        "play_album" => {
            if let (Some(album_id), Some(start)) = (arg(args, "album_id"), arg(args, "start")) {
                bridge.play_album(album_id, start);
            }
        }
        "play_random_album" => bridge.play_random_album(),
        "play_radio" => {
            if let (Some(kind), Some(id), Some(start)) =
                (arg(args, "kind"), arg(args, "id"), arg(args, "start"))
            {
                bridge.play_radio(kind, id, start);
            }
        }
        "play_radio_query" => {
            if let (Some(kind), Some(query), Some(start)) =
                (arg(args, "kind"), arg(args, "query"), arg(args, "start"))
            {
                bridge.play_radio_query(kind, query, start);
            }
        }
        "quick_radio" => {
            if let (Some(kind), Some(id)) = (arg(args, "kind"), arg(args, "id")) {
                bridge.quick_radio(kind, id);
            }
        }
        "shuffle_album" => {
            if let Some(album_id) = arg(args, "album_id") {
                bridge.shuffle_album(album_id);
            }
        }
        "shuffle_artist" => {
            if let Some(artist_id) = arg(args, "artist_id") {
                bridge.shuffle_artist(artist_id);
            }
        }
        "shuffle_album_query" => {
            if let Some(query) = arg(args, "query") {
                bridge.shuffle_album_query(query);
            }
        }
        "shuffle_artist_query" => {
            if let Some(query) = arg(args, "query") {
                bridge.shuffle_artist_query(query);
            }
        }
        "shuffle_radio" => {
            if let (Some(seed_kind), Some(seed_id)) =
                (arg(args, "seed_kind"), arg(args, "seed_id"))
            {
                bridge.shuffle_radio(seed_kind, seed_id);
            }
        }
        "queue_add_radio" => {
            if let (Some(seed_kind), Some(seed_id)) =
                (arg(args, "seed_kind"), arg(args, "seed_id"))
            {
                bridge.queue_add_radio(seed_kind, seed_id);
            }
        }
        "save_radio" => {
            if let (Some(seed_kind), Some(seed_id), Some(name)) =
                (arg(args, "seed_kind"), arg(args, "seed_id"), arg(args, "name"))
            {
                bridge.save_radio(seed_kind, seed_id, name);
            }
        }
        "save_radio_local" => {
            if let (Some(seed_kind), Some(seed_id), Some(name)) =
                (arg(args, "seed_kind"), arg(args, "seed_id"), arg(args, "name"))
            {
                bridge.save_radio_local(seed_kind, seed_id, name);
            }
        }
        "download_radio" => {
            if let (Some(seed_kind), Some(seed_id)) =
                (arg(args, "seed_kind"), arg(args, "seed_id"))
            {
                bridge.download_radio(seed_kind, seed_id);
            }
        }
        "quick_queue_add" => {
            if let (Some(kind), Some(id)) = (arg(args, "kind"), arg(args, "id")) {
                bridge.quick_queue_add(kind, id);
            }
        }
        "paste_queue" => {
            if let (Some(kind), Some(ids), Some(at)) =
                (arg(args, "kind"), arg(args, "ids"), arg(args, "at"))
            {
                bridge.paste_queue(kind, ids, at);
            }
        }
        "quick_favorite" => {
            if let (Some(kind), Some(id), Some(on)) =
                (arg(args, "kind"), arg(args, "id"), arg(args, "on"))
            {
                bridge.quick_favorite(kind, id, on);
            }
        }
        "quick_rate" => {
            if let (Some(kind), Some(id), Some(level)) =
                (arg(args, "kind"), arg(args, "id"), arg(args, "level"))
            {
                bridge.quick_rate(kind, id, level);
            }
        }
        "quick_download" => {
            if let (Some(kind), Some(id), Some(on)) =
                (arg(args, "kind"), arg(args, "id"), arg(args, "on"))
            {
                bridge.quick_download(kind, id, on);
            }
        }
        "quick_go_album" => {
            if let Some(song_id) = arg(args, "song_id") {
                bridge.quick_go_album(song_id);
            }
        }
        "quick_go_artist" => {
            if let Some(album_id) = arg(args, "album_id") {
                bridge.quick_go_artist(album_id);
            }
        }
        "quick_play_playlist" => {
            if let Some(id) = arg(args, "id") {
                bridge.quick_play_playlist(id);
            }
        }
        "open_album_artist_reveal" => {
            if let Some(album_id) = arg(args, "album_id") {
                bridge.open_album_artist_reveal(album_id);
            }
        }
        "sync" => {
            if let Some(mode) = arg::<SyncMode>(args, "mode") {
                bridge.sync(mode);
            }
        }
        "wipe" => {
            if let Some(scope) = arg(args, "scope") {
                bridge.wipe(scope);
            }
        }
        "download_artist" => {
            if let (Some(artist_id), Some(on)) = (arg(args, "artist_id"), arg(args, "on")) {
                bridge.download_artist(artist_id, on);
            }
        }
        "download_request" => {
            if let (Some(request), current) = (
                arg::<DownloadRequest>(args, "request"),
                arg::<Option<CurrentTrack>>(args, "current").flatten(),
            ) {
                bridge.download_request(request, current);
            }
        }
        "login" => {
            if let (Some(server_url), Some(username), Some(password)) =
                (arg(args, "server_url"), arg(args, "username"), arg(args, "password"))
            {
                bridge.login(server_url, username, password);
            }
        }
        "offline_login" => {
            if let (Some(server_url), Some(username)) =
                (arg(args, "server_url"), arg(args, "username"))
            {
                bridge.offline_login(server_url, username);
            }
        }
        "logout" => bridge.logout(),
        "remove_account" => {
            if let Some(keep_data) = arg(args, "keep_data") {
                bridge.remove_account(keep_data);
            }
        }
        "create_playlist" => {
            if let (Some(name), Some(song_ids), Some(offline)) = (
                arg(args, "name"),
                arg(args, "song_ids"),
                arg(args, "offline"),
            ) {
                bridge.create_playlist(name, song_ids, offline);
            }
        }
        "playlist_add" => {
            if let (Some(targets), Some(playlist)) = (arg(args, "targets"), arg(args, "playlist")) {
                bridge.playlist_add(targets, playlist, arg(args, "current_song_id"));
            }
        }
        "playlist_remove" => {
            if let (Some(targets), Some(playlist)) = (arg(args, "targets"), arg(args, "playlist")) {
                bridge.playlist_remove(targets, playlist, arg(args, "current_song_id"));
            }
        }
        "delete_playlist" => {
            if let Some(playlist) = arg(args, "playlist") {
                bridge.delete_playlist(playlist);
            }
        }
        "paste_playlist" => {
            if let (Some(playlist_id), Some(kind), Some(ids), Some(position)) = (
                arg(args, "playlist_id"),
                arg(args, "kind"),
                arg(args, "ids"),
                arg(args, "position"),
            ) {
                bridge.paste_playlist(playlist_id, kind, ids, position);
            }
        }
        "move_playlist_row" => {
            if let (Some(playlist_id), Some(from), Some(to)) =
                (arg(args, "playlist_id"), arg(args, "from"), arg(args, "to"))
            {
                bridge.move_playlist_row(playlist_id, from, to);
            }
        }
        "move_playlist_block" => {
            if let (Some(playlist_id), Some(start), Some(len), Some(delta), Some(cursor)) = (
                arg(args, "playlist_id"),
                arg(args, "start"),
                arg(args, "len"),
                arg(args, "delta"),
                arg(args, "cursor"),
            ) {
                bridge.move_playlist_block(playlist_id, start, len, delta, cursor);
            }
        }
        "delete_playlist_positions" => {
            if let (Some(playlist_id), Some(positions)) =
                (arg(args, "playlist_id"), arg(args, "positions"))
            {
                bridge.delete_playlist_positions(playlist_id, positions);
            }
        }
        "set_offline" => {
            if let Some(offline) = arg(args, "offline") {
                bridge.set_offline(offline);
            }
        }
        other => eprintln!("daemon: unknown UI bridge request {other}"),
    }
}

fn ensure_ui_child(child: &mut Option<std::process::Child>) {
    if let Some(child) = child.as_mut() {
        if child.try_wait().ok().flatten().is_none() {
            return;
        }
    }
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("daemon: cannot locate the UI binary");
        return;
    };
    let ui = std::env::var_os("MUSIKA_UI_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| exe.with_file_name("musika-ui"));
    let log = crate::db::config_dir()
        .ok()
        .map(|dir| dir.join("musika-ui.log"));
    let mut command = std::process::Command::new(&ui);
    command.stdin(std::process::Stdio::null());
    if let Some(log) = log {
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            if let Ok(file2) = file.try_clone() {
                command.stdout(std::process::Stdio::from(file));
                command.stderr(std::process::Stdio::from(file2));
            }
        }
    }
    match command.spawn() {
        Ok(spawned) => {
            eprintln!("daemon: spawned UI child pid {}", spawned.id());
            *child = Some(spawned);
        }
        Err(error) => eprintln!("daemon: could not spawn {}: {error}", ui.display()),
    }
}

async fn serve_ui(
    path: PathBuf,
    incoming: mpsc::UnboundedSender<UiIncoming>,
    state: mpsc::UnboundedSender<UiStateEvent>,
) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("daemon: cannot bind {}: {error}", path.display());
            return;
        }
    };
    let next_conn = AtomicU64::new(1);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            break;
        };
        let conn = next_conn.fetch_add(1, Ordering::Relaxed);
        let (reader, writer) = stream.into_split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<CoreMessage>();
        let _ = state.send(UiStateEvent::Connected(conn, out_tx));
        tokio::spawn(async move {
            let mut writer = writer;
            while let Some(message) = out_rx.recv().await {
                let Ok(line) = serde_json::to_string(&message) else {
                    continue;
                };
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if writer.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = writer.flush().await;
            }
            let _ = writer.shutdown().await;
        });
        let incoming = incoming.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<UiMessage>(&line) else {
                    continue;
                };
                if incoming.send(UiIncoming { conn, message }).is_err() {
                    break;
                }
            }
            let _ = state.send(UiStateEvent::Gone(conn));
        });
    }
}
