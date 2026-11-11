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

//! Pure model for the Alt-hold quick actions (ARC-UI-1 / ARC-RS-2 / UI-60).
//!
//! This mirrors two legacy UI files without any Qt/GPUI dependency:
//!
//! - `the retired action tables`: the verb tables (`song`, `album`, `artist`,
//!   `playlist`, `forSelection`) and the open quadrants.
//! - `the retired gesture model`: the gesture state machine (`aim`,
//!   `opposite`, `cycle`/`adjustRating`, `confirm`, `cancel`, `dismiss`).
//!
//! Labels are plain strings and icons are bare file names under `src/icons/`
//! (`baseline-playlist-add.svg`, not `qrc:/app/icons/...`). Target ids, page
//! kinds and backend calls deliberately stay outside this module: a GPUI view
//! resolves a [`QuickVerb`] against the current row/page and dispatches it.
//! Everything here is a deterministic state transition, so the gesture can be
//! unit-tested (or driven by the IPC `quick_aim`/`quick_cycle` hooks) without
//! a window.
//!
//! The host pushes every rating step to the core immediately (the legacy UI
//! used a 180 ms debounce), so the visible star row follows each
//! increase/decrease instead of jumping only when the hold ends. This model
//! still carries the two flags the host needs: [`AltGesture::rating_dirty`] (a
//! level that is not on the server yet) and [`AltGesture::rating_touched`] (a
//! step happened during this hold, so the release must not also toggle the
//! favourite underneath it).

/// One invokable Alt verb.
///
/// The target ids are intentionally not carried here: `AddToQueue` may mean a
/// song, an album, an artist or a selection, and the host supplies the id/kind
/// when it dispatches the verb. `on` is the *state the verb leaves behind*:
/// `Favorite { on: true }` stars, `Favorite { on: false }` unstars;
/// `Download { on: true }` requests the content, `false` removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickVerb {
    AddToQueue,
    StartRadio,
    Favorite { on: bool },
    RateAlbum { level: u8 },
    RateArtist { level: u8 },
    GoToArtist,
    GoToAlbum,
    Download { on: bool },
    PlayPlaylist,
    DeletePlaylist,
    ShuffleArtist,
    ShuffleAlbum,
    SaveRadio,
    ShuffleRadio,
    DownloadRadio,
    WipeLibrary,
    WipeIndexes,
    WipeCovers,
}

impl QuickVerb {
    /// The legacy UI tile label for this verb (favourite/download depend on `on`).
    pub fn label(self) -> &'static str {
        match self {
            QuickVerb::AddToQueue => "Add to Queue",
            QuickVerb::StartRadio => "Start Radio",
            QuickVerb::Favorite { on: true } => "Favorite",
            QuickVerb::Favorite { on: false } => "Unfavorite",
            QuickVerb::RateAlbum { .. } => "Rate Album",
            QuickVerb::RateArtist { .. } => "Rate Artist",
            QuickVerb::GoToArtist => "Go to Artist",
            QuickVerb::GoToAlbum => "Go to Album",
            QuickVerb::Download { on: true } => "Download",
            QuickVerb::Download { on: false } => "Remove Download",
            QuickVerb::PlayPlaylist => "Play Playlist",
            QuickVerb::DeletePlaylist => "Delete Playlist",
            QuickVerb::ShuffleArtist => "Shuffle Artist",
            QuickVerb::ShuffleAlbum => "Shuffle Album",
            QuickVerb::SaveRadio => "Save Radio",
            QuickVerb::ShuffleRadio => "Shuffle",
            QuickVerb::DownloadRadio => "Download Radio",
            QuickVerb::WipeLibrary => "Wipe Library",
            QuickVerb::WipeIndexes => "Wipe Search Index",
            QuickVerb::WipeCovers => "Wipe Covers",
        }
    }

    /// The bare SVG file name under `src/icons/` (legacy UI `qrc:/app/icons/<name>`).
    pub fn icon(self) -> &'static str {
        match self {
            QuickVerb::AddToQueue => "baseline-playlist-add.svg",
            QuickVerb::StartRadio => "radio-svgrepo-com.svg",
            QuickVerb::Favorite { on: true } => "heart-add-fill.svg",
            QuickVerb::Favorite { on: false } => "heart-minus.svg",
            QuickVerb::RateAlbum { .. } | QuickVerb::RateArtist { .. } => "star.svg",
            QuickVerb::GoToArtist => "person-walking-svgrepo-com.svg",
            QuickVerb::GoToAlbum => "album.svg",
            QuickVerb::Download { on: true } => "outline-download.svg",
            QuickVerb::Download { on: false } => "baseline-download.svg",
            QuickVerb::PlayPlaylist => "play-1001-svgrepo-com.svg",
            QuickVerb::DeletePlaylist => "delete-outline.svg",
            QuickVerb::ShuffleArtist | QuickVerb::ShuffleAlbum | QuickVerb::ShuffleRadio => {
                "outline-shuffle.svg"
            }
            QuickVerb::SaveRadio => "save-outline.svg",
            QuickVerb::DownloadRadio => "outline-download.svg",
            QuickVerb::WipeLibrary | QuickVerb::WipeIndexes | QuickVerb::WipeCovers => {
                "delete-outline.svg"
            }
        }
    }

    /// Which artist/album surface this rating verb belongs to, if any.
    pub fn rating_target(self) -> Option<QuickKind> {
        match self {
            QuickVerb::RateAlbum { .. } => Some(QuickKind::Album),
            QuickVerb::RateArtist { .. } => Some(QuickKind::Artist),
            _ => None,
        }
    }
}

