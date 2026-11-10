//! Core/UI IPC protocol (daemon split).
//!
//! JSON lines over a Unix socket. The UI sends [`UiMessage`] requests; the
//! core answers with [`CoreMessage`] events/player state. The bridge request
//! is method-shaped so the existing `Bridge` surface can be mirrored without a
//! second hand-maintained enum for every data request.

use serde::{Deserialize, Serialize};

use crate::bridge::Event;
use crate::player::{Repeat, State, Track};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiMessage {
    Hello { version: u32 },
    Show,
    Shutdown,
    Ping,
    /// Forwarded to the daemon's local `Bridge` by method name.
    Bridge {
        method: String,
        #[serde(default)]
        args: serde_json::Value,
    },
    Player(PlayerCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreMessage {
    Ready,
    Bridge(Event),
    PlayerFull(PlayerSnapshot),
    PlayerTick(PlayerTick),
    Seeked { position: f64 },
    Activate,
    Shutdown,
    Error(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub queue: Vec<Track>,
    pub index: usize,
    pub state: StateWire,
    pub repeat: RepeatWire,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub loading: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerTick {
    pub index: usize,
    pub state: StateWire,
    pub repeat: RepeatWire,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub loading: bool,
}

/// String-coded player state (stable across protocol versions).
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum PlayerCommand {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SetVolume { volume: f64 },
    ToggleMute,
    SetMuted { muted: bool },
    Seek { seconds: f64 },
    SeekRelative { offset: f64 },
    Jump { index: usize },
    Load { tracks: Vec<Track>, start: usize },
    LoadPaused { tracks: Vec<Track>, start: usize },
    Append { tracks: Vec<Track> },
    InsertMany { at: usize, tracks: Vec<Track> },
    Remove { index: usize },
    RemoveMany { indices: Vec<usize> },
    Move { from: usize, to: usize },
    MoveBlock { start: usize, len: usize, delta: i32 },
    ShuffleQueue,
    CycleRepeat,
    SetTrackFavorite { id: String, favorite: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_and_core_messages_round_trip_json() {
        let hello = UiMessage::Hello { version: VERSION };
        let json = serde_json::to_string(&hello).unwrap();
        assert!(matches!(
            serde_json::from_str::<UiMessage>(&json).unwrap(),
            UiMessage::Hello { version: VERSION }
        ));

        let command = UiMessage::Player(PlayerCommand::MoveBlock {
            start: 2,
            len: 3,
            delta: -1,
        });
        let json = serde_json::to_string(&command).unwrap();
        assert!(matches!(
            serde_json::from_str::<UiMessage>(&json).unwrap(),
            UiMessage::Player(PlayerCommand::MoveBlock {
                start: 2,
                len: 3,
                delta: -1
            })
        ));

        let event = CoreMessage::Bridge(Event::Error("boom".into()));
        let json = serde_json::to_string(&event).unwrap();
        assert!(matches!(
            serde_json::from_str::<CoreMessage>(&json).unwrap(),
            CoreMessage::Bridge(Event::Error(message)) if message == "boom"
        ));

        let snapshot = CoreMessage::PlayerFull(PlayerSnapshot {
            index: 3,
            state: StateWire::Paused,
            volume: 40.0,
            ..Default::default()
        });
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(matches!(
            serde_json::from_str::<CoreMessage>(&json).unwrap(),
            CoreMessage::PlayerFull(PlayerSnapshot {
                index: 3,
                state: StateWire::Paused,
                volume,
                ..
            }) if volume == 40.0
        ));
    }
}
