//! Shared textual IPC command registry.
//!
//! The Unix-socket IPC handler and the `:` command line both parse through
//! this module. Keep command names, aliases, required arguments and
//! suggestions in one place so the two surfaces cannot drift.

use std::fmt;

use crate::bridge::SyncMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WipeScope {
    Album,
    Lyrics,
    Covers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QuickDirection {
    North,
    South,
    East,
    West,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IpcCommand {
    Sync(SyncMode),
    Wipe(WipeScope),
    WipeDatabases,
    Logout,
    AddAccount,
    RemoveAccount {
        keep_data: bool,
    },
    PlaylistCreate {
        name: String,
        offline: bool,
    },
    PlaylistAdd {
        targets: Vec<String>,
        playlist: String,
    },
    PlaylistRemove {
        targets: Vec<String>,
        playlist: String,
    },
    PlaylistDelete {
        playlist: String,
    },
    Shuffle {
        target: ShuffleTarget,
    },
    Radio {
        seed: RadioSeed,
    },
    RadioSave {
        offline: bool,
    },
    Download {
        request: DownloadRequest,
    },
    Refresh,
    Top,
    Bottom,
    MoveUp,
    MoveDown,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    MoveLeft,
    MoveRight,
    PrevPlace,
    NextPlace,
    Home,
    Back,
    Enter,
    ScopeNext,
    ScopePrev,
    OrderNext,
    OrderPrev,
    Visual,
    VisualAll,
    QueueToggle,
    QueueRemove,
    QueueUp,
    QueueDown,
    Yank,
    YankRow,
    PasteBelow,
    PasteAbove,
    DeleteRow,
    RepeatCycle,
    ToggleFullscreen,
    LyricsOverlay,
    PaneCycle {
        backward: bool,
    },
    Search {
        query: String,
    },
    InsertMode {
        query: String,
    },
    SearchNext,
    SearchPrev,
    SearchScopeNext,
    SearchScopePrev,
    LineSearch,
    PlayPause,
    Play,
    Pause,
    Stop,
    NextTrack,
    PrevTrack,
    Mute,
    VolumeUp,
    VolumeDown,
    SetVolume(f64),
    SeekBack,
    SeekForward,
    Seek(f64),
    SeekRelative(f64),
    PlaySong(String),
    PlaySongInAlbum(String),
    OpenSongAlbum(String),
    DownloadSong(String),
    QuickHold(bool),
    QuickAim(QuickDirection),
    QuickCycle {
        next: bool,
    },
    QuickCancel,
    Show,
    RandomAlbum,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShuffleTarget {
    Queue,
    Album(Option<String>),
    Artist(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RadioSeed {
    Current,
    Album(Option<String>),
    Song(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DownloadScope {
    Artist,
    Album,
    Track,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DownloadMode {
    Current,
    Direct {
        scope: DownloadScope,
        query: String,
    },
    Path {
        artist: String,
        album: Option<String>,
        track: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DownloadRequest {
    pub remove: bool,
    pub mode: DownloadMode,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CurrentTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRow {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub subtitle: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuggestionContext {
    pub current: Option<CurrentTrack>,
    pub rows: Vec<ContextRow>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SuggestScope {
    Artist,
    Album,
    Track,
    /// Mixed artist/album/track/playlist results for playlist targets.
    Any,
    /// Playlist names only (the part after `playlist add ...;`).
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestRequest {
    pub scope: SuggestScope,
    pub query: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError(pub String);

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CommandError {}

#[derive(Debug, Clone, Copy)]
pub enum ArgSpec {
    None,
    Enum(&'static [&'static str]),
    OptionalEnum(&'static [&'static str]),
    Number(&'static str),
    Text(&'static str),
}

pub struct CommandSpec {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
    pub args: ArgSpec,
}

const SYNC_ARGS: &[&str] = &["all", "repair"];
const WIPE_ARGS: &[&str] = &["album", "lyrics", "covers"];
const ON_OFF: &[&str] = &["on", "off"];
const DIRECTIONS: &[&str] = &["north", "south", "east", "west"];
const NEXT_PREV: &[&str] = &["next", "prev"];

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "sync",
        usage: "sync [all|repair]",
        description: "Sync the library (incremental, deep, or repair)",
        args: ArgSpec::OptionalEnum(SYNC_ARGS),
    },
    CommandSpec {
        name: "wipe",
        usage: "wipe <album|lyrics|covers>",
        description: "Clear downloaded albums or a cache",
        args: ArgSpec::Enum(WIPE_ARGS),
    },
    CommandSpec {
        name: "wipe_databases",
        usage: "wipe_databases",
        description: "Open the destructive wipe chooser",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "logout",
        usage: "logout",
        description: "Log out of the current account",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "add_account",
        usage: "add_account",
        description: "Open the login screen to add an account",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "remove_account",
        usage: "remove_account [--keep-data]",
        description: "Remove the current account",
        args: ArgSpec::OptionalEnum(&["--keep-data"]),
    },
    CommandSpec {
        name: "playlist",
        usage: "playlist <create|add|rm> ...",
        description: "Create, add/remove targets or delete a playlist (playlist rm <target...>; <name>, playlist rm <name>)",
        args: ArgSpec::Enum(&["create", "add", "rm"]),
    },
    CommandSpec {
        name: "shuffle",
        usage: "shuffle <queue|album|artist> [name]",
        description: "Shuffle the queue, an album, or an artist",
        args: ArgSpec::Enum(&["queue", "album", "artist"]),
    },
    CommandSpec {
        name: "radio",
        usage: "radio [current|album|song] [name]",
        description: "Start radio from the current context or an explicit seed",
        args: ArgSpec::OptionalEnum(&["save", "current", "album", "song"]),
    },
    CommandSpec {
        name: "download",
        usage: "download [rm] [artist|album|track] <names...>",
        description: "Download the current track or a searched artist/album/track",
        args: ArgSpec::Text("[rm] <artist> [album] [track]"),
    },
    CommandSpec {
        name: "refresh",
        usage: "refresh",
        description: "Start an incremental sync",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "top",
        usage: "top",
        description: "Jump to the first row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "bottom",
        usage: "bottom",
        description: "Jump to the last row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "move_up",
        usage: "move_up",
        description: "Move the cursor up",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "move_down",
        usage: "move_down",
        description: "Move the cursor down",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "scroll_up",
        usage: "scroll_up",
        description: "Scroll up one row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "scroll_down",
        usage: "scroll_down",
        description: "Scroll down one row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "page_up",
        usage: "page_up",
        description: "Move one page up",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "page_down",
        usage: "page_down",
        description: "Move one page down",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "move_left",
        usage: "move_left",
        description: "Move left within the active surface",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "move_right",
        usage: "move_right",
        description: "Move right within the active surface",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "prev_place",
        usage: "prev_place",
        description: "Go to the previous place",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "next_place",
        usage: "next_place",
        description: "Go to the next place",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "home",
        usage: "home",
        description: "Return to the Home library",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "back",
        usage: "back",
        description: "Back out of the current surface",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "enter",
        usage: "enter",
        description: "Activate the current selection",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "scope_next",
        usage: "scope_next",
        description: "Next library scope",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "scope_prev",
        usage: "scope_prev",
        description: "Previous library scope",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "order_next",
        usage: "order_next",
        description: "Next library order",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "order_prev",
        usage: "order_prev",
        description: "Previous library order",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "visual",
        usage: "visual",
        description: "Toggle visual selection",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "visual_all",
        usage: "visual_all",
        description: "Select every visible row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "queue_toggle",
        usage: "queue_toggle",
        description: "Open or close the queue",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "queue_remove",
        usage: "queue_remove",
        description: "Remove the queue cursor row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "queue_up",
        usage: "queue_up",
        description: "Move the queue row up",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "queue_down",
        usage: "queue_down",
        description: "Move the queue row down",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "yank",
        usage: "yank",
        description: "Yank the visual range or cursor row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "yank_row",
        usage: "yank_row",
        description: "Yank the cursor row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "paste_below",
        usage: "paste_below",
        description: "Paste the row clipboard below the queue cursor or into the playlist",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "paste_above",
        usage: "paste_above",
        description: "Paste the row clipboard above the queue cursor or into the playlist",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "delete_row",
        usage: "delete_row",
        description: "Delete the selected queue rows or playlist entries",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "repeat_cycle",
        usage: "repeat_cycle",
        description: "Cycle repeat mode",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "toggle_fullscreen",
        usage: "toggle_fullscreen",
        description: "Toggle playback/fullscreen view",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "lyrics_overlay",
        usage: "lyrics_overlay",
        description: "Toggle the lyrics overlay",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "pane_cycle",
        usage: "pane_cycle [back]",
        description: "Cycle the active pane/cursor scope",
        args: ArgSpec::OptionalEnum(&["back"]),
    },
    CommandSpec {
        name: "search",
        usage: "search [query]",
        description: "Open floating search, optionally with a query",
        args: ArgSpec::Text("<query>"),
    },
    CommandSpec {
        name: "insert_mode",
        usage: "insert_mode [query]",
        description: "Open search in insert mode",
        args: ArgSpec::Text("<query>"),
    },
    CommandSpec {
        name: "search_next",
        usage: "search_next",
        description: "Next floating-search row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "search_prev",
        usage: "search_prev",
        description: "Previous floating-search row",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "search_scope_next",
        usage: "search_scope_next",
        description: "Next floating-search scope",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "search_scope_prev",
        usage: "search_scope_prev",
        description: "Previous floating-search scope",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "line_search",
        usage: "line_search",
        description: "Open the / line search",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "play_pause",
        usage: "play_pause",
        description: "Toggle playback",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "play",
        usage: "play",
        description: "Resume playback",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "pause",
        usage: "pause",
        description: "Pause playback",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "stop",
        usage: "stop",
        description: "Stop playback",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "next_track",
        usage: "next_track",
        description: "Skip to the next track",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "prev_track",
        usage: "prev_track",
        description: "Go to the previous track",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "mute",
        usage: "mute",
        description: "Toggle mute",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "volume_up",
        usage: "volume_up",
        description: "Raise volume by 5",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "volume_down",
        usage: "volume_down",
        description: "Lower volume by 5",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "set_volume",
        usage: "set_volume <0..100>",
        description: "Set volume",
        args: ArgSpec::Number("<0..100>"),
    },
    CommandSpec {
        name: "seek_back",
        usage: "seek_back",
        description: "Seek back 5 seconds",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "seek_forward",
        usage: "seek_forward",
        description: "Seek forward 5 seconds",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "seek",
        usage: "seek <seconds>",
        description: "Seek to an absolute position",
        args: ArgSpec::Number("<seconds>"),
    },
    CommandSpec {
        name: "seek_relative",
        usage: "seek_relative <±seconds>",
        description: "Seek relative to the playhead",
        args: ArgSpec::Number("<±seconds>"),
    },
    CommandSpec {
        name: "play_song",
        usage: "play_song <song-id>",
        description: "Play one song in the current order",
        args: ArgSpec::Text("<song-id>"),
    },
    CommandSpec {
        name: "play_song_in_album",
        usage: "play_song_in_album <song-id>",
        description: "Play a song in its album queue",
        args: ArgSpec::Text("<song-id>"),
    },
    CommandSpec {
        name: "open_song_album",
        usage: "open_song_album <song-id>",
        description: "Open the song's album page",
        args: ArgSpec::Text("<song-id>"),
    },
    CommandSpec {
        name: "download_song",
        usage: "download_song <song-id>",
        description: "Download one song",
        args: ArgSpec::Text("<song-id>"),
    },
    CommandSpec {
        name: "quick_hold",
        usage: "quick_hold <on|off>",
        description: "Arm/release the Alt quick actions",
        args: ArgSpec::Enum(ON_OFF),
    },
    CommandSpec {
        name: "quick_aim",
        usage: "quick_aim <north|south|east|west>",
        description: "Aim the Alt quick actions",
        args: ArgSpec::Enum(DIRECTIONS),
    },
    CommandSpec {
        name: "quick_cycle",
        usage: "quick_cycle <next|prev>",
        description: "Cycle the aimed Alt action",
        args: ArgSpec::Enum(NEXT_PREV),
    },
    CommandSpec {
        name: "quick_cancel",
        usage: "quick_cancel",
        description: "Cancel the Alt quick actions",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "show",
        usage: "show",
        description: "Show or raise the Musika window",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "random",
        usage: "random",
        description: "Play a random album from the library",
        args: ArgSpec::None,
    },
    CommandSpec {
        name: "quit",
        usage: "quit",
        description: "Quit Musika",
        args: ArgSpec::None,
    },
];

fn canonical_name(first: &str) -> &str {
    match first {
        "cancel" => "back",
        "next" => "next_track",
        "prev" => "prev_track",
        "previous" => "prev_track",
        "volume" => "set_volume",
        "fullscreen" => "toggle_fullscreen",
        "lyrics" => "lyrics_overlay",
        "search_action" => "line_search",
        "insert" => "insert_mode",
        "mpris_play" => "play",
        "mpris_pause" => "pause",
        "mpris_play_pause" => "play_pause",
        "mpris_stop" => "stop",
        "mpris_next" => "next_track",
        "mpris_previous" => "prev_track",
        other => other,
    }
}

fn first_word(input: &str) -> (&str, &str) {
    let trimmed = input.trim();
    match trimmed.split_once(char::is_whitespace) {
        Some((first, rest)) => (first, rest.trim()),
        None => (trimmed, ""),
    }
}

fn one_arg<'a>(rest: &'a str, _usage: &str) -> Result<&'a str, CommandError> {
    let mut words = rest.split_whitespace();
    let Some(value) = words.next() else {
        return Err(CommandError("missing argument".to_string()));
    };
    if words.next().is_some() {
        return Err(CommandError("too many arguments".to_string()));
    }
    Ok(value)
}

fn parse_number(value: &str, label: &str) -> Result<f64, CommandError> {
    value
        .parse::<f64>()
        .map_err(|_| CommandError(format!("invalid {label}: {value}")))
}

pub fn parse(input: &str) -> Result<IpcCommand, CommandError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CommandError("empty command".to_string()));
    }
    // MPRIS compatibility forms use a colon payload. Only `mpris_*` names
    // claim the colon: a free-text query may legitimately contain one.
    if let Some((name, value)) = trimmed.split_once(':')
        && name.starts_with("mpris_")
    {
        return match name {
            "mpris_volume" => {
                let volume = parse_number(value, "volume")?;
                if !(0.0..=100.0).contains(&volume) {
                    return Err(CommandError("volume must be in 0..=100".to_string()));
                }
                Ok(IpcCommand::SetVolume(volume))
            }
            "mpris_seek" => {
                let micros = parse_number(value, "microseconds")?;
                Ok(IpcCommand::SeekRelative(micros / 1_000_000.0))
            }
            "mpris_position" => {
                let micros = parse_number(value, "microseconds")?;
                Ok(IpcCommand::Seek((micros / 1_000_000.0).max(0.0)))
            }
            _ => Err(CommandError(format!("unknown command: {name}"))),
        };
    }

    let (first, rest) = first_word(trimmed);
    let name = canonical_name(first);
    let spec = COMMANDS.iter().find(|spec| spec.name == name);
    let Some(spec) = spec else {
        return Err(CommandError(format!("unknown command: {name}")));
    };
    match name {
        "sync" => match rest {
            "" => Ok(IpcCommand::Sync(SyncMode::Incremental)),
            "all" => Ok(IpcCommand::Sync(SyncMode::All)),
            "repair" => Ok(IpcCommand::Sync(SyncMode::Repair)),
            _ => Err(CommandError("usage: sync [all|repair]".to_string())),
        },
        "wipe" => match one_arg(rest, spec.usage)? {
            "album" => Ok(IpcCommand::Wipe(WipeScope::Album)),
            "lyrics" => Ok(IpcCommand::Wipe(WipeScope::Lyrics)),
            "covers" => Ok(IpcCommand::Wipe(WipeScope::Covers)),
            other => Err(CommandError(format!("unknown wipe scope: {other}"))),
        },
        "wipe_databases" => Ok(IpcCommand::WipeDatabases),
        "logout" => Ok(IpcCommand::Logout),
        "add_account" => Ok(IpcCommand::AddAccount),
        "remove_account" => match rest {
            "" => Ok(IpcCommand::RemoveAccount { keep_data: false }),
            "--keep-data" => Ok(IpcCommand::RemoveAccount { keep_data: true }),
            _ => Err(CommandError(
                "usage: remove_account [--keep-data]".to_string(),
            )),
        },
        "playlist" => parse_playlist(rest),
        "shuffle" => match split_quoted_args(rest)?.as_slice() {
            [] => Err(CommandError(
                "usage: shuffle <queue|album|artist> [id]".to_string(),
            )),
            [target] if target == "queue" => Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Queue,
            }),
            [target] if target == "album" => Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Album(None),
            }),
            [target] if target == "artist" => Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Artist(None),
            }),
            [target, id] if target == "album" => Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Album(Some(id.clone())),
            }),
            [target, id] if target == "artist" => Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Artist(Some(id.clone())),
            }),
            _ => Err(CommandError(
                "usage: shuffle <queue|album|artist> [id]".to_string(),
            )),
        },
        "radio" => match split_quoted_args(rest)?.as_slice() {
            [] => Ok(IpcCommand::Radio {
                seed: RadioSeed::Current,
            }),
            [target] if target == "save" => Ok(IpcCommand::RadioSave { offline: false }),
            [target, flag] if target == "save" && flag == "--offline" => {
                Ok(IpcCommand::RadioSave { offline: true })
            }
            [target] if target == "current" => Ok(IpcCommand::Radio {
                seed: RadioSeed::Current,
            }),
            [target] if target == "album" => Ok(IpcCommand::Radio {
                seed: RadioSeed::Album(None),
            }),
            [target] if target == "song" => Ok(IpcCommand::Radio {
                seed: RadioSeed::Song(None),
            }),
            [target, id] if target == "album" => Ok(IpcCommand::Radio {
                seed: RadioSeed::Album(Some(id.clone())),
            }),
            [target, id] if target == "song" => Ok(IpcCommand::Radio {
                seed: RadioSeed::Song(Some(id.clone())),
            }),
            _ => Err(CommandError(
                "usage: radio [current|album|song] [id]".to_string(),
            )),
        },
        "download" => parse_download(rest),
        "refresh" => Ok(IpcCommand::Refresh),
        "top" => Ok(IpcCommand::Top),
        "bottom" => Ok(IpcCommand::Bottom),
        "move_up" => Ok(IpcCommand::MoveUp),
        "move_down" => Ok(IpcCommand::MoveDown),
        "scroll_up" => Ok(IpcCommand::ScrollUp),
        "scroll_down" => Ok(IpcCommand::ScrollDown),
        "page_up" => Ok(IpcCommand::PageUp),
        "page_down" => Ok(IpcCommand::PageDown),
        "move_left" => Ok(IpcCommand::MoveLeft),
        "move_right" => Ok(IpcCommand::MoveRight),
        "prev_place" => Ok(IpcCommand::PrevPlace),
        "next_place" => Ok(IpcCommand::NextPlace),
        "home" => Ok(IpcCommand::Home),
        "back" => Ok(IpcCommand::Back),
        "enter" => Ok(IpcCommand::Enter),
        "scope_next" => Ok(IpcCommand::ScopeNext),
        "scope_prev" => Ok(IpcCommand::ScopePrev),
        "order_next" => Ok(IpcCommand::OrderNext),
        "order_prev" => Ok(IpcCommand::OrderPrev),
        "visual" => Ok(IpcCommand::Visual),
        "visual_all" => Ok(IpcCommand::VisualAll),
        "queue_toggle" => Ok(IpcCommand::QueueToggle),
        "queue_remove" => Ok(IpcCommand::QueueRemove),
        "yank" => Ok(IpcCommand::Yank),
        "yank_row" => Ok(IpcCommand::YankRow),
        "paste_below" => Ok(IpcCommand::PasteBelow),
        "paste_above" => Ok(IpcCommand::PasteAbove),
        "delete_row" => Ok(IpcCommand::DeleteRow),
        "queue_up" => Ok(IpcCommand::QueueUp),
        "queue_down" => Ok(IpcCommand::QueueDown),
        "repeat_cycle" => Ok(IpcCommand::RepeatCycle),
        "toggle_fullscreen" => Ok(IpcCommand::ToggleFullscreen),
        "lyrics_overlay" => Ok(IpcCommand::LyricsOverlay),
        "pane_cycle" => match rest {
            "" => Ok(IpcCommand::PaneCycle { backward: false }),
            "back" => Ok(IpcCommand::PaneCycle { backward: true }),
            _ => Err(CommandError("usage: pane_cycle [back]".to_string())),
        },
        "search" => Ok(IpcCommand::Search {
            query: rest.to_string(),
        }),
        "insert_mode" => Ok(IpcCommand::InsertMode {
            query: rest.to_string(),
        }),
        "search_next" => Ok(IpcCommand::SearchNext),
        "search_prev" => Ok(IpcCommand::SearchPrev),
        "search_scope_next" => Ok(IpcCommand::SearchScopeNext),
        "search_scope_prev" => Ok(IpcCommand::SearchScopePrev),
        "line_search" => Ok(IpcCommand::LineSearch),
        "play_pause" => Ok(IpcCommand::PlayPause),
        "play" => Ok(IpcCommand::Play),
        "pause" => Ok(IpcCommand::Pause),
        "stop" => Ok(IpcCommand::Stop),
        "next_track" => Ok(IpcCommand::NextTrack),
        "prev_track" => Ok(IpcCommand::PrevTrack),
        "mute" => Ok(IpcCommand::Mute),
        "volume_up" => Ok(IpcCommand::VolumeUp),
        "volume_down" => Ok(IpcCommand::VolumeDown),
        "set_volume" => {
            let volume = parse_number(one_arg(rest, spec.usage)?, "volume")?;
            if !(0.0..=100.0).contains(&volume) {
                return Err(CommandError("volume must be in 0..=100".to_string()));
            }
            Ok(IpcCommand::SetVolume(volume))
        }
        "seek_back" => Ok(IpcCommand::SeekBack),
        "seek_forward" => Ok(IpcCommand::SeekForward),
        "seek" => Ok(IpcCommand::Seek(
            parse_number(one_arg(rest, spec.usage)?, "seconds")?.max(0.0),
        )),
        "seek_relative" => Ok(IpcCommand::SeekRelative(parse_number(
            one_arg(rest, spec.usage)?,
            "seconds",
        )?)),
        "play_song" => Ok(IpcCommand::PlaySong(one_arg(rest, spec.usage)?.to_string())),
        "play_song_in_album" => Ok(IpcCommand::PlaySongInAlbum(
            one_arg(rest, spec.usage)?.to_string(),
        )),
        "open_song_album" => Ok(IpcCommand::OpenSongAlbum(
            one_arg(rest, spec.usage)?.to_string(),
        )),
        "download_song" => Ok(IpcCommand::DownloadSong(
            one_arg(rest, spec.usage)?.to_string(),
        )),
        "quick_hold" => Ok(IpcCommand::QuickHold(one_arg(rest, spec.usage)? == "on")),
        "quick_aim" => Ok(IpcCommand::QuickAim(match one_arg(rest, spec.usage)? {
            "north" => QuickDirection::North,
            "south" => QuickDirection::South,
            "east" => QuickDirection::East,
            "west" => QuickDirection::West,
            other => return Err(CommandError(format!("unknown direction: {other}"))),
        })),
        "quick_cycle" => Ok(IpcCommand::QuickCycle {
            next: one_arg(rest, spec.usage)? == "next",
        }),
        "quick_cancel" => Ok(IpcCommand::QuickCancel),
        "show" => Ok(IpcCommand::Show),
        "random" => Ok(IpcCommand::RandomAlbum),
        "quit" => Ok(IpcCommand::Quit),
        _ => Err(CommandError(format!("unknown command: {name}"))),
    }
}

