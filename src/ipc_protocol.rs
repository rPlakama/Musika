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

//! Core/UI IPC protocol v2: framed messages over a Unix socket.
//!
//! The daemon (`musika`) owns playback and all state; `musika-ui` is a client.
//! This module is the one wire contract between them:
//!
//! ```text
//! frame := u32 payload_len | u8 kind | u8 flags | u16 reserved | payload
//! ```
//!
//! All integers are little-endian and the frame header is 8 bytes. Payloads
//! are either fixed-layout binary (the hot state/queue/transport path) or
//! `serde_json` documents (the cold bridge surface and the handshake). The
//! protocol is versioned through [`Hello`]/[`Welcome`]; a peer with a different
//! [`VERSION`] is rejected instead of being allowed to desync.
//!
//! Hot-path rules that the rest of the crate relies on:
//!
//! - [`PlaybackState`] is a fixed 48-byte snapshot of transport state. Its
//!   position is an anchor: `(position_ms, anchor_ns, rate_milli)`. Readers
//!   extrapolate with [`PlaybackState::position_at`] instead of receiving a
//!   tick ten times per second.
//! - [`QueueDelta`] carries row changes; [`QueueRows`] carries the row bodies
//!   in bounded chunks. The full queue is never re-sent on a row edit.
//! - [`WireTrack`] intentionally omits the resolved stream URL: the UI renders
//!   metadata only, and the URL carries provider credentials.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::bridge::Event;
use crate::player::{Repeat, State, Track};

/// Wire protocol version. Bumped whenever a message layout changes.
pub const VERSION: u32 = 2;

/// Largest accepted frame. The queue row chunks are sized well below this.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Number of queue rows per [`QueueRows`] frame.
pub const QUEUE_CHUNK_ROWS: usize = 256;

/// Maximum rows accepted inside a [`QueueDelta::Insert`].
pub const QUEUE_INSERT_MAX_ROWS: usize = QUEUE_CHUNK_ROWS;

/// Bytes of the fixed frame header.
pub const FRAME_HEADER_BYTES: usize = 8;

pub const FLAG_REPLY: u8 = 1 << 0;
pub const FLAG_ERROR: u8 = 1 << 1;

// Frame kinds. The hot path is binary; the control path is JSON.
pub const KIND_REQUEST_JSON: u8 = 0x01;
pub const KIND_REPLY_JSON: u8 = 0x02;
pub const KIND_EVENT_JSON: u8 = 0x03;
pub const KIND_PLAYER_REQUEST: u8 = 0x10;
pub const KIND_PLAYER_REPLY: u8 = 0x11;
pub const KIND_PLAYBACK_STATE: u8 = 0x12;
pub const KIND_QUEUE_DELTA: u8 = 0x13;
pub const KIND_QUEUE_ROWS: u8 = 0x14;
pub const KIND_HELLO: u8 = 0x20;
pub const KIND_WELCOME: u8 = 0x21;
pub const KIND_REJECT: u8 = 0x22;
pub const KIND_PING: u8 = 0x23;
pub const KIND_PONG: u8 = 0x24;
pub const KIND_BYE: u8 = 0x25;
pub const KIND_PROTOCOL_ERROR: u8 = 0x26;
pub const KIND_ACTIVATE: u8 = 0x27;

/// Client feature bits exchanged in [`Hello`]/[`Welcome`].
pub const FEATURE_PLAYBACK_STREAM: u32 = 1 << 0;
pub const FEATURE_QUEUE_STREAM: u32 = 1 << 1;
pub const FEATURE_QUEUE_QUERY: u32 = 1 << 2;
pub const FEATURE_CONTROL_REPLY: u32 = 1 << 3;

/// Features a UI client wants: the full playback and queue stream.
pub const UI_FEATURES: u32 =
    FEATURE_PLAYBACK_STREAM | FEATURE_QUEUE_STREAM | FEATURE_QUEUE_QUERY;
/// Features a control client wants: acks for requests, no bulk streams.
pub const CONTROL_FEATURES: u32 = FEATURE_CONTROL_REPLY;

/// Transport playback state on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateWire {
    #[default]
    Stopped,
    Playing,
    Paused,
}

