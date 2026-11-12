//! Headless core daemon.
//!
//! The daemon owns the databases, the backend bridge, the playback service, the
//! external text socket, the framed UI socket, MPRIS and the tray. It never
//! creates a GPUI window: `musika-ui` attaches over the framed protocol and is
//! disposable.
//!
//! Playback is event-driven: the daemon sends typed inputs to the playback
//! thread and publishes its outputs; nothing polls the audio engine from the
//! async loop.

pub(crate) mod clients;
pub(crate) mod playback;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::bridge::{Bridge, Event, SyncMode};
use crate::ipc_command::{self, CommandTarget, CurrentTrack, IpcCommand, RadioSeed, ShuffleTarget};
use crate::ipc_protocol::{
    self as proto, ByeReason, Frame, Message, PlayerReply, ReplyJson, Role,
};
use crate::model::PageDescriptor;
use crate::player::core::CoreCommand;
use crate::player::{Track};

use self::clients::{Outbox, ServerEvent};
use self::playback::{PlaybackControl, PlaybackInput, PlaybackOutput, PlaybackService, Snapshot};

/// Last page hint loaded from the status row; preserved when the daemon
/// flushes so a later UI session can reopen it. The typed descriptor is the
/// single form in the daemon; only `status::{load,save}` speaks strings.
type LastPage = Option<PageDescriptor>;

#[derive(Clone, Copy, Debug)]
enum Lifecycle {
    Quit,
    Show,
}

/// Services shared with the synchronous text-socket dispatcher.
#[derive(Clone)]
struct Services {
    bridge: Arc<Bridge>,
    playback: PlaybackControl,
    lifecycle: mpsc::UnboundedSender<Lifecycle>,
    ui_forward: mpsc::UnboundedSender<String>,
}

struct UiClient {
    role: Role,
    features: u32,
    outbox: Outbox,
}

pub fn run() -> ! {
    let Some(_instance_lock) = acquire_instance_lock() else {
        eprintln!("musika: the core daemon is already running");
        std::process::exit(0);
    };
    if crate::ipc::daemon_is_running() {
        eprintln!("musika: the core daemon is already running");
        std::process::exit(0);
    }
    if !crate::entry::init_backend() {
        std::process::exit(1);
    }
    crate::app::runtime().block_on(run_async());
    std::process::exit(0)
}

/// Single-instance lock. The socket probe alone has a launch race: two
/// simultaneous `musika` invocations could both decide to spawn a daemon and
/// the second would unlink the live UI socket. The flock is held for the
/// daemon's whole lifetime.
fn acquire_instance_lock() -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let dir = crate::db::config_dir().ok()?;
    _ = std::fs::create_dir_all(&dir);
    let path = dir.join("daemon.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .ok()?;
    // SAFETY: flock only borrows the valid file descriptor.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (locked == 0).then_some(file)
}

