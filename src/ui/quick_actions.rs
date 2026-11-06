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
//! This mirrors two QML files without any Qt/GPUI dependency:
//!
//! - `src/qml/QuickActions.qml`: the verb tables (`song`, `album`, `artist`,
//!   `playlist`, `forSelection`) and the open quadrants.
//! - `src/qml/AltActionsOverlay.qml`: the gesture state machine (`aim`,
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
//! The QML 180 ms rating debounce is a host timer. This model only carries the
//! flags that timer needs: [`AltGesture::rating_dirty`] (a level is not on the
//! server yet) and [`AltGesture::rating_touched`] (a step happened during this
//! hold, so the release must not also toggle the favourite underneath it).

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
}

impl QuickVerb {
    /// The QML tile label for this verb (favourite/download depend on `on`).
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
        }
    }

    /// The bare SVG file name under `src/icons/` (QML `qrc:/app/icons/<name>`).
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
    /// QML `typeof slot.rate === "function"`: `Some(Album)`/`Some(Artist)`
    /// makes Shift-H/L step the rating instead of cycling verbs. This extra
    /// flag is needed because `rating == 0` is a real "unrated" value and
    /// cannot mean "no rating support".
    pub rating_target: Option<QuickKind>,
}

impl QuickSlot {
    /// True when Shift-H/L over this tile steps the rating (QML `rate`).
    pub fn is_rating_slot(&self) -> bool {
        self.rating_target.is_some()
    }

    /// The action at `index`, wrapped like QML `activeSlot` (negative indexes
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

/// The four tiles armed around the current item. `None` is QML's open slot
/// (drawn as "—" and never aimable).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuickMenu {
    pub north: Option<QuickSlot>,
    pub south: Option<QuickSlot>,
    pub west: Option<QuickSlot>,
    pub east: Option<QuickSlot>,
}

impl QuickMenu {
    /// Track row (QML `QuickActions.song`). The heart starts unstarred; use
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

    /// Album row/card (QML `QuickActions.album`). The heart starts unstarred
    /// and unrated; use [`QuickMenu::album_with`] for the row's real state.
    pub fn album(cached: bool) -> Self {
        Self::album_with(cached, false, 0)
    }

    /// Album row/card with its favourite and rating state.
    ///
    /// `cached` is `Backend.cachedAlbumIds`' answer: an album already holding
    /// its content gets the container's "Remove Download" verb. Go to Album is
    /// free because Enter already opens the album page, so the east quadrant
    /// carries the download toggle instead (QML).
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

    /// Artist hit (QML `QuickActions.artist`). The artist page is the row's
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

    /// Playlist row/page (QML `QuickActions.playlist`). Playlists have no
    /// server star/rating in the Subsonic API, so only Play and Add to Queue
    /// are offered and both jumps are open.
    pub fn playlist() -> Self {
        Self {
            north: Some(single_slot(QuickVerb::PlayPlaylist)),
            south: Some(single_slot(QuickVerb::AddToQueue)),
            west: None,
            east: None,
        }
    }

    /// Visual selection range (QML `QuickActions.forSelection`). `kind` is the
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
            // QML always labels this "Download" and leaves the per-row toggle
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

    /// QML `isOpen`: an open quadrant has no tile and cannot be aimed.
    pub fn is_open(&self, direction: Direction) -> bool {
        self.slot(direction).is_none()
    }

    /// QML `isDisabled`: a disabled tile is drawn gray and cannot be aimed.
    /// (The QML check also looks at the active multi-action entry; this model's
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
    /// The level is not on the server yet (the host's idle timer/release
    /// flushes it once).
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

    /// QML `arm(item, cfg)`: reset the gesture and seed the rating from the
    /// favourite slot. `sticky` is preserved (construct the gesture with
    /// [`AltGesture::with_sticky`]) because it is board configuration rather
    /// than per-arm state.
    ///
    /// Returns `false` while the hold is dismissed, mirroring QML's
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

    /// QML `aim(direction)`: h/j/k/l (or arrows).
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

    /// Shift-H/L over the aimed tile (QML `cycle(delta)`).
    ///
    /// A rating slot (the album/artist favourite) steps its level by `delta`,
    /// clamped to 0..=5; a slot with more than one action rotates its active
    /// verb; a single-action slot is a no-op. Returns `true` when something
    /// changed (QML bounces the aimed tile then).
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

