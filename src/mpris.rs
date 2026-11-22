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
//! The playback service publishes a [`Snapshot`] on every player sync; a zbus
//! task owns `org.mpris.MediaPlayer2.musika` and answers panels, media keys and
//! `playerctl`. Controls travel back as plain command strings through a
//! channel the playback service drains on its tick, so every MPRIS action goes
//! through the same dispatcher as keyboard/IPC input.
//!
//! Without a session bus this is a no-op.

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
/// FIX-8: how many queued panel commands one playback-service tick may have
/// to drain before further sends are dropped.
const COMMAND_CAP: usize = 64;

/// What the playback service publishes for the MPRIS task to read.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// `Playing` | `Paused` | `Stopped`.
    pub playback_status: String,
    /// `None` | `Track` | `Playlist`.
    pub loop_status: String,
    pub shuffle: bool,
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

/// The MPRIS defaults for a service with no player state yet: `playback_status`
/// and `loop_status` are enum strings, so an empty `String` would be an invalid
/// property value (D-25).
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            playback_status: "Stopped".to_string(),
            loop_status: "None".to_string(),
            shuffle: false,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            track_id: String::new(),
            duration_us: 0,
            position_us: 0,
            volume: 0.0,
            can_next: false,
            can_prev: false,
            art_url: String::new(),
        }
    }
}