async fn run_async() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Event>();
    let bridge = Arc::new(Bridge::new(event_tx.clone()));

    let (playback_out_tx, mut playback_rx) = mpsc::unbounded_channel::<PlaybackOutput>();
    let playback = PlaybackService::spawn(playback_out_tx);
    let playback_control = playback.control();

    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel::<Lifecycle>();
    let (ui_forward_tx, mut ui_forward_rx) = mpsc::unbounded_channel::<String>();
    let services = Services {
        bridge: Arc::clone(&bridge),
        playback: playback_control.clone(),
        lifecycle: lifecycle_tx,
        ui_forward: ui_forward_tx,
    };

    // The text socket executes core commands synchronously; presentation
    // commands are forwarded to the async loop for the UI fan-out.
    {
        let services = services.clone();
        crate::ipc::serve(crate::ipc::socket_path(), move |line| {
            dispatch_text(&services, &line, true)
        });
    }

    let (server_tx, mut server_rx) = mpsc::unbounded_channel::<ServerEvent>();
    clients::spawn(
        crate::ipc::daemon_socket_path(),
        server_tx,
        crate::consts::now_nanos() as u64,
    );

    crate::tray::start();
    let (mpris_tx, mut mpris_rx) = mpsc::unbounded_channel::<String>();
    crate::mpris::init(mpris_tx);

    let mut page: LastPage = None;
    let mut header: Option<Event> = None;
    let mut auth: Option<Event> = None;
    let mut ui_clients: HashMap<u64, UiClient> = HashMap::new();
    let mut ui_child: Option<std::process::Child> = None;
    let mut status_dirty = true;
    let status_inflight = Arc::new(AtomicBool::new(false));
    restore_status(&playback_control, &mut page).await;

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_status = Instant::now();
    let mut quit = false;

    while !quit {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match &event {
                    Event::Header { .. } => header = Some(event.clone()),
                    Event::LoginRequired { .. }
                    | Event::LoggedIn { .. }
                    | Event::LoggedOut
                    | Event::AccountRemoved { .. } => auth = Some(event.clone()),
                    Event::OpenPage { descriptor } => update_page(&mut page, descriptor),
                    Event::OpenPageReveal { descriptor, .. } => update_page(&mut page, descriptor),
                    Event::Queue { tracks, start } => {
                        let _ = playback_control.send(PlaybackInput::Load {
                            tracks: tracks.clone(),
                            start: *start,
                            paused: false,
                            position: None,
                            volume: None,
                            muted: None,
                        });
                    }
                    Event::QueueAppend { tracks } => {
                        let _ = playback_control.send(PlaybackInput::Control {
                            op: CoreCommand::Append(tracks.clone()),
                        });
                    }
                    Event::QueuePaste { tracks, at } => {
                        let _ = playback_control.send(PlaybackInput::Control {
                            op: CoreCommand::InsertMany { at: *at, tracks: tracks.clone() },
                        });
                    }
                    Event::Restore { volume, muted, tracks, start, position, .. } => {
                        let _ = playback_control.send(PlaybackInput::Load {
                            tracks: tracks.clone(),
                            start: *start,
                            paused: true,
                            position: Some(*position),
                            volume: Some(*volume),
                            muted: Some(*muted),
                        });
                    }
                    // A favourite change made through a bridge action updates
                    // the database only; mirror it into the live queue so the
                    // queue/fullscreen rows follow (and a Patch delta reaches
                    // every UI).
                    Event::RowsPatched {
                        kind: crate::model::RowKind::Song,
                        id,
                        starred: Some(true),
                        ..
                    } => {
                        let _ = playback_control.send(PlaybackInput::Control {
                            op: CoreCommand::SetTrackFavorite {
                                id: id.clone(),
                                favorite: true,
                            },
                        });
                    }
                    Event::RowsPatched {
                        kind: crate::model::RowKind::Song,
                        id,
                        starred: Some(false),
                        ..
                    } => {
                        let _ = playback_control.send(PlaybackInput::Control {
                            op: CoreCommand::SetTrackFavorite {
                                id: id.clone(),
                                favorite: false,
                            },
                        });
                    }
                    Event::Error(message) => eprintln!("daemon: backend error: {message}"),
                    _ => {}
                }
                // Queue content travels as queue deltas, never as whole-queue
                // events; the rest of the bridge surface is forwarded verbatim.
                let forward = !matches!(
                    event,
                    Event::Queue { .. }
                        | Event::QueueAppend { .. }
                        | Event::QueuePaste { .. }
                        | Event::Restore { .. }
                );
                if forward {
                    let dead = broadcast_event(&ui_clients, &event);
                    drop_clients(&mut ui_clients, dead);
                }
                // Echo-only events do not change the persisted session.
                if !matches!(event, Event::IpcRan(_)) {
                    status_dirty = true;
                }
            }
            Some(lifecycle) = lifecycle_rx.recv() => match lifecycle {
                Lifecycle::Quit => quit = true,
                Lifecycle::Show => show_window(&mut ui_clients, &mut ui_child),
            },
            Some(line) = ui_forward_rx.recv() => {
                if ui_clients.is_empty() {
                    crate::debug_log!("daemon: presentation command with no UI attached: {line}");
                } else {
                    let event = Event::Command(line);
                    let dead = broadcast_event(&ui_clients, &event);
                    drop_clients(&mut ui_clients, dead);
                }
            }
            Some(line) = mpris_rx.recv() => {
                // MPRIS is not an IPC surface: no command echo toast.
                let _ = dispatch_text(&services, &line, false);
            }
            Some(output) = playback_rx.recv() => {
                handle_playback_output(&mut ui_clients, output, &playback_control, &mut status_dirty);
            }
            Some(server_event) = server_rx.recv() => match server_event {
                ServerEvent::Connected { conn, role, features, client, outbox } => {
                    crate::debug_log!("daemon: client {conn} ({client:?}, {role:?}) connected");
                    let snapshot = playback_control.snapshot();
                    bootstrap_client(&outbox, &snapshot, header.as_ref(), auth.as_ref(), &page);
                    ui_clients.insert(conn, UiClient { role, features, outbox });
                    status_dirty = true;
                }
                ServerEvent::Gone { conn } => {
                    if ui_clients.remove(&conn).is_some() {
                        crate::debug_log!("daemon: client {conn} gone");
                    }
                }
                ServerEvent::Message { conn, message } => {
                    handle_client_message(
                        &mut ui_clients,
                        conn,
                        message,
                        &services,
                        &playback_control,
                    );
                }
            },
            _ = ticker.tick() => {
                if let Some(child) = ui_child.as_mut()
                    && child.try_wait().ok().flatten().is_some()
                {
                    ui_child = None;
                }
                publish_mpris(&playback_control.snapshot());
                if status_dirty
                    && last_status.elapsed() >= Duration::from_secs(5)
                    && !status_inflight.swap(true, Ordering::AcqRel)
                {
                    status_dirty = false;
                    last_status = Instant::now();
                    spawn_status_flush(&playback_control, &page, Arc::clone(&status_inflight));
                }
            }
        }
    }

    // Clean shutdown: tell every client why, persist the session, stop tray,
    // MPRIS, audio and the child window.
    let bye = Frame::new(
        proto::KIND_BYE,
        proto::json_encode(&proto::Bye {
            reason: ByeReason::Quit,
        })
        .unwrap_or_default(),
    );
    for client in ui_clients.values() {
        let _ = client.outbox.send(&bye);
        client.outbox.close();
    }
    drop(ui_clients);
    flush_status_now(&playback_control, &page).await;
    crate::tray::shutdown().await;
    if let Some(mut child) = ui_child.take() {
        _ = child.kill();
        _ = child.wait();
    }
    playback.shutdown();
    _ = std::fs::remove_file(crate::ipc::socket_path());
    _ = std::fs::remove_file(crate::ipc::daemon_socket_path());
    eprintln!("daemon: stopped");
}