/// What a quick menu describes. `Selection` is a multi-row range; the row kind
/// behind it is still one of the other variants and is carried by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickKind {
    Song,
    Album,
    Artist,
    Playlist,
    Selection,
}

/// The four quadrants around the neutral centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    North,
    South,
    East,
    West,
}

impl Direction {
    /// North, South, East, West. The indexes are also the order used by
    /// [`AltGesture::action_index`].
    pub const ALL: [Direction; 4] = [
        Direction::North,
        Direction::South,
        Direction::East,
        Direction::West,
    ];

    /// The quadrant across the neutral centre.
    pub const fn opposite(self) -> Direction {
        match self {
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::East => Direction::West,
            Direction::West => Direction::East,
        }
    }

    /// Stable index into per-quadrant arrays (north=0, south=1, east=2, west=3).
    pub const fn index(self) -> usize {
        match self {
            Direction::North => 0,
            Direction::South => 1,
            Direction::East => 2,
            Direction::West => 3,
        }
    }
}

/// One tile of the board.
///
/// `label`/`icon` are the currently displayed action's strings, seeded from
/// the first entry of `actions`. A multi-action slot cycles with Shift-H/L;
/// the view should render [`QuickSlot::active_label`] /
/// [`QuickSlot::active_icon`] with the gesture's per-direction index rather
/// than reading `label`/`icon` directly after a cycle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuickSlot {
    pub label: String,
    pub icon: String,
    pub disabled: bool,
    pub actions: Vec<QuickVerb>,
    /// Current 0..=5 rating carried by the favourite slot.
    pub rating: u8,
    /// Target state of the favourite slot (`true` = this verb stars it).
    pub favorite: bool,
    /// legacy UI `typeof slot.rate === "function"`: `Some(Album)`/`Some(Artist)`
    /// makes Shift-H/L step the rating instead of cycling verbs. This extra
    /// flag is needed because `rating == 0` is a real "unrated" value and
    /// cannot mean "no rating support".
    pub rating_target: Option<QuickKind>,
}

impl QuickSlot {
    /// True when Shift-H/L over this tile steps the rating (legacy UI `rate`).
    pub fn is_rating_slot(&self) -> bool {
        self.rating_target.is_some()
    }

    /// The action at `index`, wrapped like legacy UI `activeSlot` (negative indexes
    /// are never stored, so a plain modulo is enough).
    pub fn active_verb(&self, index: usize) -> Option<QuickVerb> {
        if self.actions.is_empty() {
            return None;
        }
        self.actions.get(index % self.actions.len()).copied()
    }

