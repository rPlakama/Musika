// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Native MPRIS 2.2 service (Q-8 / roadmap "MPRIS node").
//!
//! The Qt thread publishes a [`Snapshot`] on every player sync; a zbus task
//! owns `org.mpris.MediaPlayer2.musika` and answers panels, media keys and
//! `playerctl`. Controls travel back as plain command strings through a
//! channel the backend drains on its tick, so every MPRIS action goes through
//! the same dispatcher as keyboard/IPC input.
//!
//! Without a session bus this is a no-op; with `MUSIKA_MPRIS_SCRIPT` set the
//! mpv fallback stays in charge (single MPRIS node).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tokio::sync::{Notify, mpsc};
use zbus::connection::Builder;
use zbus::fdo::Properties;
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{Connection, interface};

pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.musika";
pub const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
/// Don't emit PropertiesChanged faster than this (roadmap robustness rule).
const RATE_LIMIT: std::time::Duration = std::time::Duration::from_millis(250);

/// What the Qt thread publishes for the service to read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// `Playing` | `Paused` | `Stopped`.
    pub playback_status: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_id: String,
    pub duration_us: i64,
    pub position_us: i64,
    /// 0.0..=1.0 (MPRIS scale).
    pub volume: f64,
    pub can_next: bool,
    pub can_prev: bool,
    pub art_url: String,
}

impl Snapshot {
    /// Key for change detection: everything signalled except `Position`.
    fn signal_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.playback_status,
            self.title,
            self.artist,
            self.album,
            self.track_id,
            self.duration_us,
            self.volume,
            self.can_next,
            self.can_prev,
            self.art_url,
        )
    }
}

static SNAPSHOT: OnceLock<Mutex<Snapshot>> = OnceLock::new();
static NOTIFY: OnceLock<Notify> = OnceLock::new();
static COMMAND_TX: OnceLock<mpsc::UnboundedSender<String>> = OnceLock::new();
static COMMAND_RX: OnceLock<Mutex<mpsc::UnboundedReceiver<String>>> = OnceLock::new();

fn snapshot() -> Snapshot {
    SNAPSHOT
        .get()
        .and_then(|lock| lock.lock().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

/// Publishes the current player state and wakes the signal emitter.
pub fn publish(state: Snapshot) {
    if let Some(lock) = SNAPSHOT.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = state;
        }
    }
    if let Some(notify) = NOTIFY.get() {
        notify.notify_one();
    }
}

/// Sends one control command to the Qt thread (`mpris_*` vocabulary).
fn submit(command: String) {
    if let Some(tx) = COMMAND_TX.get() {
        let _ = tx.send(command);
    }
}

fn submit_action(action: &str) {
    submit(format!("mpris_{action}"));
}

fn put_nonempty(map: &mut HashMap<String, OwnedValue>, key: &str, value: &str) {
    if !value.is_empty() {
        map.insert(key.to_string(), owned(Value::from(value.to_string())));
    }
}

pub fn mpris_script() -> Option<String> {
    std::env::var("MUSIKA_MPRIS_SCRIPT")
        .ok()
        .filter(|script| !script.is_empty())
}

/// Drains pending control commands (called from the player tick).
pub fn drain() -> Vec<String> {
    let Some(rx) = COMMAND_RX.get() else {
        return Vec::new();
    };
    let Ok(mut rx) = rx.lock() else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    while let Ok(command) = rx.try_recv() {
        commands.push(command);
    }
    commands
}

/// Starts the service (idempotent; safe without a session bus).
pub fn init() {
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = COMMAND_TX.set(tx);
    let _ = COMMAND_RX.set(Mutex::new(rx));
    let _ = SNAPSHOT.set(Mutex::new(Snapshot::default()));
    let _ = NOTIFY.set(Notify::new());
    crate::runtime().spawn(serve());
}

