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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackCommand { Next, Previous }

/// Three-state crossfade control for the text/IPC command surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossfadeCommand {
    Toggle,
    On,
    Off,
}

/// Loudness-normalization control for the text/IPC command surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizeCommand {
    Cycle,
    Off,
    Track,
    Album,
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
    /// Append the UI's current selection (the `visual` range or cursor row) to
    /// a playlist: `playlist add selected; <playlist>`.
    PlaylistAddSelected {
        playlist: String,
    },
    /// Append the whole playback queue to a playlist, in queue order:
    /// `playlist add queue; <playlist>`. The queue is core state, so the daemon
    /// resolves it from its playback snapshot; an attached UI forwards its
    /// mirror of the same queue.
    PlaylistAddQueue {
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
    Crossfade(CrossfadeCommand),
    Normalize(NormalizeCommand),
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
    Track(TrackCommand),
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
    /// Read-only: one JSON line describing the playback state.
    Status,
    /// Read-only: one JSON line with the queue ids.
    QueueQuery,
}

/// Where a command must run in the core/UI split. The classification is
/// exhaustive by construction: every command names exactly one target, and a
/// new variant cannot compile without landing in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTarget {
    /// Playback/transport, executed by the core's playback service.
    CorePlayback,
    /// Core-owned data work (sync, downloads, playlists, radio, accounts).
    CoreBridge,
    /// Read-only telemetry answered by the core without a UI.
    Query,
    /// Presentation/navigation, forwarded to an attached UI.
    Presentation,
    /// Process lifecycle (quit/show), handled by the daemon itself.
    Lifecycle,
}

impl IpcCommand {
    pub fn target(&self) -> CommandTarget {
        use CommandTarget as T;
        match self {
            // Playback and transport: the core owns the audio engine.
            Self::PlayPause
            | Self::Play
            | Self::Pause
            | Self::Stop
            | Self::Track(_)
            | Self::Mute
            | Self::VolumeUp
            | Self::VolumeDown
            | Self::SetVolume(_)
            | Self::SeekBack
            | Self::SeekForward
            | Self::Seek(_)
            | Self::SeekRelative(_)
            | Self::RepeatCycle
            | Self::Crossfade(_)
            | Self::Normalize(_) => T::CorePlayback,
            // `shuffle queue` is playback; album/artist shuffles build a queue
            // from the library through the bridge.
            Self::Shuffle {
                target: ShuffleTarget::Queue,
            } => T::CorePlayback,
            Self::Shuffle { .. } => T::CoreBridge,

            // Core-owned data work. PlaySong/PlaySongInAlbum/DownloadSong are
            // core-side so the daemon stays drivable with no UI attached.
            Self::Sync(_)
            | Self::Wipe(_)
            | Self::Logout
            | Self::RemoveAccount { .. }
            | Self::PlaylistCreate { .. }
            | Self::PlaylistAdd { .. }
            | Self::PlaylistAddQueue { .. }
            | Self::PlaylistRemove { .. }
            | Self::PlaylistDelete { .. }
            | Self::Radio { .. }
            | Self::Download { .. }
            | Self::Refresh
            | Self::RandomAlbum
            | Self::PlaySong(_)
            | Self::PlaySongInAlbum(_)
            | Self::DownloadSong(_) => T::CoreBridge,

            // Read-only telemetry.
            Self::Status | Self::QueueQuery => T::Query,

            // Lifecycle.
            Self::Quit | Self::Show => T::Lifecycle,

            // Everything else is presentation and belongs to the UI process.
            Self::Top
            | Self::Bottom
            | Self::MoveUp
            | Self::MoveDown
            | Self::ScrollUp
            | Self::ScrollDown
            | Self::PageUp
            | Self::PageDown
            | Self::MoveLeft
            | Self::MoveRight
            | Self::PrevPlace
            | Self::NextPlace
            | Self::Home
            | Self::Back
            | Self::Enter
            | Self::ScopeNext
            | Self::ScopePrev
            | Self::OrderNext
            | Self::OrderPrev
            | Self::Visual
            | Self::VisualAll
            | Self::QueueToggle
            | Self::QueueRemove
            | Self::QueueUp
            | Self::QueueDown
            | Self::Yank
            | Self::YankRow
            | Self::PasteBelow
            | Self::PasteAbove
            | Self::DeleteRow
            | Self::ToggleFullscreen
            | Self::LyricsOverlay
            | Self::PaneCycle { .. }
            | Self::Search { .. }
            | Self::InsertMode { .. }
            | Self::SearchNext
            | Self::SearchPrev
            | Self::SearchScopeNext
            | Self::SearchScopePrev
            | Self::LineSearch
            | Self::OpenSongAlbum(_)
            | Self::QuickHold(_)
            | Self::QuickAim(_)
            | Self::QuickCycle { .. }
            | Self::QuickCancel
            // Commands that open a UI surface (login form, wipe chooser,
            // radio-save page state) only make sense with a UI attached.
            | Self::AddAccount
            | Self::WipeDatabases
            | Self::RadioSave { .. }
            // The selection lives in the UI process, so the selected-rows
            // playlist add is presentation work like `yank`/`delete_row`.
            | Self::PlaylistAddSelected { .. } => T::Presentation,
        }
    }
}