    /// Label of the active action, falling back to the tile's own label for a
    /// slot with no actions (an open tile keeps its "—").
    pub fn active_label(&self, index: usize) -> &str {
        match self.active_verb(index) {
            Some(verb) => verb.label(),
            None => &self.label,
        }
    }

    /// Icon of the active action, with the same fallback as
    /// [`QuickSlot::active_label`].
    pub fn active_icon(&self, index: usize) -> &str {
        match self.active_verb(index) {
            Some(verb) => verb.icon(),
            None => &self.icon,
        }
    }
}

/// The four tiles armed around the current item. `None` is legacy UI's open slot
/// (drawn as "—" and never aimable).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuickMenu {
    pub north: Option<QuickSlot>,
    pub south: Option<QuickSlot>,
    pub west: Option<QuickSlot>,
    pub east: Option<QuickSlot>,
}

impl QuickMenu {
    /// Track row (legacy UI `QuickActions.song`). The heart starts unstarred; use
    /// [`QuickMenu::song_with`] for a row that is already favourited.
    pub fn song() -> Self {
        Self::song_with(false)
    }

    /// Track row whose heart reads the row's own `starred` state (ARC-I-4).
    pub fn song_with(starred: bool) -> Self {
        Self {
            // ARC-RS-1: one row seeds radio from its own song; Shift-H/L
            // cycles the two north verbs.
            north: Some(multi_slot(vec![
                QuickVerb::AddToQueue,
                QuickVerb::StartRadio,
            ])),
            south: Some(single_slot(QuickVerb::Favorite { on: !starred })),
            west: Some(single_slot(QuickVerb::GoToArtist)),
            east: Some(single_slot(QuickVerb::GoToAlbum)),
        }
    }

    /// Album row/card (legacy UI `QuickActions.album`). The heart starts unstarred
    /// and unrated; use [`QuickMenu::album_with`] for the row's real state.
    pub fn album(cached: bool) -> Self {
        Self::album_with(cached, false, 0)
    }

    /// Album row/card with its favourite and rating state.
    ///
    /// `cached` is `Backend.cachedAlbumIds`' answer: an album already holding
    /// its content gets the container's "Remove Download" verb. Go to Album is
    /// free because Enter already opens the album page, so the east quadrant
    /// carries the download toggle instead (legacy UI).
    pub fn album_with(cached: bool, starred: bool, rating: u8) -> Self {
        Self {
            north: Some(multi_slot(vec![
                QuickVerb::AddToQueue,
                QuickVerb::StartRadio,
            ])),
            // UI-60/ARC-RS-2: the rating rides the favourite slot.
            south: Some(rated_slot(
                QuickVerb::Favorite { on: !starred },
                rating,
                QuickKind::Album,
            )),
            west: Some(single_slot(QuickVerb::GoToArtist)),
            east: Some(single_slot(QuickVerb::Download { on: !cached })),
        }
    }

    /// Album page Tool menu: the album quick actions plus a Shuffle Album
    /// option in the west slot (Shift-H/L cycles it with Go to Artist).
    pub fn album_tool(cached: bool, starred: bool, rating: u8) -> Self {
        Self {
            north: Some(multi_slot(vec![
                QuickVerb::AddToQueue,
                QuickVerb::StartRadio,
            ])),
            south: Some(rated_slot(
                QuickVerb::Favorite { on: !starred },
                rating,
                QuickKind::Album,
            )),
            west: Some(single_slot(QuickVerb::ShuffleAlbum)),
            east: Some(single_slot(QuickVerb::Download { on: !cached })),
        }
    }

    /// The status-bar trashcan: the maintenance chooser.
    pub fn wipe() -> Self {
        Self {
            north: Some(single_slot(QuickVerb::WipeLibrary)),
            south: Some(single_slot(QuickVerb::WipeIndexes)),
            west: None,
            east: Some(single_slot(QuickVerb::WipeCovers)),
        }
    }

    /// Artist hit (legacy UI `QuickActions.artist`). The artist page is the row's
    /// Enter already, so both jumps stay open ("—").
    pub fn artist() -> Self {
        Self::artist_with(false, 0)
    }