fn update_page(page: &mut LastPage, descriptor: &PageDescriptor) {
    *page = Some(descriptor.clone());
}

async fn restore_status(playback: &PlaybackControl, page: &mut LastPage) {
    let Some(dbs) = crate::app::dbs() else {
        return;
    };
    let Ok(Some(status)) = crate::status::load(&dbs.status).await else {
        return;
    };
    *page = PageDescriptor::from_stored_parts(&status.page_kind, &status.page_id);
    let Ok(resume) = crate::queue::resume(&dbs, &status, false).await else {
        return;
    };
    if !resume.tracks.is_empty() {
        let _ = playback.send(PlaybackInput::Load {
            tracks: resume.tracks,
            start: resume.start,
            paused: true,
            position: Some(resume.position),
            volume: Some(status.volume),
            muted: Some(status.muted),
        });
    } else {
        let _ = playback.send(PlaybackInput::Control {
            op: CoreCommand::SetVolume(status.volume),
        });
        let _ = playback.send(PlaybackInput::Control {
            op: CoreCommand::SetMuted(status.muted),
        });
    }
}

/// Executes one text command. Returns a reply for read-only query commands;
/// everything else stays fire-and-forget. `announce` echoes the canonical
/// command to the attached UI as a toast (socket/tray calls; MPRIS opts out).
fn dispatch_text(services: &Services, line: &str, announce: bool) -> Option<String> {
    let command = match ipc_command::parse(line) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("daemon: ignored command {line:?}: {error}");
            return None;
        }
    };
    if announce {
        services
            .bridge
            .ipc_ran(ipc_command::canonical_string(&command));
    }
    match command.target() {
        CommandTarget::CorePlayback => {
            if let Some(op) = playback::command_for_text(&command) {
                let _ = services.playback.send(PlaybackInput::Control { op });
            }
            None
        }
        CommandTarget::CoreBridge => {
            dispatch_core_bridge(services, &command);
            None
        }
        CommandTarget::Query => Some(query_json(&services.playback.snapshot(), &command)),
        CommandTarget::Presentation => {
            _ = services.ui_forward.send(line.to_string());
            None
        }
        CommandTarget::Lifecycle => {
            match command {
                IpcCommand::Quit => {
                    _ = services.lifecycle.send(Lifecycle::Quit);
                }
                IpcCommand::Show => {
                    _ = services.lifecycle.send(Lifecycle::Show);
                }
                _ => {}
            }
            None
        }
    }
}

