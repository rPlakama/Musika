//! UI-side client for the core daemon.
//!
//! `musika-ui` never opens the databases, network or audio device. It talks
//! JSON lines to `musika` over `daemon.sock`: bridge requests go out, events,
//! player snapshots and 10 Hz ticks come back. The player type here is a
//! local mirror so row operations render optimistically; the core remains the
//! authority and replaces it from snapshots.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::bridge::Event;
use crate::ipc_command::{CurrentTrack, DownloadRequest};
use crate::model::{PageDescriptor, PageKind, Scope};
use crate::player::{Player, Repeat, State, Track};
use crate::protocol::{
    CoreMessage, PlayerCommand, PlayerSnapshot, PlayerTick, StateWire, UiMessage,
    VERSION,
};

#[derive(Clone)]
pub struct Client {
    stream: Arc<Mutex<UnixStream>>,
}

impl Client {
    /// Connects to the core, retrying while the freshly spawned daemon binds
    /// its socket. Returns the client and the update stream consumed by the
    /// GPUI root.
    pub fn connect() -> std::io::Result<(Self, tokio::sync::mpsc::UnboundedReceiver<UiUpdate>)> {
        let path = crate::ipc::daemon_socket_path();
        let mut last_error = None;
        let mut stream = None;
        for _ in 0..100 {
            match UnixStream::connect(&path) {
                Ok(connection) => {
                    stream = Some(connection);
                    break;
                }
                Err(error) => {
                    last_error = Some(error);
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let stream = stream.ok_or_else(|| {
            last_error.unwrap_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotConnected, "daemon unavailable")
            })
        })?;
        if let Ok(write) = stream.try_clone() {
            let mut write = write;
            let hello = serde_json::to_string(&UiMessage::Hello { version: VERSION })
                .unwrap_or_default();
            let _ = writeln!(write, "{hello}");
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let reader = stream.try_clone()?;
        std::thread::Builder::new()
            .name("musika-daemon-reader".to_string())
            .spawn(move || read_updates(reader, tx))
            .expect("spawn daemon reader");
        Ok((
            Self {
                stream: Arc::new(Mutex::new(stream)),
            },
            rx,
        ))
    }

    pub fn send(&self, message: &UiMessage) {
        let Ok(line) = serde_json::to_string(message) else {
            return;
        };
        if let Ok(mut stream) = self.stream.lock() {
            let _ = writeln!(*stream, "{line}");
            let _ = stream.flush();
        }
    }

    pub fn shutdown_core(&self) {
        self.send(&UiMessage::Shutdown);
    }

    pub fn show(&self) {
        self.send(&UiMessage::Show);
    }

    fn bridge(&self, method: &str, args: serde_json::Value) {
        self.send(&UiMessage::Bridge {
            method: method.to_string(),
            args,
        });
    }

    fn player(&self, command: PlayerCommand) {
        self.send(&UiMessage::Player(command));
    }
}

#[derive(Debug)]
pub enum UiUpdate {
    Event(Event),
    Full(PlayerSnapshot),
    Tick(PlayerTick),
    Activate,
    Shutdown,
}

fn read_updates(stream: UnixStream, tx: UnboundedSender<UiUpdate>) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<CoreMessage>(&line) else {
            continue;
        };
        let update = match message {
            CoreMessage::Ready => continue,
            CoreMessage::Bridge(event) => UiUpdate::Event(event),
            CoreMessage::PlayerFull(snapshot) => UiUpdate::Full(snapshot),
            CoreMessage::PlayerTick(tick) => UiUpdate::Tick(tick),
            CoreMessage::Activate => UiUpdate::Activate,
            CoreMessage::Shutdown => UiUpdate::Shutdown,
            CoreMessage::Error(message) => UiUpdate::Event(Event::Error(message)),
        };
        if tx.send(update).is_err() {
            break;
        }
    }
    // The core is gone: the window has nothing left to render.
    let _ = tx.send(UiUpdate::Shutdown);
}

#[derive(Clone)]
pub struct RemoteBridge {
    client: Client,
}

macro_rules! no_args {
    ($($name:ident),* $(,)?) => {
        $(pub fn $name(&self) {
            self.client.bridge(stringify!($name), json!({}));
        })*
    };
}