async fn serve() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        eprintln!("mpris: no session bus; native MPRIS disabled");
        return;
    }
    if mpris_script().is_some() {
        eprintln!("mpris: MUSIKA_MPRIS_SCRIPT is set; keeping the mpv fallback");
        return;
    }

    let Ok(builder) = Builder::session() else {
        eprintln!("mpris: could not reach the session bus");
        return;
    };
    let connection = match builder
        .allow_name_replacements(true)
        .replace_existing_names(true)
        .name(BUS_NAME)
        .and_then(|builder| builder.serve_at(OBJECT_PATH, MprisRoot))
        .and_then(|builder| builder.serve_at(OBJECT_PATH, MprisPlayer))
    {
        Ok(builder) => match builder.build().await {
            Ok(connection) => connection,
            Err(err) => {
                eprintln!("mpris: could not own {BUS_NAME}: {err}");
                return;
            }
        },
        Err(err) => {
            eprintln!("mpris: setup failed: {err}");
            return;
        }
    };

    if let Some(notify) = NOTIFY.get() {
        emit_changes(&connection, notify).await;
    }
}

/// Coalesces notifications and emits `PropertiesChanged` when the signalled
/// state actually moved (position-only ticks stay silent).
async fn emit_changes(connection: &Connection, notify: &Notify) {
    let iface_ref = match connection
        .object_server()
        .interface::<_, MprisPlayer>(OBJECT_PATH)
        .await
    {
        Ok(reference) => reference,
        Err(err) => {
            eprintln!("mpris: interface lookup failed: {err}");
            return;
        }
    };

    let mut last = String::new();
    loop {
        notify.notified().await;
        tokio::time::sleep(RATE_LIMIT).await;
        let state = snapshot();
        let key = state.signal_key();
        if key == last {
            continue;
        }
        last = key;

        let Ok(name) = InterfaceName::try_from(PLAYER_INTERFACE) else {
            return;
        };
        let mut changed: HashMap<&str, Value<'_>> = HashMap::new();
        changed.insert("Metadata", Value::from(metadata(&state)));
        changed.insert("PlaybackStatus", Value::from(state.playback_status));
        changed.insert("Volume", Value::from(state.volume));
        changed.insert("CanGoNext", Value::from(state.can_next));
        changed.insert("CanGoPrevious", Value::from(state.can_prev));
        changed.insert("CanSeek", Value::from(state.duration_us > 0));

        if let Err(err) = Properties::properties_changed(
            iface_ref.signal_emitter(),
            name,
            changed,
            Cow::Borrowed(&[]),
        )
        .await
        {
            eprintln!("mpris: PropertiesChanged failed: {err}");
        }
    }
}

fn owned(value: Value<'_>) -> OwnedValue {
    value
        .try_into()
        .unwrap_or_else(|_| Value::from(0u8).try_into().expect("u8 is representable"))
}

/// MPRIS object path for a song id (segments allow `[A-Za-z0-9_]` only).
/// The empty id maps to the spec's "no track" placeholder.
fn track_path(song_id: &str) -> String {
    if song_id.is_empty() {
        return "/org/mpris/MediaPlayer2/TrackList/NoTrack".to_string();
    }
    let safe: String = song_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("/org/musika/track/{safe}")
}

fn metadata(state: &Snapshot) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if let Ok(path) = ObjectPath::try_from(track_path(&state.track_id)) {
        map.insert("mpris:trackid".to_string(), owned(Value::from(path)));
    }
    if state.duration_us > 0 {
        map.insert(
            "mpris:length".to_string(),
            owned(Value::from(state.duration_us)),
        );
    }
    put_nonempty(&mut map, "mpris:artUrl", &state.art_url);
    put_nonempty(&mut map, "xesam:title", &state.title);
    if !state.artist.is_empty() {
        map.insert(
            "xesam:artist".to_string(),
            owned(Value::from(vec![state.artist.clone()])),
        );
    }
    put_nonempty(&mut map, "xesam:album", &state.album);
    map
}

/// `org.mpris.MediaPlayer2` root interface (zbus needs one type per
/// interface); state lives in the module statics.
struct MprisRoot;