fn query_json(snapshot: &Snapshot, command: &IpcCommand) -> String {
    let state = snapshot.state;
    match command {
        IpcCommand::Status => {
            let value = serde_json::json!({
                "state": format!("{:?}", state.state()).to_lowercase(),
                "index": state.index,
                "queue_rev": state.queue_rev,
                "duration": state.duration(),
                "position": state.position_at(crate::linux::monotonic_ns()),
                "volume": state.volume(),
                "muted": state.muted(),
                "loading": state.loading(),
                "engine_available": snapshot.engine_available,
                "current": snapshot
                    .queue
                    .get(state.index as usize)
                    .map(|track| serde_json::json!({
                        "id": track.id,
                        "title": track.title,
                        "artist": track.artist,
                        "album": track.album,
                    })),
                "last_error": snapshot.last_error,
            });
            value.to_string()
        }
        IpcCommand::QueueQuery => {
            let ids: Vec<&str> = snapshot.queue.iter().map(|track| track.id.as_str()).collect();
            serde_json::json!({
                "rev": snapshot.queue_rev,
                "index": state.index,
                "len": ids.len(),
                "ids": ids,
            })
            .to_string()
        }
        _ => "{}".to_string(),
    }
}

fn current_track(snapshot: &Snapshot) -> Option<&Track> {
    snapshot.queue.get(snapshot.state.index as usize)
}

fn dispatch_core_bridge(services: &Services, command: &IpcCommand) {
    let bridge = &services.bridge;
    let snapshot = services.playback.snapshot();
    let current = current_track(&snapshot);
    match command {
        IpcCommand::Sync(mode) => bridge.sync(*mode),
        IpcCommand::Refresh => bridge.sync(SyncMode::Incremental),
        IpcCommand::Wipe(scope) => {
            let scope = match scope {
                ipc_command::WipeScope::Album => "album",
                ipc_command::WipeScope::Lyrics => "lyrics",
                ipc_command::WipeScope::Covers => "covers",
            };
            bridge.wipe(scope.to_string());
        }
        IpcCommand::Logout => bridge.logout(),
        IpcCommand::RemoveAccount { keep_data } => bridge.remove_account(*keep_data),
        IpcCommand::PlaylistCreate { name, offline } => match current {
            Some(track) => bridge.create_playlist(name.clone(), vec![track.id.clone()], *offline),
            None => bridge.error("Nothing is playing; playlist create needs a current track."),
        },
        IpcCommand::PlaylistAdd { targets, playlist } => {
            let current_id = current.map(|track| track.id.clone());
            bridge.playlist_add(targets.clone(), playlist.clone(), current_id);
        }
        IpcCommand::PlaylistRemove { targets, playlist } => {
            let current_id = current.map(|track| track.id.clone());
            bridge.playlist_remove(targets.clone(), playlist.clone(), current_id);
        }
        IpcCommand::PlaylistDelete { playlist } => bridge.delete_playlist(playlist.clone()),
        IpcCommand::Download { request } => {
            let wire = current.map(|track| CurrentTrack {
                id: track.id.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                album_id: track.album_id.clone(),
            });
            bridge.download_request(request.clone(), wire);
        }
        IpcCommand::Shuffle {
            target: ShuffleTarget::Album(Some(id)),
        } => bridge.shuffle_album(id.clone()),
        IpcCommand::Shuffle {
            target: ShuffleTarget::Artist(Some(id)),
        } => bridge.shuffle_artist(id.clone()),
        IpcCommand::Shuffle { target } => {
            eprintln!("daemon: shuffle target needs an id in daemon mode: {target:?}");
        }
        IpcCommand::Radio { seed } => {
            let (kind, id) = match seed {
                RadioSeed::Current | RadioSeed::Song(None) => match current {
                    Some(track) => ("song".to_string(), track.id.clone()),
                    None => {
                        bridge.error("Radio current needs a playing track.");
                        return;
                    }
                },
                RadioSeed::Song(Some(id)) => ("song".to_string(), id.clone()),
                RadioSeed::Album(Some(id)) => ("album".to_string(), id.clone()),
                RadioSeed::Album(None) => match current.and_then(|track| {
                    (!track.album_id.is_empty()).then(|| track.album_id.clone())
                }) {
                    Some(id) => ("album".to_string(), id),
                    None => {
                        bridge.error("Radio album needs a playing album.");
                        return;
                    }
                },
            };
            bridge.play_radio(kind, id, 0);
        }
        IpcCommand::RandomAlbum => bridge.play_random_album(),
        IpcCommand::PlaySong(id) => bridge.play_song(id.clone(), String::new()),
        IpcCommand::PlaySongInAlbum(id) => bridge.play_song_in_album(id.clone()),
        IpcCommand::DownloadSong(id) => bridge.quick_download("song".to_string(), id.clone(), true),
        _ => {
            eprintln!("daemon: core bridge dispatch does not handle {command:?}");
        }
    }
}