impl IpcCommand {
    /// Translate a text playback command into the typed wire request. The core
    /// owns the authoritative position/volume, so relative controls stay
    /// relative instead of being resolved against a stale UI copy.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "open playback dispatch over the 83-variant IpcCommand; the None fallback is the default for the other ~68 commands"
    )]
    pub fn player_op(&self) -> Option<crate::ipc_protocol::PlayerOp> {
        use crate::ipc_protocol::PlayerOp as P;
        use IpcCommand as I;
        Some(match self {
            I::PlayPause => P::PlayPause,
            I::Play => P::Play,
            I::Pause => P::Pause,
            I::Stop => P::Stop,
            I::Track(TrackCommand::Next) => P::Next,
            I::Track(TrackCommand::Previous) => P::Previous,
            I::Mute => P::ToggleMute,
            I::VolumeUp => P::VolumeUp,
            I::VolumeDown => P::VolumeDown,
            I::SetVolume(volume) => P::SetVolume(*volume),
            I::Seek(seconds) => P::Seek(*seconds),
            I::SeekBack => P::SeekRelative(-5.0),
            I::SeekForward => P::SeekRelative(5.0),
            I::SeekRelative(offset) => P::SeekRelative(*offset),
            I::RepeatCycle => P::CycleRepeat,
            I::Crossfade(crossfade) => match crossfade {
                CrossfadeCommand::Toggle => P::ToggleCrossfade,
                CrossfadeCommand::On => P::SetCrossfade(true),
                CrossfadeCommand::Off => P::SetCrossfade(false),
            },
            I::Normalize(normalize) => match normalize {
                NormalizeCommand::Cycle => P::CycleNormalization,
                NormalizeCommand::Off => P::SetNormalization(crate::player::Normalization::Off.code()),
                NormalizeCommand::Track => P::SetNormalization(crate::player::Normalization::Track.code()),
                NormalizeCommand::Album => P::SetNormalization(crate::player::Normalization::Album.code()),
            },
            I::Shuffle {
                target: ShuffleTarget::Queue,
            } => P::ShuffleQueue,
            _ => return None,
        })
    }
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

impl DownloadScope {
    /// The canonical item kind this command scope names (`track` is a song).
    pub fn item_kind(self) -> crate::model::ItemKind {
        match self {
            Self::Artist => crate::model::ItemKind::Artist,
            Self::Album => crate::model::ItemKind::Album,
            Self::Track => crate::model::ItemKind::Song,
        }
    }
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

impl SuggestScope {
    /// The canonical item kind this suggestion column names; `Any` spans all
    /// of them and has no single kind.
    pub fn item_kind(self) -> Option<crate::model::ItemKind> {
        use crate::model::ItemKind;
        Some(match self {
            Self::Artist => ItemKind::Artist,
            Self::Album => ItemKind::Album,
            Self::Track => ItemKind::Song,
            Self::Playlist => ItemKind::Playlist,
            Self::Any => return None,
        })
    }
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

/// How one command is parsed and printed. The registry declares this once;
/// `parse` and `canonical_string` read the same declaration.
#[derive(Debug, Clone)]
pub enum CommandForm {
    /// No arguments. Parsing ignores trailing words (the hand-written arms
    /// did) and the canonical form is just the command name.
    Bare(IpcCommand),
    /// One word argument: `name <value>`. Unknown values name the scope.
    Word {
        values: &'static [(&'static str, IpcCommand)],
        unknown: &'static str,
    },
    /// A whole-rest argument: empty selects the `""` entry, anything else must
    /// equal a listed value. A bad argument reports the command's usage.
    Exact(&'static [(&'static str, IpcCommand)]),
    /// Grammar that needs the raw rest (ids, numbers, quoted targets, flags);
    /// `parse`/`canonical_string` keep explicit arms for these.
    Advanced(ArgHints),
}

/// The argument completion metadata an [`CommandForm::Advanced`] command
/// carries (the hand-written grammar cannot derive it).
#[derive(Debug, Clone, Copy)]
pub enum ArgHints {
    None,
    /// One of the listed words.
    Values(&'static [&'static str]),
    /// A free argument shown as a usage hint (`<seconds>`).
    Label(&'static str),
}

impl CommandSpec {
    /// Whether completing this command's name should continue into arguments.
    pub fn needs_argument(&self) -> bool {
        !matches!(self.form, CommandForm::Bare(_))
    }
}