impl RemoteBridge {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub fn shutdown_core(&self) {
        self.client.shutdown_core();
    }

    pub fn request_album_state(&self, album_id: String) {
        self.client
            .bridge("request_album_state", json!({ "album_id": album_id }));
    }

    pub fn load_library(&self, scope: Scope, order: usize) -> u64 {
        self.client
            .bridge("load_library", json!({ "scope": scope, "order": order }));
        0
    }

    pub fn request_library_window(
        &self,
        generation: u64,
        scope: Scope,
        order: usize,
        start: usize,
        count: usize,
    ) {
        self.client.bridge(
            "request_library_window",
            json!({
                "generation": generation,
                "scope": scope,
                "order": order,
                "start": start,
                "count": count,
            }),
        );
    }

    pub fn preload_library(&self, scope: Scope, order: usize) {
        self.client
            .bridge("preload_library", json!({ "scope": scope, "order": order }));
    }

    pub fn search(&self, query: String) {
        self.client.bridge("search", json!({ "query": query }));
    }

    pub fn request_command_suggestions(
        &self,
        generation: u64,
        scope: crate::ipc_command::SuggestScope,
        query: String,
        start: usize,
        end: usize,
    ) {
        self.client.bridge(
            "request_command_suggestions",
            json!({
                "generation": generation,
                "scope": scope,
                "query": query,
                "start": start,
                "end": end,
            }),
        );
    }

    pub fn load_page(&self, descriptor: PageDescriptor) {
        self.client
            .bridge("load_page", json!({ "descriptor": descriptor }));
    }

    pub fn request_notes(&self, album_id: String) {
        self.client.bridge("request_notes", json!({ "album_id": album_id }));
    }

    pub fn request_artist_notes(&self, artist_id: String) {
        self.client
            .bridge("request_artist_notes", json!({ "artist_id": artist_id }));
    }

    pub fn request_lyrics(&self, song_id: String) {
        self.client
            .bridge("request_lyrics", json!({ "song_id": song_id }));
    }

    pub fn request_marks(&self, songs: Vec<String>, albums: Vec<String>) {
        self.client
            .bridge("request_marks", json!({ "songs": songs, "albums": albums }));
    }

    pub fn request_container_state(&self, kind: PageKind, id: String) {
        self.client
            .bridge("request_container_state", json!({ "kind": kind, "id": id }));
    }

    pub fn request_colors(&self, cover_art: String) {
        self.client
            .bridge("request_colors", json!({ "cover_art": cover_art }));
    }

    pub fn request_display_cover(&self, key: String, cover_art: String, target: u32) {
        self.client.bridge(
            "request_display_cover",
            json!({ "key": key, "cover_art": cover_art, "target": target }),
        );
    }

    pub fn play_song(&self, song_id: String, order: String) {
        self.client
            .bridge("play_song", json!({ "song_id": song_id, "order": order }));
    }

    pub fn play_album(&self, album_id: String, start: usize) {
        self.client
            .bridge("play_album", json!({ "album_id": album_id, "start": start }));
    }

    pub fn play_song_in_album(&self, song_id: String) {
        self.client
            .bridge("play_song_in_album", json!({ "song_id": song_id }));
    }

    pub fn play_radio(&self, kind: String, id: String, start_index: usize) {
        self.client.bridge(
            "play_radio",
            json!({ "kind": kind, "id": id, "start": start_index }),
        );
    }

    pub fn play_radio_query(&self, kind: String, query: String, start_index: usize) {
        self.client.bridge(
            "play_radio_query",
            json!({ "kind": kind, "query": query, "start": start_index }),
        );
    }

    pub fn quick_radio(&self, kind: String, id: String) {
        self.client
            .bridge("quick_radio", json!({ "kind": kind, "id": id }));
    }

    pub fn shuffle_album(&self, album_id: String) {
        self.client
            .bridge("shuffle_album", json!({ "album_id": album_id }));
    }

    pub fn shuffle_artist(&self, artist_id: String) {
        self.client
            .bridge("shuffle_artist", json!({ "artist_id": artist_id }));
    }

    pub fn shuffle_album_query(&self, query: String) {
        self.client
            .bridge("shuffle_album_query", json!({ "query": query }));
    }