        // QML checks the base slot's `rate` function before anything else;
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
    /// QML `confirm()` semantics:
    /// - a rating step on the south tile owns the release: return the pending
    ///   `RateAlbum`/`RateArtist` commit (consuming `rating_dirty`) and do
    ///   **not** also star/unstar — if the level is already clean this is
    ///   `None`;
    /// - otherwise return the aimed tile's active verb;
    /// - a neutral release returns `None` (that is the cancel gesture).
    ///
    /// A rating that is dirty while the aim moved to another quadrant is not
    /// consumed here; the caller flushes it with
    /// [`AltGesture::commit_rating`] when it runs the plan, exactly like QML's
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

    /// Flush the pending rating level (the host's 180 ms idle timer and the
    /// Alt release both call this). Returns the `RateAlbum`/`RateArtist` verb
    /// once and clears `rating_dirty`; a second call returns `None`.
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

    /// QML `cancel()`: leave the board, clear the transient rating and sticky
    /// state. The action indexes are left alone; [`AltGesture::arm`] resets
    /// them on the next hold (as QML does).
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

    /// The Alt release edge: QML clears `dismissed` first, then confirms only a
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
mod tests {
    use super::*;

    fn slot(menu: &QuickMenu, direction: Direction) -> &QuickSlot {
        menu.slot(direction)
            .unwrap_or_else(|| panic!("{direction:?} should have a tile"))
    }

    fn armed(menu: &QuickMenu) -> AltGesture {
        let mut gesture = AltGesture::new();
        assert!(gesture.arm(menu));
        assert!(gesture.armed);
        gesture
    }

    #[test]
    fn direction_opposites_are_involutive() {
        for direction in Direction::ALL {
            assert_ne!(direction.opposite(), direction);
            assert_eq!(direction.opposite().opposite(), direction);
        }
        assert_eq!(Direction::North.opposite(), Direction::South);
        assert_eq!(Direction::South.opposite(), Direction::North);
        assert_eq!(Direction::East.opposite(), Direction::West);
        assert_eq!(Direction::West.opposite(), Direction::East);

        for (index, direction) in Direction::ALL.into_iter().enumerate() {
            assert_eq!(direction.index(), index);
        }
        assert_eq!(
            AltGesture::default(),
            AltGesture::new(),
            "default is a disarmed gesture"
        );
    }

    #[test]
    fn song_quadrants_match_qml() {
        let menu = QuickMenu::song();
        for direction in Direction::ALL {
            assert!(!menu.is_open(direction), "{direction:?} is live on a song");
            assert!(!menu.is_disabled(direction));
        }

        let north = slot(&menu, Direction::North);
        assert_eq!(north.label, "Add to Queue");
        assert_eq!(north.icon, "baseline-playlist-add.svg");
        assert_eq!(
            north.actions,
            vec![QuickVerb::AddToQueue, QuickVerb::StartRadio]
        );
        assert!(!north.disabled);
        assert_eq!(north.rating, 0);
        assert!(!north.favorite);
        assert_eq!(north.rating_target, None);

        let south = slot(&menu, Direction::South);
        assert_eq!(south.label, "Favorite");
        assert_eq!(south.icon, "heart-add-fill.svg");
        assert_eq!(south.actions, vec![QuickVerb::Favorite { on: true }]);
        assert!(south.favorite);
        assert_eq!(south.rating_target, None);

        let west = slot(&menu, Direction::West);
        assert_eq!(west.label, "Go to Artist");
        assert_eq!(west.icon, "person-walking-svgrepo-com.svg");
        assert_eq!(west.actions, vec![QuickVerb::GoToArtist]);

        let east = slot(&menu, Direction::East);
        assert_eq!(east.label, "Go to Album");
        assert_eq!(east.icon, "album.svg");
        assert_eq!(east.actions, vec![QuickVerb::GoToAlbum]);
    }

    #[test]
    fn starred_song_flips_the_heart() {
        let menu = QuickMenu::song_with(true);
        let south = slot(&menu, Direction::South);
        assert_eq!(south.label, "Unfavorite");
        assert_eq!(south.icon, "heart-minus.svg");
        assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
        assert!(!south.favorite);
    }