/// Whether completing `name` should append a space and continue into args.
pub fn needs_argument(name: &str) -> bool {
    COMMANDS
        .iter()
        .find(|spec| spec.name == name)
        .is_some_and(|spec| !matches!(spec.args, ArgSpec::None))
}

/// Shell-like argument splitting for commands whose names contain spaces.
/// Quotes group words; backslash escapes the next character.
fn split_quoted_args(input: &str) -> Result<Vec<String>, CommandError> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut in_word = false;
    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            in_word = true;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            in_word = true;
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                in_word = true;
            }
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err(CommandError("unterminated quoted argument".to_string()));
    }
    if escaped {
        current.push('\\');
    }
    if in_word {
        out.push(current);
    }
    Ok(out)
}

fn parse_download(rest: &str) -> Result<IpcCommand, CommandError> {
    let mut tokens = split_quoted_args(rest)?;
    let remove = tokens.first().is_some_and(|token| token == "rm");
    if remove {
        tokens.remove(0);
    }
    if tokens.is_empty() || tokens[0] == "current" {
        if tokens.len() > 1 {
            return Err(CommandError(
                "usage: download [rm] [current|artist|album|track] <names...>".to_string(),
            ));
        }
        return Ok(IpcCommand::Download {
            request: DownloadRequest {
                remove,
                mode: DownloadMode::Current,
            },
        });
    }
    let direct = match tokens[0].as_str() {
        "artist" => Some(DownloadScope::Artist),
        "album" => Some(DownloadScope::Album),
        "track" | "song" => Some(DownloadScope::Track),
        _ => None,
    };
    if let Some(scope) = direct {
        let query = tokens[1..].join(" ");
        if query.trim().is_empty() {
            return Err(CommandError(
                "usage: download [rm] [artist|album|track] <names...>".to_string(),
            ));
        }
        return Ok(IpcCommand::Download {
            request: DownloadRequest {
                remove,
                mode: DownloadMode::Direct { scope, query },
            },
        });
    }
    let mut positional = Vec::new();
    for token in tokens {
        if (token == "album" || token == "track") && !positional.is_empty() && positional.len() < 3
        {
            continue;
        }
        positional.push(token);
    }
    if positional.is_empty()
        || positional.len() > 3
        || positional.iter().any(|p| p.trim().is_empty())
    {
        return Err(CommandError(
            "usage: download [rm] <artist> [album] [track]".to_string(),
        ));
    }
    let mut positional = positional.into_iter();
    Ok(IpcCommand::Download {
        request: DownloadRequest {
            remove,
            mode: DownloadMode::Path {
                artist: positional.next().unwrap_or_default(),
                album: positional.next(),
                track: positional.next(),
            },
        },
    })
}

