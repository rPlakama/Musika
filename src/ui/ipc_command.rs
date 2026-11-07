//! Shared textual IPC command registry.
//!
//! The Unix-socket IPC handler and the `:` command line both parse through
//! this module. Keep command names, aliases, required arguments and
//! suggestions in one place so the two surfaces cannot drift.

use std::fmt;

use super::bridge::SyncMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeScope {
    Album,
    Lyrics,
    Covers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    RepeatCycle,
    ToggleFullscreen,
    LyricsOverlay,
    PaneCycle { backward: bool },
    Search { query: String },
    InsertMode { query: String },
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
    QuickCycle { next: bool },
    QuickCancel,
    Quit,
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

pub fn suggestions(input: &str, cursor: usize) -> Vec<Suggestion> {
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