    /// Artist hit with its favourite and rating state.
    pub fn artist_with(starred: bool, rating: u8) -> Self {
        Self {
            north: Some(single_slot(QuickVerb::AddToQueue)),
            south: Some(rated_slot(
                QuickVerb::Favorite { on: !starred },
                rating,
                QuickKind::Artist,
            )),
            west: None,
            east: None,
        }
    }

    /// Artist page Tool menu: the favourite/rating slot plus the container
    /// download toggle (the artist header's separate actions folded into one
    /// discoverable board).
    pub fn artist_tool(cached: bool, starred: bool, rating: u8) -> Self {
        Self {
            north: Some(multi_slot(vec![
                QuickVerb::AddToQueue,
                QuickVerb::StartRadio,
            ])),
            south: Some(rated_slot(
                QuickVerb::Favorite { on: !starred },
                rating,
                QuickKind::Artist,
            )),
            west: Some(single_slot(QuickVerb::ShuffleArtist)),
            east: Some(single_slot(QuickVerb::Download { on: !cached })),
        }
    }

    /// Generated Radio page Tool menu. Radios have no star/rating; all four
    /// quadrants carry the radio container actions.
    pub fn radio_tool() -> Self {
        Self {
            north: Some(single_slot(QuickVerb::AddToQueue)),
            south: Some(single_slot(QuickVerb::SaveRadio)),
            west: Some(single_slot(QuickVerb::ShuffleRadio)),
            east: Some(single_slot(QuickVerb::DownloadRadio)),
        }
    }

    /// Playlist row/page (legacy UI `QuickActions.playlist`). Playlists have no
    /// server star/rating in the Subsonic API, so only Play, Add to Queue and
    /// Delete are offered and the west jump stays open.
    pub fn playlist() -> Self {
        Self {
            north: Some(single_slot(QuickVerb::PlayPlaylist)),
            south: Some(single_slot(QuickVerb::AddToQueue)),
            west: None,
            east: Some(single_slot(QuickVerb::DeletePlaylist)),
        }
    }

    /// Visual selection range (legacy UI `QuickActions.forSelection`). `kind` is the
    /// row kind behind the range; the verb table is identical for every kind
    /// (the host carries the kind/ids alongside the menu). `all_starred`
    /// means every row is already favourited: the heart unstars, otherwise it
    /// stars only the rows that are not (mixed selections converge).
    ///
    /// The go-to verbs are single-row only, so west is a disabled "Go to
    /// Artist"; the free east quadrant carries the selection download.
    pub fn selection(_kind: QuickKind, all_starred: bool) -> Self {
        let mut west = single_slot(QuickVerb::GoToArtist);
        west.disabled = true;
        Self {
            north: Some(single_slot(QuickVerb::AddToQueue)),
            south: Some(single_slot(QuickVerb::Favorite { on: !all_starred })),
            west: Some(west),
            // legacy UI always labels this "Download" and leaves the per-row toggle
            // to `downloadSelection`, so there is no cached flag to encode.
            east: Some(single_slot(QuickVerb::Download { on: true })),
        }
    }

    pub fn slot(&self, direction: Direction) -> Option<&QuickSlot> {
        match direction {
            Direction::North => self.north.as_ref(),
            Direction::South => self.south.as_ref(),
            Direction::East => self.east.as_ref(),
            Direction::West => self.west.as_ref(),
        }
    }

    pub fn slot_mut(&mut self, direction: Direction) -> Option<&mut QuickSlot> {
        match direction {
            Direction::North => self.north.as_mut(),
            Direction::South => self.south.as_mut(),
            Direction::East => self.east.as_mut(),
            Direction::West => self.west.as_mut(),
        }
    }

    pub fn set_slot(&mut self, direction: Direction, slot: Option<QuickSlot>) {
        match direction {
            Direction::North => self.north = slot,
            Direction::South => self.south = slot,
            Direction::East => self.east = slot,
            Direction::West => self.west = slot,
        }
    }