    #[test]
    fn album_quadrants_and_download_toggle() {
        let uncached = QuickMenu::album(false);
        for direction in Direction::ALL {
            assert!(uncached.slot(direction).is_some());
        }
        assert_eq!(
            slot(&uncached, Direction::North).actions,
            vec![QuickVerb::AddToQueue, QuickVerb::StartRadio]
        );
        let south = slot(&uncached, Direction::South);
        assert_eq!(south.label, "Favorite");
        assert_eq!(south.icon, "heart-add-fill.svg");
        assert!(south.favorite);
        assert_eq!(south.rating, 0);
        assert_eq!(south.rating_target, Some(QuickKind::Album));

        let east = slot(&uncached, Direction::East);
        assert_eq!(east.label, "Download");
        assert_eq!(east.icon, "outline-download.svg");
        assert_eq!(east.actions, vec![QuickVerb::Download { on: true }]);

        let cached = QuickMenu::album(true);
        let east = slot(&cached, Direction::East);
        assert_eq!(east.label, "Remove Download");
        assert_eq!(east.icon, "baseline-download.svg");
        assert_eq!(east.actions, vec![QuickVerb::Download { on: false }]);
    }

    #[test]
    fn album_rating_seed_and_favourite_state() {
        let rated = QuickMenu::album_with(false, true, 4);
        let south = slot(&rated, Direction::South);
        assert_eq!(south.rating, 4);
        assert_eq!(south.rating_target, Some(QuickKind::Album));
        assert_eq!(south.label, "Unfavorite");
        assert_eq!(south.icon, "heart-minus.svg");
        assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
        assert!(!south.favorite);

        // A stale 6+ payload is clamped to the server's 0..=5 range.
        let capped = QuickMenu::album_with(false, false, 9);
        assert_eq!(slot(&capped, Direction::South).rating, 5);
        assert_eq!(
            slot(&capped, Direction::South).rating_target,
            Some(QuickKind::Album)
        );
    }

    #[test]
    fn artist_quadrants_leave_jumps_open() {
        let menu = QuickMenu::artist();
        assert!(menu.is_open(Direction::West));
        assert!(menu.is_open(Direction::East));
        assert!(menu.west.is_none() && menu.east.is_none());

        let north = slot(&menu, Direction::North);
        assert_eq!(north.label, "Add to Queue");
        assert_eq!(north.icon, "baseline-playlist-add.svg");
        assert_eq!(north.actions, vec![QuickVerb::AddToQueue]);

        let south = slot(&menu, Direction::South);
        assert_eq!(south.label, "Favorite");
        assert!(south.favorite);
        assert_eq!(south.rating, 0);
        assert_eq!(south.rating_target, Some(QuickKind::Artist));

        let rated = QuickMenu::artist_with(true, 5);
        let south = slot(&rated, Direction::South);
        assert_eq!(south.label, "Unfavorite");
        assert_eq!(south.rating, 5);
        assert_eq!(south.rating_target, Some(QuickKind::Artist));
        assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
    }

    #[test]
    fn playlist_quadrants_leave_jumps_open() {
        let menu = QuickMenu::playlist();
        assert!(menu.is_open(Direction::West));
        assert!(menu.is_open(Direction::East));
        assert_eq!(
            slot(&menu, Direction::North).actions,
            vec![QuickVerb::PlayPlaylist]
        );
        assert_eq!(slot(&menu, Direction::North).label, "Play Playlist");
        assert_eq!(
            slot(&menu, Direction::North).icon,
            "play-1001-svgrepo-com.svg"
        );
        assert_eq!(
            slot(&menu, Direction::South).actions,
            vec![QuickVerb::AddToQueue]
        );
        assert_eq!(slot(&menu, Direction::South).label, "Add to Queue");
        assert_eq!(slot(&menu, Direction::South).rating_target, None);
    }