impl Snapshot {
    /// Key for change detection: everything signalled except `Position`.
    pub(crate) fn signal_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.playback_status,
            self.loop_status,
            self.shuffle,
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

/// MPRIS only defines these three playback states; a malformed publisher value
/// reports as `Stopped` instead of exposing an invalid property.
pub(crate) fn playback_status(value: &str) -> &str {
    match value {
        "Playing" | "Paused" | "Stopped" => value,
        _ => "Stopped",
    }
}

/// MPRIS only defines these three loop states; anything else reports as `None`.
pub(crate) fn loop_status(value: &str) -> &str {
    match value {
        "None" | "Track" | "Playlist" => value,
        _ => "None",
    }
}

/// MPRIS `Volume` is `0.0..=1.0`; the core's `mpris_volume:` vocabulary is
/// `0..=100`, so the payload is scaled by 100 (D-01). Non-finite values are
/// rejected: `playerctl volume nan` must not reach the parser as `NaN`.
pub(crate) fn volume_command(volume: f64) -> Option<String> {
    volume
        .is_finite()
        .then(|| format!("mpris_volume:{:.2}", volume.clamp(0.0, 1.0) * 100.0))
}

/// Command for MPRIS `SetShuffle` (D-02). The persistent shuffle *mode* keeps
/// the visible queue order; `shuffle queue` permutes and restarts the queue, so
/// it must not be used here. Only send when the published mode disagrees.
pub(crate) fn shuffle_command(current: bool, shuffle: bool) -> Option<String> {
    (current != shuffle).then(|| {
        if shuffle {
            "mpris_shuffle:on".to_string()
        } else {
            "mpris_shuffle:off".to_string()
        }
    })
}

/// An absolute `SetPosition` target accepted by MPRIS 2.2: negative values are
/// invalid, and a position past the track length (or any position when the
/// track length is unknown) must be ignored.
pub(crate) fn seeks_within_track(position: i64, duration_us: i64) -> Option<i64> {
    (duration_us > 0 && (0..=duration_us).contains(&position)).then_some(position)
}

static SNAPSHOT: OnceLock<Mutex<Snapshot>> = OnceLock::new();
static NOTIFY: OnceLock<Notify> = OnceLock::new();
/// FIX-8: a *bounded* command channel — a D-Bus control burst between two
/// service ticks coalesces by dropping instead of growing the queue; a full
/// buffer simply discards the excess (the newest panel intent matters, a
/// stale `SetVolume`/`SetPosition` under a flood is harmless to lose).
static COMMAND_TX: OnceLock<mpsc::Sender<String>> = OnceLock::new();
/// D-16: graceful-shutdown half of the service. The emitter task watches this
/// channel and drops the D-Bus connection (releasing the bus name) when the
/// daemon stops.
static SHUTDOWN: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();

fn snapshot() -> Snapshot {
    SNAPSHOT
        .get()
        .and_then(|lock| lock.lock().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

/// Publishes the current player state and wakes the signal emitter.
pub fn publish(state: Snapshot) {
    if let Some(lock) = SNAPSHOT.get()
        && let Ok(mut guard) = lock.lock()
    {
        *guard = state;
    }
    if let Some(notify) = NOTIFY.get() {
        notify.notify_one();
    }
}

/// Sends one control command to the playback service (`mpris_*` vocabulary).
/// FIX-8: `try_send` drops when the bounded queue is full.
fn submit(command: String) {
    if let Some(tx) = COMMAND_TX.get() {
        queue_command(tx, command);
    }
}

/// The non-blocking send policy, split out so the bound is testable.
pub(crate) fn queue_command(tx: &mpsc::Sender<String>, command: String) {
    _ = tx.try_send(command);
}

fn submit_action(action: &str) {
    submit(format!("mpris_{action}"));
}

/// Command line for the MPRIS next/previous controls.
///
/// `track next` / `track previous` are two-word commands, so they cannot use
/// the single-word `mpris_*` compatibility vocabulary: prefixing them produced
/// `mpris_track next`, which the parser rejects (the UI silently did nothing).
pub(crate) fn track_command(action: &str) -> String {
    format!("track {action}")
}

fn put_nonempty(map: &mut HashMap<String, OwnedValue>, key: &str, value: &str) {
    if !value.is_empty() {
        map.insert(key.to_string(), owned(Value::from(value.to_string())));
    }
}

/// Starts the service (idempotent; safe without a session bus).
///
/// Control commands are pushed into `commands` (the daemon's command channel)
/// as soon as the D-Bus task receives them: no polling tick, and the bounded
/// queue keeps a panel flood from growing memory (FIX-8).
pub fn init(commands: mpsc::UnboundedSender<String>) {
    let (tx, mut rx) = mpsc::channel(COMMAND_CAP);
    _ = COMMAND_TX.set(tx);
    _ = SNAPSHOT.set(Mutex::new(Snapshot::default()));
    _ = NOTIFY.set(Notify::new());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    _ = SHUTDOWN.set(shutdown_tx);
    crate::app::runtime().spawn(async move {
        while let Some(command) = rx.recv().await {
            if commands.send(command).is_err() {
                break;
            }
        }
    });
    crate::app::runtime().spawn(serve(shutdown_rx));
}

/// Asks the MPRIS service to stop: the emitter task ends and drops its bus
/// connection, which releases `org.mpris.MediaPlayer2.musika`. Idempotent, and
/// a no-op if MPRIS was never initialised (D-16).
pub fn shutdown() {
    if let Some(tx) = SHUTDOWN.get() {
        _ = tx.send(true);
    }
}

async fn serve(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        eprintln!("mpris: no session bus; native MPRIS disabled");
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

    let Some(notify) = NOTIFY.get() else {
        return;
    };
    tokio::select! {
        () = emit_changes(&connection, notify) => {}
        // Dropping the future above cancels the emitter; returning drops the
        // connection, which releases the well-known name.
        _ = shutdown.wait_for(|stop| *stop) => {}
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
    // Trailing-edge rate limit (D-25): the first change after a quiet period is
    // published immediately; a burst is coalesced into one emission per
    // RATE_LIMIT window so panels still see the final state promptly.
    let mut last_emit: Option<tokio::time::Instant> = None;
    loop {
        notify.notified().await;
        if let Some(at) = last_emit {
            let now = tokio::time::Instant::now();
            if now < at + RATE_LIMIT {
                tokio::time::sleep_until(at + RATE_LIMIT).await;
            }
        }
        let state = snapshot();
        let key = state.signal_key();
        if key == last {
            continue;
        }
        last = key;
        last_emit = Some(tokio::time::Instant::now());

        let Ok(name) = InterfaceName::try_from(PLAYER_INTERFACE) else {
            return;
        };
        let mut changed: HashMap<&str, Value<'_>> = HashMap::new();
        changed.insert("Metadata", Value::from(metadata(&state)));
        changed.insert(
            "PlaybackStatus",
            Value::from(playback_status(&state.playback_status)),
        );
        changed.insert("LoopStatus", Value::from(loop_status(&state.loop_status)));
        changed.insert("Shuffle", Value::from(state.shuffle));
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

pub(crate) fn owned(value: Value<'_>) -> OwnedValue {
    value.try_into().unwrap_or_else(|_| OwnedValue::from(0u8))
}

/// MPRIS object path for a song id (segments allow `[A-Za-z0-9_]` only).
/// The empty id maps to the spec's "no track" placeholder.
pub(crate) fn track_path(song_id: &str) -> String {
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

pub(crate) fn metadata(state: &Snapshot) -> HashMap<String, OwnedValue> {
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
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
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

    fn raise(&self) {
        submit("show".to_string());
    }

    fn quit(&self) {
        submit("quit".to_string());
    }
}

struct MprisPlayer;

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl MprisPlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        playback_status(&snapshot().playback_status).to_string()
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        loop_status(&snapshot().loop_status).to_string()
    }

    #[zbus(property)]
    fn set_loop_status(&self, status: String) {
        let published = snapshot().loop_status;
        let current = match loop_status(&published) {
            "Track" => 1,
            "Playlist" => 2,
            _ => 0,
        };
        let target = match status.as_str() {
            "None" => 0,
            "Track" => 1,
            "Playlist" => 2,
            _ => return,
        };
        for _ in 0..(target + 3 - current) % 3 {
            submit("repeat_cycle".to_string());
        }
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn set_rate(&self, _rate: f64) {}

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        snapshot().shuffle
    }

    #[zbus(property)]
    fn set_shuffle(&self, shuffle: bool) {
        // The persistent mode, not `shuffle queue` (which permutes/restarts the
        // queue); only submit when the published mode disagrees (D-02).
        if let Some(command) = shuffle_command(snapshot().shuffle, shuffle) {
            submit(command);
        }
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&snapshot())
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        snapshot().volume
    }

    #[zbus(property)]
    fn set_volume(&self, volume: f64) {
        if let Some(command) = volume_command(volume) {
            submit(command);
        }
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
        submit_action("play_pause");
    }

    fn stop(&self) {
        submit_action("stop");
    }

    fn next(&self) {
        submit(track_command("next"));
    }

    fn previous(&self) {
        submit(track_command("previous"));
    }

    fn seek(&self, offset: i64) {
        // MPRIS 2.2: there is nothing to seek when the track length is unknown
        // (`CanSeek` is false). The applied position is only authoritative in
        // the core, which publishes it with the next snapshot, so an eager
        // `Seeked` is not emitted from this possibly stale snapshot (D-25).
        if snapshot().duration_us <= 0 {
            return;
        }
        submit(format!("mpris_seek:{offset}"));
    }

    async fn set_position(
        &self,
        track_id: ObjectPath<'_>,
        position: i64,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let state = snapshot();
        let expected = track_path(&state.track_id);
        if track_id.as_str() != expected {
            return;
        }
        // Negative, past-the-end or unknown-length positions are ignored per
        // MPRIS 2.2; an accepted absolute position is what the core is asked to
        // apply, so it is safe to announce right away (D-25).
        let Some(position) = seeks_within_track(position, state.duration_us) else {
            return;
        };
        submit(format!("mpris_position:{position}"));
        _ = MprisPlayer::seeked(&emitter, position).await;
    }

    #[zbus(signal)]
    async fn seeked(signal_emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}