impl From<State> for StateWire {
    fn from(state: State) -> Self {
        match state {
            State::Stopped => Self::Stopped,
            State::Playing => Self::Playing,
            State::Paused => Self::Paused,
        }
    }
}

impl From<StateWire> for State {
    fn from(state: StateWire) -> Self {
        match state {
            StateWire::Stopped => Self::Stopped,
            StateWire::Playing => Self::Playing,
            StateWire::Paused => Self::Paused,
        }
    }
}

/// Repeat mode on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatWire {
    #[default]
    Off,
    One,
    All,
}

impl From<Repeat> for RepeatWire {
    fn from(repeat: Repeat) -> Self {
        match repeat {
            Repeat::Off => Self::Off,
            Repeat::One => Self::One,
            Repeat::All => Self::All,
        }
    }
}

impl From<RepeatWire> for Repeat {
    fn from(repeat: RepeatWire) -> Self {
        match repeat {
            RepeatWire::Off => Repeat::Off,
            RepeatWire::One => Repeat::One,
            RepeatWire::All => Repeat::All,
        }
    }
}

/// Who is on the other end of a daemon socket connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// `musika-ui`: renders playback and queue streams, sends UI requests.
    Ui,
    /// External tooling: method-shaped requests, acks, no bulk streams.
    Control,
}

impl Role {
    pub fn default_features(self) -> u32 {
        match self {
            Self::Ui => UI_FEATURES,
            Self::Control => CONTROL_FEATURES,
        }
    }
}

/// Typed error codes shared by replies and control frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    TooLarge,
    UnknownKind,
    BadPayload,
    VersionMismatch,
    UnknownRequest,
    InvalidArgs,
    NoUiAttached,
    EngineUnavailable,
    QueueRevMismatch,
    DuplicateRequest,
    SlowConsumer,
    Internal,
}

impl ErrorCode {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::BadFrame => 1,
            Self::TooLarge => 2,
            Self::UnknownKind => 3,
            Self::BadPayload => 4,
            Self::VersionMismatch => 5,
            Self::UnknownRequest => 6,
            Self::InvalidArgs => 7,
            Self::NoUiAttached => 8,
            Self::EngineUnavailable => 9,
            Self::QueueRevMismatch => 10,
            Self::DuplicateRequest => 11,
            Self::SlowConsumer => 12,
            Self::Internal => 13,
        }
    }

    pub fn from_u8(code: u8) -> Self {
        match code {
            1 => Self::BadFrame,
            2 => Self::TooLarge,
            3 => Self::UnknownKind,
            4 => Self::BadPayload,
            5 => Self::VersionMismatch,
            6 => Self::UnknownRequest,
            7 => Self::InvalidArgs,
            8 => Self::NoUiAttached,
            9 => Self::EngineUnavailable,
            10 => Self::QueueRevMismatch,
            11 => Self::DuplicateRequest,
            12 => Self::SlowConsumer,
            _ => Self::Internal,
        }
    }
}

/// Why the daemon closed a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByeReason {
    /// The user asked the core to quit; the UI should close its window.
    Quit,
    /// The core is shutting down without a user request (e.g. an error).
    Shutdown,
    /// The peer violated the protocol.
    Protocol,
    /// The peer runs a different protocol version.
    VersionMismatch,
    /// The peer stopped draining its outbound queue.
    SlowConsumer,
    /// The heartbeat timed out.
    Heartbeat,
    /// A second connection replaced this one (single-window takeover).
    Replaced,
}

/// Requested reply-expected JSON request (the bridge surface).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestJson {
    pub req: u32,
    pub method: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

/// Reply to a [`RequestJson`] or [`PlayerRequest`]. Results travel as events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyJson {
    pub req: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ReplyJson {
    pub fn ok(req: u32) -> Self {
        Self {
            req,
            ok: true,
            code: None,
            message: None,
        }
    }

    pub fn err(req: u32, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            req,
            ok: false,
            code: Some(code),
            message: Some(message.into()),
        }
    }
}

/// Handshake sent by every client before any other frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    pub role: Role,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub features: u32,
}

/// Daemon answer to an accepted [`Hello`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Welcome {
    pub proto: u32,
    pub core_version: String,
    /// Random per-daemon-run id; changes across restarts.
    pub session: u64,
    pub features: u32,
}