    #[test]
    fn selection_quadrants_and_disabled_go_to_artist() {
        let menu = QuickMenu::selection(QuickKind::Song, false);
        for direction in Direction::ALL {
            assert!(!menu.is_open(direction));
        }
        assert_eq!(
            slot(&menu, Direction::North).actions,
            vec![QuickVerb::AddToQueue]
        );

        let south = slot(&menu, Direction::South);
        assert_eq!(south.label, "Favorite");
        assert_eq!(south.icon, "heart-add-fill.svg");
        assert_eq!(south.actions, vec![QuickVerb::Favorite { on: true }]);
        assert!(south.favorite);
        assert_eq!(south.rating_target, None);

        let west = slot(&menu, Direction::West);
        assert!(west.disabled);
        assert!(!menu.is_open(Direction::West));
        assert!(menu.is_disabled(Direction::West));
        assert_eq!(west.label, "Go to Artist");
        assert_eq!(west.icon, "person-walking-svgrepo-com.svg");

        let east = slot(&menu, Direction::East);
        assert_eq!(east.label, "Download");
        assert_eq!(east.icon, "outline-download.svg");
        assert_eq!(east.actions, vec![QuickVerb::Download { on: true }]);

        let all_starred = QuickMenu::selection(QuickKind::Album, true);
        let south = slot(&all_starred, Direction::South);
        assert_eq!(south.label, "Unfavorite");
        assert_eq!(south.icon, "heart-minus.svg");
        assert_eq!(south.actions, vec![QuickVerb::Favorite { on: false }]);
        assert!(!south.favorite);

        // The row kind belongs to the host: the menu itself is identical.
        for kind in [
            QuickKind::Song,
            QuickKind::Album,
            QuickKind::Artist,
            QuickKind::Playlist,
            QuickKind::Selection,
        ] {
            assert_eq!(
                QuickMenu::selection(kind, false),
                QuickMenu::selection(QuickKind::Song, false),
                "{kind:?} selection uses the same verb table"
            );
        }
    }

    #[test]
    fn verb_labels_and_icons_are_qml_file_names() {
        let cases = [
            (
                QuickVerb::AddToQueue,
                "Add to Queue",
                "baseline-playlist-add.svg",
            ),
            (
                QuickVerb::StartRadio,
                "Start Radio",
                "radio-svgrepo-com.svg",
            ),
            (
                QuickVerb::Favorite { on: true },
                "Favorite",
                "heart-add-fill.svg",
            ),
            (
                QuickVerb::Favorite { on: false },
                "Unfavorite",
                "heart-minus.svg",
            ),
            (QuickVerb::RateAlbum { level: 0 }, "Rate Album", "star.svg"),
            (
                QuickVerb::RateArtist { level: 5 },
                "Rate Artist",
                "star.svg",
            ),
            (
                QuickVerb::GoToArtist,
                "Go to Artist",
                "person-walking-svgrepo-com.svg",
            ),
            (QuickVerb::GoToAlbum, "Go to Album", "album.svg"),
            (
                QuickVerb::Download { on: true },
                "Download",
                "outline-download.svg",
            ),
            (
                QuickVerb::Download { on: false },
                "Remove Download",
                "baseline-download.svg",
            ),
            (
                QuickVerb::PlayPlaylist,
                "Play Playlist",
                "play-1001-svgrepo-com.svg",
            ),
        ];
        for (verb, label, icon) in cases {
            assert_eq!(verb.label(), label, "{verb:?} label");
            assert_eq!(verb.icon(), icon, "{verb:?} icon");
            // QML uses qrc:/app/icons/<file>; the model only keeps the file.
            assert!(!verb.icon().contains("qrc:"), "{verb:?} icon is bare");
        }

        assert_eq!(
            QuickVerb::RateAlbum { level: 2 }.rating_target(),
            Some(QuickKind::Album)
        );
        assert_eq!(
            QuickVerb::RateArtist { level: 2 }.rating_target(),
            Some(QuickKind::Artist)
        );
        assert_eq!(QuickVerb::Favorite { on: true }.rating_target(), None);
    }

    #[test]
    fn slot_helpers_wrap_and_fall_back_to_the_tile_label() {
        let menu = QuickMenu::song();
        let north = slot(&menu, Direction::North);
        assert_eq!(north.active_verb(0), Some(QuickVerb::AddToQueue));
        assert_eq!(north.active_verb(1), Some(QuickVerb::StartRadio));
        assert_eq!(north.active_verb(2), Some(QuickVerb::AddToQueue));
        assert_eq!(north.active_label(1), "Start Radio");
        assert_eq!(north.active_icon(1), "radio-svgrepo-com.svg");
        assert!(!north.is_rating_slot());

        let empty = QuickSlot {
            label: "—".to_string(),
            icon: String::new(),
            actions: Vec::new(),
            ..QuickSlot::default()
        };
        assert_eq!(empty.active_verb(0), None);
        assert_eq!(empty.active_label(0), "—");
        assert_eq!(empty.active_icon(0), "");

        let album = QuickMenu::album(false);
        assert!(slot(&album, Direction::South).is_rating_slot());
    }