fn handle_client_message(
    ui_clients: &mut HashMap<u64, UiClient>,
    conn: u64,
    message: Message,
    services: &Services,
    playback: &PlaybackControl,
) {
    match message {
        Message::PlayerRequest(request) => {
            let input = if matches!(request.op, proto::PlayerOp::Resync) {
                PlaybackInput::Resync {
                    conn,
                    req: request.req,
                }
            } else {
                PlaybackInput::Ui {
                    conn,
                    req: request.req,
                    op: request.op,
                }
            };
            let _ = playback.send(input);
        }
        Message::Request(request) => {
            // `qq` asks the core to quit through the bridge-shaped surface; it
            // is a lifecycle action, not a bridge method.
            if request.method == "shutdown_core" {
                _ = services.lifecycle.send(Lifecycle::Quit);
                if let Ok(bytes) = proto::json_encode(&ReplyJson::ok(request.req))
                    && let Some(client) = ui_clients.get(&conn)
                {
                    let _ = client
                        .outbox
                        .send(&Frame::new(proto::KIND_REPLY_JSON, bytes));
                }
                return;
            }
            let handled = dispatch_bridge(&services.bridge, &request.method, &request.args);
            let reply = if handled {
                ReplyJson::ok(request.req)
            } else {
                ReplyJson::err(
                    request.req,
                    proto::ErrorCode::UnknownRequest,
                    format!("unknown bridge method {}", request.method),
                )
            };
            if let Ok(bytes) = proto::json_encode(&reply)
                && let Some(client) = ui_clients.get(&conn)
            {
                let _ = client
                    .outbox
                    .send(&Frame::new(proto::KIND_REPLY_JSON, bytes));
            }
        }
        other => {
            crate::debug_log!("daemon: unexpected client frame: {other:?}");
        }
    }
}

fn handle_playback_output(
    ui_clients: &mut HashMap<u64, UiClient>,
    output: PlaybackOutput,
    playback: &PlaybackControl,
    status_dirty: &mut bool,
) {
    match output {
        PlaybackOutput::Ack {
            conn,
            req,
            ok,
            code,
            applied_seq,
        } => {
            if let Some(conn) = conn
                && let Some(client) = ui_clients.get(&conn)
            {
                let reply = PlayerReply {
                    req,
                    ok,
                    code,
                    applied_seq,
                };
                let _ = client
                    .outbox
                    .send(&Frame::new(proto::KIND_PLAYER_REPLY, reply.encode()));
            }
            if !ok {
                eprintln!("daemon: player request failed: {code:?}");
            }
        }
        PlaybackOutput::State(state) => {
            let frame = Frame::new(proto::KIND_PLAYBACK_STATE, state.encode());
            let dead: Vec<u64> = ui_clients
                .iter()
                .filter(|(_, client)| client.role == Role::Ui)
                .filter(|(_, client)| !client.outbox.send_latest(&frame))
                .map(|(conn, _)| *conn)
                .collect();
            drop_clients(ui_clients, dead);
            *status_dirty = true;
        }
        PlaybackOutput::Delta(delta) => {
            if let Ok(bytes) = delta.encode() {
                let frame = Frame::new(proto::KIND_QUEUE_DELTA, bytes);
                let dead = broadcast_frame(ui_clients, &frame);
                drop_clients(ui_clients, dead);
            }
            *status_dirty = true;
        }
        PlaybackOutput::Reset { rev, queue } => {
            for frame in playback::reset_frames(rev, &queue) {
                let dead: Vec<u64> = ui_clients
                    .iter()
                    .filter(|(_, client)| client.role == Role::Ui)
                    .filter(|(_, client)| !client.outbox.send_bytes(Arc::clone(&frame)))
                    .map(|(conn, _)| *conn)
                    .collect();
                drop_clients(ui_clients, dead);
            }
            *status_dirty = true;
        }
        PlaybackOutput::Error(message) => {
            eprintln!("daemon: playback error: {message}");
            let dead = broadcast_event(ui_clients, &Event::Error(message));
            drop_clients(ui_clients, dead);
        }
        PlaybackOutput::EngineUnavailable(message) => {
            eprintln!("daemon: audio engine unavailable: {message}");
            let dead = broadcast_event(
                ui_clients,
                &Event::Error(format!("Audio engine unavailable: {message}")),
            );
            drop_clients(ui_clients, dead);
        }
    }
    publish_mpris(&playback.snapshot());
}