#[interface(name = "org.mpris.MediaPlayer2")]
impl MprisRoot {
    #[zbus(property)]
    fn identity(&self) -> String {
        "Musika".to_string()
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "musika".to_string()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }

    fn raise(&self) {}

    fn quit(&self) {}
}

struct MprisPlayer;

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl MprisPlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        snapshot().playback_status
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        "None".to_string()
    }

    #[zbus(property)]
    fn set_loop_status(&mut self, _status: String) {}

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn set_rate(&mut self, _rate: f64) {}

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn set_shuffle(&mut self, _shuffle: bool) {}

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&snapshot())
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        snapshot().volume
    }

    #[zbus(property)]
    fn set_volume(&mut self, volume: f64) {
        submit(format!("mpris_volume:{}", volume.clamp(0.0, 1.0)));
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        snapshot().position_us
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        snapshot().can_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        snapshot().can_prev
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        snapshot().duration_us > 0
    }

    #[zbus(property)]
    fn can_stop(&self) -> bool {
        true
    }

    fn play(&self) {
        submit_action("play");
    }

    fn pause(&self) {
        submit_action("pause");
    }

    fn play_pause(&self) {
        submit_action("toggle");
    }

    fn stop(&self) {
        submit_action("stop");
    }

    fn next(&self) {
        submit_action("next");
    }

    fn previous(&self) {
        submit_action("prev");
    }

    async fn seek(&self, offset: i64, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        submit(format!("mpris_seek:{offset}"));
        let position = snapshot().position_us + offset;
        let _ = MprisPlayer::seeked(&emitter, position).await;
    }

    async fn set_position(
        &self,
        track_id: ObjectPath<'_>,
        position: i64,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let expected = track_path(&snapshot().track_id);
        if track_id.as_str() == expected {
            submit(format!("mpris_position:{position}"));
            let _ = MprisPlayer::seeked(&emitter, position).await;
        }
    }

    #[zbus(signal)]
    async fn seeked(signal_emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_paths_are_valid_object_paths() {
        assert_eq!(
            track_path("3PlpTsrmYRDfmuo3fe3PEJ"),
            "/org/musika/track/3PlpTsrmYRDfmuo3fe3PEJ"
        );
        assert_eq!(track_path("a-b.c"), "/org/musika/track/a_b_c");
        assert_eq!(track_path(""), "/org/mpris/MediaPlayer2/TrackList/NoTrack");
        assert!(ObjectPath::try_from(track_path("a-b.c")).is_ok());
    }

    #[test]
    fn metadata_carries_the_expected_roles() {
        let state = Snapshot {
            title: "Idioteque".to_string(),
            artist: "Radiohead".to_string(),
            album: "Kid A".to_string(),
            track_id: "abc123".to_string(),
            duration_us: 297_000_000,
            art_url: "file:///tmp/a.jpg".to_string(),
            ..Snapshot::default()
        };
        let map = metadata(&state);
        assert!(map.contains_key("mpris:trackid"));
        assert_eq!(map["mpris:length"], owned(Value::from(297_000_000i64)));
        assert_eq!(map["xesam:title"], owned(Value::from("Idioteque")));
        assert_eq!(
            map["xesam:artist"],
            owned(Value::from(vec!["Radiohead".to_string()]))
        );
        assert!(map.contains_key("mpris:artUrl"));
    }

    #[test]
    fn idle_state_exposes_the_no_track_placeholder() {
        let map = metadata(&Snapshot::default());
        assert_eq!(
            map["mpris:trackid"],
            owned(Value::from(
                ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack").unwrap()
            ))
        );
        assert!(!map.contains_key("mpris:length"));
    }

    #[test]
    fn signal_key_ignores_position_only_updates() {
        let mut state = Snapshot {
            title: "Idioteque".to_string(),
            ..Snapshot::default()
        };
        let before = state.signal_key();
        state.position_us = 42_000_000;
        assert_eq!(before, state.signal_key());

        state.volume = 0.5;
        assert_ne!(before, state.signal_key());
    }
}