    #[test]
    fn menu_slot_accessors_and_overrides() {
        let mut menu = QuickMenu::playlist();
        assert!(menu.is_open(Direction::West));
        assert!(!menu.is_disabled(Direction::West));
        assert!(menu.slot_mut(Direction::North).is_some());
        menu.slot_mut(Direction::North).unwrap().disabled = true;
        assert!(menu.is_disabled(Direction::North));

        menu.set_slot(Direction::North, None);
        assert!(menu.is_open(Direction::North));
        assert!(menu.north.is_none());

        let mut menu = QuickMenu::default();
        let north = single_slot(QuickVerb::PlayPlaylist);
        menu.set_slot(Direction::North, Some(north.clone()));
        assert_eq!(menu.slot(Direction::North), Some(&north));
    }

    #[test]
    fn opposite_aim_passes_through_neutral() {
        let menu = QuickMenu::song();
        let mut gesture = armed(&menu);
        assert_eq!(gesture.aimed, None);

        assert!(gesture.aim(Direction::North, &menu, false));
        assert_eq!(gesture.aimed, Some(Direction::North));

        // North + j = neutral; the next j continues to south.
        assert!(!gesture.aim(Direction::South, &menu, false));
        assert_eq!(gesture.aimed, None);
        assert!(gesture.aim(Direction::South, &menu, false));
        assert_eq!(gesture.aimed, Some(Direction::South));

        // Re-aiming the same quadrant stays there.
        assert!(gesture.aim(Direction::South, &menu, false));
        assert_eq!(gesture.aimed, Some(Direction::South));

        for (from, to) in [
            (Direction::North, Direction::South),
            (Direction::South, Direction::North),
            (Direction::East, Direction::West),
            (Direction::West, Direction::East),
        ] {
            let mut gesture = armed(&menu);
            assert!(gesture.aim(from, &menu, false));
            assert!(!gesture.aim(to, &menu, false), "{from:?} -> {to:?}");
            assert_eq!(gesture.aimed, None);
        }
    }

    #[test]
    fn open_slots_cannot_be_aimed() {
        let menu = QuickMenu::artist();
        let mut gesture = armed(&menu);

        assert!(gesture.aim(Direction::North, &menu, false));
        assert!(!gesture.aim(Direction::West, &menu, false));
        assert_eq!(gesture.aimed, None, "open west lands on neutral");

        assert!(!gesture.aim(Direction::East, &menu, false));
        assert_eq!(gesture.aimed, None, "open east lands on neutral");

        // Open south is still a live tile on an artist.
        assert!(gesture.aim(Direction::South, &menu, false));
        assert_eq!(gesture.aimed, Some(Direction::South));

        let playlist = QuickMenu::playlist();
        let mut gesture = armed(&playlist);
        assert!(!gesture.aim(Direction::West, &playlist, false));
        assert!(!gesture.aim(Direction::East, &playlist, false));
        assert_eq!(gesture.aimed, None);
    }

    #[test]
    fn disabled_slots_cannot_be_aimed() {
        let menu = QuickMenu::selection(QuickKind::Song, false);
        let mut gesture = armed(&menu);

        assert!(gesture.aim(Direction::North, &menu, false));
        assert!(!gesture.aim(Direction::West, &menu, false));
        assert_eq!(gesture.aimed, None, "disabled west lands on neutral");

        assert!(!gesture.aim(Direction::West, &menu, false));
        assert_eq!(gesture.aimed, None);

        // A host veto disables an otherwise live direction for this press.
        assert!(!gesture.aim(Direction::South, &menu, true));
        assert_eq!(gesture.aimed, None);
        assert!(gesture.aim(Direction::South, &menu, false));
        assert_eq!(gesture.aimed, Some(Direction::South));
    }

    #[test]
    fn aim_and_cycle_are_ignored_before_arm_or_after_cancel() {
        let menu = QuickMenu::song();
        let mut gesture = AltGesture::new();
        assert!(!gesture.aim(Direction::North, &menu, false));
        assert!(!gesture.cycle(1, &menu));
        assert_eq!(gesture.confirm_plan(&menu), None);
        assert_eq!(gesture.aimed, None);

        assert!(gesture.arm(&menu));
        assert!(gesture.aim(Direction::North, &menu, false));
        assert!(gesture.cycle(1, &menu));
        gesture.cancel();
        assert!(!gesture.armed);
        assert_eq!(gesture.aimed, None);
        assert!(!gesture.aim(Direction::North, &menu, false));
        assert!(!gesture.cycle(1, &menu));
        assert_eq!(gesture.confirm_plan(&menu), None);
    }