pub struct CommandSpec {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
    /// How the command is parsed and printed.
    pub form: CommandForm,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "sync",
        usage: "sync [all|repair]",
        description: "Sync the library (incremental, deep, or repair)",
        form: CommandForm::Exact(&[("", IpcCommand::Sync(SyncMode::Incremental)), ("all", IpcCommand::Sync(SyncMode::All)), ("repair", IpcCommand::Sync(SyncMode::Repair))]),
    },
    CommandSpec {
        name: "wipe",
        usage: "wipe <album|lyrics|covers>",
        description: "Clear downloaded albums or a cache",
        form: CommandForm::Word { values: &[("album", IpcCommand::Wipe(WipeScope::Album)), ("lyrics", IpcCommand::Wipe(WipeScope::Lyrics)), ("covers", IpcCommand::Wipe(WipeScope::Covers))], unknown: "wipe scope" },
    },
    CommandSpec {
        name: "wipe_databases",
        usage: "wipe_databases",
        description: "Open the destructive wipe chooser",
        form: CommandForm::Bare(IpcCommand::WipeDatabases),
    },
    CommandSpec {
        name: "logout",
        usage: "logout",
        description: "Log out of the current account",
        form: CommandForm::Bare(IpcCommand::Logout),
    },
    CommandSpec {
        name: "add_account",
        usage: "add_account",
        description: "Open the login screen to add an account",
        form: CommandForm::Bare(IpcCommand::AddAccount),
    },
    CommandSpec {
        name: "remove_account",
        usage: "remove_account [--keep-data]",
        description: "Remove the current account",
        form: CommandForm::Exact(&[("", IpcCommand::RemoveAccount { keep_data: false }), ("--keep-data", IpcCommand::RemoveAccount { keep_data: true })]),
    },
    CommandSpec {
        name: "playlist",
        usage: "playlist <create|add|rm> ...",
        description: "Create, add/remove targets or delete a playlist (playlist add queue; <name>, playlist add selected; <name>, playlist rm <name>)",
        form: CommandForm::Advanced(ArgHints::Values(&["create", "add", "rm"])),
    },
    CommandSpec {
        name: "shuffle",
        usage: "shuffle <queue|album|artist> [name]",
        description: "Shuffle the queue, an album, or an artist",
        form: CommandForm::Advanced(ArgHints::Values(&["queue", "album", "artist"])),
    },
    CommandSpec {
        name: "radio",
        usage: "radio [current|album|song] [name]",
        description: "Start radio from the current context or an explicit seed",
        form: CommandForm::Advanced(ArgHints::Values(&["save", "current", "album", "song"])),
    },
    CommandSpec {
        name: "download",
        usage: "download [rm] [artist|album|track] <names...>",
        description: "Download the current track or a searched artist/album/track",
        form: CommandForm::Advanced(ArgHints::Label("[rm] <artist> [album] [track]")),
    },
    CommandSpec {
        name: "refresh",
        usage: "refresh",
        description: "Start an incremental sync",
        form: CommandForm::Bare(IpcCommand::Refresh),
    },
    CommandSpec {
        name: "status",
        usage: "status",
        description: "Print the playback state as one JSON line",
        form: CommandForm::Bare(IpcCommand::Status),
    },
    CommandSpec {
        name: "queue",
        usage: "queue",
        description: "Print the queue ids as one JSON line",
        form: CommandForm::Bare(IpcCommand::QueueQuery),
    },
    CommandSpec {
        name: "top",
        usage: "top",
        description: "Jump to the first row",
        form: CommandForm::Bare(IpcCommand::Top),
    },
    CommandSpec {
        name: "bottom",
        usage: "bottom",
        description: "Jump to the last row",
        form: CommandForm::Bare(IpcCommand::Bottom),
    },
    CommandSpec {
        name: "move_up",
        usage: "move_up",
        description: "Move the cursor up",
        form: CommandForm::Bare(IpcCommand::MoveUp),
    },
    CommandSpec {
        name: "move_down",
        usage: "move_down",
        description: "Move the cursor down",
        form: CommandForm::Bare(IpcCommand::MoveDown),
    },
    CommandSpec {
        name: "scroll_up",
        usage: "scroll_up",
        description: "Scroll up one row",
        form: CommandForm::Bare(IpcCommand::ScrollUp),
    },
    CommandSpec {
        name: "scroll_down",
        usage: "scroll_down",
        description: "Scroll down one row",
        form: CommandForm::Bare(IpcCommand::ScrollDown),
    },
    CommandSpec {
        name: "page_up",
        usage: "page_up",
        description: "Move one page up",
        form: CommandForm::Bare(IpcCommand::PageUp),
    },
    CommandSpec {
        name: "page_down",
        usage: "page_down",
        description: "Move one page down",
        form: CommandForm::Bare(IpcCommand::PageDown),
    },
    CommandSpec {
        name: "move_left",
        usage: "move_left",
        description: "Move left within the active surface",
        form: CommandForm::Bare(IpcCommand::MoveLeft),
    },
    CommandSpec {
        name: "move_right",
        usage: "move_right",
        description: "Move right within the active surface",
        form: CommandForm::Bare(IpcCommand::MoveRight),
    },
    CommandSpec {
        name: "prev_place",
        usage: "prev_place",
        description: "Go to the previous place",
        form: CommandForm::Bare(IpcCommand::PrevPlace),
    },
    CommandSpec {
        name: "next_place",
        usage: "next_place",
        description: "Go to the next place",
        form: CommandForm::Bare(IpcCommand::NextPlace),
    },
    CommandSpec {
        name: "home",
        usage: "home",
        description: "Return to the Home library",
        form: CommandForm::Bare(IpcCommand::Home),
    },
    CommandSpec {
        name: "back",
        usage: "back",
        description: "Back out of the current surface",
        form: CommandForm::Bare(IpcCommand::Back),
    },
    CommandSpec {
        name: "enter",
        usage: "enter",
        description: "Activate the current selection",
        form: CommandForm::Bare(IpcCommand::Enter),
    },
    CommandSpec {
        name: "scope_next",
        usage: "scope_next",
        description: "Next library scope",
        form: CommandForm::Bare(IpcCommand::ScopeNext),
    },
    CommandSpec {
        name: "scope_prev",
        usage: "scope_prev",
        description: "Previous library scope",
        form: CommandForm::Bare(IpcCommand::ScopePrev),
    },
    CommandSpec {
        name: "order_next",
        usage: "order_next",
        description: "Next library order",
        form: CommandForm::Bare(IpcCommand::OrderNext),
    },
    CommandSpec {
        name: "order_prev",
        usage: "order_prev",
        description: "Previous library order",
        form: CommandForm::Bare(IpcCommand::OrderPrev),
    },
    CommandSpec {
        name: "visual",
        usage: "visual",
        description: "Toggle visual selection",
        form: CommandForm::Bare(IpcCommand::Visual),
    },
    CommandSpec {
        name: "visual_all",
        usage: "visual_all",
        description: "Select every visible row",
        form: CommandForm::Bare(IpcCommand::VisualAll),
    },
    CommandSpec {
        name: "queue_toggle",
        usage: "queue_toggle",
        description: "Open or close the queue",
        form: CommandForm::Bare(IpcCommand::QueueToggle),
    },
    CommandSpec {
        name: "queue_remove",
        usage: "queue_remove",
        description: "Remove the queue cursor row",
        form: CommandForm::Bare(IpcCommand::QueueRemove),
    },
    CommandSpec {
        name: "queue_up",
        usage: "queue_up",
        description: "Move the queue row up",
        form: CommandForm::Bare(IpcCommand::QueueUp),
    },
    CommandSpec {
        name: "queue_down",
        usage: "queue_down",
        description: "Move the queue row down",
        form: CommandForm::Bare(IpcCommand::QueueDown),
    },
    CommandSpec {
        name: "yank",
        usage: "yank",
        description: "Yank the visual range or cursor row",
        form: CommandForm::Bare(IpcCommand::Yank),
    },
    CommandSpec {
        name: "yank_row",
        usage: "yank_row",
        description: "Yank the cursor row",
        form: CommandForm::Bare(IpcCommand::YankRow),
    },
    CommandSpec {
        name: "paste_below",
        usage: "paste_below",
        description: "Paste the row clipboard below the queue cursor or into the playlist",
        form: CommandForm::Bare(IpcCommand::PasteBelow),
    },
    CommandSpec {
        name: "paste_above",
        usage: "paste_above",
        description: "Paste the row clipboard above the queue cursor or into the playlist",
        form: CommandForm::Bare(IpcCommand::PasteAbove),
    },
    CommandSpec {
        name: "delete_row",
        usage: "delete_row",
        description: "Delete the selected queue rows or playlist entries",
        form: CommandForm::Bare(IpcCommand::DeleteRow),
    },
    CommandSpec {
        name: "repeat_cycle",
        usage: "repeat_cycle",
        description: "Cycle repeat mode",
        form: CommandForm::Bare(IpcCommand::RepeatCycle),
    },
    CommandSpec {
        name: "crossfade",
        usage: "crossfade [on|off]",
        description: "Toggle or set the optional track crossfade",
        form: CommandForm::Exact(&[
            ("", IpcCommand::Crossfade(CrossfadeCommand::Toggle)),
            ("on", IpcCommand::Crossfade(CrossfadeCommand::On)),
            ("off", IpcCommand::Crossfade(CrossfadeCommand::Off)),
        ]),
    },
    CommandSpec {
        name: "normalize",
        usage: "normalize [off|track|album]",
        description: "Cycle or set ReplayGain loudness normalization",
        form: CommandForm::Exact(&[
            ("", IpcCommand::Normalize(NormalizeCommand::Cycle)),
            ("off", IpcCommand::Normalize(NormalizeCommand::Off)),
            ("track", IpcCommand::Normalize(NormalizeCommand::Track)),
            ("album", IpcCommand::Normalize(NormalizeCommand::Album)),
        ]),
    },
    CommandSpec {
        name: "toggle_fullscreen",
        usage: "toggle_fullscreen",
        description: "Toggle playback/fullscreen view",
        form: CommandForm::Bare(IpcCommand::ToggleFullscreen),
    },
    CommandSpec {
        name: "lyrics_overlay",
        usage: "lyrics_overlay",
        description: "Toggle the lyrics overlay",
        form: CommandForm::Bare(IpcCommand::LyricsOverlay),
    },
    CommandSpec {
        name: "pane_cycle",
        usage: "pane_cycle [back]",
        description: "Cycle the active pane/cursor scope",
        form: CommandForm::Exact(&[("", IpcCommand::PaneCycle { backward: false }), ("back", IpcCommand::PaneCycle { backward: true })]),
    },
    CommandSpec {
        name: "search",
        usage: "search [query]",
        description: "Open floating search, optionally with a query",
        form: CommandForm::Advanced(ArgHints::Label("<query>")),
    },
    CommandSpec {
        name: "insert_mode",
        usage: "insert_mode [query]",
        description: "Open search in insert mode",
        form: CommandForm::Advanced(ArgHints::Label("<query>")),
    },
    CommandSpec {
        name: "search_next",
        usage: "search_next",
        description: "Next floating-search row",
        form: CommandForm::Bare(IpcCommand::SearchNext),
    },
    CommandSpec {
        name: "search_prev",
        usage: "search_prev",
        description: "Previous floating-search row",
        form: CommandForm::Bare(IpcCommand::SearchPrev),
    },
    CommandSpec {
        name: "search_scope_next",
        usage: "search_scope_next",
        description: "Next floating-search scope",
        form: CommandForm::Bare(IpcCommand::SearchScopeNext),
    },
    CommandSpec {
        name: "search_scope_prev",
        usage: "search_scope_prev",
        description: "Previous floating-search scope",
        form: CommandForm::Bare(IpcCommand::SearchScopePrev),
    },
    CommandSpec {
        name: "line_search",
        usage: "line_search",
        description: "Open the / line search",
        form: CommandForm::Bare(IpcCommand::LineSearch),
    },
    CommandSpec {
        name: "play_pause",
        usage: "play_pause",
        description: "Toggle playback",
        form: CommandForm::Bare(IpcCommand::PlayPause),
    },
    CommandSpec {
        name: "play",
        usage: "play",
        description: "Resume playback",
        form: CommandForm::Bare(IpcCommand::Play),
    },
    CommandSpec {
        name: "pause",
        usage: "pause",
        description: "Pause playback",
        form: CommandForm::Bare(IpcCommand::Pause),
    },
    CommandSpec {
        name: "stop",
        usage: "stop",
        description: "Stop playback",
        form: CommandForm::Bare(IpcCommand::Stop),
    },
    CommandSpec {
        name: "track",
        usage: "track <next|previous>",
        description: "Control track playback",
        form: CommandForm::Word { values: &[("next", IpcCommand::Track(TrackCommand::Next)), ("previous", IpcCommand::Track(TrackCommand::Previous))], unknown: "track command" },
    },
    CommandSpec {
        name: "mute",
        usage: "mute",
        description: "Toggle mute",
        form: CommandForm::Bare(IpcCommand::Mute),
    },
    CommandSpec {
        name: "volume_up",
        usage: "volume_up",
        description: "Raise volume by 5",
        form: CommandForm::Bare(IpcCommand::VolumeUp),
    },
    CommandSpec {
        name: "volume_down",
        usage: "volume_down",
        description: "Lower volume by 5",
        form: CommandForm::Bare(IpcCommand::VolumeDown),
    },
    CommandSpec {
        name: "set_volume",
        usage: "set_volume <0..100>",
        description: "Set volume",
        form: CommandForm::Advanced(ArgHints::Label("<0..100>")),
    },
    CommandSpec {
        name: "seek_back",
        usage: "seek_back",
        description: "Seek back 5 seconds",
        form: CommandForm::Bare(IpcCommand::SeekBack),
    },
    CommandSpec {
        name: "seek_forward",
        usage: "seek_forward",
        description: "Seek forward 5 seconds",
        form: CommandForm::Bare(IpcCommand::SeekForward),
    },
    CommandSpec {
        name: "seek",
        usage: "seek <seconds>",
        description: "Seek to an absolute position",
        form: CommandForm::Advanced(ArgHints::Label("<seconds>")),
    },
    CommandSpec {
        name: "seek_relative",
        usage: "seek_relative <±seconds>",
        description: "Seek relative to the playhead",
        form: CommandForm::Advanced(ArgHints::Label("<±seconds>")),
    },
    CommandSpec {
        name: "play_song",
        usage: "play_song <song-id>",
        description: "Play one song in the current order",
        form: CommandForm::Advanced(ArgHints::Label("<song-id>")),
    },
    CommandSpec {
        name: "play_song_in_album",
        usage: "play_song_in_album <song-id>",
        description: "Play a song in its album queue",
        form: CommandForm::Advanced(ArgHints::Label("<song-id>")),
    },
    CommandSpec {
        name: "open_song_album",
        usage: "open_song_album <song-id>",
        description: "Open the song's album page",
        form: CommandForm::Advanced(ArgHints::Label("<song-id>")),
    },
    CommandSpec {
        name: "download_song",
        usage: "download_song <song-id>",
        description: "Download one song",
        form: CommandForm::Advanced(ArgHints::Label("<song-id>")),
    },
    CommandSpec {
        name: "quick_hold",
        usage: "quick_hold <on|off>",
        description: "Arm/release the Alt quick actions",
        form: CommandForm::Advanced(ArgHints::Values(&["on", "off"])),
    },
    CommandSpec {
        name: "quick_aim",
        usage: "quick_aim <north|south|east|west>",
        description: "Aim the Alt quick actions",
        form: CommandForm::Advanced(ArgHints::Values(&["north", "south", "east", "west"])),
    },
    CommandSpec {
        name: "quick_cycle",
        usage: "quick_cycle <next|prev>",
        description: "Cycle the aimed Alt action",
        form: CommandForm::Advanced(ArgHints::Values(&["next", "prev"])),
    },
    CommandSpec {
        name: "quick_cancel",
        usage: "quick_cancel",
        description: "Cancel the Alt quick actions",
        form: CommandForm::Bare(IpcCommand::QuickCancel),
    },
    CommandSpec {
        name: "show",
        usage: "show",
        description: "Show or raise the Musika window",
        form: CommandForm::Bare(IpcCommand::Show),
    },
    CommandSpec {
        name: "random",
        usage: "random",
        description: "Play a random album from the library",
        form: CommandForm::Bare(IpcCommand::RandomAlbum),
    },
    CommandSpec {
        name: "quit",
        usage: "quit",
        description: "Quit Musika",
        form: CommandForm::Bare(IpcCommand::Quit),
    },
];