    pub fn shuffle_artist_query(&self, query: String) {
        self.client
            .bridge("shuffle_artist_query", json!({ "query": query }));
    }

    pub fn shuffle_radio(&self, seed_kind: String, seed_id: String) {
        self.client.bridge(
            "shuffle_radio",
            json!({ "seed_kind": seed_kind, "seed_id": seed_id }),
        );
    }

    pub fn queue_add_radio(&self, seed_kind: String, seed_id: String) {
        self.client.bridge(
            "queue_add_radio",
            json!({ "seed_kind": seed_kind, "seed_id": seed_id }),
        );
    }

    pub fn save_radio(&self, seed_kind: String, seed_id: String, name: String) {
        self.client.bridge(
            "save_radio",
            json!({ "seed_kind": seed_kind, "seed_id": seed_id, "name": name }),
        );
    }

    pub fn save_radio_local(&self, seed_kind: String, seed_id: String, name: String) {
        self.client.bridge(
            "save_radio_local",
            json!({ "seed_kind": seed_kind, "seed_id": seed_id, "name": name }),
        );
    }

    pub fn download_radio(&self, seed_kind: String, seed_id: String) {
        self.client.bridge(
            "download_radio",
            json!({ "seed_kind": seed_kind, "seed_id": seed_id }),
        );
    }

    pub fn quick_queue_add(&self, kind: String, id: String) {
        self.client
            .bridge("quick_queue_add", json!({ "kind": kind, "id": id }));
    }

    pub fn paste_queue(&self, kind: String, ids: Vec<String>, at: usize) {
        self.client
            .bridge("paste_queue", json!({ "kind": kind, "ids": ids, "at": at }));
    }

    pub fn quick_favorite(&self, kind: String, id: String, on: bool) {
        self.client
            .bridge("quick_favorite", json!({ "kind": kind, "id": id, "on": on }));
    }

    pub fn quick_rate(&self, kind: String, id: String, level: u8) {
        self.client.bridge(
            "quick_rate",
            json!({ "kind": kind, "id": id, "level": level }),
        );
    }

    pub fn quick_download(&self, kind: String, id: String, on: bool) {
        self.client
            .bridge("quick_download", json!({ "kind": kind, "id": id, "on": on }));
    }

    pub fn quick_go_album(&self, song_id: String) {
        self.client
            .bridge("quick_go_album", json!({ "song_id": song_id }));
    }

    pub fn quick_go_artist(&self, album_id: String) {
        self.client
            .bridge("quick_go_artist", json!({ "album_id": album_id }));
    }

    pub fn quick_play_playlist(&self, id: String) {
        self.client
            .bridge("quick_play_playlist", json!({ "id": id }));
    }

    pub fn open_album_artist_reveal(&self, album_id: String) {
        self.client
            .bridge("open_album_artist_reveal", json!({ "album_id": album_id }));
    }

    pub fn sync(&self, mode: crate::bridge::SyncMode) {
        self.client.bridge("sync", json!({ "mode": mode }));
    }

    pub fn wipe(&self, scope: String) {
        self.client.bridge("wipe", json!({ "scope": scope }));
    }

    pub fn download_artist(&self, artist_id: String, on: bool) {
        self.client.bridge(
            "download_artist",
            json!({ "artist_id": artist_id, "on": on }),
        );
    }

    pub fn download_request(
        &self,
        request: DownloadRequest,
        current: Option<CurrentTrack>,
    ) {
        self.client.bridge(
            "download_request",
            json!({ "request": request, "current": current }),
        );
    }

    pub fn login(&self, server_url: String, username: String, password: String) {
        self.client.bridge(
            "login",
            json!({ "server_url": server_url, "username": username, "password": password }),
        );
    }

    pub fn offline_login(&self, server_url: String, username: String) {
        self.client.bridge(
            "offline_login",
            json!({ "server_url": server_url, "username": username }),
        );
    }

    no_args!(logout, refresh_header, play_random_album);

    pub fn remove_account(&self, keep_data: bool) {
        self.client
            .bridge("remove_account", json!({ "keep_data": keep_data }));
    }