    #[test]
    fn cycle_wraps_multi_action_slots() {
        let menu = QuickMenu::song();
        let mut gesture = armed(&menu);
        assert!(gesture.aim(Direction::North, &menu, false));

        assert_eq!(gesture.active_label(&menu), Some("Add to Queue"));
        assert_eq!(
            gesture.active_icon(&menu),
            Some("baseline-playlist-add.svg")
        );

        assert!(gesture.cycle(1, &menu));
        assert_eq!(gesture.action_index[Direction::North.index()], 1);
        assert_eq!(gesture.active_verb(&menu), Some(QuickVerb::StartRadio));
        assert_eq!(gesture.active_label(&menu), Some("Start Radio"));
        assert_eq!(gesture.active_icon(&menu), Some("radio-svgrepo-com.svg"));

        assert!(gesture.cycle(1, &menu));
        assert_eq!(gesture.action_index[Direction::North.index()], 0);
        assert_eq!(gesture.active_verb(&menu), Some(QuickVerb::AddToQueue));

        // Shift-L at the first entry wraps backwards to the last.
        assert!(gesture.cycle(-1, &menu));
        assert_eq!(gesture.action_index[Direction::North.index()], 1);

        // Larger deltas wrap through rem_euclid, never panic.
        assert!(gesture.cycle(5, &menu));
        assert_eq!(gesture.action_index[Direction::North.index()], 0);
        assert!(gesture.cycle(-5, &menu));
        assert_eq!(gesture.action_index[Direction::North.index()], 1);

        // A zero delta still counts as a handled press.
        assert!(gesture.cycle(0, &menu));
        assert_eq!(gesture.action_index[Direction::North.index()], 1);

        // The per-quadrant indexes are independent.
        assert_eq!(gesture.action_index[Direction::South.index()], 0);
        assert_eq!(gesture.action_index[Direction::East.index()], 0);
        assert_eq!(gesture.action_index[Direction::West.index()], 0);
    }

    #[test]
    fn cycle_is_a_noop_for_single_action_and_unaimed_slots() {
        let menu = QuickMenu::artist();
        let mut gesture = armed(&menu);
        assert!(!gesture.cycle(1, &menu), "neutral cannot cycle");
        assert!(gesture.aim(Direction::North, &menu, false));
        let before = gesture.action_index;
        assert!(!gesture.cycle(1, &menu));
        assert_eq!(gesture.action_index, before);

        let playlist = QuickMenu::playlist();
        let mut gesture = armed(&playlist);
        assert!(gesture.aim(Direction::North, &playlist, false));
        assert!(!gesture.cycle(1, &playlist));
        assert_eq!(gesture.action_index, [0; 4]);

        // A song's favourite is a single verb and has no rating.
        let song = QuickMenu::song();
        let mut gesture = armed(&song);
        assert!(gesture.aim(Direction::South, &song, false));
        assert!(!gesture.cycle(1, &song));
        assert_eq!(gesture.rating_value, 0);
        assert!(!gesture.rating_touched);
        assert!(!gesture.rating_dirty);
    }

    #[test]
    fn rating_steps_clamp_zero_to_five() {
        let menu = QuickMenu::album_with(false, false, 4);
        let mut gesture = armed(&menu);
        assert_eq!(gesture.rating_value, 4, "arm seeds the row's rating");
        assert!(gesture.aim(Direction::South, &menu, false));

        assert!(gesture.cycle(1, &menu));
        assert_eq!(gesture.rating_value, 5);
        assert!(gesture.rating_touched);
        assert!(gesture.rating_dirty);

        // Clamped at the top, but the step still marks a commit.
        assert!(gesture.cycle(1, &menu));
        assert_eq!(gesture.rating_value, 5);
        assert!(gesture.rating_touched && gesture.rating_dirty);

        assert!(gesture.cycle(-1, &menu));
        assert_eq!(gesture.rating_value, 4);
        for _ in 0..7 {
            assert!(gesture.cycle(-1, &menu));
        }
        assert_eq!(gesture.rating_value, 0);

        assert!(gesture.cycle(-1, &menu));
        assert_eq!(gesture.rating_value, 0);
        assert!(gesture.cycle(0, &menu));
        assert_eq!(gesture.rating_value, 0);
        assert!(gesture.rating_dirty);

        // The seed is clamped too.
        let capped = QuickMenu::album_with(false, false, 9);
        assert_eq!(armed(&capped).rating_value, 5);
    }