/// Daemon answer to a rejected [`Hello`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reject {
    pub code: ErrorCode,
    pub message: String,
}

/// Clean close notification (never used for transport errors).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bye {
    pub reason: ByeReason,
}

/// Fatal protocol error (sent just before the daemon closes the connection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn io(&self) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("protocol error {:?}: {}", self.code, self.message),
        )
    }
}

/// Heartbeat payload. Both sides echo `nonce`; `monotonic_ns` allows a round
/// trip time measurement without assuming clock synchronisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heartbeat {
    pub nonce: u64,
    pub monotonic_ns: u64,
}

/// A UI-facing row. No stream URL: the UI only renders metadata.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WireTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    #[serde(default)]
    pub album_id: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub cover_art: String,
    #[serde(default)]
    pub cover_url: String,
    #[serde(default)]
    pub cover_remote: String,
}

impl From<&Track> for WireTrack {
    fn from(track: &Track) -> Self {
        Self {
            artist: track.artist.clone(),
            album: track.album.clone(),
            favorite: track.favorite,
            duration: track.duration,
            cover_art: track.cover_art.clone(),
            ..Self::from_meta(&track.item)
        }
    }
}

impl WireTrack {
    fn from_meta(item: &crate::model::ItemMeta) -> Self {
        Self {
            id: item.id.clone(),
            title: item.title.clone(),
            album_id: item.album_id.clone(),
            cover_url: item.cover_url.clone(),
            cover_remote: item.cover_remote.clone(),
            ..Self::default()
        }
    }
}

impl From<WireTrack> for Track {
    fn from(track: WireTrack) -> Self {
        Self {
            artist: track.artist,
            album: track.album,
            favorite: track.favorite,
            duration: track.duration,
            url: String::new(),
            cover_art: track.cover_art,
            item: crate::model::ItemMeta {
                id: track.id,
                title: track.title,
                album_id: track.album_id,
                cover_url: track.cover_url,
                cover_remote: track.cover_remote,
            },
        }
    }
}

/// A hot-path request from the UI. The core owns the queue rows, so no variant
/// carries track bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRequest {
    pub req: u32,
    pub op: PlayerOp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerOp {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SetVolume(f64),
    VolumeUp,
    VolumeDown,
    ToggleMute,
    SetMuted(bool),
    Seek(f64),
    SeekRelative(f64),
    Jump(u32),
    Remove(u32),
    RemoveMany(Vec<u32>),
    Move { from: u32, to: u32 },
    MoveBlock { start: u32, len: u32, delta: i32 },
    CycleRepeat,
    ShuffleQueue,
    SetTrackFavorite { id: String, favorite: bool },
    /// Ask the core to replay the playback state and the queue rows.
    Resync,
}

impl PlayerOp {
    fn tag(&self) -> u8 {
        match self {
            Self::PlayPause => 0,
            Self::Play => 1,
            Self::Pause => 2,
            Self::Stop => 3,
            Self::Next => 4,
            Self::Previous => 5,
            Self::SetVolume(_) => 6,
            Self::VolumeUp => 7,
            Self::VolumeDown => 8,
            Self::ToggleMute => 9,
            Self::SetMuted(_) => 10,
            Self::Seek(_) => 11,
            Self::SeekRelative(_) => 12,
            Self::Jump(_) => 13,
            Self::Remove(_) => 14,
            Self::RemoveMany(_) => 15,
            Self::Move { .. } => 16,
            Self::MoveBlock { .. } => 17,
            Self::CycleRepeat => 18,
            Self::ShuffleQueue => 19,
            Self::SetTrackFavorite { .. } => 20,
            Self::Resync => 21,
        }
    }
}

/// Ack for a [`PlayerRequest`]; the resulting state arrives as a
/// [`PlaybackState`] frame, never inside the reply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerReply {
    pub req: u32,
    pub ok: bool,
    pub code: ErrorCode,
    /// Sequence of the playback state at which the command was applied.
    pub applied_seq: u64,
}