    pub fn create_playlist(&self, name: String, song_ids: Vec<String>, offline: bool) {
        self.client.bridge(
            "create_playlist",
            json!({ "name": name, "song_ids": song_ids, "offline": offline }),
        );
    }

    pub fn playlist_add(
        &self,
        targets: Vec<String>,
        playlist: String,
        current_song_id: Option<String>,
    ) {
        self.client.bridge(
            "playlist_add",
            json!({ "targets": targets, "playlist": playlist, "current_song_id": current_song_id }),
        );
    }

    pub fn playlist_remove(
        &self,
        targets: Vec<String>,
        playlist: String,
        current_song_id: Option<String>,
    ) {
        self.client.bridge(
            "playlist_remove",
            json!({ "targets": targets, "playlist": playlist, "current_song_id": current_song_id }),
        );
    }

    pub fn delete_playlist(&self, playlist: String) {
        self.client
            .bridge("delete_playlist", json!({ "playlist": playlist }));
    }

    pub fn paste_playlist(&self, playlist_id: String, kind: String, ids: Vec<String>, position: usize) {
        self.client.bridge(
            "paste_playlist",
            json!({ "playlist_id": playlist_id, "kind": kind, "ids": ids, "position": position }),
        );
    }

    pub fn move_playlist_row(&self, playlist_id: String, from: usize, to: usize) {
        self.client.bridge(
            "move_playlist_row",
            json!({ "playlist_id": playlist_id, "from": from, "to": to }),
        );
    }

    pub fn move_playlist_block(
        &self,
        playlist_id: String,
        start: usize,
        len: usize,
        delta: i32,
        cursor: usize,
    ) {
        self.client.bridge(
            "move_playlist_block",
            json!({ "playlist_id": playlist_id, "start": start, "len": len, "delta": delta, "cursor": cursor }),
        );
    }

    pub fn delete_playlist_positions(&self, playlist_id: String, positions: Vec<usize>) {
        self.client.bridge(
            "delete_playlist_positions",
            json!({ "playlist_id": playlist_id, "positions": positions }),
        );
    }
}

/// UI-side player projection. Reads are immediate; mutations apply locally for
/// optimistic rendering and send the same mutation to the core, whose next
/// snapshot reconciles any drift.
pub struct PlayerMirror {
    player: Player,
    client: Client,
}

impl PlayerMirror {
    pub fn new(client: Client) -> Self {
        Self {
            player: Player::mirror(),
            client,
        }
    }

    pub fn apply_full(&mut self, snapshot: PlayerSnapshot) {
        self.player.queue = Arc::new(snapshot.queue);
        self.player.index = snapshot.index.min(self.player.queue.len().saturating_sub(1));
        self.player.state = match snapshot.state {
            StateWire::Stopped => State::Stopped,
            StateWire::Playing => State::Playing,
            StateWire::Paused => State::Paused,
        };
        self.player.repeat = snapshot.repeat.into();
        self.player.volume = snapshot.volume;
        self.player.muted = snapshot.muted;
        self.player.fallback_duration = snapshot.duration;
        {
            let mut shared = self.player.shared();
            shared.position = snapshot.position;
            shared.duration = snapshot.duration;
        }
    }

    pub fn apply_tick(&mut self, tick: PlayerTick) {
        self.player.index = tick.index.min(self.player.len().saturating_sub(1));
        self.player.state = match tick.state {
            StateWire::Stopped => State::Stopped,
            StateWire::Playing => State::Playing,
            StateWire::Paused => State::Paused,
        };
        self.player.repeat = tick.repeat.into();
        self.player.volume = tick.volume;
        self.player.muted = tick.muted;
        self.player.fallback_duration = tick.duration;
        {
            let mut shared = self.player.shared();
            shared.position = tick.position;
            shared.duration = tick.duration;
        }
    }