    #[test]
    fn rating_steps_only_on_the_south_favourite_slot() {
        let menu = QuickMenu::artist_with(false, 2);
        let mut gesture = armed(&menu);
        assert_eq!(gesture.rating_value, 2);

        assert!(gesture.aim(Direction::North, &menu, false));
        assert!(!gesture.cycle(1, &menu), "north is a single action");
        assert_eq!(gesture.rating_value, 2);
        assert!(!gesture.rating_touched);
        assert!(!gesture.rating_dirty);

        // South is opposite north: neutral first, then south.
        assert!(!gesture.aim(Direction::South, &menu, false));
        assert!(gesture.aim(Direction::South, &menu, false));
        assert!(gesture.cycle(1, &menu));
        assert_eq!(gesture.rating_value, 3);
        assert!(gesture.rating_touched);
    }

    #[test]
    fn rating_commit_consumes_the_dirty_flag() {
        let menu = QuickMenu::album_with(false, false, 1);
        let mut gesture = armed(&menu);
        assert_eq!(gesture.commit_rating(&menu), None, "nothing dirty yet");

        assert!(gesture.aim(Direction::South, &menu, false));
        assert!(gesture.cycle(1, &menu));
        assert_eq!(
            gesture.commit_rating(&menu),
            Some(QuickVerb::RateAlbum { level: 2 })
        );
        assert!(!gesture.rating_dirty, "commit clears the flag");
        assert!(gesture.rating_touched, "the hold still remembers the step");
        assert_eq!(gesture.commit_rating(&menu), None, "committed once");
        assert_eq!(
            gesture.confirm_plan(&menu),
            None,
            "a touched south slot does not also toggle the favourite"
        );

        let artist = QuickMenu::artist_with(false, 3);
        let mut gesture = armed(&artist);
        assert!(gesture.aim(Direction::South, &artist, false));
        assert!(gesture.cycle(1, &artist));
        assert_eq!(
            gesture.commit_rating(&artist),
            Some(QuickVerb::RateArtist { level: 4 })
        );

        // A surface without a rating slot never emits a rate verb.
        let song = QuickMenu::song();
        let mut gesture = armed(&song);
        gesture.rating_dirty = true;
        gesture.rating_value = 4;
        assert_eq!(gesture.commit_rating(&song), None);
        assert!(gesture.rating_dirty);
    }

    #[test]
    fn confirm_plan_returns_the_aimed_verb() {
        let menu = QuickMenu::song();
        let mut gesture = armed(&menu);

        assert_eq!(gesture.confirm_plan(&menu), None, "neutral cancels");

        assert!(gesture.aim(Direction::South, &menu, false));
        assert_eq!(
            gesture.confirm_plan(&menu),
            Some(QuickVerb::Favorite { on: true })
        );

        gesture.cancel();
        let mut gesture = armed(&menu);
        assert!(gesture.aim(Direction::North, &menu, false));
        assert!(gesture.cycle(1, &menu));
        assert_eq!(
            gesture.confirm_plan(&menu),
            Some(QuickVerb::StartRadio),
            "confirm sees the cycled action"
        );

        gesture.cancel();
        assert!(!gesture.armed);
        assert_eq!(gesture.confirm_plan(&menu), None);
    }

    #[test]
    fn rating_step_owns_the_favourite_release() {
        let menu = QuickMenu::album_with(false, false, 2);
        let mut gesture = armed(&menu);
        assert!(gesture.aim(Direction::South, &menu, false));
        assert!(gesture.cycle(1, &menu));

        assert_eq!(
            gesture.confirm_plan(&menu),
            Some(QuickVerb::RateAlbum { level: 3 }),
            "the release invokes the rating, not the heart"
        );
        assert!(!gesture.rating_dirty);
        assert_eq!(
            gesture.confirm_plan(&menu),
            None,
            "a clean rating step skips the favourite"
        );
    }