    /// legacy UI `isOpen`: an open quadrant has no tile and cannot be aimed.
    pub fn is_open(&self, direction: Direction) -> bool {
        self.slot(direction).is_none()
    }

    /// legacy UI `isDisabled`: a disabled tile is drawn gray and cannot be aimed.
    /// (The legacy UI check also looks at the active multi-action entry; this model's
    /// verbs carry no individual disabled flag, only the slot does.)
    pub fn is_disabled(&self, direction: Direction) -> bool {
        self.slot(direction).is_some_and(|slot| slot.disabled)
    }
}

fn single_slot(verb: QuickVerb) -> QuickSlot {
    QuickSlot {
        label: verb.label().to_string(),
        icon: verb.icon().to_string(),
        disabled: false,
        actions: vec![verb],
        rating: 0,
        favorite: matches!(verb, QuickVerb::Favorite { on: true }),
        rating_target: None,
    }
}

fn multi_slot(actions: Vec<QuickVerb>) -> QuickSlot {
    let first = actions.first().copied();
    QuickSlot {
        label: first.map(QuickVerb::label).unwrap_or_default().to_string(),
        icon: first.map(QuickVerb::icon).unwrap_or_default().to_string(),
        disabled: false,
        actions,
        rating: 0,
        favorite: first.is_some_and(|verb| matches!(verb, QuickVerb::Favorite { on: true })),
        rating_target: None,
    }
}

fn rated_slot(verb: QuickVerb, rating: u8, target: QuickKind) -> QuickSlot {
    QuickSlot {
        label: verb.label().to_string(),
        icon: verb.icon().to_string(),
        disabled: false,
        actions: vec![verb],
        // The server range is 0..=5; a stale payload cannot seed something
        // the stepper could never reach.
        rating: rating.min(5),
        favorite: matches!(verb, QuickVerb::Favorite { on: true }),
        rating_target: Some(target),
    }
}

/// The Alt-hold gesture state. One value lives for one hold (or one sticky
/// board); the host drives it from Alt press/release, h/j/k/l and Shift-H/L.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AltGesture {
    /// The board is up and accepting aim/cycle input.
    pub armed: bool,
    /// The aimed quadrant; `None` is the neutral centre.
    pub aimed: Option<Direction>,
    /// Active action index per quadrant, used by multi-action slots.
    pub action_index: [usize; 4],
    /// Live 0..=5 rating level while the favourite slot is adjusted.
    pub rating_value: u8,
    /// The level is not on the server yet. The host flushes each step right
    /// away and flushes a leftover once more on the release edge.
    pub rating_dirty: bool,
    /// A rating step happened in this hold: the release must not also toggle
    /// the favourite underneath it.
    pub rating_touched: bool,
    /// A mouse-armed menu stays up until Enter fires it or a click outside
    /// cancels; an Alt-hold menu confirms on Alt release.
    pub sticky: bool,
    /// Esc (or IPC `quick_cancel`) retired this hold; arming stays off until
    /// the Alt release clears the latch.
    pub dismissed: bool,
}

impl AltGesture {
    pub const fn new() -> Self {
        Self {
            armed: false,
            aimed: None,
            action_index: [0; 4],
            rating_value: 0,
            rating_dirty: false,
            rating_touched: false,
            sticky: false,
            dismissed: false,
        }
    }

    /// A sticky board (the wipe chooser / a mouse click): Alt release must not
    /// confirm it, Enter does.
    pub const fn with_sticky(sticky: bool) -> Self {
        let mut gesture = Self::new();
        gesture.sticky = sticky;
        gesture
    }

    /// legacy UI `arm(item, cfg)`: reset the gesture and seed the rating from the
    /// favourite slot. `sticky` is preserved (construct the gesture with
    /// [`AltGesture::with_sticky`]) because it is board configuration rather
    /// than per-arm state.
    ///
    /// Returns `false` while the hold is dismissed, mirroring legacy UI's
    /// `if (!available || dismissed || !item || !cfg) return;`.
    pub fn arm(&mut self, menu: &QuickMenu) -> bool {
        if self.dismissed {
            return false;
        }
        self.armed = true;
        self.aimed = None;
        self.action_index = [0; 4];
        self.rating_value = menu
            .south
            .as_ref()
            .filter(|slot| slot.rating_target.is_some())
            .map_or(0, |slot| slot.rating.min(5));
        self.rating_dirty = false;
        self.rating_touched = false;
        true
    }