fn canonical_name(first: &str) -> &str {
    match first {
        "cancel" => "back",
        "volume" => "set_volume",
        "fullscreen" => "toggle_fullscreen",
        "lyrics" => "lyrics_overlay",
        "search_action" => "line_search",
        "insert" => "insert_mode",
        "mpris_play" => "play",
        "mpris_pause" => "pause",
        "mpris_play_pause" => "play_pause",
        "mpris_stop" => "stop",
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
    match &spec.form {
        CommandForm::Bare(command) => Ok(command.clone()),
        CommandForm::Word { values, unknown } => {
            let value = one_arg(rest, spec.usage)?;
            values
                .iter()
                .find(|(label, _)| *label == value)
                .map(|(_, command)| command.clone())
                .ok_or_else(|| CommandError(format!("unknown {unknown}: {value}")))
        }
        CommandForm::Exact(values) => {
            let value = if rest.is_empty() { "" } else { rest };
            values
                .iter()
                .find(|(label, _)| *label == value)
                .map(|(_, command)| command.clone())
                .ok_or_else(|| CommandError(format!("usage: {}", spec.usage)))
        }
        CommandForm::Advanced(_) => advanced_parse(name, rest, spec.usage),
    }
}

/// The explicit grammar for the [`CommandForm::Advanced`] commands: ids,
/// numbers, quoted targets and the playlist/download/radio sub-grammars.
fn advanced_parse(name: &str, rest: &str, usage: &str) -> Result<IpcCommand, CommandError> {
    match name {

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

        "search" => Ok(IpcCommand::Search {
            query: rest.to_string(),
        }),

        "insert_mode" => Ok(IpcCommand::InsertMode {
            query: rest.to_string(),
        }),

        "set_volume" => {
            let volume = parse_number(one_arg(rest, usage)?, "volume")?;
            if !(0.0..=100.0).contains(&volume) {
                return Err(CommandError("volume must be in 0..=100".to_string()));
            }
            Ok(IpcCommand::SetVolume(volume))
        }
        "seek_back" => Ok(IpcCommand::SeekBack),

        "seek" => Ok(IpcCommand::Seek(
            parse_number(one_arg(rest, usage)?, "seconds")?.max(0.0),
        )),

        "seek_relative" => Ok(IpcCommand::SeekRelative(parse_number(
            one_arg(rest, usage)?,
            "seconds",
        )?)),

        "play_song" => Ok(IpcCommand::PlaySong(one_arg(rest, usage)?.to_string())),

        "play_song_in_album" => Ok(IpcCommand::PlaySongInAlbum(
            one_arg(rest, usage)?.to_string(),
        )),

        "open_song_album" => Ok(IpcCommand::OpenSongAlbum(
            one_arg(rest, usage)?.to_string(),
        )),

        "download_song" => Ok(IpcCommand::DownloadSong(
            one_arg(rest, usage)?.to_string(),
        )),

        "quick_hold" => Ok(IpcCommand::QuickHold(one_arg(rest, usage)? == "on")),

        "quick_aim" => Ok(IpcCommand::QuickAim(match one_arg(rest, usage)? {
            "north" => QuickDirection::North,
            "south" => QuickDirection::South,
            "east" => QuickDirection::East,
            "west" => QuickDirection::West,
            other => return Err(CommandError(format!("unknown direction: {other}"))),
        })),

        "quick_cycle" => Ok(IpcCommand::QuickCycle {
            next: one_arg(rest, usage)? == "next",
        }),
        _ => Err(CommandError(format!("unknown command: {name}"))),
    }
}


/// Whether completing `name` should append a space and continue into args.
pub fn needs_argument(name: &str) -> bool {
    COMMANDS
        .iter()
        .find(|spec| spec.name == name)
        .is_some_and(CommandSpec::needs_argument)
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
            } else if targets.len() == 1 && is_selected_target(&targets[0]) {
                Ok(IpcCommand::PlaylistAddSelected { playlist })
            } else if targets.len() == 1 && is_queue_target(&targets[0]) {
                Ok(IpcCommand::PlaylistAddQueue { playlist })
            } else if targets.iter().any(|target| is_selected_target(target)) {
                // `selected` names the UI's live selection, so it cannot be
                // mixed with free-form targets (those resolve in the daemon).
                Err(CommandError(
                    "usage: playlist add selected; <playlist>".to_string(),
                ))
            } else if targets.iter().any(|target| is_queue_target(target)) {
                // `queue` names the whole playback queue, not a search phrase.
                Err(CommandError(
                    "usage: playlist add queue; <playlist>".to_string(),
                ))
            } else {
                Ok(IpcCommand::PlaylistAdd { targets, playlist })
            }
        }
        _ => Err(CommandError("usage: playlist <create|add|rm> ...".to_string())),
    }
}