    #[test]
    fn pending_rating_commits_even_when_aiming_away() {
        let menu = QuickMenu::album_with(false, false, 1);
        let mut gesture = armed(&menu);
        assert!(gesture.aim(Direction::South, &menu, false));
        assert!(gesture.cycle(1, &menu));
        assert_eq!(gesture.rating_value, 2);

        // South -> north is the opposite pass-through.
        assert!(!gesture.aim(Direction::North, &menu, false));
        assert!(gesture.aim(Direction::North, &menu, false));
        assert_eq!(gesture.aimed, Some(Direction::North));

        assert_eq!(
            gesture.confirm_plan(&menu),
            Some(QuickVerb::AddToQueue),
            "north keeps its own verb"
        );
        assert!(gesture.rating_dirty, "the rating is still pending");
        assert_eq!(
            gesture.commit_rating(&menu),
            Some(QuickVerb::RateAlbum { level: 2 }),
            "the caller flushes the rating alongside the aimed verb"
        );
        assert!(!gesture.rating_dirty);
    }

    #[test]
    fn cancel_and_rearm_reset_the_transient_state() {
        let menu = QuickMenu::album_with(false, false, 3);
        let mut gesture = AltGesture::with_sticky(true);
        assert!(gesture.arm(&menu));
        assert!(gesture.aim(Direction::South, &menu, false));
        assert!(gesture.cycle(1, &menu));
        assert_eq!(gesture.action_index, [0; 4]);

        gesture.cancel();
        assert!(!gesture.armed);
        assert_eq!(gesture.aimed, None);
        assert!(!gesture.rating_touched);
        assert!(!gesture.rating_dirty);
        assert!(!gesture.sticky);
        assert!(!gesture.dismissed);

        // Re-arming reseeds the rating and every action index.
        assert!(gesture.arm(&menu));
        assert_eq!(gesture.rating_value, 3);
        assert_eq!(gesture.action_index, [0; 4]);
    }

    #[test]
    fn sticky_boards_survive_alt_release() {
        let menu = QuickMenu::song();
        let mut gesture = AltGesture::with_sticky(true);
        assert!(gesture.arm(&menu));
        assert!(gesture.aim(Direction::South, &menu, false));

        assert!(
            !gesture.release_hold(),
            "sticky menus fire on Enter, not on the Alt release"
        );
        assert!(gesture.armed);
        assert_eq!(gesture.aimed, Some(Direction::South));
        assert!(!gesture.dismissed);

        // Alt release clears the dismissed latch even for a sticky board.
        gesture.dismiss();
        assert!(gesture.dismissed);
        assert!(!gesture.armed);
        assert!(!gesture.release_hold());
        assert!(!gesture.dismissed);

        // Enter on a sticky board confirms explicitly.
        gesture.sticky = true;
        assert!(gesture.arm(&menu));
        assert!(gesture.aim(Direction::South, &menu, false));
        assert_eq!(
            gesture.confirm_plan(&menu),
            Some(QuickVerb::Favorite { on: true })
        );
        gesture.cancel();
        assert!(!gesture.armed);
    }

    #[test]
    fn non_sticky_release_asks_the_host_to_confirm() {
        let menu = QuickMenu::song();
        let mut gesture = armed(&menu);
        assert!(gesture.aim(Direction::North, &menu, false));

        assert!(gesture.release_hold());
        assert!(
            gesture.armed,
            "release_hold leaves confirm/cancel to the host"
        );
        assert_eq!(gesture.confirm_plan(&menu), Some(QuickVerb::AddToQueue));
        gesture.cancel();
        assert!(!gesture.armed);
    }

    #[test]
    fn dismiss_retires_the_hold_until_release() {
        let menu = QuickMenu::song();
        let mut gesture = armed(&menu);
        assert!(gesture.aim(Direction::North, &menu, false));

        gesture.dismiss();
        assert!(gesture.dismissed);
        assert!(!gesture.armed);
        assert_eq!(gesture.aimed, None);

        assert!(!gesture.arm(&menu), "a dismissed hold cannot re-arm");
        assert!(!gesture.release_hold(), "nothing to confirm after Esc");
        assert!(!gesture.dismissed, "the Alt release clears the latch");
        assert!(gesture.arm(&menu), "the next hold may arm again");

        // A new hold that is dismissed can be re-armed after a release.
        gesture.dismiss();
        assert!(!gesture.arm(&menu));
        gesture.clear_dismissed();
        assert!(gesture.arm(&menu));
    }
}