    /// legacy UI `aim(direction)`: h/j/k/l (or arrows).
    ///
    /// Aiming toward the quadrant opposite the current one first steps back to
    /// neutral (`north + j = neutral`, `j` again = south). A press toward an
    /// open or disabled slot also lands on neutral. `disabled` is the host's
    /// extra veto for this press (a verb that is dynamically unavailable even
    /// though the tile exists); it is additive to the slot's own flag.
    ///
    /// Returns `true` when the gesture ends aimed at `direction`.
    pub fn aim(&mut self, direction: Direction, menu: &QuickMenu, disabled: bool) -> bool {
        if !self.armed {
            return false;
        }
        let mut next = if self.aimed == Some(direction.opposite()) {
            None
        } else {
            Some(direction)
        };
        if let Some(target) = next
            && (disabled || menu.is_open(target) || menu.is_disabled(target))
        {
            next = None;
        }
        self.aimed = next;
        next == Some(direction)
    }

    /// Shift-H/L over the aimed tile (legacy UI `cycle(delta)`).
    ///
    /// A rating slot (the album/artist favourite) steps its level by `delta`,
    /// clamped to 0..=5; a slot with more than one action rotates its active
    /// verb; a single-action slot is a no-op. Returns `true` when something
    /// changed (legacy UI bounces the aimed tile then).
    ///
    /// A rating step only updates the live level here: the host flushes it with
    /// [`AltGesture::commit_rating`] immediately so the star row tracks every
    /// step instead of only the release.
    pub fn cycle(&mut self, delta: i32, menu: &QuickMenu) -> bool {
        if !self.armed {
            return false;
        }
        let Some(direction) = self.aimed else {
            return false;
        };
        let Some(slot) = menu.slot(direction) else {
            return false;
        };

        // legacy UI checks the base slot's `rate` function before anything else;
        // `adjustRating` only accepts the south tile, so a rating flag on any
        // other quadrant would still consume the press without cycling.
        if slot.rating_target.is_some() {
            if direction != Direction::South {
                return false;
            }
            let stepped = (i64::from(self.rating_value) + i64::from(delta)).clamp(0, 5) as u8;
            self.rating_value = stepped;
            // UI-60: a step marks the level for commit even when the clamp
            // left the value unchanged, and the release must not star/unstar.
            self.rating_touched = true;
            self.rating_dirty = true;
            return true;
        }

        if slot.actions.len() < 2 {
            return false;
        }
        let count = slot.actions.len() as i64;
        let current = self.action_index[direction.index()] as i64;
        let next = (current + i64::from(delta)).rem_euclid(count);
        self.action_index[direction.index()] = next as usize;
        true
    }