/// Fixed 48-byte playback snapshot. `anchor_ns` is `CLOCK_MONOTONIC` in
/// nanoseconds (see [`crate::linux::monotonic_ns`]); readers extrapolate the
/// displayed position with [`Self::position_at`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaybackState {
    /// Monotonic per-daemon sequence of playback state changes.
    pub seq: u64,
    /// Monotonic queue revision; matches the latest [`QueueDelta::rev`].
    pub queue_rev: u64,
    pub anchor_ns: u64,
    pub index: u32,
    pub duration_ms: u32,
    pub position_ms: u32,
    pub flags: u32,
    pub volume_milli: u16,
    pub state: u8,
    pub repeat: u8,
    pub rate_milli: u16,
    pub reserved: u16,
}

pub const STATE_FLAG_MUTED: u32 = 1 << 0;
pub const STATE_FLAG_LOADING: u32 = 1 << 1;
pub const STATE_FLAG_GAPLESS_ARMED: u32 = 1 << 2;
pub const STATE_FLAG_ENGINE_UNAVAILABLE: u32 = 1 << 3;
pub const STATE_FLAG_SEEKING: u32 = 1 << 4;
pub const STATE_FLAG_ENDED: u32 = 1 << 5;

impl PlaybackState {
    pub const ENCODED_BYTES: usize = 48;

    pub fn state(&self) -> StateWire {
        match self.state {
            1 => StateWire::Playing,
            2 => StateWire::Paused,
            _ => StateWire::Stopped,
        }
    }

    pub fn repeat(&self) -> RepeatWire {
        match self.repeat {
            1 => RepeatWire::One,
            2 => RepeatWire::All,
            _ => RepeatWire::Off,
        }
    }

    pub fn muted(&self) -> bool {
        self.flags & STATE_FLAG_MUTED != 0
    }

    pub fn loading(&self) -> bool {
        self.flags & STATE_FLAG_LOADING != 0
    }

    pub fn engine_unavailable(&self) -> bool {
        self.flags & STATE_FLAG_ENGINE_UNAVAILABLE != 0
    }

    pub fn seeking(&self) -> bool {
        self.flags & STATE_FLAG_SEEKING != 0
    }

    pub fn volume(&self) -> f64 {
        f64::from(self.volume_milli) / 10.0
    }

    pub fn position(&self) -> f64 {
        f64::from(self.position_ms) / 1000.0
    }

    pub fn duration(&self) -> f64 {
        f64::from(self.duration_ms) / 1000.0
    }

    /// Extrapolated playhead at `now_ns` (also `CLOCK_MONOTONIC`).
    pub fn position_at(&self, now_ns: u64) -> f64 {
        let anchor = self.position();
        if self.state() != StateWire::Playing || self.rate_milli == 0 {
            return anchor;
        }
        let elapsed_ns = now_ns.saturating_sub(self.anchor_ns);
        let elapsed = elapsed_ns as f64 * f64::from(self.rate_milli) / 1_000_000_000_000.0;
        let position = anchor + elapsed;
        let duration = self.duration();
        if duration > 0.0 {
            position.min(duration)
        } else {
            position
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::ENCODED_BYTES);
        w.u64(self.seq);
        w.u64(self.queue_rev);
        w.u64(self.anchor_ns);
        w.u32(self.index);
        w.u32(self.duration_ms);
        w.u32(self.position_ms);
        w.u32(self.flags);
        w.u16(self.volume_milli);
        w.u8(self.state);
        w.u8(self.repeat);
        w.u16(self.rate_milli);
        w.u16(self.reserved);
        debug_assert_eq!(w.0.len(), Self::ENCODED_BYTES);
        w.0
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != Self::ENCODED_BYTES {
            return Err(bad_payload("playback state", Self::ENCODED_BYTES, payload.len()));
        }
        let mut r = Reader::new(payload);
        Ok(Self {
            seq: r.u64()?,
            queue_rev: r.u64()?,
            anchor_ns: r.u64()?,
            index: r.u32()?,
            duration_ms: r.u32()?,
            position_ms: r.u32()?,
            flags: r.u32()?,
            volume_milli: r.u16()?,
            state: r.u8()?,
            repeat: r.u8()?,
            rate_milli: r.u16()?,
            reserved: r.u16()?,
        })
    }
}