/// Byte index of the first unquoted `;` in `input`. Used by the `playlist add`
/// grammar to cut the target list from the playlist name.
fn unquoted_semicolon(input: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            ';' => return Some(index),
            _ => {}
        }
    }
    None
}

fn parse_playlist(rest: &str) -> Result<IpcCommand, CommandError> {
    // New syntax: `playlist add <target...>; <playlist name>` — the first
    // unquoted semicolon cuts the target list from the playlist name. The old
    // `playlist add <target...> <playlist>` form (playlist = last token) is
    // still accepted.
    let (head, playlist_tail) = match unquoted_semicolon(rest) {
        Some(index) => (&rest[..index], Some(&rest[index + 1..])),
        None => (rest, None),
    };
    let tokens = split_quoted_args(head)?;
    let Some(subcommand) = tokens.first().map(String::as_str) else {
        return Err(CommandError("usage: playlist <create|add> ...".to_string()));
    };
    match subcommand {
        "create" => {
            if playlist_tail.is_some() {
                return Err(CommandError(
                    "usage: playlist create <name> [--offline]".to_string(),
                ));
            }
            let mut values = tokens[1..].to_vec();
            let offline = values.last().is_some_and(|value| value == "--offline");
            if offline {
                values.pop();
            }
            let name = values.join(" ").trim().trim_matches(',').trim().to_string();
            if name.is_empty() {
                return Err(CommandError(
                    "usage: playlist create <name> [--offline]".to_string(),
                ));
            }
            Ok(IpcCommand::PlaylistCreate { name, offline })
        }
        "add" | "rm" => {
            let mut targets: Vec<String> = tokens[1..]
                .iter()
                .flat_map(|value| value.split(','))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
            // `playlist rm ; <playlist>` (an explicit cutoff with no targets)
            // deletes the playlist itself. The `;` is also what switches the
            // command-line suggestions to playlist names.
            if subcommand == "rm"
                && targets.is_empty()
                && let Some(tail) = playlist_tail
            {
                let values = split_quoted_args(tail)?;
                let playlist = values.join(" ").trim().trim_matches(',').trim().to_string();
                if playlist.is_empty() {
                    return Err(CommandError(
                        "usage: playlist rm; <playlist>".to_string(),
                    ));
                }
                return Ok(IpcCommand::PlaylistDelete { playlist });
            }
            let playlist = if let Some(tail) = playlist_tail {
                let values = split_quoted_args(tail)?;
                values.join(" ").trim().trim_matches(',').trim().to_string()
            } else {
                if targets.len() < 2 {
                    return Err(CommandError(format!(
                        "usage: playlist {subcommand} <target...>; <playlist>"
                    )));
                }
                targets.pop().unwrap_or_default()
            };
            if targets.is_empty() || playlist.is_empty() {
                return Err(CommandError(format!(
                    "usage: playlist {subcommand} <target...>; <playlist>"
                )));
            }
            if subcommand == "rm" {
                Ok(IpcCommand::PlaylistRemove { targets, playlist })
            } else {
                Ok(IpcCommand::PlaylistAdd { targets, playlist })
            }
        }
        _ => Err(CommandError("usage: playlist <create|add|rm> ...".to_string())),
    }
}