    pub fn state(&self) -> State {
        self.player.state()
    }
    pub fn repeat(&self) -> Repeat {
        self.player.repeat()
    }
    pub fn index(&self) -> usize {
        self.player.index()
    }
    pub fn len(&self) -> usize {
        self.player.len()
    }
    pub fn current(&self) -> Option<&Track> {
        self.player.current()
    }
    pub fn queue(&self) -> &[Track] {
        self.player.queue()
    }
    pub fn queue_snapshot(&self) -> Arc<Vec<Track>> {
        self.player.queue_snapshot()
    }
    pub fn position(&self) -> f64 {
        self.player.position()
    }
    pub fn duration(&self) -> f64 {
        self.player.duration()
    }
    pub fn volume(&self) -> f64 {
        self.player.volume()
    }
    pub fn muted(&self) -> bool {
        self.player.muted()
    }
    pub fn is_loading(&self) -> bool {
        self.player.is_loading()
    }
    pub fn set_track_favorite(&mut self, song_id: &str, favorite: bool) {
        self.player.set_track_favorite(song_id, favorite);
        self.client.player(PlayerCommand::SetTrackFavorite {
            id: song_id.to_string(),
            favorite,
        });
    }
    pub fn toggle(&mut self) {
        self.player.toggle();
        self.client.player(PlayerCommand::PlayPause);
    }
    pub fn pause(&mut self) {
        self.player.pause();
        self.client.player(PlayerCommand::Pause);
    }
    pub fn resume(&mut self) {
        self.player.resume();
        self.client.player(PlayerCommand::Play);
    }
    pub fn stop(&mut self) {
        self.player.stop();
        self.client.player(PlayerCommand::Stop);
    }
    pub fn next(&mut self) {
        self.player.next();
        self.client.player(PlayerCommand::Next);
    }
    pub fn previous(&mut self) {
        self.player.previous();
        self.client.player(PlayerCommand::Previous);
    }
    pub fn set_volume(&mut self, volume: f64) {
        self.player.set_volume(volume);
        self.client.player(PlayerCommand::SetVolume { volume });
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.player.set_muted(muted);
        self.client.player(PlayerCommand::SetMuted { muted });
    }
    pub fn toggle_mute(&mut self) {
        self.player.toggle_mute();
        self.client.player(PlayerCommand::ToggleMute);
    }
    pub fn seek(&mut self, seconds: f64) {
        self.player.seek(seconds);
        self.client.player(PlayerCommand::Seek { seconds });
    }
    pub fn jump(&mut self, index: usize) {
        self.player.jump(index);
        self.client.player(PlayerCommand::Jump { index });
    }
    pub fn load(&mut self, tracks: Vec<Track>, start: usize) {
        self.player.load(tracks, start);
    }
    pub fn move_item(&mut self, from: usize, to: usize) {
        self.player.move_item(from, to);
        self.client.player(PlayerCommand::Move { from, to });
    }
    pub fn append(&mut self, tracks: Vec<Track>) {
        self.player.append(tracks.clone());
        self.client.player(PlayerCommand::Append { tracks });
    }
    pub fn insert_many(&mut self, at: usize, tracks: Vec<Track>) {
        self.player.insert_many(at, tracks.clone());
        self.client.player(PlayerCommand::InsertMany { at, tracks });
    }
    pub fn remove_many(&mut self, indices: &[usize]) -> usize {
        let removed = self.player.remove_many(indices);
        self.client.player(PlayerCommand::RemoveMany {
            indices: indices.to_vec(),
        });
        removed
    }
    pub fn move_block(&mut self, start: usize, len: usize, delta: i32) -> bool {
        let moved = self.player.move_block(start, len, delta);
        if moved {
            self.client.player(PlayerCommand::MoveBlock { start, len, delta });
        }
        moved
    }
    pub fn cycle_repeat(&mut self) -> Repeat {
        let repeat = self.player.cycle_repeat();
        self.client.player(PlayerCommand::CycleRepeat);
        repeat
    }
    pub fn shuffle_queue(&mut self) {
        self.player.shuffle_queue();
        self.client.player(PlayerCommand::ShuffleQueue);
    }
    pub fn clear(&mut self) {
        self.player.queue = Arc::new(Vec::new());
        self.player.index = 0;
        self.player.state = State::Stopped;
        self.player.repeat = Repeat::Off;
        {
            let mut shared = self.player.shared();
            shared.position = 0.0;
            shared.duration = 0.0;
        }
    }
    pub fn load_paused(&mut self, tracks: Vec<Track>, start: usize) {
        self.player.load_paused(tracks, start);
    }
}