/// One row-level queue change. [`QueueOp::Reset`] is followed by [`QueueRows`]
/// frames carrying the row bodies in bounded chunks; every other op carries its
/// own rows or indices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueueDelta {
    pub rev: u64,
    pub op: QueueOp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueueOp {
    /// The queue was replaced. Row bodies follow in `QueueRows` chunks.
    Reset {
        count: u32,
    },
    /// A contiguous window of the current queue (resync / first load).
    Rows {
        start: u32,
        rows: Vec<WireTrack>,
    },
    /// Rows inserted at `at`; the inserted rows are included.
    Insert {
        at: u32,
        rows: Vec<WireTrack>,
    },
    /// Existing rows removed by their positions (ascending, deduped).
    Remove {
        indices: Vec<u32>,
    },
    /// One row moved.
    Move {
        from: u32,
        to: u32,
    },
    /// A metadata-only change (favourite marks).
    Patch {
        index: u32,
        favorite: bool,
    },
}

impl Default for QueueOp {
    fn default() -> Self {
        Self::Reset { count: 0 }
    }
}

impl QueueDelta {
    pub fn reset(rev: u64, count: usize) -> Self {
        Self {
            rev,
            op: QueueOp::Reset {
                count: count as u32,
            },
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut w = Writer::with_capacity(32);
        w.u64(self.rev);
        match &self.op {
            QueueOp::Reset { count } => {
                w.u8(0);
                w.u32(*count);
            }
            QueueOp::Rows { start, rows } => {
                w.u8(1);
                w.u32(*start);
                write_rows(&mut w, rows)?;
            }
            QueueOp::Insert { at, rows } => {
                if rows.len() > QUEUE_INSERT_MAX_ROWS {
                    return Err(ProtocolError::new(
                        ErrorCode::TooLarge,
                        format!(
                            "queue insert carries {} rows, max {}",
                            rows.len(),
                            QUEUE_INSERT_MAX_ROWS
                        ),
                    ));
                }
                w.u8(2);
                w.u32(*at);
                write_rows(&mut w, rows)?;
            }
            QueueOp::Remove { indices } => {
                w.u8(3);
                w.u16(indices.len() as u16);
                for index in indices {
                    w.u32(*index);
                }
            }
            QueueOp::Move { from, to } => {
                w.u8(4);
                w.u32(*from);
                w.u32(*to);
            }
            QueueOp::Patch { index, favorite } => {
                w.u8(5);
                w.u32(*index);
                w.u8(u8::from(*favorite));
            }
        }
        Ok(w.0)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(payload);
        let rev = r.u64()?;
        let op = match r.u8()? {
            0 => QueueOp::Reset { count: r.u32()? },
            1 => {
                let start = r.u32()?;
                QueueOp::Rows {
                    start,
                    rows: read_rows(&mut r)?,
                }
            }
            2 => {
                let at = r.u32()?;
                let rows = read_rows(&mut r)?;
                if rows.len() > QUEUE_INSERT_MAX_ROWS {
                    return Err(ProtocolError::new(
                        ErrorCode::TooLarge,
                        "queue insert rows exceed the frame limit",
                    ));
                }
                QueueOp::Insert { at, rows }
            }
            3 => {
                let count = r.u16()? as usize;
                let mut indices = Vec::with_capacity(count);
                for _ in 0..count {
                    indices.push(r.u32()?);
                }
                QueueOp::Remove { indices }
            }
            4 => QueueOp::Move {
                from: r.u32()?,
                to: r.u32()?,
            },
            5 => QueueOp::Patch {
                index: r.u32()?,
                favorite: r.u8()? != 0,
            },
            tag => {
                return Err(ProtocolError::new(
                    ErrorCode::BadPayload,
                    format!("unknown queue delta op {tag}"),
                ));
            }
        };
        Ok(Self { rev, op })
    }
}

/// Row bodies for the queue at a revision, in chunks of [`QUEUE_CHUNK_ROWS`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueRows {
    pub rev: u64,
    pub start: u32,
    pub rows: Vec<WireTrack>,
}

impl QueueRows {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        serde_json::to_vec(self)
            .map_err(|error| ProtocolError::new(ErrorCode::BadPayload, error.to_string()))
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        serde_json::from_slice(payload)
            .map_err(|error| ProtocolError::new(ErrorCode::BadPayload, error.to_string()))
    }
}

fn write_rows(w: &mut Writer, rows: &[WireTrack]) -> Result<(), ProtocolError> {
    let json = serde_json::to_vec(rows)
        .map_err(|error| ProtocolError::new(ErrorCode::BadPayload, error.to_string()))?;
    w.u32(json.len() as u32);
    w.0.extend_from_slice(&json);
    Ok(())
}