pub(crate) fn quote_arg(value: &str) -> String {
    if value.is_empty() || value.chars().any(char::is_whitespace) || value.contains('"') {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn canonical_download(request: &DownloadRequest) -> String {
    let mut out = String::from("download");
    if request.remove {
        out.push_str(" rm");
    }
    match &request.mode {
        DownloadMode::Current => out.push_str(" current"),
        DownloadMode::Direct { scope, query } => {
            out.push(' ');
            out.push_str(match scope {
                DownloadScope::Artist => "artist",
                DownloadScope::Album => "album",
                DownloadScope::Track => "track",
            });
            out.push(' ');
            out.push_str(&quote_arg(query));
        }
        DownloadMode::Path {
            artist,
            album,
            track,
        } => {
            out.push(' ');
            out.push_str(&quote_arg(artist));
            if let Some(album) = album {
                out.push_str(" album ");
                out.push_str(&quote_arg(album));
            }
            if let Some(track) = track {
                out.push_str(" track ");
                out.push_str(&quote_arg(track));
            }
        }
    }
    out
}

pub fn canonical_string(command: &IpcCommand) -> String {
    use IpcCommand::*;
    match command {
        Sync(SyncMode::Incremental) => "sync".to_string(),
        Sync(SyncMode::All) => "sync all".to_string(),
        Sync(SyncMode::Repair) => "sync repair".to_string(),
        Wipe(WipeScope::Album) => "wipe album".to_string(),
        Wipe(WipeScope::Lyrics) => "wipe lyrics".to_string(),
        Wipe(WipeScope::Covers) => "wipe covers".to_string(),
        WipeDatabases => "wipe_databases".to_string(),
        Logout => "logout".to_string(),
        AddAccount => "add_account".to_string(),
        RemoveAccount { keep_data: false } => "remove_account".to_string(),
        RemoveAccount { keep_data: true } => "remove_account --keep-data".to_string(),
        PlaylistCreate {
            name,
            offline: false,
        } => format!("playlist create {name}"),
        PlaylistCreate {
            name,
            offline: true,
        } => format!("playlist create {name} --offline"),
        PlaylistAdd { targets, playlist } => {
            let mut out = String::from("playlist add");
            for target in targets {
                out.push(' ');
                out.push_str(&quote_arg(target));
            }
            out.push_str("; ");
            out.push_str(&quote_arg(playlist));
            out
        }
        PlaylistDelete { playlist } => format!("playlist rm; {}", quote_arg(playlist)),
        PlaylistRemove { targets, playlist } => {
            let mut out = String::from("playlist rm");
            for target in targets {
                out.push(' ');
                out.push_str(&quote_arg(target));
            }
            out.push_str("; ");
            out.push_str(&quote_arg(playlist));
            out
        }
        Shuffle {
            target: ShuffleTarget::Queue,
        } => "shuffle queue".to_string(),
        Shuffle {
            target: ShuffleTarget::Album(None),
        } => "shuffle album".to_string(),
        Shuffle {
            target: ShuffleTarget::Album(Some(id)),
        } => format!("shuffle album {id}"),
        Shuffle {
            target: ShuffleTarget::Artist(None),
        } => "shuffle artist".to_string(),
        Shuffle {
            target: ShuffleTarget::Artist(Some(id)),
        } => format!("shuffle artist {id}"),
        Radio {
            seed: RadioSeed::Current,
        } => "radio current".to_string(),
        Radio {
            seed: RadioSeed::Album(None),
        } => "radio album".to_string(),
        Radio {
            seed: RadioSeed::Album(Some(id)),
        } => format!("radio album {id}"),
        Radio {
            seed: RadioSeed::Song(None),
        } => "radio song".to_string(),
        Radio {
            seed: RadioSeed::Song(Some(id)),
        } => format!("radio song {id}"),
        RadioSave { offline: false } => "radio save".to_string(),
        RadioSave { offline: true } => "radio save --offline".to_string(),
        Download { request } => canonical_download(request),
        Refresh => "refresh".to_string(),
        Top => "top".to_string(),
        Bottom => "bottom".to_string(),
        MoveUp => "move_up".to_string(),
        MoveDown => "move_down".to_string(),
        ScrollUp => "scroll_up".to_string(),
        ScrollDown => "scroll_down".to_string(),
        PageUp => "page_up".to_string(),
        PageDown => "page_down".to_string(),
        MoveLeft => "move_left".to_string(),
        MoveRight => "move_right".to_string(),
        PrevPlace => "prev_place".to_string(),
        NextPlace => "next_place".to_string(),
        Home => "home".to_string(),
        Back => "back".to_string(),
        Enter => "enter".to_string(),
        ScopeNext => "scope_next".to_string(),
        ScopePrev => "scope_prev".to_string(),
        OrderNext => "order_next".to_string(),
        OrderPrev => "order_prev".to_string(),
        Visual => "visual".to_string(),
        VisualAll => "visual_all".to_string(),
        QueueToggle => "queue_toggle".to_string(),
        QueueRemove => "queue_remove".to_string(),
        Yank => "yank".to_string(),
        YankRow => "yank_row".to_string(),
        PasteBelow => "paste_below".to_string(),
        PasteAbove => "paste_above".to_string(),
        DeleteRow => "delete_row".to_string(),
        QueueUp => "queue_up".to_string(),
        QueueDown => "queue_down".to_string(),
        RepeatCycle => "repeat_cycle".to_string(),
        ToggleFullscreen => "toggle_fullscreen".to_string(),
        LyricsOverlay => "lyrics_overlay".to_string(),
        PaneCycle { backward: false } => "pane_cycle".to_string(),
        PaneCycle { backward: true } => "pane_cycle back".to_string(),
        Search { query } => format!("search {query}").trim_end().to_string(),
        InsertMode { query } => format!("insert_mode {query}").trim_end().to_string(),
        SearchNext => "search_next".to_string(),
        SearchPrev => "search_prev".to_string(),
        SearchScopeNext => "search_scope_next".to_string(),
        SearchScopePrev => "search_scope_prev".to_string(),
        LineSearch => "line_search".to_string(),
        PlayPause => "play_pause".to_string(),
        Play => "play".to_string(),
        Pause => "pause".to_string(),
        Stop => "stop".to_string(),
        NextTrack => "next_track".to_string(),
        PrevTrack => "prev_track".to_string(),
        Mute => "mute".to_string(),
        VolumeUp => "volume_up".to_string(),
        VolumeDown => "volume_down".to_string(),
        SetVolume(volume) => format!("set_volume {volume:.0}"),
        SeekBack => "seek_back".to_string(),
        SeekForward => "seek_forward".to_string(),
        Seek(position) => format!("seek {position:.0}"),
        SeekRelative(offset) => format!("seek_relative {offset:.0}"),
        PlaySong(id) => format!("play_song {id}"),
        PlaySongInAlbum(id) => format!("play_song_in_album {id}"),
        OpenSongAlbum(id) => format!("open_song_album {id}"),
        DownloadSong(id) => format!("download_song {id}"),
        QuickHold(true) => "quick_hold on".to_string(),
        QuickHold(false) => "quick_hold off".to_string(),
        QuickAim(QuickDirection::North) => "quick_aim north".to_string(),
        QuickAim(QuickDirection::South) => "quick_aim south".to_string(),
        QuickAim(QuickDirection::East) => "quick_aim east".to_string(),
        QuickAim(QuickDirection::West) => "quick_aim west".to_string(),
        QuickCycle { next: true } => "quick_cycle next".to_string(),
        QuickCycle { next: false } => "quick_cycle prev".to_string(),
        QuickCancel => "quick_cancel".to_string(),
        Show => "show".to_string(),
        RandomAlbum => "random".to_string(),
        Quit => "quit".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub start: usize,
    pub end: usize,
    pub label: String,
    pub description: String,
    /// None for non-applicable syntax placeholders (numeric/text values).
    pub completion: Option<String>,
}

fn token_at(input: &str, cursor: usize) -> (usize, usize, usize) {
    let mut cursor = cursor.min(input.len());
    while cursor > 0 && !input.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let mut start = cursor;
    while start > 0 {
        let prev = input[..start]
            .char_indices()
            .next_back()
            .map(|(index, _)| index);
        let Some(prev) = prev else { break };
        if input[prev..start].trim().is_empty() {
            break;
        }
        start = prev;
    }
    let mut end = cursor;
    while end < input.len() {
        let next = input[end..]
            .chars()
            .next()
            .map(|ch| ch.len_utf8())
            .unwrap_or(0);
        if next == 0 || input[end..end + next].trim().is_empty() {
            break;
        }
        end += next;
    }
    let index = input[..start].split_whitespace().count();
    (start, end, index)
}

/// Parse the current command-line token into a scoped search request. Only
/// commands whose previous argument names a scope participate; the query is
/// the text after that scope plus the partially typed current token.
pub fn suggestion_request(input: &str, cursor: usize) -> Option<SuggestRequest> {
    let (start, end, index) = token_at(input, cursor);
    if index == 0 {
        return None;
    }
    let before = &input[..start];
    let args = split_quoted_args(before).ok()?;
    let command = canonical_name(args.first()?.as_str());
    let current = input[start..end]
        .trim_matches(['"', '\''])
        .trim()
        .to_string();
    let (scope, query_start) = match command {
        "shuffle" => match args.get(1).map(String::as_str) {
            Some("artist") => (SuggestScope::Artist, 2),
            Some("album") => (SuggestScope::Album, 2),
            _ => return None,
        },
        "radio" => match args.get(1).map(String::as_str) {
            Some("album") => (SuggestScope::Album, 2),
            Some("song") => (SuggestScope::Track, 2),
            _ => return None,
        },
        "playlist" => {
            if !matches!(args.get(1).map(String::as_str), Some("add" | "rm")) {
                return None;
            }
            // After the target cutoff only playlist names make sense; the
            // query is the name typed so far (empty lists all playlists).
            if let Some(cut) = unquoted_semicolon(&input[..end]) {
                let mut query = if cut < start {
                    split_quoted_args(&input[cut + 1..start])
                        .unwrap_or_default()
                        .join(" ")
                } else {
                    String::new()
                };
                let typed = input[start..end].trim_matches(['"', '\'']).trim();
                if !typed.is_empty() {
                    if !query.is_empty() {
                        query.push(' ');
                    }
                    query.push_str(typed);
                }
                let suggestion_start = if cut < start { start } else { cut + 1 };
                return Some(SuggestRequest {
                    scope: SuggestScope::Playlist,
                    query: query.trim().to_string(),
                    start: suggestion_start,
                    end,
                });
            }
            (SuggestScope::Any, args.len())
        }
        "download" => {
            let (scope, index) =
                args.iter()
                    .enumerate()
                    .rev()
                    .find_map(|(index, arg)| match arg.as_str() {
                        "artist" => Some((SuggestScope::Artist, index + 1)),
                        "album" => Some((SuggestScope::Album, index + 1)),
                        "track" | "song" => Some((SuggestScope::Track, index + 1)),
                        _ => None,
                    })?;
            (scope, index)
        }
        _ => return None,
    };
    let mut query = args
        .iter()
        .skip(query_start)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if !query.is_empty() && !current.is_empty() {
        query.push(' ');
    }
    query.push_str(&current);
    let query = query.trim().to_string();
    if query.is_empty() {
        return None;
    }
    Some(SuggestRequest {
        scope,
        query,
        start,
        end,
    })
}

pub fn suggestions(input: &str, cursor: usize) -> Vec<Suggestion> {
    suggestions_with(input, cursor, &SuggestionContext::default())
}

pub fn suggestions_with(
    input: &str,
    cursor: usize,
    context: &SuggestionContext,
) -> Vec<Suggestion> {
    let (start, end, index) = token_at(input, cursor);
    let prefix = &input[start..end];
    if index == 0 {
        return COMMANDS
            .iter()
            .filter(|spec| spec.name.starts_with(prefix))
            .map(|spec| Suggestion {
                start,
                end,
                label: spec.usage.to_string(),
                description: spec.description.to_string(),
                completion: Some(spec.name.to_string()),
            })
            .collect();
    }

    let command = input[..start].split_whitespace().next().unwrap_or("");
    let name = canonical_name(command);
    let Some(spec) = COMMANDS.iter().find(|spec| spec.name == name) else {
        return Vec::new();
    };
    if name == "download" {
        return download_suggestions(input, cursor, start, end, context);
    }
    if name == "playlist" {
        return playlist_suggestions(input, start, end, context);
    }
    if index != 1 {
        return Vec::new();
    }
    let args = match spec.args {
        ArgSpec::Enum(args) => args,
        ArgSpec::OptionalEnum(args) => args,
        ArgSpec::Number(label) | ArgSpec::Text(label) => {
            if prefix.is_empty() {
                return vec![Suggestion {
                    start,
                    end,
                    label: format!("{} {label}", spec.name),
                    description: spec.description.to_string(),
                    completion: None,
                }];
            }
            return Vec::new();
        }
        ArgSpec::None => return Vec::new(),
    };
    args.iter()
        .filter(|arg| arg.starts_with(prefix))
        .map(|arg| Suggestion {
            start,
            end,
            label: format!("{} {}", spec.name, arg),
            description: spec.description.to_string(),
            completion: Some((*arg).to_string()),
        })
        .collect()
}

/// Completion for `playlist add`.
///
/// - before the `;` cutoff the surface offers the `current` target plus the
///   visible rows (playlists included: a playlist is a valid target);
/// - after the cutoff only playlist names are offered, and the whole typed
///   name region is replaced (so a two-word query completes in one Tab).
/// Async search hits replace the list after the debounce.
fn playlist_suggestions(
    input: &str,
    start: usize,
    end: usize,
    context: &SuggestionContext,
) -> Vec<Suggestion> {
    let before = input[..start].split_whitespace().collect::<Vec<_>>();
    if !matches!(before.get(1).copied(), Some("add" | "rm")) {
        return Vec::new();
    }
    let cutoff = unquoted_semicolon(&input[..end]);
    let playlist_only = cutoff.is_some();
    let (start, end, prefix) = match cutoff {
        Some(cut) => {
            // Replace from the first non-space after the cutoff through the
            // current token, so `; Road T` completes as `; "Road Trip"`.
            let mut region = cut + 1;
            while region < end && input.as_bytes()[region].is_ascii_whitespace() {
                region += 1;
            }
            let prefix = input[region..end]
                .trim_matches(['"', '\''])
                .trim()
                .to_string();
            (region.min(end), end, prefix)
        }
        None => (
            start,
            end,
            input[start..end].trim_matches(['"', '\'']).to_string(),
        ),
    };
    let prefix_lower = prefix.to_lowercase();
    let mut out = Vec::new();
    if !playlist_only && "current".starts_with(&prefix_lower) {
        out.push(Suggestion {
            start,
            end,
            label: "current".to_string(),
            description: "Add the currently playing track".to_string(),
            completion: Some("current".to_string()),
        });
    }
    for row in context.rows.iter().filter(|row| row.kind == "playlist") {
        if !prefix_lower.is_empty() && !row.title.to_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        out.push(Suggestion {
            start,
            end,
            label: row.title.clone(),
            description: row.subtitle.clone(),
            completion: Some(quote_arg(&row.title)),
        });
    }
    out
}

/// Completion for the download grammar. The command line context is a
/// snapshot, so suggestions stay synchronous; execution resolves live search.
fn download_suggestions(
    input: &str,
    cursor: usize,
    start: usize,
    end: usize,
    context: &SuggestionContext,
) -> Vec<Suggestion> {
    let _ = cursor;
    let prefix = input[start..end].trim_matches(['"', '\'']);
    let before = input[..start].trim_end();
    let args: Vec<&str> = before.split_whitespace().skip(1).collect();
    let args: Vec<&str> = args.into_iter().filter(|arg| *arg != "rm").collect();
    let mut out = Vec::new();
    let mut push = |label: String, description: &str, completion: Option<String>| {
        out.push(Suggestion {
            start,
            end,
            label,
            description: description.to_string(),
            completion,
        });
    };
    if args.is_empty() {
        if let Some(current) = context.current.as_ref() {
            let path = current_download_path(current);
            push(
                format!("current track: {path}"),
                "Download the currently playing track (or add album/track scopes)",
                Some(path),
            );
        }
        for (value, description) in [
            ("current", "Use the currently playing track"),
            ("rm", "Remove a download instead"),
            ("artist", "Search and download an artist"),
            ("album", "Search and download an album"),
            ("track", "Search and download a track"),
        ] {
            if value.starts_with(prefix) {
                push(value.to_string(), description, Some(value.to_string()));
            }
        }
        return out;
    }
    let scope = args.iter().rev().find_map(|arg| match *arg {
        "artist" => Some(("artist", DownloadScope::Artist)),
        "album" => Some(("album", DownloadScope::Album)),
        "track" | "song" => Some(("track", DownloadScope::Track)),
        _ => None,
    });
    if let Some((kind, _)) = scope {
        if let Some(current) = context.current.as_ref()
            && kind == "track"
        {
            let path = current_download_path(current);
            if path.to_lowercase().contains(&prefix.to_lowercase()) {
                push(
                    format!("current track: {path}"),
                    "Use the currently playing track",
                    Some(path),
                );
            }
        }
        for row in context.rows.iter().filter(|row| row.kind == kind) {
            if !prefix.is_empty() && !row.title.to_lowercase().starts_with(&prefix.to_lowercase()) {
                continue;
            }
            push(
                row.title.clone(),
                &row.subtitle,
                Some(quote_arg(&row.title)),
            );
        }
    }
    out
}

fn current_download_path(current: &CurrentTrack) -> String {
    let mut path = quote_arg(&current.artist);
    if !current.album.is_empty() {
        path.push_str(" album ");
        path.push_str(&quote_arg(&current.album));
    }
    if !current.title.is_empty() {
        path.push_str(" track ");
        path.push_str(&quote_arg(&current.title));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_maintenance_commands() {
        assert_eq!(parse("sync"), Ok(IpcCommand::Sync(SyncMode::Incremental)));
        assert_eq!(parse("sync all"), Ok(IpcCommand::Sync(SyncMode::All)));
        assert_eq!(parse("sync repair"), Ok(IpcCommand::Sync(SyncMode::Repair)));
        assert_eq!(parse("wipe album"), Ok(IpcCommand::Wipe(WipeScope::Album)));
        assert_eq!(
            parse("wipe lyrics"),
            Ok(IpcCommand::Wipe(WipeScope::Lyrics))
        );
        assert_eq!(
            parse("wipe covers"),
            Ok(IpcCommand::Wipe(WipeScope::Covers))
        );
        assert!(parse("wipe").is_err());
        assert!(parse("wipe nope").is_err());
        assert!(parse("wipe album extra").is_err());
    }

    #[test]
    fn parses_aliases_and_numbers() {
        assert_eq!(parse("next"), Ok(IpcCommand::NextTrack));
        assert_eq!(parse("prev"), Ok(IpcCommand::PrevTrack));
        assert_eq!(parse("volume 40"), Ok(IpcCommand::SetVolume(40.0)));
        assert_eq!(parse("seek 30"), Ok(IpcCommand::Seek(30.0)));
        assert!(parse("set_volume 101").is_err());
        assert_eq!(parse("mpris_volume:40"), Ok(IpcCommand::SetVolume(40.0)));
        assert_eq!(parse("fullscreen"), Ok(IpcCommand::ToggleFullscreen));
    }

    #[test]
    fn free_text_keeps_colons() {
        assert_eq!(
            parse("search http://example.test/a:b"),
            Ok(IpcCommand::Search {
                query: "http://example.test/a:b".into()
            })
        );
    }

    #[test]
    fn parses_shuffle_radio_and_download() {
        assert_eq!(
            parse("shuffle queue"),
            Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Queue
            })
        );
        assert_eq!(
            parse("shuffle album"),
            Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Album(None)
            })
        );
        assert_eq!(
            parse("shuffle artist a1"),
            Ok(IpcCommand::Shuffle {
                target: ShuffleTarget::Artist(Some("a1".into()))
            })
        );
        assert!(parse("shuffle").is_err());
        assert!(parse("shuffle album extra nope").is_err());
        assert_eq!(
            parse("radio"),
            Ok(IpcCommand::Radio {
                seed: RadioSeed::Current
            })
        );
        assert_eq!(
            parse("radio song s1"),
            Ok(IpcCommand::Radio {
                seed: RadioSeed::Song(Some("s1".into()))
            })
        );
        assert!(parse("radio artist").is_err());
        assert_eq!(
            parse("download"),
            Ok(IpcCommand::Download {
                request: DownloadRequest {
                    remove: false,
                    mode: DownloadMode::Current
                }
            })
        );
        assert_eq!(
            parse("download rm"),
            Ok(IpcCommand::Download {
                request: DownloadRequest {
                    remove: true,
                    mode: DownloadMode::Current
                }
            })
        );
        assert_eq!(
            parse("download track \"Fat Old Sun\""),
            Ok(IpcCommand::Download {
                request: DownloadRequest {
                    remove: false,
                    mode: DownloadMode::Direct {
                        scope: DownloadScope::Track,
                        query: "Fat Old Sun".into()
                    }
                }
            })
        );
        assert_eq!(
            parse("download \"Pink Floyd\" album \"Atom Heart Mother\" \"Fat Old Sun\""),
            Ok(IpcCommand::Download {
                request: DownloadRequest {
                    remove: false,
                    mode: DownloadMode::Path {
                        artist: "Pink Floyd".into(),
                        album: Some("Atom Heart Mother".into()),
                        track: Some("Fat Old Sun".into())
                    }
                }
            })
        );
        assert_eq!(
            parse("download rm \"Pink Floyd\""),
            Ok(IpcCommand::Download {
                request: DownloadRequest {
                    remove: true,
                    mode: DownloadMode::Path {
                        artist: "Pink Floyd".into(),
                        album: None,
                        track: None
                    }
                }
            })
        );
        assert!(parse("download \"unterminated").is_err());
        assert!(parse("download a b c d").is_err());
    }

    #[test]
    fn scoped_suggestion_request_uses_the_previous_arg() {
        let request = suggestion_request("shuffle artist \"P", 19).expect("artist scope");
        assert_eq!(request.scope, SuggestScope::Artist);
        assert_eq!(request.query, "P");
        let request = suggestion_request("radio album \"K", 16).expect("album scope");
        assert_eq!(request.scope, SuggestScope::Album);
        assert_eq!(request.query, "K");
        let request = suggestion_request("download track \"Fat", 21).expect("track scope");
        assert_eq!(request.scope, SuggestScope::Track);
        let request =
            suggestion_request("download \"Pink Floyd\" album \"A", 31).expect("album path scope");
        assert_eq!(request.scope, SuggestScope::Album);
        assert_eq!(request.query, "A");
        assert!(suggestion_request("shuffle queue", 13).is_none());
    }

    #[test]
    fn playlist_add_suggests_current_and_playlist_names() {
        let context = SuggestionContext {
            current: None,
            rows: vec![ContextRow {
                kind: "playlist",
                id: "pl1".into(),
                title: "Road Trip".into(),
                subtitle: "3 songs".into(),
            }],
        };
        let request = suggestion_request("playlist add P", 16).expect("mixed scope");
        assert_eq!(request.scope, SuggestScope::Any);
        assert_eq!(request.query, "P");
        let suggestions = suggestions_with("playlist add ", 14, &context);
        assert!(suggestions.iter().any(|s| s.label == "current"));
        assert!(suggestions.iter().any(|s| s.label == "Road Trip"));
    }

    #[test]
    fn playlist_add_after_cutoff_suggests_only_playlist_names() {
        let context = SuggestionContext {
            current: None,
            rows: vec![ContextRow {
                kind: "playlist",
                id: "p1".into(),
                title: "Road Trip".into(),
                subtitle: "Playlist · 4 songs".into(),
            }],
        };
        // Right after the cutoff with an empty name: async playlist scope,
        // empty query (all playlists), and only playlist rows locally.
        let empty = "playlist add current; ";
        let request = suggestion_request(empty, empty.len()).expect("playlist scope");
        assert_eq!(request.scope, SuggestScope::Playlist);
        assert_eq!(request.query, "");
        let suggestions = suggestions_with(empty, empty.len(), &context);
        assert!(suggestions.iter().any(|s| s.label == "Road Trip"));
        assert!(!suggestions.iter().any(|s| s.label == "current"));

        // A typed name filters to playlist names and replaces the whole name
        // region, so a multi-word name completes in one Tab.
        let typed = "playlist add current; Road T";
        let request = suggestion_request(typed, typed.len()).expect("playlist scope");
        assert_eq!(request.scope, SuggestScope::Playlist);
        assert_eq!(request.query, "Road T");
        let suggestions = suggestions_with(typed, typed.len(), &context);
        let road = suggestions
            .iter()
            .find(|s| s.label == "Road Trip")
            .expect("playlist name suggested");
        assert_eq!(road.completion.as_deref(), Some("\"Road Trip\""));
        let region = &typed[road.start..road.end];
        assert_eq!(region, "Road T", "the whole typed name is replaced");
        assert!(!suggestions.iter().any(|s| s.label == "current"));
    }

    #[test]
    fn playlist_rm_uses_the_same_cutoff_suggestions() {
        let context = SuggestionContext {
            current: None,
            rows: vec![ContextRow {
                kind: "playlist",
                id: "p1".into(),
                title: "Road Trip".into(),
                subtitle: "Playlist · 4 songs".into(),
            }],
        };
        let input = "playlist rm current; Ro";
        let request = suggestion_request(input, input.len()).expect("playlist scope");
        assert_eq!(request.scope, SuggestScope::Playlist);
        assert_eq!(request.query, "Ro");
        let suggestions = suggestions_with(input, input.len(), &context);
        assert!(suggestions.iter().any(|s| s.label == "Road Trip"));
        assert!(!suggestions.iter().any(|s| s.label == "current"));
    }

    #[test]
    fn playlist_rm_semicolon_asks_for_playlist_name_suggestions() {
        let context = SuggestionContext {
            current: None,
            rows: vec![ContextRow {
                kind: "playlist",
                id: "p1".into(),
                title: "Soft Songs".into(),
                subtitle: "Playlist · 6 songs".into(),
            }],
        };
        let input = "playlist rm ; ";
        let request = suggestion_request(input, input.len()).expect("playlist scope");
        assert_eq!(request.scope, SuggestScope::Playlist);
        assert_eq!(request.query, "");
        let suggestions = suggestions_with(input, input.len(), &context);
        assert!(suggestions.iter().any(|s| s.label == "Soft Songs"));
        assert!(
            !suggestions.iter().any(|s| s.label == "current"),
            "deleting a playlist does not offer the current-track target"
        );
    }

    #[test]
    fn playlist_add_before_cutoff_keeps_target_suggestions() {
        let input = "playlist add Pin";
        let request = suggestion_request(input, input.len()).expect("target scope");
        assert_eq!(request.scope, SuggestScope::Any);
        assert_eq!(request.query, "Pin");
    }

    #[test]
    fn suggests_the_current_track_for_download() {
        let context = SuggestionContext {
            current: Some(CurrentTrack {
                id: "s1".into(),
                title: "Fat Old Sun".into(),
                artist: "Pink Floyd".into(),
                album: "Atom Heart Mother".into(),
                album_id: "al1".into(),
            }),
            rows: vec![ContextRow {
                kind: "album",
                id: "al1".into(),
                title: "Atom Heart Mother".into(),
                subtitle: "Pink Floyd".into(),
            }],
        };
        let suggestions = suggestions_with("download ", 9, &context);
        assert!(suggestions.iter().any(|s| s.completion.as_deref()
            == Some("\"Pink Floyd\" album \"Atom Heart Mother\" track \"Fat Old Sun\"")));
        let scopes = suggestions_with("download album ", 15, &context);
        assert!(scopes.iter().any(|s| s.label == "Atom Heart Mother"));
    }

    #[test]
    fn parses_account_playlist_and_radio_save_commands() {
        assert_eq!(parse("logout"), Ok(IpcCommand::Logout));
        assert_eq!(parse("add_account"), Ok(IpcCommand::AddAccount));
        assert_eq!(
            parse("remove_account"),
            Ok(IpcCommand::RemoveAccount { keep_data: false })
        );
        assert_eq!(
            parse("remove_account --keep-data"),
            Ok(IpcCommand::RemoveAccount { keep_data: true })
        );
        assert_eq!(
            parse("playlist create Road trip"),
            Ok(IpcCommand::PlaylistCreate {
                name: "Road trip".into(),
                offline: false
            })
        );
        assert_eq!(
            parse("playlist create \"Road trip\" --offline"),
            Ok(IpcCommand::PlaylistCreate {
                name: "Road trip".into(),
                offline: true
            })
        );
        assert!(parse("playlist create").is_err());
        assert!(parse("playlist create --offline").is_err());
        assert_eq!(
            parse("playlist add \"Pink Floyd\", \"King Crimson\" \"Model Man\" \"Road Trip\""),
            Ok(IpcCommand::PlaylistAdd {
                targets: vec![
                    "Pink Floyd".into(),
                    "King Crimson".into(),
                    "Model Man".into()
                ],
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("playlist add \"Pink Floyd\",\"King Crimson\" \"Road Trip\""),
            Ok(IpcCommand::PlaylistAdd {
                targets: vec!["Pink Floyd".into(), "King Crimson".into()],
                playlist: "Road Trip".into()
            })
        );
        assert!(parse("playlist add \"Pink Floyd\"").is_err());
        assert!(parse("playlist nope").is_err());
        assert_eq!(
            canonical_string(&parse("playlist add \"Pink Floyd\" \"Road Trip\"").unwrap()),
            "playlist add \"Pink Floyd\"; \"Road Trip\""
        );
        // New cutoff syntax: targets before `;`, playlist name after it.
        assert_eq!(
            parse("playlist add current; \"Road Trip\""),
            Ok(IpcCommand::PlaylistAdd {
                targets: vec!["current".into()],
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("playlist add \"Pink Floyd\", \"King Crimson\"; Road Trip"),
            Ok(IpcCommand::PlaylistAdd {
                targets: vec!["Pink Floyd".into(), "King Crimson".into()],
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("playlist add \"A;B\"; \"P\""),
            Ok(IpcCommand::PlaylistAdd {
                targets: vec!["A;B".into()],
                playlist: "P".into()
            }),
            "a semicolon inside a quoted target must not cut the list"
        );
        assert!(parse("playlist add current;").is_err());
        assert!(parse("playlist add ; \"P\"").is_err());
        // The legacy last-token form still parses.
        assert_eq!(
            parse("playlist add current \"Road Trip\""),
            Ok(IpcCommand::PlaylistAdd {
                targets: vec!["current".into()],
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("radio save"),
            Ok(IpcCommand::RadioSave { offline: false })
        );
        assert_eq!(
            parse("radio save --offline"),
            Ok(IpcCommand::RadioSave { offline: true })
        );
        assert!(parse("radio save extra").is_err());
    }

    #[test]
    fn parses_payload_and_gesture_commands() {
        assert_eq!(parse("play_song s1"), Ok(IpcCommand::PlaySong("s1".into())));
        assert_eq!(
            parse("quick_aim west"),
            Ok(IpcCommand::QuickAim(QuickDirection::West))
        );
        assert_eq!(parse("quick_hold on"), Ok(IpcCommand::QuickHold(true)));
        assert_eq!(
            parse("search hello world"),
            Ok(IpcCommand::Search {
                query: "hello world".into()
            })
        );
    }

    #[test]
    fn suggests_placeholder_for_free_arguments() {
        let volume = suggestions("set_volume ", 11);
        assert_eq!(volume.len(), 1);
        assert!(volume[0].completion.is_none());
        assert!(volume[0].label.contains("<0..100>"));
        let payload = suggestions("play_song ", 10);
        assert_eq!(payload.len(), 1);
        assert!(payload[0].label.contains("<song-id>"));
    }

    #[test]
    fn suggests_commands_and_arguments() {
        let commands = suggestions("syn", 3);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].completion.as_deref(), Some("sync"));

        let args = suggestions("sync ", 5);
        assert_eq!(
            args.iter()
                .filter_map(|s| s.completion.as_deref())
                .collect::<Vec<_>>(),
            vec!["all", "repair"]
        );
        let scopes = suggestions("wipe a", 6);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].completion.as_deref(), Some("album"));
    }

    #[test]
    fn parses_every_canonical_command() {
        let samples = [
            "sync",
            "sync all",
            "sync repair",
            "wipe album",
            "wipe lyrics",
            "wipe covers",
            "wipe_databases",
            "logout",
            "add_account",
            "remove_account",
            "remove_account --keep-data",
            "playlist create Road Trip",
            "playlist create Road Trip --offline",
            "playlist add \"Pink Floyd\" \"King Crimson\" \"Road Trip\"",
            "radio save",
            "radio save --offline",
            "shuffle queue",
            "shuffle album",
            "shuffle album a1",
            "shuffle artist",
            "shuffle artist ar1",
            "radio current",
            "radio album",
            "radio album al1",
            "radio song",
            "radio song s1",
            "download current",
            "download rm current",
            "download track \"Fat Old Sun\"",
            "download \"Pink Floyd\"",
            "download \"Pink Floyd\" album \"Atom Heart Mother\"",
            "download \"Pink Floyd\" album \"Atom Heart Mother\" track \"Fat Old Sun\"",
            "refresh",
            "top",
            "bottom",
            "move_up",
            "move_down",
            "scroll_up",
            "scroll_down",
            "page_up",
            "page_down",
            "move_left",
            "move_right",
            "prev_place",
            "next_place",
            "home",
            "back",
            "enter",
            "scope_next",
            "scope_prev",
            "order_next",
            "order_prev",
            "visual",
            "visual_all",
            "queue_toggle",
            "queue_remove",
            "queue_up",
            "queue_down",
            "yank",
            "yank_row",
            "paste_below",
            "paste_above",
            "delete_row",
            "repeat_cycle",
            "toggle_fullscreen",
            "lyrics_overlay",
            "pane_cycle",
            "pane_cycle back",
            "search",
            "search hello world",
            "insert_mode",
            "insert_mode hello",
            "search_next",
            "search_prev",
            "search_scope_next",
            "search_scope_prev",
            "line_search",
            "play_pause",
            "play",
            "pause",
            "stop",
            "next_track",
            "prev_track",
            "mute",
            "volume_up",
            "volume_down",
            "set_volume 40",
            "seek_back",
            "seek_forward",
            "seek 30",
            "seek_relative -5",
            "play_song s1",
            "play_song_in_album s1",
            "open_song_album s1",
            "download_song s1",
            "quick_hold on",
            "quick_hold off",
            "quick_aim north",
            "quick_cycle next",
            "quick_cycle prev",
            "quick_cancel",
            "show",
            "random",
            "quit",
        ];
        for sample in samples {
            let parsed = parse(sample).unwrap_or_else(|err| panic!("{sample}: {err}"));
            let canonical = canonical_string(&parsed);
            assert_eq!(
                parse(&canonical).unwrap_or_else(|err| panic!("{canonical}: {err}")),
                parsed,
                "round trip for {sample}"
            );
        }
    }

    #[test]
    fn parses_aliases_and_mpris_forms() {
        let samples = [
            "next",
            "prev",
            "previous",
            "volume 40",
            "fullscreen",
            "lyrics",
            "cancel",
            "search_action",
            "insert",
            "mpris_play",
            "mpris_pause",
            "mpris_play_pause",
            "mpris_stop",
            "mpris_next",
            "mpris_previous",
            "mpris_volume:40",
            "mpris_seek:5000000",
            "mpris_position:5000000",
        ];
        for sample in samples {
            assert!(parse(sample).is_ok(), "{sample}");
        }
    }

    #[test]
    fn canonical_round_trips() {
        assert_eq!(
            canonical_string(&IpcCommand::Sync(SyncMode::All)),
            "sync all"
        );
        assert_eq!(
            canonical_string(&IpcCommand::Wipe(WipeScope::Lyrics)),
            "wipe lyrics"
        );
        assert_eq!(
            canonical_string(&IpcCommand::QuickCycle { next: false }),
            "quick_cycle prev"
        );
        assert_eq!(canonical_string(&parse("next").unwrap()), "next_track");
    }
}