    /// The aimed slot, if the board is armed and the aim is not neutral.
    pub fn aimed_slot<'a>(&self, menu: &'a QuickMenu) -> Option<&'a QuickSlot> {
        if !self.armed {
            return None;
        }
        self.aimed.and_then(|direction| menu.slot(direction))
    }

    /// The verb the aimed tile would invoke (active action, disabled guarded).
    /// Ratings are committed separately; see [`AltGesture::confirm_plan`].
    pub fn active_verb(&self, menu: &QuickMenu) -> Option<QuickVerb> {
        let direction = self.aimed?;
        let slot = self.aimed_slot(menu)?;
        if slot.disabled {
            return None;
        }
        slot.active_verb(self.action_index[direction.index()])
    }

    /// The label the aimed tile should show, including a multi-action cycle.
    pub fn active_label<'a>(&self, menu: &'a QuickMenu) -> Option<&'a str> {
        let direction = self.aimed?;
        let slot = self.aimed_slot(menu)?;
        Some(slot.active_label(self.action_index[direction.index()]))
    }

    /// The icon the aimed tile should show, including a multi-action cycle.
    pub fn active_icon<'a>(&self, menu: &'a QuickMenu) -> Option<&'a str> {
        let direction = self.aimed?;
        let slot = self.aimed_slot(menu)?;
        Some(slot.active_icon(self.action_index[direction.index()]))
    }

    /// The verb an Alt release (or Enter on a sticky board) should invoke,
    /// without ending the gesture. The host invokes the returned verb, then
    /// calls [`AltGesture::cancel`].
    ///
    /// legacy UI `confirm()` semantics:
    /// - a rating step on the south tile owns the release: return the pending
    ///   `RateAlbum`/`RateArtist` commit (consuming `rating_dirty`) and do
    ///   **not** also star/unstar — if the level is already clean this is
    ///   `None`;
    /// - otherwise return the aimed tile's active verb;
    /// - a neutral release returns `None` (that is the cancel gesture).
    ///
    /// A rating that is dirty while the aim moved to another quadrant is not
    /// consumed here; the caller flushes it with
    /// [`AltGesture::commit_rating`] when it runs the plan, exactly like legacy UI's
    /// `commitRating()` next to `slot.invoke()`.
    pub fn confirm_plan(&mut self, menu: &QuickMenu) -> Option<QuickVerb> {
        if !self.armed {
            return None;
        }
        if self.aimed == Some(Direction::South)
            && self.rating_touched
            && menu
                .south
                .as_ref()
                .is_some_and(|slot| slot.rating_target.is_some())
        {
            return self.commit_rating(menu);
        }
        self.active_verb(menu)
    }

    /// Flush the pending rating level. The host calls this after every rating
    /// step (so the star row follows each increase/decrease) and once more on
    /// the Alt release; a second call returns `None`. Returns the
    /// `RateAlbum`/`RateArtist` verb once and clears `rating_dirty`.
    pub fn commit_rating(&mut self, menu: &QuickMenu) -> Option<QuickVerb> {
        if !self.rating_dirty {
            return None;
        }
        let target = menu.south.as_ref().and_then(|slot| slot.rating_target)?;
        let level = self.rating_value.min(5);
        let verb = match target {
            QuickKind::Album => QuickVerb::RateAlbum { level },
            QuickKind::Artist => QuickVerb::RateArtist { level },
            // Only album/artist surfaces carry a server rating.
            _ => return None,
        };
        self.rating_dirty = false;
        Some(verb)
    }

    /// Drop a pending rating level without emitting it. A clamped increase or
    /// decrease (already 0 or 5) changes nothing, so there is no step to push;
    /// the hold still remembers it so the release does not toggle the
    /// favourite underneath the rating slot.
    pub fn discard_rating(&mut self) {
        self.rating_dirty = false;
    }

    /// legacy UI `cancel()`: leave the board, clear the transient rating and sticky
    /// state. The action indexes are left alone; [`AltGesture::arm`] resets
    /// them on the next hold (as legacy UI does).
    pub fn cancel(&mut self) {
        self.armed = false;
        self.aimed = None;
        self.rating_touched = false;
        self.rating_dirty = false;
        self.sticky = false;
    }

    /// IPC `quick_cancel` / Esc: retire the overlay for the rest of this Alt
    /// hold. Only the Alt release (or [`AltGesture::clear_dismissed`]) re-arms
    /// it.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
        self.cancel();
    }

    /// Clear the dismissed latch without arming (the host may also use
    /// [`AltGesture::release_hold`], which does this on the Alt edge).
    pub fn clear_dismissed(&mut self) {
        self.dismissed = false;
    }

    /// The Alt release edge: legacy UI clears `dismissed` first, then confirms only a
    /// non-sticky board. Returns `true` when the host should confirm now
    /// (`armed && !sticky`); a sticky board stays armed for Enter.
    ///
    /// This does not itself confirm/cancel, so the host can invoke
    /// [`AltGesture::confirm_plan`], flush [`AltGesture::commit_rating`], and
    /// then [`AltGesture::cancel`].
    pub fn release_hold(&mut self) -> bool {
        self.dismissed = false;
        self.armed && !self.sticky
    }
}

impl Default for AltGesture {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