fn broadcast_frame(ui_clients: &HashMap<u64, UiClient>, frame: &Frame) -> Vec<u64> {
    ui_clients
        .iter()
        .filter(|(_, client)| client.role == Role::Ui)
        .filter(|(_, client)| !client.outbox.send(frame))
        .map(|(conn, _)| *conn)
        .collect()
}

fn broadcast_event(ui_clients: &HashMap<u64, UiClient>, event: &Event) -> Vec<u64> {
    let Ok(frame) = proto::event_frame(event) else {
        return Vec::new();
    };
    broadcast_frame(ui_clients, &frame)
}

fn drop_clients(ui_clients: &mut HashMap<u64, UiClient>, dead: Vec<u64>) {
    for conn in dead {
        if let Some(client) = ui_clients.remove(&conn) {
            crate::debug_log!("daemon: dropping slow client {conn}");
            client.outbox.close();
        }
    }
}

fn bootstrap_client(
    outbox: &Outbox,
    snapshot: &Snapshot,
    header: Option<&Event>,
    auth: Option<&Event>,
    page: &LastPage,
) {
    let mut state = snapshot.state;
    state.seq = state.seq.max(1);
    let _ = outbox.send(&Frame::new(proto::KIND_PLAYBACK_STATE, state.encode()));
    for frame in playback::reset_frames(snapshot.queue_rev, &snapshot.queue) {
        let _ = outbox.send_bytes(frame);
    }
    for event in [header, auth].into_iter().flatten() {
        if let Ok(frame) = proto::event_frame(event) {
            let _ = outbox.send(&frame);
        }
    }
    if let Some(descriptor) = page {
        let event = Event::OpenPage {
            descriptor: descriptor.clone(),
        };
        if let Ok(frame) = proto::event_frame(&event) {
            let _ = outbox.send(&frame);
        }
    }
}

fn publish_mpris(snapshot: &Snapshot) {
    let state = snapshot.state;
    // The tray icon doubles as a playback indicator: the pause glyph while
    // audio plays, the play glyph while the core is paused or stopped.
    crate::tray::set_playing(matches!(state.state(), proto::StateWire::Playing));
    let current = current_track(snapshot);
    crate::mpris::publish(crate::mpris::Snapshot {
        playback_status: match state.state() {
            proto::StateWire::Playing => "Playing",
            proto::StateWire::Paused => "Paused",
            proto::StateWire::Stopped => "Stopped",
        }
        .to_string(),
        loop_status: match state.repeat() {
            proto::RepeatWire::Off => "None",
            proto::RepeatWire::One => "Track",
            proto::RepeatWire::All => "Playlist",
        }
        .to_string(),
        title: current.map(|track| track.title.clone()).unwrap_or_default(),
        artist: current
            .map(|track| track.artist.clone())
            .unwrap_or_default(),
        album: current.map(|track| track.album.clone()).unwrap_or_default(),
        track_id: current.map(|track| track.id.clone()).unwrap_or_default(),
        duration_us: (state.duration() * 1_000_000.0) as i64,
        position_us: (state.position_at(crate::linux::monotonic_ns()) * 1_000_000.0) as i64,
        volume: state.volume() / 100.0,
        can_next: !snapshot.queue.is_empty(),
        can_prev: !snapshot.queue.is_empty(),
        art_url: current
            .map(|track| track.cover_url.clone())
            .unwrap_or_default(),
    });
}