fn read_rows(r: &mut Reader<'_>) -> Result<Vec<WireTrack>, ProtocolError> {
    let len = r.u32()? as usize;
    let bytes = r.take(len)?;
    serde_json::from_slice(bytes)
        .map_err(|error| ProtocolError::new(ErrorCode::BadPayload, error.to_string()))
}

fn bad_payload(what: &str, expected: usize, actual: usize) -> ProtocolError {
    ProtocolError::new(
        ErrorCode::BadPayload,
        format!("{what}: expected {expected} bytes, got {actual}"),
    )
}

/// A decoded frame: the header already validated, payload opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(kind: u8, payload: Vec<u8>) -> Self {
        Self {
            kind,
            flags: 0,
            payload,
        }
    }

    pub fn with_flags(kind: u8, flags: u8, payload: Vec<u8>) -> Self {
        Self {
            kind,
            flags,
            payload,
        }
    }

    /// Serializes the frame, header included, into one buffer so the writer
    /// can hand it to a single `write_all`.
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        if self.payload.len() > MAX_FRAME_BYTES {
            return Err(ProtocolError::new(
                ErrorCode::TooLarge,
                format!(
                    "frame payload {} exceeds {} bytes",
                    self.payload.len(),
                    MAX_FRAME_BYTES
                ),
            ));
        }
        let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + self.payload.len());
        out.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        out.push(self.kind);
        out.push(self.flags);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    /// Parses a header and returns `(payload_len, kind, flags)`.
    pub fn parse_header(header: &[u8]) -> Result<(usize, u8, u8), ProtocolError> {
        if header.len() != FRAME_HEADER_BYTES {
            return Err(ProtocolError::new(
                ErrorCode::BadFrame,
                "short frame header",
            ));
        }
        let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if len > MAX_FRAME_BYTES {
            return Err(ProtocolError::new(
                ErrorCode::TooLarge,
                format!("frame payload {len} exceeds {MAX_FRAME_BYTES} bytes"),
            ));
        }
        Ok((len, header[4], header[5]))
    }

    pub fn decode(header: &[u8], payload: Vec<u8>) -> Result<Self, ProtocolError> {
        let (len, kind, flags) = Self::parse_header(header)?;
        if payload.len() != len {
            return Err(ProtocolError::new(
                ErrorCode::BadFrame,
                format!("frame payload length {len} but got {}", payload.len()),
            ));
        }
        Ok(Self {
            kind,
            flags,
            payload,
        })
    }
}

/// Async frame reader for tokio tasks. `Ok(None)` is a clean EOF.
pub async fn read_frame<R>(reader: &mut R) -> io::Result<Option<Frame>>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; FRAME_HEADER_BYTES];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let (len, kind, flags) = Frame::parse_header(&header).map_err(|error| error.io())?;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Some(Frame {
        kind,
        flags,
        payload,
    }))
}

/// Async frame writer. The frame is serialized to one buffer first, so a frame
/// is never partially interleaved with another writer.
pub async fn write_frame<W>(writer: &mut W, frame: &Frame) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let bytes = frame.encode().map_err(|error| error.io())?;
    writer.write_all(&bytes).await
}

/// Blocking frame reader for the UI reader thread.
pub fn read_frame_blocking<R: Read>(reader: &mut R) -> io::Result<Option<Frame>> {
    let mut header = [0u8; FRAME_HEADER_BYTES];
    match reader.read_exact(&mut header) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let (len, kind, flags) = Frame::parse_header(&header).map_err(|error| error.io())?;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok(Some(Frame {
        kind,
        flags,
        payload,
    }))
}

/// Blocking frame writer for the UI writer thread.
pub fn write_frame_blocking<W: Write>(writer: &mut W, frame: &Frame) -> io::Result<()> {
    let bytes = frame.encode().map_err(|error| error.io())?;
    writer.write_all(&bytes)
}