/// The `selected` target keyword of `playlist add selected; <playlist>`.
fn is_selected_target(value: &str) -> bool {
    value.eq_ignore_ascii_case("selected")
}

/// The `queue` target keyword of `playlist add queue; <playlist>`.
fn is_queue_target(value: &str) -> bool {
    value.eq_ignore_ascii_case("queue")
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
    for spec in COMMANDS {
        match &spec.form {
            CommandForm::Bare(value) => {
                if command == value {
                    return spec.name.to_string();
                }
            }
            CommandForm::Word { values, .. } | CommandForm::Exact(values) => {
                for (label, value) in values.iter() {
                    if command == value {
                        return if label.is_empty() {
                            spec.name.to_string()
                        } else {
                            format!("{} {label}", spec.name)
                        };
                    }
                }
            }
            CommandForm::Advanced(_) => {}
        }
    }
    advanced_canonical(command)
}

/// The canonical printer for the [`CommandForm::Advanced`] commands. Their
/// grammar carries payloads (ids, numbers, quoted targets), so the printer
/// stays explicit next to the parser; the registry covers everything else.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "open printer dispatch over the 83-variant IpcCommand; the empty fallback covers the commands the registry already formats"
)]
fn advanced_canonical(command: &IpcCommand) -> String {
    use IpcCommand::*;
    match command {

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
        PlaylistAddSelected { playlist } => {
            format!("playlist add selected; {}", quote_arg(playlist))
        }
        PlaylistAddQueue { playlist } => {
            format!("playlist add queue; {}", quote_arg(playlist))
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

        Search { query } => format!("search {query}").trim_end().to_string(),

        InsertMode { query } => format!("insert_mode {query}").trim_end().to_string(),

        SetVolume(volume) => format!("set_volume {volume:.0}"),

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
        _ => String::new(),
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
    let args: Vec<&'static str> = match &spec.form {
        CommandForm::Bare(_) | CommandForm::Advanced(ArgHints::None) => return Vec::new(),
        CommandForm::Word { values, .. } | CommandForm::Exact(values) => values
            .iter()
            .filter(|(label, _)| !label.is_empty())
            .map(|(label, _)| *label)
            .collect(),
        CommandForm::Advanced(ArgHints::Values(values)) => values.to_vec(),
        CommandForm::Advanced(ArgHints::Label(label)) => {
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
///
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
    if !playlist_only
        && before.get(1).copied() == Some("add")
        && "selected".starts_with(&prefix_lower)
    {
        out.push(Suggestion {
            start,
            end,
            label: "selected".to_string(),
            description: "Add the visual selection (or the cursor row)".to_string(),
            completion: Some("selected".to_string()),
        });
    }
    if !playlist_only
        && before.get(1).copied() == Some("add")
        && "queue".starts_with(&prefix_lower)
    {
        out.push(Suggestion {
            start,
            end,
            label: "queue".to_string(),
            description: "Add the current playback queue".to_string(),
            completion: Some("queue".to_string()),
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
        assert_eq!(parse("track next"), Ok(IpcCommand::Track(TrackCommand::Next)));
        assert_eq!(parse("track previous"), Ok(IpcCommand::Track(TrackCommand::Previous)));
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
    fn playlist_add_suggests_the_selection_target() {
        let context = SuggestionContext::default();
        let all = suggestions_with("playlist add ", "playlist add ".len(), &context);
        assert!(
            all.iter()
                .any(|s| s.label == "selected" && s.completion.as_deref() == Some("selected"))
        );
        assert!(
            all.iter()
                .any(|s| s.label == "queue" && s.completion.as_deref() == Some("queue"))
        );
        let prefixed = suggestions_with("playlist add sel", "playlist add sel".len(), &context);
        assert_eq!(prefixed.len(), 1);
        assert_eq!(prefixed[0].completion.as_deref(), Some("selected"));
        let queue_prefixed = suggestions_with("playlist add que", "playlist add que".len(), &context);
        assert_eq!(queue_prefixed.len(), 1);
        assert_eq!(queue_prefixed[0].completion.as_deref(), Some("queue"));
        // `playlist rm` has no selected/queue target form.
        let rm = suggestions_with("playlist rm ", "playlist rm ".len(), &context);
        assert!(!rm.iter().any(|s| s.label == "selected"));
        assert!(!rm.iter().any(|s| s.label == "queue"));
        // After the cutoff only playlist names are offered.
        let input = "playlist add selected; ";
        let after = suggestions_with(input, input.len(), &context);
        assert!(!after.iter().any(|s| s.label == "selected"));
        assert!(!after.iter().any(|s| s.label == "queue"));
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
        // `selected` names the UI's live selection, not a resolved target.
        assert_eq!(
            parse("playlist add selected; \"Road Trip\""),
            Ok(IpcCommand::PlaylistAddSelected {
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("playlist add SELECTED; Road Trip"),
            Ok(IpcCommand::PlaylistAddSelected {
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("playlist add selected \"Road Trip\""),
            Ok(IpcCommand::PlaylistAddSelected {
                playlist: "Road Trip".into()
            }),
            "the legacy last-token form accepts selected too"
        );
        assert!(
            parse("playlist add \"Pink Floyd\" selected; Road Trip").is_err(),
            "selected cannot be mixed with free-form targets"
        );
        assert!(parse("playlist add selected;").is_err());
        assert_eq!(
            canonical_string(&parse("playlist add selected; \"Road Trip\"").unwrap()),
            "playlist add selected; \"Road Trip\""
        );
        // `queue` saves the whole playback queue, in queue order.
        assert_eq!(
            parse("playlist add queue; \"Road Trip\""),
            Ok(IpcCommand::PlaylistAddQueue {
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("playlist add QUEUE; Road Trip"),
            Ok(IpcCommand::PlaylistAddQueue {
                playlist: "Road Trip".into()
            })
        );
        assert_eq!(
            parse("playlist add queue \"Road Trip\""),
            Ok(IpcCommand::PlaylistAddQueue {
                playlist: "Road Trip".into()
            }),
            "the legacy last-token form accepts queue too"
        );
        assert!(
            parse("playlist add queue \"Pink Floyd\"; Road Trip").is_err(),
            "queue cannot be mixed with free-form targets"
        );
        assert!(
            parse("playlist add selected queue; Road Trip").is_err(),
            "queue cannot be mixed with selected either"
        );
        assert!(parse("playlist add queue;").is_err());
        assert_eq!(
            canonical_string(&parse("playlist add queue; \"Road Trip\"").unwrap()),
            "playlist add queue; \"Road Trip\""
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
            "playlist add selected; \"Road Trip\"",
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
            "crossfade",
            "crossfade on",
            "crossfade off",
            "normalize",
            "normalize off",
            "normalize track",
            "normalize album",
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
            "track next",
            "track previous",
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
    fn crossfade_and_normalize_commands_are_core_playback() {
        assert_eq!(
            parse("crossfade").unwrap(),
            IpcCommand::Crossfade(CrossfadeCommand::Toggle)
        );
        assert_eq!(
            parse("crossfade off").unwrap(),
            IpcCommand::Crossfade(CrossfadeCommand::Off)
        );
        assert_eq!(
            canonical_string(&parse("crossfade on").unwrap()),
            "crossfade on"
        );
        assert_eq!(
            parse("normalize").unwrap(),
            IpcCommand::Normalize(NormalizeCommand::Cycle)
        );
        assert_eq!(
            parse("normalize album").unwrap(),
            IpcCommand::Normalize(NormalizeCommand::Album)
        );
        assert_eq!(
            canonical_string(&parse("normalize track").unwrap()),
            "normalize track"
        );
        for sample in ["crossfade", "normalize album"] {
            assert_eq!(
                parse(sample).unwrap().target(),
                CommandTarget::CorePlayback,
                "{sample}"
            );
        }
        assert!(parse("normalize loud").is_err());
        assert!(parse("crossfade maybe").is_err());
    }

    #[test]
    fn parses_aliases_and_mpris_forms() {
        let samples = [
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
        assert_eq!(canonical_string(&parse("track next").unwrap()), "track next");
    }
}