fn build_status(snapshot: &Snapshot, page: &LastPage) -> crate::status::SessionStatus {
    let account = crate::accounts::active_account();
    let current = current_track(snapshot);
    let queue: Vec<&str> = snapshot.queue.iter().map(|track| track.id.as_str()).collect();
    let queue = serde_json::to_string(&queue).unwrap_or_default();
    crate::status::SessionStatus {
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
        page_kind: page
            .as_ref()
            .map(|page| page.kind.api_kind().to_string())
            .unwrap_or_default(),
        page_id: page
            .as_ref()
            .map(|page| page.id.clone())
            .unwrap_or_default(),
        queue,
        queue_index: snapshot.state.index as i64,
        position: snapshot.state.position_at(crate::linux::monotonic_ns()),
        volume: snapshot.state.volume(),
        muted: snapshot.state.muted(),
        updated_at: crate::consts::now_secs(),
    }
}

fn spawn_status_flush(
    playback: &PlaybackControl,
    page: &LastPage,
    inflight: Arc<AtomicBool>,
) {
    let Some(dbs) = crate::app::dbs() else {
        inflight.store(false, Ordering::Release);
        return;
    };
    let status = build_status(&playback.snapshot(), page);
    crate::app::runtime().spawn(async move {
        if let Err(error) = crate::status::save(&dbs.status, &status).await {
            eprintln!("daemon: status flush failed: {error:#}");
        }
        inflight.store(false, Ordering::Release);
    });
}

async fn flush_status_now(playback: &PlaybackControl, page: &LastPage) {
    let Some(dbs) = crate::app::dbs() else {
        return;
    };
    let status = build_status(&playback.snapshot(), page);
    if let Err(error) = crate::status::save(&dbs.status, &status).await {
        eprintln!("daemon: status flush failed: {error:#}");
    }
}

fn show_window(ui_clients: &mut HashMap<u64, UiClient>, ui_child: &mut Option<std::process::Child>) {
    let ui_attached = ui_clients.values().any(|client| client.role == Role::Ui);
    if ui_attached {
        let frame = Frame::new(proto::KIND_ACTIVATE, Vec::new());
        let dead = broadcast_frame(ui_clients, &frame);
        drop_clients(ui_clients, dead);
    } else {
        ensure_ui_child(ui_child);
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

fn arg<T: DeserializeOwned>(args: &serde_json::Value, key: &str) -> Option<T> {
    args.get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Generate the core's request dispatcher from the shared bridge RPC table:
/// each arm extracts the declared arguments from the request's JSON object and
/// calls the matching `Bridge` handler. A request with missing or malformed
/// arguments is consumed silently (handled), like the hand-written arms did.
macro_rules! bridge_dispatch {
    ($( $name:ident ( $( $field:ident : $ty:ty ),* $(,)? ), )*) => {
        fn dispatch_bridge(bridge: &Bridge, method: &str, args: &serde_json::Value) -> bool {
            match method {
                $(
                    stringify!($name) => {
                        $(
                            let Some($field) = arg::<$ty>(args, stringify!($field)) else {
                                return true;
                            };
                        )*
                        let _ = bridge.$name($($field),*);
                    }
                )*
                _ => return false,
            }
            true
        }
    };
}

crate::for_each_bridge_rpc!(bridge_dispatch);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_exactly_one_target() {
        // The classification is exhaustive by construction; this guards against
        // a variant silently changing category.
        assert_eq!(
            ipc_command::parse("play_pause").unwrap().target(),
            CommandTarget::CorePlayback
        );
        assert_eq!(
            ipc_command::parse("status").unwrap().target(),
            CommandTarget::Query
        );
        assert_eq!(
            ipc_command::parse("sync").unwrap().target(),
            CommandTarget::CoreBridge
        );
        assert_eq!(
            ipc_command::parse("move_down").unwrap().target(),
            CommandTarget::Presentation
        );
        // The selection lives in the UI, so the selected-rows playlist add is
        // forwarded to the attached UI instead of running in the core.
        assert_eq!(
            ipc_command::parse("playlist add selected; Mix")
                .unwrap()
                .target(),
            CommandTarget::Presentation
        );
        assert_eq!(
            ipc_command::parse("playlist add current; Mix")
                .unwrap()
                .target(),
            CommandTarget::CoreBridge
        );
        assert_eq!(
            ipc_command::parse("quit").unwrap().target(),
            CommandTarget::Lifecycle
        );
    }
}