impl PlayerRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(24);
        w.u32(self.req);
        w.u8(self.op.tag());
        match &self.op {
            PlayerOp::SetVolume(volume) | PlayerOp::Seek(volume) | PlayerOp::SeekRelative(volume) => {
                w.f64(*volume)
            }
            PlayerOp::Jump(index) | PlayerOp::Remove(index) => w.u32(*index),
            PlayerOp::RemoveMany(indices) => {
                w.u16(indices.len() as u16);
                for index in indices {
                    w.u32(*index);
                }
            }
            PlayerOp::Move { from, to } => {
                w.u32(*from);
                w.u32(*to);
            }
            PlayerOp::MoveBlock { start, len, delta } => {
                w.u32(*start);
                w.u32(*len);
                w.i32(*delta);
            }
            PlayerOp::SetMuted(muted) => w.u8(u8::from(*muted)),
            PlayerOp::SetTrackFavorite { id, favorite } => {
                w.string(id);
                w.u8(u8::from(*favorite));
            }
            PlayerOp::PlayPause
            | PlayerOp::Play
            | PlayerOp::Pause
            | PlayerOp::Stop
            | PlayerOp::Next
            | PlayerOp::Previous
            | PlayerOp::VolumeUp
            | PlayerOp::VolumeDown
            | PlayerOp::ToggleMute
            | PlayerOp::CycleRepeat
            | PlayerOp::ShuffleQueue
            | PlayerOp::Resync => {}
        }
        w.0
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(payload);
        let req = r.u32()?;
        let op = match r.u8()? {
            0 => PlayerOp::PlayPause,
            1 => PlayerOp::Play,
            2 => PlayerOp::Pause,
            3 => PlayerOp::Stop,
            4 => PlayerOp::Next,
            5 => PlayerOp::Previous,
            6 => PlayerOp::SetVolume(r.f64()?),
            7 => PlayerOp::VolumeUp,
            8 => PlayerOp::VolumeDown,
            9 => PlayerOp::ToggleMute,
            10 => PlayerOp::SetMuted(r.u8()? != 0),
            11 => PlayerOp::Seek(r.f64()?),
            12 => PlayerOp::SeekRelative(r.f64()?),
            13 => PlayerOp::Jump(r.u32()?),
            14 => PlayerOp::Remove(r.u32()?),
            15 => {
                let count = r.u16()? as usize;
                let mut indices = Vec::with_capacity(count);
                for _ in 0..count {
                    indices.push(r.u32()?);
                }
                PlayerOp::RemoveMany(indices)
            }
            16 => PlayerOp::Move {
                from: r.u32()?,
                to: r.u32()?,
            },
            17 => PlayerOp::MoveBlock {
                start: r.u32()?,
                len: r.u32()?,
                delta: r.i32()?,
            },
            18 => PlayerOp::CycleRepeat,
            19 => PlayerOp::ShuffleQueue,
            20 => PlayerOp::SetTrackFavorite {
                id: r.string()?,
                favorite: r.u8()? != 0,
            },
            21 => PlayerOp::Resync,
            tag => {
                return Err(ProtocolError::new(
                    ErrorCode::BadPayload,
                    format!("unknown player op {tag}"),
                ));
            }
        };
        Ok(Self { req, op })
    }
}

impl PlayerReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(14);
        w.u32(self.req);
        w.u8(u8::from(self.ok));
        w.u8(self.code.as_u8());
        w.u64(self.applied_seq);
        w.0
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(payload);
        Ok(Self {
            req: r.u32()?,
            ok: r.u8()? != 0,
            code: ErrorCode::from_u8(r.u8()?),
            applied_seq: r.u64()?,
        })
    }
}

impl Heartbeat {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(16);
        w.u64(self.nonce);
        w.u64(self.monotonic_ns);
        w.0
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != 16 {
            return Err(bad_payload("heartbeat", 16, payload.len()));
        }
        let mut r = Reader::new(payload);
        Ok(Self {
            nonce: r.u64()?,
            monotonic_ns: r.u64()?,
        })
    }
}

/// JSON codec helper for the control frames.
pub fn json_encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(value).map_err(|error| ProtocolError::new(ErrorCode::BadPayload, error.to_string()))
}

pub fn json_decode<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> Result<T, ProtocolError> {
    serde_json::from_slice(payload)
        .map_err(|error| ProtocolError::new(ErrorCode::BadPayload, error.to_string()))
}

/// Event frame helper: bridge events travel as JSON documents.
pub fn event_frame(event: &Event) -> Result<Frame, ProtocolError> {
    Ok(Frame::new(KIND_EVENT_JSON, json_encode(event)?))
}

fn event_decode(payload: &[u8]) -> Result<Event, ProtocolError> {
    json_decode(payload)
}

/// The daemon-side classification of a frame's payload. Keeping it explicit
/// makes the dispatch exhaustive and testable.
#[derive(Debug)]
pub enum Message {
    Hello(Hello),
    Welcome(Welcome),
    Reject(Reject),
    Bye(Bye),
    ProtocolError(ProtocolError),
    /// Ask a UI client to raise/activate its window (`show`).
    Activate,
    Ping(Heartbeat),
    Pong(Heartbeat),
    Request(RequestJson),
    Reply(ReplyJson),
    Event(Event),
    PlayerRequest(PlayerRequest),
    PlayerReply(PlayerReply),
    PlaybackState(PlaybackState),
    QueueDelta(QueueDelta),
    QueueRows(QueueRows),
}

impl Message {
    pub fn decode(frame: &Frame) -> Result<Self, ProtocolError> {
        match frame.kind {
            KIND_HELLO => Ok(Self::Hello(json_decode(&frame.payload)?)),
            KIND_WELCOME => Ok(Self::Welcome(json_decode(&frame.payload)?)),
            KIND_REJECT => Ok(Self::Reject(json_decode(&frame.payload)?)),
            KIND_BYE => Ok(Self::Bye(json_decode(&frame.payload)?)),
            KIND_PROTOCOL_ERROR => Ok(Self::ProtocolError(json_decode(&frame.payload)?)),
            KIND_ACTIVATE => {
                if frame.payload.is_empty() {
                    Ok(Self::Activate)
                } else {
                    Err(ProtocolError::new(
                        ErrorCode::BadPayload,
                        "activate frame must have an empty payload",
                    ))
                }
            }
            KIND_PING => Ok(Self::Ping(Heartbeat::decode(&frame.payload)?)),
            KIND_PONG => Ok(Self::Pong(Heartbeat::decode(&frame.payload)?)),
            KIND_REQUEST_JSON => Ok(Self::Request(json_decode(&frame.payload)?)),
            KIND_REPLY_JSON => Ok(Self::Reply(json_decode(&frame.payload)?)),
            KIND_EVENT_JSON => Ok(Self::Event(event_decode(&frame.payload)?)),
            KIND_PLAYER_REQUEST => Ok(Self::PlayerRequest(PlayerRequest::decode(&frame.payload)?)),
            KIND_PLAYER_REPLY => Ok(Self::PlayerReply(PlayerReply::decode(&frame.payload)?)),
            KIND_PLAYBACK_STATE => Ok(Self::PlaybackState(PlaybackState::decode(&frame.payload)?)),
            KIND_QUEUE_DELTA => Ok(Self::QueueDelta(QueueDelta::decode(&frame.payload)?)),
            KIND_QUEUE_ROWS => Ok(Self::QueueRows(QueueRows::decode(&frame.payload)?)),
            kind => Err(ProtocolError::new(
                ErrorCode::UnknownKind,
                format!("unknown frame kind 0x{kind:02x}"),
            )),
        }
    }
}

// --- small, explicit little-endian codecs ---------------------------------

struct Writer(Vec<u8>);

impl Writer {
    fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn i32(&mut self, value: i32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn f64(&mut self, value: f64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn string(&mut self, value: &str) {
        self.u32(value.len() as u32);
        self.0.extend_from_slice(value.as_bytes());
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn short(&self, want: usize) -> ProtocolError {
        ProtocolError::new(
            ErrorCode::BadPayload,
            format!(
                "payload too short at {}: wanted {want} more bytes, have {}",
                self.pos,
                self.bytes.len().saturating_sub(self.pos)
            ),
        )
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| self.short(len))?;
        if end > self.bytes.len() {
            return Err(self.short(len));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtocolError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    fn u32(&mut self) -> Result<u32, ProtocolError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn i32(&mut self) -> Result<i32, ProtocolError> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64, ProtocolError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
    fn f64(&mut self) -> Result<f64, ProtocolError> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn string(&mut self) -> Result<String, ProtocolError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|error| ProtocolError::new(ErrorCode::BadPayload, error.to_string()))
    }
}
